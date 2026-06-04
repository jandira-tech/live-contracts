use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::Client;
use tracing::{debug, error, warn};

use crate::common::{Submission, SubmissionSource};
use crate::format_accession::format_accession_str;
use crate::rate_limiter::RateLimiter;

const RSS_URL: &str =
    "https://www.sec.gov/cgi-bin/browse-edgar?count=100&action=getcurrent&output=rss";

fn xml_preview(xml: &str) -> String {
    String::from_utf8_lossy(&xml.as_bytes()[..xml.len().min(1000)]).into_owned()
}

fn push_or_merge_submission(
    submissions: &mut Vec<Submission>,
    accession: u64,
    cik: Option<u64>,
    submission_type: &str,
    filing_date: &str,
    size_bytes: Option<u64>,
    detected_time: &chrono::DateTime<chrono::Utc>,
) {
    if let Some(existing) = submissions.iter_mut().find(|s| s.accession == accession) {
        if let Some(cik) = cik {
            if !existing.ciks.contains(&cik) {
                existing.ciks.push(cik);
            }
        }
        if existing.submission_type.is_empty() && !submission_type.is_empty() {
            existing.submission_type = submission_type.to_string();
        }
        if existing.filing_date.is_empty() && !filing_date.is_empty() {
            existing.filing_date = filing_date.to_string();
        }
        if existing.size_bytes.is_none() {
            existing.size_bytes = size_bytes;
        }
        return;
    }

    submissions.push(Submission {
        accession,
        submission_type: submission_type.to_string(),
        ciks: cik.into_iter().collect(),
        filing_date: filing_date.to_string(),
        size_bytes,
        source: SubmissionSource::Rss,
        detected_time: detected_time.clone(),
    });
}

fn parse_summary_size_bytes(summary: &str) -> Option<u64> {
    let pos = summary.find("Size:</b>")?;
    let mut parts = summary[pos + "Size:</b>".len()..].split_whitespace();
    let value = parts.next()?.parse::<f64>().ok()?;
    let unit = parts
        .next()
        .map(|u| {
            u.trim_matches(|c: char| !c.is_ascii_alphabetic())
                .to_ascii_uppercase()
        })
        .unwrap_or_else(|| "B".to_string());
    let multiplier = match unit.as_str() {
        "B" | "BYTE" | "BYTES" => 1.0,
        "KB" | "K" => 1024.0,
        "MB" | "M" => 1024.0 * 1024.0,
        "GB" | "G" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };

    if value.is_finite() && value >= 0.0 {
        Some((value * multiplier).round() as u64)
    } else {
        None
    }
}

