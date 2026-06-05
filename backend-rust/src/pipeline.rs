use crate::config::Config;
use crate::extract::{fetch_sgml, filing_url, gather, header_events, Ex10Doc};
use crate::header::header_json;
use crate::images::{capture_images_with, hf_upload, image_filenames, is_image_only};
use crate::ingest::{resolve_filed_at, IngestRecord};
use crate::markdown::{html_to_markdown, status_for};
use secinfra::{ParsedSgml, ParsedSubmissionMetadata, Submission};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

/// Map a discovery source to its canonical lowercase wire string.
pub fn source_str(source: secinfra::SubmissionSource) -> &'static str {
    match source {
        secinfra::SubmissionSource::Rss => "rss",
        secinfra::SubmissionSource::Efts => "efts",
    }
}

/// Fields shared by every record in one filing.
pub struct RecordMeta<'a> {
    pub accession: &'a str,
    pub cik: &'a str,
    pub form_type: &'a str,
    pub filing_url: &'a str,
    pub found_at: &'a str,
    pub detected_at: &'a str,
    pub source: &'a str,
    pub size_bytes: Option<u64>,
    pub filing_metadata: Option<&'a str>,
}

/// A processed EX-10 document ready to become a record.
pub struct DocResult {
    pub doc: Ex10Doc,
    pub markdown: String,
    pub status: String,
    pub image_urls: Option<Vec<String>>,
}

/// Pure: assemble ingest records for one filing. `id_counter` is the shared
/// per-process echo token, bumped once per record (D1 mints the real UUIDv7,
/// so only per-process uniqueness matters — Relaxed ordering is sufficient).
pub fn build_records(
    id_counter: &AtomicU64,
    meta: &RecordMeta<'_>,
    docs: Vec<DocResult>,
) -> Vec<IngestRecord> {
    docs.into_iter()
        .map(|d| {
            let id = id_counter.fetch_add(1, Ordering::Relaxed) + 1;
            IngestRecord {
                id,
                accession: meta.accession.to_string(),
                cik: meta.cik.to_string(),
                form_type: meta.form_type.to_string(),
                doc_type: d.doc.doc_type,
                filename: d.doc.filename,
                description: d.doc.description,
                sequence: d.doc.sequence,
                filing_url: meta.filing_url.to_string(),
                found_at: meta.found_at.to_string(),
                filed_at: resolve_filed_at("", meta.filing_metadata),
                markdown_status: d.status,
                filing_metadata: meta.filing_metadata.map(|s| s.to_string()),
                image_urls: d.image_urls.map(|u| serde_json::to_string(&u).unwrap_or_else(|_| "[]".into())),
                markdown: d.markdown,
                source: meta.source.to_string(),
                size_bytes: meta.size_bytes,
                detected_at: meta.detected_at.to_string(),
            }
        })
        .collect()
}

/// One EX-10 doc after the CPU-bound markdown conversion (owned, `'static`).
struct ParsedDoc {
    doc: Ex10Doc,
    markdown: String,
    status: String,
}

/// Fully owned output of the CPU-bound parse/convert section, safe to return
/// across the `spawn_blocking` await boundary.
struct ParsedSubmission {
    docs: Vec<ParsedDoc>,
    graphics: Vec<(String, Vec<u8>)>,
    meta_json: String,
}

/// CPU-bound: parse SGML, gather EX-10 + graphics, convert each EX-10 to
/// markdown, and build the header meta JSON. Returns `None` when there is
/// nothing to emit (parse error or no EX-10 docs). Runs inside `spawn_blocking`
/// so it never blocks the async runtime.
fn parse_submission_blocking(
    sgml: &[u8],
    convert_markdown: bool,
    accession_str: &str,
) -> Option<ParsedSubmission> {
    let parsed = match ParsedSgml::parse(sgml) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("SGML parse failed for {accession_str}: {e}");
            return None;
        }
    };
    let gathered = gather(&parsed);
    if gathered.ex10.is_empty() {
        return None;
    }
    let meta_json = match ParsedSubmissionMetadata::parse(sgml) {
        Ok(m) => header_json(&header_events(&m)),
        Err(_) => header_json(&[]),
    };

    let docs = gathered
        .ex10
        .into_iter()
        .map(|doc| {
            let html = String::from_utf8_lossy(&doc.content);
            let markdown = if convert_markdown { html_to_markdown(&html) } else { String::new() };
            let status = status_for(&markdown).as_str().to_string();
            ParsedDoc { doc, markdown, status }
        })
        .collect();

    Some(ParsedSubmission { docs, graphics: gathered.graphics, meta_json })
}

