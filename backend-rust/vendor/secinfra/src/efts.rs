use std::sync::Arc;

use async_stream::stream;
use chrono::{Duration, NaiveDate};
use futures::Stream;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, error, warn};

use crate::common::{Submission, SubmissionSource};
use crate::rate_limiter::RateLimiter;

const EFTS_URL: &str = "https://efts.sec.gov/LATEST/search-index";
const MAX_PAGE_SIZE: usize = 100;
const MAX_EFTS_HITS: usize = 10_000;
const MAX_DEPTH: usize = 4;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct QueryParams {
    ciks: Vec<u64>,
    start_date: NaiveDate,
    end_date: NaiveDate,
}

impl QueryParams {
    fn to_reqwest_query(&self) -> Vec<(&'static str, String)> {
        let mut q = vec![
            ("forms", "-0".to_string()),
            ("startdt", self.start_date.to_string()),
            ("enddt", self.end_date.to_string()),
        ];
        if !self.ciks.is_empty() {
            q.push((
                "ciks",
                self.ciks
                    .iter()
                    .map(|c| format!("{c:010}"))
                    .collect::<Vec<_>>()
                    .join(","),
            ));
        }
        q
    }

    fn with_date_range(&self, start: NaiveDate, end: NaiveDate) -> Self {
        Self {
            start_date: start,
            end_date: end,
            ..self.clone()
        }
    }
}

#[derive(Debug, Deserialize)]
struct EftsHit {
    #[serde(rename = "_source")]
    source: EftsSource,
}

