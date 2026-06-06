/// Environment-driven configuration with sensible defaults.
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub ingest_url: String,
    pub api_key: String,
    pub hf_token: Option<String>,
    pub image_repo: String,
    pub poll_interval_ms: u64,
    pub push_batch: usize,
    pub port: u16,
    pub convert_markdown: bool,
    /// Discovery sources. Mirrors datamule's Monitor, which combines the speed of
    /// the RSS `getcurrent` feed with the accuracy of EFTS (which sweeps up the
    /// filings RSS drops). Both default on; secinfra::Monitor requires ≥1 true.
    pub use_rss: bool,
    pub use_efts: bool,
    /// Opt-in local SQLite store path (SEC_STORE_PATH). When set, the producer
    /// runs the stateful "Python-parity" mode (store + backfill + push + query
    /// API); when unset (default) it stays the lean stateless inline-POST producer.
    pub store_path: Option<String>,
    /// LRU capacity for the cross-feed accession dedup cache.
    /// Sized via SEC_ACCESSION_CACHE_SIZE; invalid values fall back to the default.
    pub accession_cache_size: usize,
}

impl Config {
    pub fn from_env() -> Self {
        Self::from_map(|k| env::var(k).ok())
    }

    /// At least one discovery source must be enabled — `secinfra::Monitor::build()`
    /// asserts `use_rss || use_efts` and would panic otherwise. Call this at
    /// startup so a both-disabled env fails fast with a clear message instead of
    /// silently killing the spawned pipeline task.
    pub fn validate(&self) -> Result<(), String> {
        if !self.use_rss && !self.use_efts {
            return Err(
                "SEC_USE_RSS and SEC_USE_EFTS are both disabled — enable at least one \
                 discovery source"
                    .into(),
            );
        }
        Ok(())
    }

