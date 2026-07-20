use crate::decision::Decision;
use std::time::Duration;

pub mod gcra;
pub mod token_bucket;

pub trait Algorithm: Send + Sync {
    /// Small, `Copy` per-key state (memory footprint matters — FR-071).
    type State: Copy + Send + 'static;

    fn initial(&self, now: Duration) -> Self::State;

    /// PURE: no I/O, no clock reads. Given state + now + cost → new state + decision.
    fn step(&self, state: Self::State, now: Duration, cost: u64) -> (Self::State, Decision);
}

/// Convert integer nanoseconds to a [`Duration`], saturating rather than
/// panicking on overflow. Shared by the algorithms so all schedule arithmetic
/// stays in exact integer units (no floating-point drift — ADR-005/017).
pub(crate) fn nanos_to_duration(nanos: u128) -> Duration {
    const NANOS_PER_SEC: u128 = 1_000_000_000;
    let secs = (nanos / NANOS_PER_SEC).min(u64::MAX as u128) as u64;
    let subsec = (nanos % NANOS_PER_SEC) as u32;
    Duration::new(secs, subsec)
}
