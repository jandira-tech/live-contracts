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

/// EX-10 exhibits + GRAPHIC (image) documents from one parsed filing.
pub struct Gathered {
    pub ex10: Vec<Ex10Doc>,
    pub graphics: Vec<(String, Vec<u8>)>,
}

fn s(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// Pure: split a parsed filing into traditional EX-10 docs and GRAPHIC docs.
pub fn gather(parsed: &ParsedSgml) -> Gathered {
    let mut ex10 = Vec::new();
    let mut graphics = Vec::new();
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
        } else if dt == "GRAPHIC" {
            let fname = s(d.filename());
            if !fname.is_empty() {
                graphics.push((fname, d.content().to_vec()));
            }
        }
    }
    Gathered { ex10, graphics }
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
pub async fn fetch_sgml(
    client: &reqwest::Client,
    accession: u64,
    cik: u64,
) -> anyhow::Result<Vec<u8>> {
    let url = secinfra::construct_sgml_url(accession, cik);
    let resp = client.get(&url).send().await?.error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

/// The canonical filing URL stored in `filing_url` (same .txt URL).
pub fn filing_url(accession: u64, cik: u64) -> String {
    secinfra::construct_sgml_url(accession, cik)
}

#[cfg(test)]
mod tests {
    use super::*;

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