fn parse_rss(xml: &str) -> Vec<Submission> {
    let mut submissions = Vec::new();
    let detected_time = chrono::Utc::now();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut current_accession: Option<u64> = None;
    let mut current_cik: Option<u64> = None;
    let mut current_type = String::new();
    let mut current_date = String::new();
    let mut current_size_bytes: Option<u64> = None;
    let mut in_summary = false;
    let mut in_title = false;
    let mut summary_text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"category" => {
                    if let Some(term) = e.attributes().flatten().find(|a| a.key.as_ref() == b"term")
                    {
                        current_type = String::from_utf8_lossy(&term.value).into_owned();
                    }
                }
                b"summary" => {
                    in_summary = true;
                    summary_text.clear();
                }
                b"title" => in_title = true,
                _ => {}
            },
            Ok(Event::Text(e)) => {
                let text = e.unescape().unwrap_or_default();
                if in_summary {
                    summary_text.push_str(&text);
                } else if in_title {
                    // "10-K/A - NutriBand Inc. (0001676047) (Filer)"
                    // CIK is in the second-to-last parenthesised group
                    let parens: Vec<&str> = text
                        .split('(')
                        .filter_map(|s| s.split(')').next())
                        .collect();
                    if parens.len() >= 2 {
                        current_cik = parens[parens.len() - 2].parse::<u64>().ok();
                    }
                } else if text.starts_with("urn:tag:sec.gov") {
                    // "urn:tag:sec.gov,2008:accession-number=0001213900-26-058507"
                    if let Some(acc_str) = text.split('=').last() {
                        current_accession =
                            format_accession_str(acc_str, "int").parse::<u64>().ok();
                    }
                }
            }
            Ok(Event::End(e)) => {
                match e.name().as_ref() {
                    b"title" => in_title = false,
                    b"summary" => {
                        in_summary = false;
                        // "Filed:</b> 2026-05-18 <b>AccNo:..."
                        if let Some(pos) = summary_text.find("Filed:</b>") {
                            let after = summary_text[pos + 10..].trim();
                            current_date = after[..10].to_string();
                        }
                        current_size_bytes = parse_summary_size_bytes(&summary_text);
                    }
                    b"entry" => {
                        if let Some(accession) = current_accession {
                            push_or_merge_submission(
                                &mut submissions,
                                accession,
                                current_cik,
                                &current_type,
                                &current_date,
                                current_size_bytes,
                                &detected_time,
                            );
                        } else {
                            debug!("Skipping RSS entry without accession");
                        }
                        current_accession = None;
                        current_cik = None;
                        current_type.clear();
                        current_date.clear();
                        current_size_bytes = None;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                let preview = xml_preview(xml);
                error!(error = %e, preview = %preview, "XML parse error");
                break;
            }
            _ => {}
        }
        buf.clear();
    }

    debug!(count = submissions.len(), "Parsed RSS submissions");
    submissions
}

/// Fetch and parse the SEC RSS feed, returning all current submissions.
pub async fn poll_rss(client: &Client, limiter: &RateLimiter) -> anyhow::Result<Vec<Submission>> {
    limiter.acquire().await;
    debug!(url = RSS_URL, "Polling SEC RSS feed");
    let resp = client.get(RSS_URL).send().await?;
    let status = resp.status();
    let xml = resp.text().await?;

    if status.is_success() {
        debug!(%status, bytes = xml.len(), "Fetched SEC RSS feed");
    } else {
        warn!(%status, bytes = xml.len(), "Fetched SEC RSS feed with non-success status");
    }

    Ok(parse_rss(&xml))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_summary_size_bytes() {
        assert_eq!(
            parse_summary_size_bytes("<b>Filed:</b> 2026-05-22 <b>Size:</b> 4 KB"),
            Some(4 * 1024)
        );
        assert_eq!(
            parse_summary_size_bytes("<b>Filed:</b> 2026-05-22 <b>Size:</b> 21 MB"),
            Some(21 * 1024 * 1024)
        );
        assert_eq!(
            parse_summary_size_bytes("<b>Filed:</b> 2026-05-22 <b>Size:</b> 2 GB"),
            Some(2 * 1024 * 1024 * 1024)
        );
        assert_eq!(
            parse_summary_size_bytes("<b>Filed:</b> 2026-05-22 <b>Size:</b> 512 B"),
            Some(512)
        );
    }

    #[test]
    fn parse_rss_carries_size_bytes() {
        let xml = r#"<?xml version="1.0" encoding="ISO-8859-1" ?>
<feed xmlns="http://www.w3.org/2005/Atom">
<entry>
<title>10-K - 8X8 INC /DE/ (0001023731) (Filer)</title>
<summary type="html">
 &lt;b&gt;Filed:&lt;/b&gt; 2026-05-22 &lt;b&gt;AccNo:&lt;/b&gt; 0001023731-26-000041 &lt;b&gt;Size:&lt;/b&gt; 21 MB
</summary>
<category scheme="https://www.sec.gov/" label="form type" term="10-K"/>
<id>urn:tag:sec.gov,2008:accession-number=0001023731-26-000041</id>
</entry>
</feed>"#;

        let submissions = parse_rss(xml);

        assert_eq!(submissions.len(), 1);
        assert_eq!(submissions[0].size_bytes, Some(21 * 1024 * 1024));
    }
}
