use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::cooldown::Cooldown;

/// One row in the `/api/ingest` POST body. Field names and order mirror
/// d1_sync.py::_FIELDS and the Astro ingest route's InRow exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRecord {
    pub id: u64,                 // echo token only; D1 mints the real UUIDv7
    pub accession: String,
    pub cik: String,
    pub form_type: String,
    pub doc_type: String,
    pub filename: String,
    pub description: String,
    pub sequence: String,
    pub filing_url: String,
    pub found_at: String,
    pub filed_at: String,
    pub markdown_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filing_metadata: Option<String>,
    pub image_urls: Option<String>, // null when none; serialized explicitly
    pub markdown: String,
    /// Discovery source: "rss" or "efts".
    pub source: String,
    /// Raw filing size in bytes, when the monitor reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Precise canonical detection timestamp (RFC3339 UTC).
    pub detected_at: String,
}

/// filed_at fallback: use the direct value; if empty, read filing_metadata.filed_at.
/// Mirrors d1_sync.py::to_ingest_record.
pub fn resolve_filed_at(direct: &str, filing_metadata: Option<&str>) -> String {
    if !direct.is_empty() {
        return direct.to_string();
    }
    if let Some(meta) = filing_metadata
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(meta)
        && let Some(s) = v.get("filed_at").and_then(|x| x.as_str())
    {
        return s.to_string();
    }
    String::new()
}

/// D1 rejects a row over 2,000,000 bytes, and the ingest route writes a batch
/// atomically, so one oversized exhibit used to sink every row posted with it.
/// Leave headroom for the other columns.
pub const MAX_ROW_BYTES: usize = 1_900_000;
/// Keep one POST well under the Worker's request limits.
pub const MAX_BATCH_BYTES: usize = 8_000_000;

/// Approximate stored size of a row: the text columns that can be large.
pub fn row_bytes(r: &IngestRecord) -> usize {
    r.markdown.len()
        + r.filing_metadata.as_deref().map_or(0, str::len)
        + r.image_urls.as_deref().map_or(0, str::len)
        + r.description.len()
        + r.filing_url.len()
        + r.filename.len()
        + 512 // the short columns
}

/// Truncate the markdown of a row that would not fit in D1, mark it `truncated`
/// and point to the full exhibit. Rows that fit are returned unchanged.
pub fn fit_for_d1(mut r: IngestRecord) -> IngestRecord {
    if row_bytes(&r) <= MAX_ROW_BYTES {
        return r;
    }
    let note = format!(
        "\n\n---\n\n*Truncated: this exhibit is longer than the database stores. The full text is in the SEC filing: {}*\n",
        r.filing_url
    );
    let room = MAX_ROW_BYTES
        .saturating_sub(row_bytes(&r) - r.markdown.len())
        .saturating_sub(note.len());
    let mut cut = room.min(r.markdown.len());
    while cut > 0 && !r.markdown.is_char_boundary(cut) {
        cut -= 1;
    }
    r.markdown.truncate(cut);
    r.markdown.push_str(&note);
    r.markdown_status = "truncated".into();
    r
}

/// Chunk rows by count and by bytes, so a few large exhibits travel alone.
pub fn chunk_by_size(rows: &[IngestRecord], max_rows: usize, max_bytes: usize) -> Vec<&[IngestRecord]> {
    let mut out = Vec::new();
    let (mut start, mut bytes) = (0usize, 0usize);
    for (i, r) in rows.iter().enumerate() {
        let b = row_bytes(r);
        let full = i > start && (i - start >= max_rows.max(1) || bytes + b > max_bytes);
        if full {
            out.push(&rows[start..i]);
            start = i;
            bytes = 0;
        }
        bytes += b;
    }
    if start < rows.len() {
        out.push(&rows[start..]);
    }
    out
}

/// What happened to one POST. `Retry` is worth trying again (transport error, 408,
/// 429, 5xx, unreadable 2xx body: the route upserts, so a resend is harmless);
/// `Rejected` will not get better by resending (bad key, bad payload).
#[derive(Debug, Clone, PartialEq)]
pub enum PushOutcome {
    Accepted(usize),
    Retry { reason: String, cool_down: bool },
    Rejected { status: u16, reason: String },
}

