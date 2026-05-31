use crate::config::Config;
use crate::extract::{fetch_sgml, filing_url, gather, header_events, Ex10Doc};
use crate::header::header_json;
use crate::images::{capture_images_with, hf_upload, image_filenames, is_image_only};
use crate::ingest::{resolve_filed_at, IngestRecord};
use crate::markdown::{html_to_markdown, status_for};
use secinfra::{ParsedSgml, ParsedSubmissionMetadata, Submission};
use std::collections::HashSet;

/// A processed EX-10 document ready to become a record.
pub struct DocResult {
    pub doc: Ex10Doc,
    pub markdown: String,
    pub status: String,
    pub image_urls: Option<Vec<String>>,
}

/// Pure: assemble ingest records for one filing. `id_counter` is the
/// monotonic per-process echo token, bumped once per record.
#[allow(clippy::too_many_arguments)]
pub fn build_records(
    id_counter: &mut u64,
    accession: &str,
    cik: &str,
    form_type: &str,
    filing_url: &str,
    found_at: &str,
    filing_metadata: Option<&str>,
    docs: Vec<DocResult>,
) -> Vec<IngestRecord> {
    docs.into_iter()
        .map(|d| {
            *id_counter += 1;
            IngestRecord {
                id: *id_counter,
                accession: accession.to_string(),
                cik: cik.to_string(),
                form_type: form_type.to_string(),
                doc_type: d.doc.doc_type,
                filename: d.doc.filename,
                description: d.doc.description,
                sequence: d.doc.sequence,
                filing_url: filing_url.to_string(),
                found_at: found_at.to_string(),
                filed_at: resolve_filed_at("", filing_metadata),
                markdown_status: d.status,
                filing_metadata: filing_metadata.map(|s| s.to_string()),
                image_urls: d.image_urls.map(|u| serde_json::to_string(&u).unwrap_or_else(|_| "[]".into())),
                markdown: d.markdown,
            }
        })
        .collect()
}

/// Process one submission end-to-end into ingest records. Never panics.
pub async fn process_submission(
    client: &reqwest::Client,
    cfg: &Config,
    id_counter: &mut u64,
    sub: &Submission,
) -> Vec<IngestRecord> {
    let cik = match sub.ciks.first() {
        Some(c) => *c,
        None => return Vec::new(),
    };
    let accession_str = secinfra::format_accession_int(sub.accession, "dash");
    let f_url = filing_url(sub.accession, cik);
    let found_at = sub.detected_time.to_rfc3339();

    let sgml = match fetch_sgml(client, sub.accession, cik).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("fetch_sgml failed for {accession_str}: {e}");
            return Vec::new();
        }
    };

    // Scope parsed SGML so the borrow is dropped before any .await.
    let (gathered, meta_json) = {
        let parsed = match ParsedSgml::parse(&sgml) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("SGML parse failed for {accession_str}: {e}");
                return Vec::new();
            }
        };
        let g = gather(&parsed);
        if g.ex10.is_empty() {
            return Vec::new();
        }
        let meta = match ParsedSubmissionMetadata::parse(&sgml) {
            Ok(m) => header_json(&header_events(&m)),
            Err(_) => header_json(&[]),
        };
        (g, meta)
    };

    let mut results = Vec::new();
    for doc in gathered.ex10 {
        let html = String::from_utf8_lossy(&doc.content);
        let md = if cfg.convert_markdown { html_to_markdown(&html) } else { String::new() };
        let status = status_for(&md).as_str().to_string();

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
                gathered.graphics.clone(),
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
        &accession_str,
        &secinfra::format_accession_int(cik, "nodash"),
        &sub.submission_type,
        &f_url,
        &found_at,
        Some(&meta_json),
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
        let recs = build_records(
            &mut 0, "0001-25-000001", "123", "8-K",
            "https://sec.gov/f.txt", "2025-02-01T00:00:00Z",
            Some("{\"filed_at\":\"20250201080000\"}"),
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
    }
}
