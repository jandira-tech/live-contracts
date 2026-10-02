mod classify;
mod config;
mod cooldown;
mod delivery;
mod extract;
mod header;
mod health;
mod images;
mod ingest;
mod markdown;
mod outbox;
mod pipeline;
mod preflight;
mod store;
#[cfg(test)]
mod test_support;

use config::Config;
use cooldown::Cooldown;
use delivery::{Delivery, PUSH_BACKOFF};
use health::HealthState;
use outbox::Outbox;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// How often the outbox is retried.
const DRAIN_EVERY: Duration = Duration::from_secs(60);

fn http_client(cfg: &Config) -> reqwest::Client {
    // secinfra's discovery requests read SEC_USER_AGENT themselves; validate()
    // has already refused to start without a real one.
    let mut builder = reqwest::Client::builder()
        .user_agent(&cfg.user_agent)
        .timeout(Duration::from_secs(120));
    if let Some(proxy_url) = cfg.proxy.as_deref() {
        match reqwest::Proxy::all(proxy_url) {
            Ok(p) => {
                tracing::info!("routing SEC fetches through proxy {proxy_url}");
                builder = builder.proxy(p);
            }
            Err(e) => tracing::error!("invalid SEC_PROXY {proxy_url:?}: {e}; ignoring"),
        }
    }
    builder.build().expect("reqwest client")
}

struct Shared {
    outbox: Arc<Outbox>,
    cooldown: Arc<Cooldown>,
}

async fn drain_loop(client: reqwest::Client, cfg: Config, shared: Arc<Shared>, state: HealthState) {
    loop {
        let d = Delivery {
            client: &client,
            cfg: &cfg,
            outbox: &shared.outbox,
            cooldown: &shared.cooldown,
            backoff: PUSH_BACKOFF,
        };
        let r = d.drain_once().await;
        state.outbox_pending.store(r.kept as u64, Ordering::Relaxed);
        state.rows_accepted.fetch_add(r.delivered as u64, Ordering::Relaxed);
        if r.delivered > 0 || r.kept > 0 {
            tracing::info!("outbox drain: {} delivered, {} still queued", r.delivered, r.kept);
        }
        tokio::time::sleep(DRAIN_EVERY).await;
    }
}

async fn run(cfg: Config, state: HealthState, store: Option<Arc<store::Store>>, shared: Arc<Shared>) {
    let client = http_client(&cfg);

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
                let shared = shared.clone();
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

                    // One bad filing must not take the batch down with it.
                    let label = format!("filing {accession}");
                    let Some(p) = preflight::isolate(
                        &label,
                        pipeline::process_submission(&client, &cfg, &id_counter, &sub),
                    )
                    .await
                    else {
                        state.item_failures.fetch_add(1, Ordering::Relaxed);
                        return;
                    };
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
                        // Stateless mode: POST EX-10 records to /api/ingest; whatever
                        // is not accepted waits in the outbox for the drain loop.
                        if p.ex10.is_empty() {
                            return;
                        }
                        state.total_seen.fetch_add(1, Ordering::Relaxed);
                        let d = Delivery {
                            client: &client,
                            cfg: &cfg,
                            outbox: &shared.outbox,
                            cooldown: &shared.cooldown,
                            backoff: PUSH_BACKOFF,
                        };
                        let n = d.deliver(&p.ex10).await;
                        state.rows_accepted.fetch_add(n as u64, Ordering::Relaxed);
                        tracing::info!("ingested {n} of {} records for {accession}", p.ex10.len());
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
    match std::env::args().nth(1).as_deref() {
        None | Some("run") => {}
        Some("smoke") => std::process::exit(report(&preflight::smoke(&cfg))),
        Some("preflight") => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let checks = rt.block_on(preflight::preflight(&http_client(&cfg), &cfg, preflight::SEC_PROBE_URL));
            std::process::exit(report(&checks));
        }
        Some(other) => {
            eprintln!("unknown command {other:?}; expected run, smoke or preflight");
            std::process::exit(64);
        }
    }
    // Fail closed: no key, no declared SEC identity, or no discovery source
    // means no useful work, so refuse to start (secinfra::Monitor::build would
    // also panic the spawned pipeline task on the last one).
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

    let outbox = match Outbox::open(&cfg.outbox_path) {
        Ok(o) => Arc::new(o),
        Err(e) => {
            tracing::error!("cannot open outbox at {}: {e}", cfg.outbox_path);
            std::process::exit(2);
        }
    };
    let shared = Arc::new(Shared {
        outbox,
        cooldown: Arc::new(Cooldown::new(Duration::from_secs(cfg.push_cooldown_secs))),
    });

    let state = HealthState::default();

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let health_state = state.clone();
        let cfg2 = cfg.clone();
        let health_port = cfg.port;

        // Health server
        let health = tokio::spawn(async move {
            health::serve(health_port, health_state).await;
        });

        let drain = tokio::spawn(drain_loop(http_client(&cfg), cfg.clone(), shared.clone(), state.clone()));

        // Pipeline (with restart loop)
        let pipeline = tokio::spawn(async move {
            loop {
                tracing::info!("pipeline starting");
                run(cfg2.clone(), state.clone(), store.clone(), shared.clone()).await;
                tracing::warn!("pipeline exited, restarting in 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });

        // Wait for Ctrl-C
        tokio::signal::ctrl_c().await.expect("ctrl_c");
        tracing::info!("shutting down");
        health.abort();
        drain.abort();
        pipeline.abort();
    });
}

/// Print checks one per line; exit code 0 when all pass, 1 otherwise.
fn report(checks: &[preflight::Check]) -> i32 {
    for c in checks {
        println!("{} {:<7} {}", if c.ok { "ok  " } else { "FAIL" }, c.name, c.detail);
    }
    i32::from(!checks.iter().all(|c| c.ok))
}