/// Process one submission end-to-end into ingest records. Never panics.
pub async fn process_submission(
    client: &reqwest::Client,
    cfg: &Config,
    id_counter: &AtomicU64,
    sub: &Submission,
) -> Vec<IngestRecord> {
    let cik = match sub.ciks.first() {
        Some(c) => *c,
        None => return Vec::new(),
    };
    let accession_str = secinfra::format_accession_int(sub.accession, "dash");
    let f_url = filing_url(sub.accession, cik);
    // Display form for the D1 frontend's `found_at >= datetime('now',...)` string
    // comparison/sort; the precise machine timestamp lives in `detected_at`.
    let found_at = sub.detected_time.format("%Y-%m-%d %H:%M:%S").to_string();
    let detected_at = sub.detected_time.to_rfc3339();
    let source = source_str(sub.source);

    let sgml = match fetch_sgml(client, sub.accession, cik).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("fetch_sgml failed for {accession_str}: {e}");
            return Vec::new();
        }
    };

    // The CPU-bound section (SGML parse, gather, HTML→markdown) blocks the Tokio
    // worker thread driving the `buffer_unordered` stream; offload it to a blocking
    // thread so it can't stall the async runtime (e.g. the health server). The
    // closure takes ownership of the fetched bytes + needed cfg flags and returns
    // fully owned data — no borrows survive the await boundary.
    let convert_markdown = cfg.convert_markdown;
    let accession_for_blocking = accession_str.clone();
    let parsed = tokio::task::spawn_blocking(move || {
        parse_submission_blocking(&sgml, convert_markdown, &accession_for_blocking)
    })
    .await;
    let ParsedSubmission { docs, graphics, meta_json } = match parsed {
        Ok(Some(p)) => p,
        Ok(None) => return Vec::new(),
        Err(e) => {
            tracing::warn!("parse task failed for {accession_str}: {e}");
            return Vec::new();
        }
    };

    // Image capture is async (network) — run it here on the owned parse output.
    let mut results = Vec::new();
    for parsed_doc in docs {
        let ParsedDoc { doc, markdown: md, status } = parsed_doc;

        // image-only → capture this doc's images (best-effort).
        let mut image_urls: Option<Vec<String>> = None;
        if cfg.hf_token.is_some() && is_image_only(&md) {
            let only_refs: HashSet<String> = image_filenames(&md).into_iter().collect();
            let repo = cfg.image_repo.clone();
            let client2 = client.clone();
            let token = cfg.hf_token.clone().unwrap_or_default();
            let repo_for_closure = repo.clone();
            let urls = capture_images_with(
                &accession_str,
                graphics.clone(),
                Some(&only_refs),
                &repo,
                move |uploads| {
                    let c = client2.clone();
                    let t = token.clone();
                    let r = repo_for_closure;
                    async move { hf_upload(&c, &r, &t, uploads).await }
                },
            )
            .await;
            if !urls.is_empty() {
                image_urls = Some(urls);
            }
        }

        results.push(DocResult {
            doc,
            markdown: md,
            status,
            image_urls,
        });
    }

    build_records(
        id_counter,
        &RecordMeta {
            accession: &accession_str,
            cik: &cik.to_string(),
            form_type: &sub.submission_type,
            filing_url: &f_url,
            found_at: &found_at,
            detected_at: &detected_at,
            source,
            size_bytes: sub.size_bytes,
            filing_metadata: Some(&meta_json),
        },
        results,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Ex10Doc;

    fn doc(dt: &str, fname: &str) -> Ex10Doc {
        Ex10Doc { doc_type: dt.into(), filename: fname.into(),
                  description: "d".into(), sequence: "2".into(), content: b"<p>x</p>".to_vec() }
    }

    #[test]
    fn builds_one_record_per_ex10_doc() {
        let inputs = vec![
            DocResult {
                doc: doc("EX-10.1", "a.htm"),
                markdown: "# A".into(),
                status: "done".into(),
                image_urls: None,
            },
            DocResult {
                doc: doc("EX-10.2", "b.htm"),
                markdown: "".into(),
                status: "empty".into(),
                image_urls: Some(vec!["https://hf/x.jpg".into()]),
            },
        ];
        let counter = AtomicU64::new(0);
        let recs = build_records(
            &counter,
            &RecordMeta {
                accession: "0001-25-000001",
                cik: "123",
                form_type: "8-K",
                filing_url: "https://sec.gov/f.txt",
                found_at: "2025-02-01 08:00:00",
                detected_at: "2025-02-01T08:00:00.123456+00:00",
                source: "efts",
                size_bytes: Some(4096),
                filing_metadata: Some("{\"filed_at\":\"20250201080000\"}"),
            },
            inputs,
        );
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].id, 1);
        assert_eq!(recs[1].id, 2);
        assert_eq!(recs[0].doc_type, "EX-10.1");
        assert_eq!(recs[0].markdown, "# A");
        assert_eq!(recs[0].filed_at, "20250201080000"); // from metadata fallback
        assert_eq!(recs[0].image_urls, None);
        assert_eq!(recs[1].markdown_status, "empty");
        assert_eq!(recs[1].image_urls, Some("[\"https://hf/x.jpg\"]".into()));
        // New fields: display found_at, precise detected_at, source, size_bytes.
        assert_eq!(recs[0].found_at, "2025-02-01 08:00:00");
        assert_eq!(recs[0].detected_at, "2025-02-01T08:00:00.123456+00:00");
        assert_eq!(recs[0].source, "efts");
        assert_eq!(recs[0].size_bytes, Some(4096));
        assert_eq!(recs[1].size_bytes, Some(4096));
    }

    #[test]
    fn source_str_maps_to_lowercase() {
        assert_eq!(source_str(secinfra::SubmissionSource::Rss), "rss");
        assert_eq!(source_str(secinfra::SubmissionSource::Efts), "efts");
    }
}
