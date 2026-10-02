//! Getting rows into D1 without losing them: push with a retry budget, and when
//! that fails, park the rows in the outbox for the drain loop.
use crate::config::Config;
use crate::cooldown::Cooldown;
use crate::ingest::{IngestRecord, MAX_BATCH_BYTES, PushOutcome, chunk_by_size, fit_for_d1, push_with_budget};
use crate::outbox::Outbox;
use std::time::Duration;

/// Backoff before the second attempt; doubles each time after that.
pub const PUSH_BACKOFF: Duration = Duration::from_secs(2);

pub struct Delivery<'a> {
    pub client: &'a reqwest::Client,
    pub cfg: &'a Config,
    pub outbox: &'a Outbox,
    pub cooldown: &'a Cooldown,
    pub backoff: Duration,
}

#[derive(Debug, Default, PartialEq)]
pub struct DrainReport {
    pub delivered: usize,
    pub kept: usize,
}

impl Delivery<'_> {
    /// Push `rows` in ≤push_batch chunks. Returns how many D1 accepted; every row
    /// not accepted is in the outbox when this returns.
    pub async fn deliver(&self, rows: &[IngestRecord]) -> usize {
        let rows: Vec<IngestRecord> = rows.iter().cloned().map(fit_for_d1).collect();
        let mut accepted = 0;
        for chunk in chunk_by_size(&rows, self.cfg.push_batch, MAX_BATCH_BYTES) {
            match self.push(chunk).await {
                PushOutcome::Accepted(n) => accepted += n,
                PushOutcome::Retry { reason, .. } | PushOutcome::Rejected { reason, .. } => {
                    match self.outbox.put(chunk, &reason) {
                        Ok(n) => tracing::warn!("{n} rows parked in the outbox: {reason}"),
                        // The one way a row is lost: the outbox itself is unwritable.
                        Err(e) => tracing::error!(
                            "LOST {} rows: push failed ({reason}) and outbox write failed: {e}",
                            chunk.len()
                        ),
                    }
                }
            }
        }
        accepted
    }

    async fn push(&self, rows: &[IngestRecord]) -> PushOutcome {
        push_with_budget(
            self.client,
            &self.cfg.ingest_url,
            &self.cfg.api_key,
            rows,
            self.cfg.push_retries,
            self.backoff,
            self.cooldown,
        )
        .await
    }

    /// One pass over the outbox, oldest first, one chunk at a time. Stops at the
    /// first chunk that is not accepted, so a down endpoint costs one call a pass.
    pub async fn drain_once(&self) -> DrainReport {
        let mut report = DrainReport::default();
        loop {
            if self.cooldown.is_cooling() {
                break;
            }
            let batch = match self.outbox.peek(self.cfg.push_batch.max(1)) {
                Ok(b) if b.is_empty() => break,
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("outbox read failed: {e}");
                    break;
                }
            };
            // Fit rows to D1 (rows parked before this existed may be oversized), and send
            // only as many as fit one POST; the rest wait for the next loop turn.
            let fitted: Vec<(i64, IngestRecord)> =
                batch.into_iter().map(|(id, r)| (id, fit_for_d1(r))).collect();
            let take = chunk_by_size(
                &fitted.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
                self.cfg.push_batch,
                MAX_BATCH_BYTES,
            )[0]
            .len();
            let (ids, rows): (Vec<i64>, Vec<IngestRecord>) = fitted.into_iter().take(take).unzip();
            match self.push(&rows).await {
                PushOutcome::Accepted(_) => {
                    if let Err(e) = self.outbox.ack(&ids) {
                        // Rows stay queued and are re-sent; the route upserts, so no harm.
                        tracing::error!("outbox ack failed: {e}");
                        break;
                    }
                    report.delivered += ids.len();
                }
                PushOutcome::Retry { reason, .. } | PushOutcome::Rejected { reason, .. } => {
                    if let Err(e) = self.outbox.note_failure(&ids, &reason) {
                        tracing::error!("outbox update failed: {e}");
                    }
                    break;
                }
            }
        }
        report.kept = self.outbox.len().map(|n| n as usize).unwrap_or(0);
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ingest_server;

    fn cfg(url: &str, batch: usize) -> Config {
        let mut c = Config::from_map(|k| match k {
            "SEC_API_KEY" => Some("k".into()),
            "SEC_USER_AGENT" => Some("Test Co test@arthur.law".into()),
            _ => None,
        });
        c.ingest_url = url.into();
        c.push_batch = batch;
        c.push_retries = 2;
        c
    }

    fn rows(n: u64) -> Vec<IngestRecord> {
        (0..n)
            .map(|i| {
                let mut r = crate::outbox::tests_rec(&format!("A{i}"), "x.htm");
                r.id = i;
                r
            })
            .collect()
    }

    #[tokio::test]
    async fn accepted_rows_never_touch_the_outbox() {
        let (url, script) = ingest_server(&[]).await;
        let (c, o, cd) = (cfg(&url, 2), Outbox::open(":memory:").unwrap(), Cooldown::new(Duration::from_secs(300)));
        let client = reqwest::Client::new();
        let d = Delivery { client: &client, cfg: &c, outbox: &o, cooldown: &cd, backoff: Duration::from_millis(1) };
        assert_eq!(d.deliver(&rows(5)).await, 5);
        assert_eq!(script.lock().unwrap().calls, 3, "chunks of push_batch");
        assert_eq!(o.len().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_failed_chunk_is_parked_not_dropped() {
        // First chunk exhausts its budget (2 x 500); the second chunk goes through.
        let (url, _) = ingest_server(&[500, 500]).await;
        let (c, o, cd) = (cfg(&url, 2), Outbox::open(":memory:").unwrap(), Cooldown::new(Duration::from_secs(300)));
        let client = reqwest::Client::new();
        let d = Delivery { client: &client, cfg: &c, outbox: &o, cooldown: &cd, backoff: Duration::from_millis(1) };
        assert_eq!(d.deliver(&rows(4)).await, 2);
        assert_eq!(o.len().unwrap(), 2);
    }

    #[tokio::test]
    async fn a_rejected_key_parks_every_row_for_after_the_fix() {
        let (url, _) = ingest_server(&[401, 401]).await;
        let (c, o, cd) = (cfg(&url, 2), Outbox::open(":memory:").unwrap(), Cooldown::new(Duration::from_secs(300)));
        let client = reqwest::Client::new();
        let d = Delivery { client: &client, cfg: &c, outbox: &o, cooldown: &cd, backoff: Duration::from_millis(1) };
        assert_eq!(d.deliver(&rows(4)).await, 0);
        assert_eq!(o.len().unwrap(), 4);
    }

    #[tokio::test]
    async fn drain_delivers_the_backlog_oldest_first() {
        let (url, script) = ingest_server(&[]).await;
        let (c, o, cd) = (cfg(&url, 2), Outbox::open(":memory:").unwrap(), Cooldown::new(Duration::from_secs(300)));
        o.put(&rows(5), "earlier 401").unwrap();
        let client = reqwest::Client::new();
        let d = Delivery { client: &client, cfg: &c, outbox: &o, cooldown: &cd, backoff: Duration::from_millis(1) };
        assert_eq!(d.drain_once().await, DrainReport { delivered: 5, kept: 0 });
        assert_eq!(o.len().unwrap(), 0);
        assert_eq!(script.lock().unwrap().rows_seen, 5);
    }

    #[tokio::test]
    async fn drain_stops_at_the_first_failure_and_keeps_the_rest() {
        let (url, script) = ingest_server(&[401]).await;
        let (c, o, cd) = (cfg(&url, 2), Outbox::open(":memory:").unwrap(), Cooldown::new(Duration::from_secs(300)));
        o.put(&rows(5), "x").unwrap();
        let client = reqwest::Client::new();
        let d = Delivery { client: &client, cfg: &c, outbox: &o, cooldown: &cd, backoff: Duration::from_millis(1) };
        assert_eq!(d.drain_once().await, DrainReport { delivered: 0, kept: 5 });
        assert_eq!(script.lock().unwrap().calls, 1);
    }

    #[tokio::test]
    async fn drain_does_nothing_while_cooling() {
        let (url, script) = ingest_server(&[]).await;
        let (c, o, cd) = (cfg(&url, 2), Outbox::open(":memory:").unwrap(), Cooldown::new(Duration::from_secs(300)));
        o.put(&rows(3), "x").unwrap();
        cd.trip();
        let client = reqwest::Client::new();
        let d = Delivery { client: &client, cfg: &c, outbox: &o, cooldown: &cd, backoff: Duration::from_millis(1) };
        assert_eq!(d.drain_once().await, DrainReport { delivered: 0, kept: 3 });
        assert_eq!(script.lock().unwrap().calls, 0);
    }

    #[tokio::test]
    async fn one_oversized_exhibit_no_longer_sinks_its_neighbours() {
        let (url, script) = ingest_server(&[]).await;
        let (c, o, cd) = (cfg(&url, 100), Outbox::open(":memory:").unwrap(), Cooldown::new(Duration::from_secs(300)));
        let client = reqwest::Client::new();
        let d = Delivery { client: &client, cfg: &c, outbox: &o, cooldown: &cd, backoff: Duration::from_millis(1) };
        let mut rs = rows(3);
        rs[0].markdown = "x".repeat(3_700_000);
        assert_eq!(d.deliver(&rs).await, 3);
        assert_eq!(o.len().unwrap(), 0);
        let _ = script;
    }

    #[tokio::test]
    async fn drain_fits_oversized_rows_already_in_the_outbox() {
        let (url, script) = ingest_server(&[]).await;
        let (c, o, cd) = (cfg(&url, 100), Outbox::open(":memory:").unwrap(), Cooldown::new(Duration::from_secs(300)));
        let mut rs = rows(2);
        rs[1].markdown = "x".repeat(3_700_000);
        o.put(&rs, "status 500").unwrap();
        let client = reqwest::Client::new();
        let d = Delivery { client: &client, cfg: &c, outbox: &o, cooldown: &cd, backoff: Duration::from_millis(1) };
        assert_eq!(d.drain_once().await, DrainReport { delivered: 2, kept: 0 });
        let _ = script;
    }
}
