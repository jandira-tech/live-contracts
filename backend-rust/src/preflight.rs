//! `smoke` (offline) and `preflight` (one live call to each dependency) commands,
//! after the ones in auto-apps-2026 PR #15. Run them before trusting a deploy:
//!   sec-ex10-rust smoke
//!   sec-ex10-rust preflight
use crate::config::Config;
use crate::ingest::PushOutcome;
use crate::outbox::Outbox;
use std::future::Future;

/// A cheap SEC endpoint that needs the declared User-Agent to answer 200.
pub const SEC_PROBE_URL: &str =
    "https://www.sec.gov/cgi-bin/browse-edgar?action=getcurrent&type=8-K&count=10&output=atom";

#[derive(Debug, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Offline: configuration is valid and the outbox opens. No network.
pub fn smoke(cfg: &Config) -> Vec<Check> {
    let config = match cfg.validate() {
        Ok(()) => Check { name: "config", ok: true, detail: format!("{cfg:?}") },
        Err(e) => Check { name: "config", ok: false, detail: e },
    };
    let outbox = match Outbox::open(&cfg.outbox_path).and_then(|o| o.len()) {
        Ok(n) => Check { name: "outbox", ok: true, detail: format!("{} ({n} rows queued)", cfg.outbox_path) },
        Err(e) => Check { name: "outbox", ok: false, detail: format!("{}: {e}", cfg.outbox_path) },
    };
    vec![config, outbox]
}

/// Online: smoke, plus an empty authenticated POST to the ingest route (proves
/// the key) and one SEC request with the configured User-Agent.
pub async fn preflight(client: &reqwest::Client, cfg: &Config, sec_url: &str) -> Vec<Check> {
    let mut checks = smoke(cfg);
    // An empty batch is answered 200 only when the key is right; nothing is written.
    let ingest = match crate::ingest::post_batch_raw(client, &cfg.ingest_url, &cfg.api_key, &[]).await {
        PushOutcome::Accepted(_) => Check { name: "ingest", ok: true, detail: format!("{} accepts the key", cfg.ingest_url) },
        PushOutcome::Retry { reason, .. } | PushOutcome::Rejected { reason, .. } => {
            Check { name: "ingest", ok: false, detail: format!("{}: {reason}", cfg.ingest_url) }
        }
    };
    checks.push(ingest);
    let sec = match client
        .get(sec_url)
        .header(reqwest::header::USER_AGENT, &cfg.user_agent)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => Check { name: "sec", ok: true, detail: format!("{} as {:?}", r.status(), cfg.user_agent) },
        Ok(r) => Check { name: "sec", ok: false, detail: format!("status {} as {:?}", r.status(), cfg.user_agent) },
        Err(e) => Check { name: "sec", ok: false, detail: format!("transport: {e}") },
    };
    checks.push(sec);
    checks
}

/// Run one unit of work so that a panic inside it is logged and counted instead of
/// taking the whole batch (and every other filing in it) down. None on panic.
pub async fn isolate<F, T>(label: &str, fut: F) -> Option<T>
where
    F: Future<Output = T>,
{
    use futures::FutureExt;
    match std::panic::AssertUnwindSafe(fut).catch_unwind().await {
        Ok(v) => Some(v),
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic".into());
            tracing::error!("{label} panicked and was skipped: {msg}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ingest_server;

    fn cfg(url: &str, outbox: &str) -> Config {
        let mut c = Config::from_map(|k| match k {
            "SEC_API_KEY" => Some("k".into()),
            "SEC_USER_AGENT" => Some("Test Co test@arthur.law".into()),
            _ => None,
        });
        c.ingest_url = url.into();
        c.outbox_path = outbox.into();
        c
    }

    #[test]
    fn smoke_passes_on_a_good_config() {
        let checks = smoke(&cfg("http://127.0.0.1:9/api/ingest", ":memory:"));
        assert!(checks.iter().all(|c| c.ok), "{checks:?}");
        assert!(checks.iter().any(|c| c.name == "config"));
        assert!(checks.iter().any(|c| c.name == "outbox"));
    }

    #[test]
    fn smoke_fails_on_a_placeholder_identity_and_never_prints_the_key() {
        let mut c = cfg("http://127.0.0.1:9/api/ingest", ":memory:");
        c.user_agent = "John Smith johnsmith@gmail.com".into();
        c.api_key = "very-secret".into();
        let checks = smoke(&c);
        assert!(!checks.iter().find(|x| x.name == "config").unwrap().ok);
        assert!(checks.iter().all(|x| !x.detail.contains("very-secret")));
    }

    #[tokio::test]
    async fn preflight_proves_key_and_identity() {
        let (url, script) = ingest_server(&[]).await;
        let sec = url.replace("/api/ingest", "/sec");
        let checks = preflight(&reqwest::Client::new(), &cfg(&url, ":memory:"), &sec).await;
        assert!(checks.iter().all(|c| c.ok), "{checks:?}");
        let s = script.lock().unwrap();
        assert_eq!(s.calls, 1, "one empty POST");
        assert_eq!(s.rows_seen, 0);
        assert_eq!(s.user_agents, vec!["Test Co test@arthur.law".to_string()]);
    }

    #[tokio::test]
    async fn preflight_reports_a_rejected_key() {
        let (url, _) = ingest_server(&[401]).await;
        let sec = url.replace("/api/ingest", "/sec");
        let checks = preflight(&reqwest::Client::new(), &cfg(&url, ":memory:"), &sec).await;
        let ingest = checks.iter().find(|c| c.name == "ingest").unwrap();
        assert!(!ingest.ok);
        assert!(ingest.detail.contains("401"), "{}", ingest.detail);
    }

    #[tokio::test]
    async fn isolate_contains_a_panic() {
        assert_eq!(isolate("ok", async { 7 }).await, Some(7));
        let got: Option<u8> = isolate("boom", async { panic!("bad filing") }).await;
        assert_eq!(got, None);
    }
}