#[derive(Debug, Deserialize)]
struct EftsSource {
    adsh: String,
    file_date: String,
    file_type: String,
    #[serde(default)]
    ciks: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn clean_hit(hit: EftsHit) -> Option<Submission> {
    let accession: u64 = hit
        .source
        .adsh
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;

    let ciks: Vec<u64> = hit
        .source
        .ciks
        .iter()
        .filter_map(|c| c.parse::<u64>().ok())
        .collect();

    Some(Submission {
        accession,
        submission_type: hit.source.file_type,
        ciks,
        filing_date: hit.source.file_date,
        size_bytes: None,
        source: SubmissionSource::Efts,
        detected_time: chrono::Utc::now(),
    })
}

fn split_date_range(start: NaiveDate, end: NaiveDate, n: usize) -> Vec<(NaiveDate, NaiveDate)> {
    if start == end {
        return vec![(start, end)];
    }
    let total_days = (end - start).num_days();
    let chunk = total_days / n as i64;
    (0..n as i64)
        .map(|i| {
            let s = start + Duration::days(chunk * i);
            let e = if i == n as i64 - 1 {
                end
            } else {
                start + Duration::days(chunk * (i + 1)) - Duration::days(1)
            };
            (s, e)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

async fn fetch_total(
    client: &Client,
    limiter: &RateLimiter,
    params: &QueryParams,
) -> Option<usize> {
    let mut query = params.to_reqwest_query();
    query.push(("from", "0".to_string()));
    query.push(("size", "1".to_string()));

    limiter.acquire().await;
    debug!(
        start = %params.start_date,
        end = %params.end_date,
        "Probing EFTS total"
    );
    let resp = client
        .get(EFTS_URL)
        .query(&query)
        .send()
        .await
        .map_err(|e| error!("EFTS probe failed: {e}"))
        .ok()?;

    let status = resp.status();
    if !status.is_success() {
        error!(
            %status,
            start = %params.start_date,
            end = %params.end_date,
            "EFTS probe returned non-success status"
        );
        return None;
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| error!("EFTS probe parse failed: {e}"))
        .ok()?;

    let total = json["hits"]["total"]["value"].as_u64().map(|n| n as usize);
    if let Some(total) = total {
        debug!(
            total,
            start = %params.start_date,
            end = %params.end_date,
            "EFTS total"
        );
    }
    total
}

async fn fetch_page(
    client: &Client,
    limiter: &RateLimiter,
    params: &QueryParams,
    from: usize,
) -> Vec<Submission> {
    let mut query = params.to_reqwest_query();
    query.push(("from", from.to_string()));
    query.push(("size", MAX_PAGE_SIZE.to_string()));

    limiter.acquire().await;
    debug!(
        from,
        size = MAX_PAGE_SIZE,
        start = %params.start_date,
        end = %params.end_date,
        "Fetching EFTS page"
    );
    let resp = match client.get(EFTS_URL).query(&query).send().await {
        Ok(r) => r,
        Err(e) => {
            error!("EFTS fetch failed: {e}");
            return vec![];
        }
    };

    let status = resp.status();
    if !status.is_success() {
        error!(
            %status,
            from,
            size = MAX_PAGE_SIZE,
            start = %params.start_date,
            end = %params.end_date,
            "EFTS page returned non-success status"
        );
        return vec![];
    }

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => {
            error!("EFTS parse failed: {e}");
            return vec![];
        }
    };

    let results: Vec<Submission> = json["hits"]["hits"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|h| serde_json::from_value::<EftsHit>(h).ok())
        .filter_map(clean_hit)
        .collect();

    debug!(
        from,
        count = results.len(),
        start = %params.start_date,
        end = %params.end_date,
        "Fetched EFTS page"
    );
    results
}

// ---------------------------------------------------------------------------
// Recursive fetch — plan and fetch collapsed into one pass
// ---------------------------------------------------------------------------

#[async_recursion::async_recursion]
async fn fetch_recursive(
    client: &Client,
    limiter: &RateLimiter,
    params: QueryParams,
    depth: usize,
) -> Vec<Submission> {
    let total = match fetch_total(client, limiter, &params).await {
        Some(t) => t,
        None => return vec![],
    };

    debug!(
        total,
        depth,
        start = %params.start_date,
        end = %params.end_date,
        "EFTS chunk"
    );

    if total == 0 {
        return vec![];
    }

    if total < MAX_EFTS_HITS || depth >= MAX_DEPTH {
        if depth >= MAX_DEPTH && total >= MAX_EFTS_HITS {
            warn!(
                total,
                start = %params.start_date,
                end = %params.end_date,
                "Max depth reached with oversized chunk, truncating to 10k"
            );
        }
        let num_pages = ((total.min(MAX_EFTS_HITS) + MAX_PAGE_SIZE - 1) / MAX_PAGE_SIZE).min(100);
        let mut results = vec![];
        for page in 0..num_pages {
            let mut hits = fetch_page(client, limiter, &params, page * MAX_PAGE_SIZE).await;
            results.append(&mut hits);
        }
        return results;
    }

    let ranges = split_date_range(params.start_date, params.end_date, 4);
    debug!(
        total,
        depth,
        start = %params.start_date,
        end = %params.end_date,
        chunks = ranges.len(),
        "Splitting oversized EFTS range"
    );
    let mut results = vec![];
    for (start, end) in ranges {
        let mut sub = fetch_recursive(
            client,
            limiter,
            params.with_date_range(start, end),
            depth + 1,
        )
        .await;
        results.append(&mut sub);
    }
    results
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Fetch all primary document submissions for a single date.
/// Used by the monitor for periodic EFTS validation.
pub async fn fetch_date(
    client: &Client,
    limiter: &RateLimiter,
    date: NaiveDate,
) -> Vec<Submission> {
    let params = QueryParams {
        ciks: vec![],
        start_date: date,
        end_date: date,
    };
    debug!(%date, "EFTS fetch_date");
    let results = fetch_recursive(client, limiter, params, 0).await;
    debug!(%date, count = results.len(), "EFTS fetch_date complete");
    results
}

/// Stream all primary document submissions between two dates.
/// Used for backfill at startup.
pub fn backfill_stream(
    client: Arc<Client>,
    limiter: Arc<RateLimiter>,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> impl Stream<Item = Vec<Submission>> {
    stream! {
        let params = QueryParams { ciks: vec![], start_date, end_date };
        debug!(start = %start_date, end = %end_date, "Starting EFTS backfill");
        let results = fetch_recursive(&client, &limiter, params, 0).await;
        debug!(
            start = %start_date,
            end = %end_date,
            count = results.len(),
            "EFTS backfill complete"
        );
        if !results.is_empty() {
            yield results;
        }
    }
}
