//! Cooldown for the ingest endpoint: after it answers 429/503, stop calling it for
//! a while and let batches go straight to the outbox. Same idea as the `CooledModel`
//! arm in auto-apps-2026 PR #15, applied to the one endpoint this producer talks to.
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Cooldown {
    period: Duration,
    until: Mutex<Option<Instant>>,
}

impl Cooldown {
    pub fn new(period: Duration) -> Self {
        Self { period, until: Mutex::new(None) }
    }

    /// Start (or restart) the cooldown from now.
    pub fn trip(&self) {
        *self.until.lock().unwrap() = Some(Instant::now() + self.period);
    }

    /// Time left, or None when the endpoint may be called.
    pub fn remaining(&self) -> Option<Duration> {
        let until = (*self.until.lock().unwrap())?;
        let left = until.saturating_duration_since(Instant::now());
        (!left.is_zero()).then_some(left)
    }

    pub fn is_cooling(&self) -> bool {
        self.remaining().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_open() {
        let c = Cooldown::new(Duration::from_secs(300));
        assert!(!c.is_cooling());
        assert_eq!(c.remaining(), None);
    }

    #[test]
    fn trip_closes_it_for_the_period_then_reopens() {
        let c = Cooldown::new(Duration::from_millis(40));
        c.trip();
        assert!(c.is_cooling());
        assert!(c.remaining().unwrap() <= Duration::from_millis(40));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!c.is_cooling());
    }

    #[test]
    fn zero_period_never_cools() {
        let c = Cooldown::new(Duration::ZERO);
        c.trip();
        assert!(!c.is_cooling());
    }
}
