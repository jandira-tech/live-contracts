use std::sync::Arc;
use std::time::{Duration, Instant};

use async_stream::stream;
use chrono::NaiveDate;
use futures::{Stream, StreamExt};
use indexmap::IndexSet;
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::common::{Submission, sec_filing_date_now, sec_user_agent};
use crate::efts::{backfill_stream, fetch_date};
use crate::rate_limiter::RateLimiter;
use crate::rss::poll_rss;

const MAX_ACCESSIONS: usize = 50_000;

#[derive(Clone)]
pub struct AccessionCache {
    accessions: Arc<Mutex<IndexSet<u64>>>,
    max_accessions: usize,
}

impl Default for AccessionCache {
    fn default() -> Self {
        Self::new(MAX_ACCESSIONS)
    }
}

impl AccessionCache {
    pub fn new(max_accessions: usize) -> Self {
        Self {
            accessions: Arc::new(Mutex::new(IndexSet::with_capacity(max_accessions))),
            max_accessions,
        }
    }

    pub async fn check_and_insert(&self, accession: u64) -> bool {
        if self.max_accessions == 0 {
            return false;
        }

        let mut accessions = self.accessions.lock().await;
        if accessions.contains(&accession) {
            return false;
        }
        if accessions.len() >= self.max_accessions {
            accessions.shift_remove_index(0);
        }
        accessions.insert(accession);
        true
    }

    pub async fn filter_new(&self, batch: Vec<Submission>) -> Vec<Submission> {
        if self.max_accessions == 0 {
            return Vec::new();
        }

        let mut new = Vec::new();
        let mut accessions = self.accessions.lock().await;

        for submission in batch {
            if accessions.contains(&submission.accession) {
                continue;
            }
            if accessions.len() >= self.max_accessions {
                accessions.shift_remove_index(0);
            }
            accessions.insert(submission.accession);
            new.push(submission);
        }

        new
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub struct Monitor {
    polling_interval_ms: u64,
    validation_interval_ms: u64,
    start_date: Option<NaiveDate>,
    use_rss: bool,
    use_efts: bool,
    accession_cache: Option<AccessionCache>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            polling_interval_ms: 200,
            validation_interval_ms: 60_000,
            start_date: None,
            use_rss: true,
            use_efts: true,
            accession_cache: None,
        }
    }
}

impl Monitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn polling_interval_ms(mut self, ms: u64) -> Self {
        self.polling_interval_ms = ms;
        self
    }

    pub fn validation_interval_ms(mut self, ms: u64) -> Self {
        self.validation_interval_ms = ms;
        self
    }

    pub fn start_date(mut self, date: NaiveDate) -> Self {
        self.start_date = Some(date);
        self
    }

    pub fn use_rss(mut self, val: bool) -> Self {
        self.use_rss = val;
        self
    }

    pub fn use_efts(mut self, val: bool) -> Self {
        self.use_efts = val;
        self
    }

    pub fn with_cache(mut self, cache: AccessionCache) -> Self {
        self.accession_cache = Some(cache);
        self
    }

    pub fn build(self) -> impl Stream<Item = Vec<Submission>> {
        assert!(
            self.use_rss || self.use_efts,
            "At least one of use_rss or use_efts must be true"
        );
        let accession_cache = self.accession_cache.unwrap_or_default();
        monitor_submissions_stream(
            self.polling_interval_ms,
            self.validation_interval_ms,
            self.start_date,
            self.use_rss,
            self.use_efts,
            accession_cache,
        )
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn monitor_submissions_stream(
    polling_interval_ms: u64,
    validation_interval_ms: u64,
    start_date: Option<NaiveDate>,
    use_rss: bool,
    use_efts: bool,
    accession_cache: AccessionCache,
) -> impl Stream<Item = Vec<Submission>> {
    stream! {
        info!(
            polling_interval_ms,
            validation_interval_ms,
            start_date = ?start_date,
            use_rss,
            use_efts,
            "Starting submission monitor"
        );

        let user_agent = sec_user_agent();
        debug!(%user_agent, "Using SEC user agent");

        let client = Arc::new(
            Client::builder()
                .user_agent(user_agent)
                .build()
                .expect("Failed to build HTTP client"),
        );

        let limiter = Arc::new(RateLimiter::new(polling_interval_ms));

        // Backfill
        if let Some(start) = start_date {
            let end = sec_filing_date_now();
            info!(start = %start, end = %end, "Starting backfill");
            let mut bf = Box::pin(backfill_stream(client.clone(), limiter.clone(), start, end));
            while let Some(batch) = bf.next().await {
                let count = batch.len();
                let new = accession_cache.filter_new(batch).await;
                debug!(source = "backfill", count, new = new.len(), "Filtered submissions");
                if !new.is_empty() {
                    info!(source = "backfill", count = new.len(), "Emitting new submissions");
                    yield new;
                }
            }
            info!("Backfill complete");
        }

        let poll_interval = Duration::from_millis(polling_interval_ms);
        let val_interval = Duration::from_millis(validation_interval_ms);

        // Initialise as already due so first iteration fires immediately
        let mut last_poll = Instant::now().checked_sub(poll_interval).unwrap_or_else(Instant::now);
        let mut last_val = Instant::now().checked_sub(val_interval).unwrap_or_else(Instant::now);

        loop {
            let now = Instant::now();

            // EFTS validation takes priority when due — RSS pauses until complete
            if use_efts && now.duration_since(last_val) >= val_interval {
                let date = sec_filing_date_now();
                debug!(%date, "Running EFTS validation");

                let results = fetch_date(&client, &limiter, date).await;
                let count = results.len();
                let new = accession_cache.filter_new(results).await;
                debug!(source = "efts", count, new = new.len(), "Filtered submissions");
                if !new.is_empty() {
                    info!(source = "efts", count = new.len(), "Emitting new submissions");
                    yield new;
                }

                last_val = Instant::now();
                continue; // re-check timers before next RSS poll
            }

            // RSS poll
            if use_rss && now.duration_since(last_poll) >= poll_interval {
                debug!("Running RSS poll");
                match poll_rss(&client, &limiter).await {
                    Ok(batch) => {
                        let count = batch.len();
                        let new = accession_cache.filter_new(batch).await;
                        debug!(source = "rss", count, new = new.len(), "Filtered submissions");
                        if !new.is_empty() {
                            info!(source = "rss", count = new.len(), "Emitting new submissions");
                            yield new;
                        }
                    }
                    Err(e) => tracing::error!("RSS poll error: {e}"),
                }
                last_poll = Instant::now();
            }

            // Sleep until next scheduled event
            let next = [
                use_rss.then_some(last_poll + poll_interval),
                use_efts.then_some(last_val + val_interval),
            ]
            .into_iter()
            .flatten()
            .min();

            if let Some(wake) = next {
                let now = Instant::now();
                if wake > now {
                    tokio::time::sleep(wake - now).await;
                }
            }
        }
    }
}
