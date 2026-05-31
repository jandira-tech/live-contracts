/// Environment-driven configuration with sensible defaults.
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub ingest_url: String,
    pub api_key: String,
    pub hf_token: Option<String>,
    pub image_repo: String,
    pub poll_interval_ms: u64,
    pub concurrency: usize,
    pub push_batch: usize,
    pub port: u16,
    pub convert_markdown: bool,
}

impl Config {
    pub fn from_env() -> Self {
        Self::from_map(|k| env::var(k).ok())
    }

    pub fn from_map<G>(get: G) -> Self
    where
        G: Fn(&str) -> Option<String>,
    {
        let push_batch = get("SEC_PUSH_BATCH")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100)
            .min(200);
        let convert_markdown = get("SEC_CONVERT_MARKDOWN")
            .map(|v| v != "false" && v != "0")
            .unwrap_or(true);
        Config {
            ingest_url: get("SEC_INGEST_URL")
                .unwrap_or_else(|| "https://live-contracts.arthur.law/api/ingest".into()),
            api_key: get("SEC_API_KEY").unwrap_or_default(),
            hf_token: get("HF_TOKEN"),
            image_repo: get("SEC_IMAGE_REPO")
                .unwrap_or_else(|| "arthrod/sec-ex10-exhibits".into()),
            poll_interval_ms: get("SEC_POLL_INTERVAL_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(200),
            concurrency: get("SEC_CONCURRENCY")
                .and_then(|v| v.parse().ok())
                .unwrap_or(8),
            push_batch,
            port: get("PORT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(7860),
            convert_markdown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn defaults_apply_without_env() {
        let m = map(&[("SEC_API_KEY", "k")]);
        let cfg = Config::from_map(|k| m.get(k).cloned());
        assert_eq!(cfg.ingest_url, "https://live-contracts.arthur.law/api/ingest");
        assert_eq!(cfg.image_repo, "arthrod/sec-ex10-exhibits");
        assert_eq!(cfg.poll_interval_ms, 200);
        assert_eq!(cfg.concurrency, 8);
        assert_eq!(cfg.push_batch, 100);
        assert_eq!(cfg.port, 7860);
        assert!(cfg.convert_markdown);
        assert!(cfg.hf_token.is_none());
    }

    #[test]
    fn reads_overrides_and_caps_batch() {
        let m = map(&[
            ("SEC_API_KEY", "k"),
            ("SEC_PUSH_BATCH", "500"),
            ("SEC_CONCURRENCY", "4"),
            ("PORT", "9090"),
            ("SEC_CONVERT_MARKDOWN", "false"),
            ("HF_TOKEN", "hf_x"),
        ]);
        let cfg = Config::from_map(|k| m.get(k).cloned());
        assert_eq!(cfg.push_batch, 200); // capped at 200
        assert_eq!(cfg.concurrency, 4);
        assert_eq!(cfg.port, 9090);
        assert!(!cfg.convert_markdown);
        assert_eq!(cfg.hf_token.as_deref(), Some("hf_x"));
    }
}
