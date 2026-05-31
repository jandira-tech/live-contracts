use serde::Serialize;

/// One row in the `/api/ingest` POST body. Field names and order mirror
/// d1_sync.py::_FIELDS and the Astro ingest route's InRow exactly.
#[derive(Debug, Clone, Serialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filing_metadata: Option<String>,
    pub image_urls: Option<String>, // null when none; serialized explicitly
    pub markdown: String,
}

/// filed_at fallback: use the direct value; if empty, read filing_metadata.filed_at.
/// Mirrors d1_sync.py::to_ingest_record.
pub fn resolve_filed_at(direct: &str, filing_metadata: Option<&str>) -> String {
    if !direct.is_empty() {
        return direct.to_string();
    }
    if let Some(meta) = filing_metadata {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(meta) {
            if let Some(s) = v.get("filed_at").and_then(|x| x.as_str()) {
                return s.to_string();
            }
        }
    }
    String::new()
}

/// Split a slice into chunks of at most `max` (D1 ingest caps at 200 rows/POST).
pub fn chunk_rows<T>(rows: &[T], max: usize) -> Vec<&[T]> {
    rows.chunks(max.max(1)).collect()
}

/// POST one batch (≤200 rows) to the ingest route. Returns the count of accepted
/// ids. Never panics; on any error logs and returns 0 (caller decides retry).
/// Mirrors d1_sync.py::_http_poster + push_finalized accounting.
pub async fn post_batch(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    rows: &[IngestRecord],
) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let body = serde_json::json!({ "rows": rows });
    let resp = match client
        .post(url)
        .header("X-API-Key", key)
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("ingest POST failed: {e}");
            return 0;
        }
    };
    if !resp.status().is_success() {
        tracing::warn!("ingest POST status {}", resp.status());
        return 0;
    }
    #[derive(serde::Deserialize)]
    struct AcceptedResp { accepted: Vec<serde_json::Value> }
    match resp.json::<AcceptedResp>().await {
        Ok(a) => a.accepted.len(),
        Err(e) => {
            tracing::warn!("ingest response parse failed: {e}");
            0
        }
    }
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
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        for k in ["id","accession","cik","form_type","doc_type","filename","description",
                  "sequence","filing_url","found_at","filed_at","markdown_status",
                  "filing_metadata","image_urls","markdown"] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        assert_eq!(v["id"], serde_json::json!(1));
        assert_eq!(v["image_urls"], serde_json::Value::Null);
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

    #[test]
    fn chunks_at_200() {
        let rows: Vec<u64> = (0..450).collect();
        let chunks = chunk_rows(&rows, 200);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 200);
        assert_eq!(chunks[1].len(), 200);
        assert_eq!(chunks[2].len(), 50);
    }
}
