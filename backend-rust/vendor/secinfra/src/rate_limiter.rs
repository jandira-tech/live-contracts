use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, MissedTickBehavior, interval};

pub struct RateLimiter {
    interval: Arc<Mutex<tokio::time::Interval>>,
}

impl RateLimiter {
    pub fn new(interval_ms: u64) -> Self {
        let mut iv = interval(Duration::from_millis(interval_ms));
        iv.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            interval: Arc::new(Mutex::new(iv)),
        }
    }

    pub async fn acquire(&self) {
        self.interval.lock().await.tick().await;
    }
}
