mod classify;
mod config;
mod extract;
mod header;
mod health;
mod images;
mod ingest;
mod markdown;
mod pipeline;

use config::Config;
use health::HealthState;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

async fn run(cfg: Config, state: HealthState) {
    let ua = secinfra::sec_user_agent();
    let client = reqwest::Client::builder()
        .user_agent(&ua)
        .build()
        .expect("reqwest client");

    let monitor = secinfra::Monitor::new()
        .polling_interval_ms(cfg.poll_interval_ms)
        .use_rss(false)
        .use_efts(true)
        .build();

    let mut id_counter: u64 = 0u64;
    use futures::StreamExt;
    let mut stream = std::pin::pin!(monitor);

    loop {
        let batch = match stream.next().await {
            Some(b) => b,
            None => {
                tracing::info!("monitor stream ended");
                break;
            }
        };

        for sub in batch {
            let accession = sub.accession;
            let form = sub.submission_type.clone();
            tracing::debug!("processing {accession} ({form})");

            let records = pipeline::process_submission(&client, &cfg, &mut id_counter, &sub).await;
            if records.is_empty() {
                continue;
            }

            state.total_seen.fetch_add(1, Ordering::Relaxed);

            // POST in chunks
            let chunks = ingest::chunk_rows(&records, cfg.push_batch);
            for chunk in chunks {
                let n = ingest::post_batch(&client, &cfg.ingest_url, &cfg.api_key, chunk).await;
                tracing::info!("ingested {n} records for {accession}");
            }
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = Config::from_env();
    tracing::info!("starting sec-ex10-rust on port {}", cfg.port);

    let total_seen = Arc::new(AtomicU64::new(0));
    let state = HealthState {
        total_seen: total_seen.clone(),
    };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let health_state = state.clone();
        let cfg2 = cfg.clone();
        let health_port = cfg.port;

        // Health server
        let health = tokio::spawn(async move {
            health::serve(health_port, health_state).await;
        });

        // Pipeline (with restart loop)
        let pipeline = tokio::spawn(async move {
            loop {
                tracing::info!("pipeline starting");
                run(cfg2.clone(), state.clone()).await;
                tracing::warn!("pipeline exited, restarting in 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });

        // Wait for Ctrl-C
        tokio::signal::ctrl_c().await.expect("ctrl_c");
        tracing::info!("shutting down");
        health.abort();
        pipeline.abort();
    });
}