    pub fn from_map<G>(get: G) -> Self
    where
        G: Fn(&str) -> Option<String>,
    {
        let push_batch = get("SEC_PUSH_BATCH")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100)
            .min(200);
        // One place for boolean-like env parsing ("false"/"0" → false, default true).
        let flag = |key: &str| get(key).map(|v| v != "false" && v != "0").unwrap_or(true);
        let convert_markdown = flag("SEC_CONVERT_MARKDOWN");
        Config {
            ingest_url: get("D1_INGEST_URL")
                .unwrap_or_else(|| "https://live-contracts.arthur.law/api/ingest".into()),
            api_key: get("SEC_API_KEY").unwrap_or_default(),
            // Treat an empty HF_TOKEN as absent — docker-compose always passes the
            // var, and Some("") would wrongly enable the HF upload path.
            hf_token: get("HF_TOKEN").filter(|s| !s.is_empty()),
            image_repo: get("SEC_IMAGE_REPO")
                .unwrap_or_else(|| "arthrod/sec-ex10-exhibits".into()),
            poll_interval_ms: get("SEC_POLL_INTERVAL_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(200),
            push_batch,
            port: get("PORT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(7860),
            convert_markdown,
            use_rss: flag("SEC_USE_RSS"),
            use_efts: flag("SEC_USE_EFTS"),
            store_path: get("SEC_STORE_PATH").filter(|s| !s.is_empty()),
            accession_cache_size: get("SEC_ACCESSION_CACHE_SIZE")
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|&n| n > 0)
                .unwrap_or(65536),
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
        assert_eq!(cfg.push_batch, 100);
        assert_eq!(cfg.port, 7860);
        assert!(cfg.convert_markdown);
        assert!(cfg.hf_token.is_none());
        // Mimic datamule: both discovery sources on by default — RSS for speed,
        // EFTS as the accuracy backstop for the filings the RSS feed drops.
        assert!(cfg.use_rss);
        assert!(cfg.use_efts);
        // Stateless by default — no local store.
        assert!(cfg.store_path.is_none());
    }

    #[test]
    fn store_path_enables_stateful_mode() {
        let off = map(&[("SEC_API_KEY", "k")]);
        assert!(Config::from_map(|k| off.get(k).cloned()).store_path.is_none());
        // empty string is treated as unset
        let empty = map(&[("SEC_API_KEY", "k"), ("SEC_STORE_PATH", "")]);
        assert!(Config::from_map(|k| empty.get(k).cloned()).store_path.is_none());
        let on = map(&[("SEC_API_KEY", "k"), ("SEC_STORE_PATH", "/data/sec.db")]);
        assert_eq!(
            Config::from_map(|k| on.get(k).cloned()).store_path.as_deref(),
            Some("/data/sec.db")
        );
    }

    #[test]
    fn reads_overrides_and_caps_batch() {
        let m = map(&[
            ("SEC_API_KEY", "k"),
            ("D1_INGEST_URL", "https://example.com/api/ingest"),
            ("SEC_PUSH_BATCH", "500"),
            ("PORT", "9090"),
            ("SEC_CONVERT_MARKDOWN", "false"),
            ("HF_TOKEN", "hf_x"),
            ("SEC_USE_RSS", "false"),
            ("SEC_USE_EFTS", "0"),
        ]);
        let cfg = Config::from_map(|k| m.get(k).cloned());
        assert_eq!(cfg.ingest_url, "https://example.com/api/ingest"); // D1_INGEST_URL, not SEC_INGEST_URL
        assert_eq!(cfg.push_batch, 200); // capped at 200
        assert_eq!(cfg.port, 9090);
        assert!(!cfg.convert_markdown);
        assert_eq!(cfg.hf_token.as_deref(), Some("hf_x"));
        // Either source can be disabled via env ("false"/"0"); build() still
        // requires at least one to be true (enforced by secinfra::Monitor).
        assert!(!cfg.use_rss);
        assert!(!cfg.use_efts);
    }

    #[test]
    fn empty_hf_token_is_treated_as_absent() {
        // docker-compose always passes HF_TOKEN (empty when left blank). An empty
        // token must NOT enable the HF upload path — pipeline.rs gates image
        // capture on hf_token.is_some(). (qodo on PR #50.)
        let empty = map(&[("SEC_API_KEY", "k"), ("HF_TOKEN", "")]);
        let cfg = Config::from_map(|k| empty.get(k).cloned());
        assert!(cfg.hf_token.is_none());

        let present = map(&[("SEC_API_KEY", "k"), ("HF_TOKEN", "hf_x")]);
        let cfg = Config::from_map(|k| present.get(k).cloned());
        assert_eq!(cfg.hf_token.as_deref(), Some("hf_x"));
    }

    #[test]
    fn discovery_sources_default_on_and_toggle_independently() {
        let only_efts = map(&[("SEC_API_KEY", "k"), ("SEC_USE_RSS", "false")]);
        let cfg = Config::from_map(|k| only_efts.get(k).cloned());
        assert!(!cfg.use_rss);
        assert!(cfg.use_efts); // untouched → still defaults on

        let only_rss = map(&[("SEC_API_KEY", "k"), ("SEC_USE_EFTS", "false")]);
        let cfg = Config::from_map(|k| only_rss.get(k).cloned());
        assert!(cfg.use_rss);
        assert!(!cfg.use_efts);
    }

    #[test]
    fn validate_rejects_both_discovery_sources_disabled() {
        // secinfra::Monitor::build() asserts at least one source — guard before it
        // so a both-false env fails fast at startup instead of panicking a spawned
        // pipeline task. (gemini/qodo on PR #49.)
        let both_off = map(&[("SEC_API_KEY", "k"), ("SEC_USE_RSS", "0"), ("SEC_USE_EFTS", "false")]);
        let cfg = Config::from_map(|k| both_off.get(k).cloned());
        assert!(cfg.validate().is_err());

        // Either source on → valid.
        let one_on = map(&[("SEC_API_KEY", "k"), ("SEC_USE_RSS", "false")]);
        let cfg = Config::from_map(|k| one_on.get(k).cloned());
        assert!(cfg.validate().is_ok());

        // Defaults (both on) → valid.
        let defaults = map(&[("SEC_API_KEY", "k")]);
        let cfg = Config::from_map(|k| defaults.get(k).cloned());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn accession_cache_size_defaults_to_65536() {
        let m = map(&[("SEC_API_KEY", "k")]);
        let cfg = Config::from_map(|k| m.get(k).cloned());
        assert_eq!(cfg.accession_cache_size, 65536);
    }

    #[test]
    fn accession_cache_size_reads_env_override() {
        let m = map(&[("SEC_API_KEY", "k"), ("SEC_ACCESSION_CACHE_SIZE", "131072")]);
        let cfg = Config::from_map(|k| m.get(k).cloned());
        assert_eq!(cfg.accession_cache_size, 131072);
    }

    #[test]
    fn accession_cache_size_invalid_env_falls_back_to_default() {
        let m = map(&[("SEC_API_KEY", "k"), ("SEC_ACCESSION_CACHE_SIZE", "not-a-number")]);
        let cfg = Config::from_map(|k| m.get(k).cloned());
        assert_eq!(cfg.accession_cache_size, 65536);
    }

    #[test]
    fn accession_cache_size_zero_falls_back_to_default() {
        // 0 disables the cache's filter_new() (monitor would emit nothing) and can
        // panic capacity-based caches; treat it as invalid and use the default.
        let m = map(&[("SEC_API_KEY", "k"), ("SEC_ACCESSION_CACHE_SIZE", "0")]);
        let cfg = Config::from_map(|k| m.get(k).cloned());
        assert_eq!(cfg.accession_cache_size, 65536);
    }
}