/// Classify a non-2xx status. 429 and 503 also start the endpoint cooldown.
pub fn classify_status(status: u16) -> PushOutcome {
    match status {
        429 | 503 => PushOutcome::Retry { reason: format!("status {status}"), cool_down: true },
        408 | 500..=599 => PushOutcome::Retry { reason: format!("status {status}"), cool_down: false },
        _ => PushOutcome::Rejected { status, reason: format!("status {status}") },
    }
}

/// POST one batch (≤200 rows) to the ingest route. Never panics.
/// Mirrors d1_sync.py::_http_poster + push_finalized accounting.
pub async fn post_batch(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    rows: &[IngestRecord],
) -> PushOutcome {
    if rows.is_empty() {
        return PushOutcome::Accepted(0);
    }
    post_batch_raw(client, url, key, rows).await
}

/// `post_batch` without the empty-batch short cut: preflight sends `{"rows": []}`
/// to prove the key without writing anything.
pub async fn post_batch_raw(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    rows: &[IngestRecord],
) -> PushOutcome {
    let body = serde_json::json!({ "rows": rows });
    let resp = match client.post(url).header("X-API-Key", key).json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            return PushOutcome::Retry { reason: format!("transport: {e}"), cool_down: false };
        }
    };
    let status = resp.status();
    if !status.is_success() {
        return classify_status(status.as_u16());
    }
    #[derive(serde::Deserialize)]
    struct AcceptedResp {
        accepted: Vec<serde_json::Value>,
    }
    match resp.json::<AcceptedResp>().await {
        Ok(a) => PushOutcome::Accepted(a.accepted.len()),
        Err(e) => PushOutcome::Retry { reason: format!("unreadable response: {e}"), cool_down: false },
    }
}

