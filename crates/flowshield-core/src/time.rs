use std::time::Duration;

/// A monotonic, injectable source of time (ADR-017).
///
/// Algorithms measure *elapsed* time, never wall-clock time, so the value is a
/// non-decreasing `Duration` since a fixed opaque origin.
pub trait TimeSource: Send + Sync {
    fn now(&self) -> Duration;
}

/// Production clock backed by `std::time::Instant` (monotonic).
#[derive(Debug)]
pub struct MonotonicClock {
    origin: std::time::Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}

impl TimeSource for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// Deterministic clock for tests — advance it by hand.
#[derive(Debug, Default)]
pub struct TestClock {
    nanos: std::sync::atomic::AtomicU64,
}

impl TestClock {
    pub fn advance(&self, by: Duration) {
        use std::sync::atomic::Ordering;
        self.nanos
            .fetch_add(by.as_nanos() as u64, Ordering::Relaxed);
    }
}
impl TimeSource for TestClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.nanos.load(std::sync::atomic::Ordering::Relaxed))
    }
}

/// Blanket impl so a shared clock (e.g. `Arc<TestClock>`) can be handed to a limiter
/// while the test still holds a handle to advance it.
impl<T: TimeSource + ?Sized> TimeSource for std::sync::Arc<T> {
    fn now(&self) -> Duration {
        (**self).now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn advance_moves_time_forward() {
        let clock = TestClock::default();

        assert_eq!(clock.now(), Duration::ZERO);

        clock.advance(Duration::from_secs(5));

        assert_eq!(clock.now(), Duration::from_secs(5));
    }

    #[test]
    fn advance_accumulates() {
        let clock = TestClock::default();

        clock.advance(Duration::from_secs(2));
        clock.advance(Duration::from_secs(3));

        assert_eq!(clock.now(), Duration::from_secs(5));
    }
}
