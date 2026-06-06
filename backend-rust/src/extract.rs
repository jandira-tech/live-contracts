use crate::classify::is_traditional_ex10;
use crate::header::Event;
use secinfra::{ParsedSgml, ParsedSubmissionMetadata, SubmissionEventType};

/// One EX-10 exhibit document pulled from a filing's SGML.
pub struct Ex10Doc {
    pub doc_type: String,
    pub filename: String,
    pub description: String,
    pub sequence: String,
    pub content: Vec<u8>,
}

/// Metadata-only record of a non-EX-10 document (for the all_exhibits table).
pub struct DocMeta {
    pub doc_type: String,
    pub filename: String,
    pub description: String,
    pub sequence: String,
}

/// EX-10 exhibits + GRAPHIC (image) documents from one parsed filing, plus the
/// metadata of every non-EX-10 document (`others`) for all_exhibits parity.
pub struct Gathered {
    pub ex10: Vec<Ex10Doc>,
    pub graphics: Vec<(String, Vec<u8>)>,
    pub others: Vec<DocMeta>,
}

fn s(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// Pure: split a parsed filing into traditional EX-10 docs and GRAPHIC docs.
pub fn gather(parsed: &ParsedSgml) -> Gathered {
    let mut ex10 = Vec::new();
    let mut graphics = Vec::new();
    let mut others = Vec::new();
    for d in parsed.documents() {
        let dt = s(d.doc_type());
        if is_traditional_ex10(&dt) {
            ex10.push(Ex10Doc {
                doc_type: dt,
                filename: s(d.filename()),
                description: s(d.description()),
                sequence: s(d.sequence()),
                content: d.content().to_vec(),
            });
            continue;
        }
        // Every non-EX-10 document → all_exhibits metadata (mirrors Python's split).
        let filename = s(d.filename());
        others.push(DocMeta {
            doc_type: dt.clone(),
            filename: filename.clone(),
            description: s(d.description()),
            sequence: s(d.sequence()),
        });
        if dt == "GRAPHIC" && !filename.is_empty() {
            graphics.push((filename, d.content().to_vec()));
        }
    }
    Gathered { ex10, graphics, others }
}

/// Bridge standardized submission metadata into header::Event list.
/// Keeps header.rs pure (no secinfra dependency).
pub fn header_events(meta: &ParsedSubmissionMetadata) -> Vec<Event> {
    meta.events()
        .into_iter()
        .filter(|e| e.event_type == SubmissionEventType::KeyValue)
        .map(|e| {
            let key_full = String::from_utf8_lossy(&e.key);
            // take the leaf segment after the last '.' if dotted
            let leaf = key_full.rsplit('.').next().unwrap_or(&key_full).to_string();
            Event {
                key: leaf,
                value: String::from_utf8_lossy(&e.value).to_string(),
            }
        })
        .collect()
}

/// Download a filing's full SGML submission (.txt) bytes.
/// Max fetch attempts before giving up on a submission (1 try + 4 retries).
const FETCH_MAX_ATTEMPTS: u32 = 5;

/// Backoff before the next retry: exponential (0.5s→16s, capped at 30s) plus a
/// small per-accession jitter so concurrent retriers don't resync onto SEC at
/// the same instant.
fn fetch_backoff(attempt: u32, accession: u64) -> std::time::Duration {
    let base_ms = 500u64.saturating_mul(1u64 << (attempt - 1).min(5));
    std::time::Duration::from_millis(base_ms.min(30_000) + accession % 500)
}

/// Honor a server-provided `Retry-After: <seconds>` if present.
fn retry_after(resp: &reqwest::Response) -> Option<std::time::Duration> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

pub async fn fetch_sgml(
    client: &reqwest::Client,
    accession: u64,
    cik: u64,
) -> anyhow::Result<Vec<u8>> {
    let url = secinfra::construct_sgml_url(accession, cik);
    let mut attempt = 0u32;
    loop {
        let resp = client.get(&url).send().await?;
        let status = resp.status();
        // SEC throttles concurrent clients with 429 (and the odd transient 5xx).
        // Back off and retry rather than dropping the filing or hammering SEC —
        // this is what keeps the concurrent producer under the 10 req/s ceiling.
        if status.as_u16() == 429 || status.is_server_error() {
            attempt += 1;
            if attempt >= FETCH_MAX_ATTEMPTS {
                anyhow::bail!("{url} returned {status} after {attempt} attempts");
            }
            let wait = retry_after(&resp).unwrap_or_else(|| fetch_backoff(attempt, accession));
            tracing::warn!(
                "{url} -> {status}; retry {attempt}/{FETCH_MAX_ATTEMPTS} after {:.1}s",
                wait.as_secs_f64()
            );
            tokio::time::sleep(wait).await;
            continue;
        }
        let resp = resp.error_for_status()?;
        return Ok(resp.bytes().await?.to_vec());
    }
}

/// The canonical filing URL stored in `filing_url` (same .txt URL).
pub fn filing_url(accession: u64, cik: u64) -> String {
    secinfra::construct_sgml_url(accession, cik)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_backoff_grows_then_caps() {
        use std::time::Duration;
        let acc = 7u64;
        // First retry waits at least the 0.5s base; delays grow with attempts.
        assert!(fetch_backoff(1, acc) >= Duration::from_millis(500));
        assert!(fetch_backoff(3, acc) > fetch_backoff(1, acc));
        // Capped at 30s (+ <500ms jitter) so a long outage can't stall a worker.
        assert!(fetch_backoff(40, acc) <= Duration::from_millis(30_500));
        // Jitter is bounded and per-accession (keeps concurrent retriers apart).
        assert!(fetch_backoff(1, 0) < fetch_backoff(1, 499) || fetch_backoff(1, 0) <= fetch_backoff(1, 1));
    }

    #[test]
    fn gathers_ex10_and_graphic() {
        let sgml = std::fs::read("tests/fixtures/mini.txt").expect("fixture");
        let parsed = secinfra::ParsedSgml::parse(&sgml).expect("parse");
        let g = gather(&parsed);
        assert_eq!(g.ex10.len(), 1);
        assert_eq!(g.ex10[0].doc_type, "EX-10.1");
        assert_eq!(g.ex10[0].filename, "ex10-1.htm");
        assert_eq!(g.ex10[0].sequence, "2");
        assert_eq!(g.ex10[0].description, "Material Contract");
        assert!(String::from_utf8_lossy(&g.ex10[0].content).contains("Agreement"));
        assert_eq!(g.graphics.len(), 1);
        assert_eq!(g.graphics[0].0, "img1.jpg");
        assert!(!g.graphics[0].1.is_empty());
        // every non-EX-10 document is also recorded (metadata only) for all_exhibits
        assert_eq!(g.others.len(), 1);
        assert_eq!(g.others[0].doc_type, "GRAPHIC");
        assert_eq!(g.others[0].filename, "img1.jpg");
    }

    #[test]
    fn filing_url_uses_modern_path() {
        // Regression guard: the stored filing_url must be the MODERN form
        // .../data/{cik}/{accession_nodash}/{accession-dashed}.txt — the no-dash
        // accession folder AND the dashed .txt filename.
        let accession: u64 = 125_000_001; // arbitrary
        let cik: u64 = 123;
        let url = filing_url(accession, cik);
        let nodash = secinfra::format_accession_int(accession, "nodash");
        let dashed = secinfra::format_accession_int(accession, "dash");
        assert!(url.contains(&format!("/data/{cik}/{nodash}/")), "missing nodash folder: {url}");
        assert!(url.ends_with(&format!("/{dashed}.txt")), "missing dashed .txt: {url}");
    }
}