/// POST with a bounded retry budget: up to `attempts` tries, doubling `backoff`
/// between them. Returns early on acceptance, on a `Rejected`, or when the
/// endpoint is (or becomes) cooling down; a cooling endpoint is not called.
pub async fn push_with_budget(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    rows: &[IngestRecord],
    attempts: u32,
    backoff: Duration,
    cooldown: &Cooldown,
) -> PushOutcome {
    let mut last = PushOutcome::Retry { reason: "no attempt made".into(), cool_down: false };
    let mut wait = backoff;
    for attempt in 1..=attempts.max(1) {
        if let Some(left) = cooldown.remaining() {
            return PushOutcome::Retry {
                reason: format!("ingest cooling down for {}s", left.as_secs()),
                cool_down: true,
            };
        }
        last = post_batch(client, url, key, rows).await;
        match &last {
            PushOutcome::Accepted(_) | PushOutcome::Rejected { .. } => return last,
            PushOutcome::Retry { cool_down: true, .. } => {
                cooldown.trip();
                return last;
            }
            PushOutcome::Retry { reason, .. } => {
                tracing::warn!(attempt, attempts, "ingest push failed: {reason}");
            }
        }
        if attempt < attempts {
            tokio::time::sleep(wait).await;
            wait = wait.saturating_mul(2);
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_exact_keys() {
        let r = IngestRecord {
            id: 1,
            accession: "0001-25-000001".into(),
            cik: "123".into(),
            form_type: "8-K".into(),
            doc_type: "EX-10.1".into(),
            filename: "ex10-1.htm".into(),
            description: "Material Contract".into(),
            sequence: "2".into(),
            filing_url: "https://sec.gov/x.txt".into(),
            found_at: "2025-02-01T08:00:00Z".into(),
            filed_at: "20250201080000".into(),
            markdown_status: "done".into(),
            filing_metadata: Some("{\"filed_at\":\"20250201080000\"}".into()),
            image_urls: None,
            markdown: "# hi".into(),
            source: "rss".into(),
            size_bytes: None,
            detected_at: "2025-02-01T08:00:00+00:00".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        for k in ["id","accession","cik","form_type","doc_type","filename","description",
                  "sequence","filing_url","found_at","filed_at","markdown_status",
                  "filing_metadata","image_urls","markdown","source","detected_at"] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        assert_eq!(v["id"], serde_json::json!(1));
        assert_eq!(v["image_urls"], serde_json::Value::Null);
        assert_eq!(v["source"], serde_json::json!("rss"));
        assert_eq!(v["detected_at"], serde_json::json!("2025-02-01T08:00:00+00:00"));
        // size_bytes: None must be omitted entirely.
        assert!(v.get("size_bytes").is_none(), "size_bytes None should be omitted");

        // size_bytes: Some(N) must serialize.
        let r2 = IngestRecord { size_bytes: Some(4096), ..r };
        let v2: serde_json::Value = serde_json::to_value(&r2).unwrap();
        assert_eq!(v2["size_bytes"], serde_json::json!(4096));
    }

    #[test]
    fn filed_at_falls_back_to_metadata() {
        let got = resolve_filed_at("", Some("{\"filed_at\":\"20250201080000\"}"));
        assert_eq!(got, "20250201080000");
        // direct value wins
        assert_eq!(resolve_filed_at("DIRECT", Some("{\"filed_at\":\"X\"}")), "DIRECT");
        // missing/garbage metadata → empty
        assert_eq!(resolve_filed_at("", Some("not json")), "");
        assert_eq!(resolve_filed_at("", None), "");
    }

    fn row(id: u64) -> IngestRecord {
        IngestRecord {
            id,
            accession: format!("A{id}"),
            cik: "1".into(),
            form_type: "8-K".into(),
            doc_type: "EX-10.1".into(),
            filename: format!("f{id}.htm"),
            description: String::new(),
            sequence: "1".into(),
            filing_url: String::new(),
            found_at: String::new(),
            filed_at: String::new(),
            markdown_status: "done".into(),
            filing_metadata: None,
            image_urls: None,
            markdown: String::new(),
            source: "rss".into(),
            size_bytes: None,
            detected_at: String::new(),
        }
    }

    #[test]
    fn statuses_are_classified() {
        for s in [408, 429, 500, 502, 503, 504] {
            assert!(matches!(classify_status(s), PushOutcome::Retry { .. }), "{s}");
        }
        for s in [400, 401, 403, 404, 413] {
            assert!(matches!(classify_status(s), PushOutcome::Rejected { status, .. } if status == s), "{s}");
        }
        assert!(matches!(classify_status(429), PushOutcome::Retry { cool_down: true, .. }));
        assert!(matches!(classify_status(503), PushOutcome::Retry { cool_down: true, .. }));
        assert!(matches!(classify_status(500), PushOutcome::Retry { cool_down: false, .. }));
    }

    #[tokio::test]
    async fn post_batch_reports_accepted_count_and_sends_the_key() {
        let (url, script) = crate::test_support::ingest_server(&[]).await;
        let client = reqwest::Client::new();
        let got = post_batch(&client, &url, "k1", &[row(1), row(2)]).await;
        assert_eq!(got, PushOutcome::Accepted(2));
        assert_eq!(script.lock().unwrap().keys, vec!["k1".to_string()]);
    }

    #[tokio::test]
    async fn post_batch_distinguishes_rejected_from_retryable() {
        let (url, _) = crate::test_support::ingest_server(&[401, 503]).await;
        let client = reqwest::Client::new();
        assert!(matches!(post_batch(&client, &url, "bad", &[row(1)]).await, PushOutcome::Rejected { status: 401, .. }));
        assert!(matches!(post_batch(&client, &url, "k", &[row(1)]).await, PushOutcome::Retry { cool_down: true, .. }));
        // Nothing listening: a transport error is retryable.
        let gone = post_batch(&client, "http://127.0.0.1:9/api/ingest", "k", &[row(1)]).await;
        assert!(matches!(gone, PushOutcome::Retry { cool_down: false, .. }));
    }

    #[tokio::test]
    async fn empty_batch_is_accepted_without_a_call() {
        let (url, script) = crate::test_support::ingest_server(&[]).await;
        let got = post_batch(&reqwest::Client::new(), &url, "k", &[]).await;
        assert_eq!(got, PushOutcome::Accepted(0));
        assert_eq!(script.lock().unwrap().calls, 0);
    }

    #[tokio::test]
    async fn budget_retries_transient_failures_then_succeeds() {
        let (url, script) = crate::test_support::ingest_server(&[500, 502]).await;
        let cd = Cooldown::new(Duration::from_secs(300));
        let got = push_with_budget(&reqwest::Client::new(), &url, "k", &[row(1)], 3, Duration::from_millis(1), &cd).await;
        assert_eq!(got, PushOutcome::Accepted(1));
        assert_eq!(script.lock().unwrap().calls, 3);
    }

    #[tokio::test]
    async fn budget_is_bounded() {
        let (url, script) = crate::test_support::ingest_server(&[500, 500, 500, 500, 500]).await;
        let cd = Cooldown::new(Duration::from_secs(300));
        let got = push_with_budget(&reqwest::Client::new(), &url, "k", &[row(1)], 2, Duration::from_millis(1), &cd).await;
        assert!(matches!(got, PushOutcome::Retry { .. }));
        assert_eq!(script.lock().unwrap().calls, 2);
    }

    #[tokio::test]
    async fn rejection_is_not_retried() {
        let (url, script) = crate::test_support::ingest_server(&[401]).await;
        let cd = Cooldown::new(Duration::from_secs(300));
        let got = push_with_budget(&reqwest::Client::new(), &url, "k", &[row(1)], 3, Duration::from_millis(1), &cd).await;
        assert!(matches!(got, PushOutcome::Rejected { status: 401, .. }));
        assert_eq!(script.lock().unwrap().calls, 1);
    }

    #[tokio::test]
    async fn rate_limit_trips_the_cooldown_and_a_cooling_endpoint_is_not_called() {
        let (url, script) = crate::test_support::ingest_server(&[429]).await;
        let cd = Cooldown::new(Duration::from_secs(300));
        let client = reqwest::Client::new();
        let first = push_with_budget(&client, &url, "k", &[row(1)], 3, Duration::from_millis(1), &cd).await;
        assert!(matches!(first, PushOutcome::Retry { cool_down: true, .. }));
        assert_eq!(script.lock().unwrap().calls, 1, "429 stops the budget at once");
        assert!(cd.is_cooling());
        let second = push_with_budget(&client, &url, "k", &[row(1)], 3, Duration::from_millis(1), &cd).await;
        assert!(matches!(second, PushOutcome::Retry { .. }));
        assert_eq!(script.lock().unwrap().calls, 1, "no call while cooling");
    }

    fn big(id: u64, md_bytes: usize) -> IngestRecord {
        let mut r = row(id);
        r.markdown = "é".repeat(md_bytes / 2); // 2-byte chars: truncation must respect boundaries
        r.filing_url = "https://www.sec.gov/Archives/edgar/data/1/000000000026000001.txt".into();
        r
    }

    #[test]
    fn oversized_rows_are_truncated_marked_and_linked() {
        let r = fit_for_d1(big(1, 3_700_000));
        assert!(row_bytes(&r) <= MAX_ROW_BYTES, "{}", row_bytes(&r));
        assert_eq!(r.markdown_status, "truncated");
        assert!(r.markdown.contains("https://www.sec.gov/Archives/edgar/data/1/000000000026000001.txt"));
        assert!(r.markdown.is_char_boundary(r.markdown.len()));
    }

    #[test]
    fn rows_that_fit_are_untouched() {
        let small = big(2, 10_000);
        let out = fit_for_d1(small.clone());
        assert_eq!(out.markdown, small.markdown);
        assert_eq!(out.markdown_status, small.markdown_status);
    }

    #[test]
    fn chunks_respect_both_count_and_bytes() {
        let rows: Vec<IngestRecord> = vec![big(1, 1_800_000), big(2, 1_800_000), big(3, 1_000), big(4, 1_000), big(5, 1_800_000)];
        let chunks = chunk_by_size(&rows, 100, 3_000_000);
        // No chunk over the byte budget unless it is a single row.
        for c in &chunks {
            assert!(c.len() == 1 || c.iter().map(row_bytes).sum::<usize>() <= 3_000_000);
        }
        assert_eq!(chunks.iter().map(|c| c.len()).sum::<usize>(), 5, "nothing dropped");
        assert_eq!(chunk_by_size(&rows, 2, usize::MAX).len(), 3, "count cap still applies");
        assert!(chunk_by_size(&[], 100, 1).is_empty());
    }
}
