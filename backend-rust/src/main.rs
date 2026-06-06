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
    let mut builder = reqwest::Client::builder().user_agent(&ua);
    if let Some(proxy_url) = cfg.proxy.as_deref() {
        match reqwest::Proxy::all(proxy_url) {
            Ok(p) => {
                tracing::info!("routing SEC fetches through proxy {proxy_url}");
                builder = builder.proxy(p);
            }
            Err(e) => tracing::error!("invalid SEC_PROXY {proxy_url:?}: {e}; ignoring"),
        }
    }
    let client = builder.build().expect("reqwest client");

    // Mimic datamule: RSS (fast, lossy) + EFTS (slower, sweeps up RSS's misses).
    // Both default on via Config; either can be disabled with SEC_USE_RSS /
    // SEC_USE_EFTS. secinfra::Monitor::build() panics if neither is enabled.
    // The accession cache dedups submissions seen across RSS + EFTS so each
    // filing is processed once.
    let monitor = secinfra::Monitor::new()
        .polling_interval_ms(cfg.poll_interval_ms)
        .use_rss(cfg.use_rss)
        .use_efts(cfg.use_efts)
        .with_cache(secinfra::AccessionCache::new(cfg.accession_cache_size))
        .build();

    // Shared echo-token counter (D1 mints the real UUIDv7, so we only need
    // per-process uniqueness — safe to share across concurrent tasks).
    let id_counter = Arc::new(AtomicU64::new(0));

    // Proactive global pace: even with `cfg.concurrency` workers in flight, keep
    // fetch *starts* at/under cfg.max_rps so we don't machine-gun SEC (it caps
    // clients at 10/s). The reactive 429 backoff in fetch_sgml handles the rest.
    let min_fetch_interval =
        std::time::Duration::from_nanos(1_000_000_000 / cfg.max_rps.max(1));
    let fetch_gate = Arc::new(tokio::sync::Mutex::new(None::<std::time::Instant>));

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

        // Process the batch concurrently (bounded by cfg.concurrency). Each task
        // POSTs its own records; order across submissions may interleave.
        futures::stream::iter(batch)
            .map(|sub| {
                // Clone the cheap handles into the future so each one is `'static`
                // (reqwest::Client is an Arc internally; Config/HealthState/the
                // counter are Arc/cheap). This is what lets process_submission use
                // spawn_blocking without borrowing the surrounding scope.
                let client = client.clone();
                let cfg = cfg.clone();
                let state = state.clone();
                let id_counter = id_counter.clone();
                let store = store.clone();
                let fetch_gate = fetch_gate.clone();
                async move {
                    let accession = sub.accession;
                    let form = sub.submission_type.clone();
                    tracing::debug!(size_bytes = ?sub.size_bytes, "processing {accession} ({form})");

                    // Global rate gate: wait until min_fetch_interval has elapsed
                    // since the last worker's fetch start, then claim this slot.
                    {
                        let mut last = fetch_gate.lock().await;
                        if let Some(prev) = *last {
                            let elapsed = prev.elapsed();
                            if elapsed < min_fetch_interval {
                                tokio::time::sleep(min_fetch_interval - elapsed).await;
                            }
                        }
                        *last = Some(std::time::Instant::now());
                    }

                    let p = pipeline::process_submission(&client, &cfg, &id_counter, &sub).await;
                    if p.is_empty() {
                        return;
                    }

                    if let Some(store) = &store {
                        // Stateful mode (default when SEC_STORE_PATH is set): persist
                        // EX-10 + other exhibits, then mark the accession seen only
                        // after the writes land so a failed insert stays retryable.
                        let mut writes_ok = true;
                        for r in &p.ex10 {
                            if let Err(e) = store.upsert_ex10(r) {
                                tracing::warn!("store upsert_ex10 failed for {}: {e}", p.accession);
                                writes_ok = false;
                            }
                        }
                        for r in &p.others {
                            if let Err(e) = store.insert_all_exhibit(r) {
                                tracing::warn!(
                                    "store insert_all_exhibit failed for {}: {e}",
                                    p.accession
                                );
                                writes_ok = false;
                            }
                        }
                        if writes_ok
                            && let Err(e) = store.mark_seen(&p.accession, &p.form_type, &p.cik) {
                                tracing::warn!("store mark_seen failed for {}: {e}", p.accession);
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
                        // Stateless mode: POST EX-10 records straight to /api/ingest.
                        if p.ex10.is_empty() {
                            return;
                        }
                        state.total_seen.fetch_add(1, Ordering::Relaxed);
                        let chunks = ingest::chunk_rows(&p.ex10, cfg.push_batch);
                        for chunk in chunks {
                            let n = ingest::post_batch(
                                &client,
                                &cfg.ingest_url,
                                &cfg.api_key,
                                chunk,
                            )
                            .await;
                            tracing::info!("ingested {n} records for {accession}");
                        }
                    }
                }
            })
            .buffer_unordered(cfg.concurrency)
            .for_each(|()| async {})
            .await;
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

    // Opt-in stateful mode (default when SEC_STORE_PATH is set): open the local
    // SQLite store; unset keeps the lean stateless inline-POST producer.
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
