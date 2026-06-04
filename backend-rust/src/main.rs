mod classify;
mod config;
mod extract;
mod header;
mod health;
mod images;
mod ingest;
mod markdown;
mod pipeline;
mod store;

use config::Config;
use health::HealthState;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

async fn run(cfg: Config, state: HealthState, store: Option<Arc<store::Store>>) {
    let ua = secinfra::sec_user_agent();
    let client = reqwest::Client::builder()
        .user_agent(&ua)
        .build()
        .expect("reqwest client");

    // Mimic datamule: RSS (fast, lossy) + EFTS (slower, sweeps up RSS's misses).
    // Both default on via Config; either can be disabled with SEC_USE_RSS /
    // SEC_USE_EFTS. secinfra::Monitor::build() panics if neither is enabled.
    let monitor = secinfra::Monitor::new()
        .polling_interval_ms(cfg.poll_interval_ms)
        .use_rss(cfg.use_rss)
        .use_efts(cfg.use_efts)
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

            let p = pipeline::process_submission(&client, &cfg, &mut id_counter, &sub).await;
            if p.is_empty() {
                continue;
            }

            if let Some(store) = &store {
                // Stateful mode: persist to the local store (like the Python
                // worker); the D1 push happens from the store in a later PR.
                if let Err(e) = store.mark_seen(&p.accession, &p.form_type, &p.cik) {
                    tracing::warn!("store mark_seen failed for {}: {e}", p.accession);
                }
                for r in &p.ex10 {
                    if let Err(e) = store.upsert_ex10(r) {
                        tracing::warn!("store upsert_ex10 failed for {}: {e}", p.accession);
                    }
                }
                for r in &p.others {
                    if let Err(e) = store.insert_all_exhibit(r) {
                        tracing::warn!("store insert_all_exhibit failed for {}: {e}", p.accession);
                    }
                }
                if !p.ex10.is_empty() {
                    state.total_seen.fetch_add(1, Ordering::Relaxed);
                }
                tracing::info!(
                    "stored {} EX-10 + {} other exhibits for {}",
                    p.ex10.len(),
                    p.others.len(),
                    p.accession
                );
            } else {
                // Stateless mode: POST EX-10 records straight to the ingest route.
                if p.ex10.is_empty() {
                    continue;
                }
                state.total_seen.fetch_add(1, Ordering::Relaxed);
                let chunks = ingest::chunk_rows(&p.ex10, cfg.push_batch);
                for chunk in chunks {
                    let n = ingest::post_batch(&client, &cfg.ingest_url, &cfg.api_key, chunk).await;
                    tracing::info!("ingested {n} records for {}", p.accession);
                }
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
    // Fail fast on an unusable discovery config rather than panicking the spawned
    // pipeline task (secinfra::Monitor::build asserts at least one source on).
    if let Err(e) = cfg.validate() {
        tracing::error!("invalid configuration: {e}");
        std::process::exit(2);
    }
    tracing::info!("starting sec-ex10-rust on port {}", cfg.port);

    // Opt-in stateful mode: open the local SQLite store (fail fast on a bad path).
    // When set, the pipeline persists every exhibit to the store instead of
    // POSTing EX-10 inline; unset keeps the lean stateless producer.
    let store: Option<Arc<store::Store>> = match cfg.store_path.as_deref() {
        Some(path) => match store::Store::open(path).and_then(|s| {
            s.init()?;
            s.count_ex10().map(|n| (s, n))
        }) {
            Ok((s, n)) => {
                tracing::info!("stateful mode: local store at {path} ({n} EX-10 rows)");
                Some(Arc::new(s))
            }
            Err(e) => {
                tracing::error!("failed to open local store at {path}: {e}");
                std::process::exit(2);
            }
        },
        None => None,
    };

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
                run(cfg2.clone(), state.clone(), store.clone()).await;
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
