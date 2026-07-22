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

    /// A short, static, low-cardinality name used as the `algorithm` label on
    /// telemetry (ADR-012), e.g. `"token_bucket"`. This is a **provided** method so
    /// adding it does not break existing out-of-crate implementations (ADR-013); it
    /// defaults to `"unknown"` and concrete algorithms override it. It must never
    /// encode per-key or otherwise high-cardinality data.
    fn name(&self) -> &'static str {
        "unknown"
    }

    /// The minimum time per-key state must **persist** for the limit to remain in force —
    /// i.e. how long until an idle key's state decays back to its initial (fully-permissive)
    /// value. If a store expires state sooner, an idle client's next request is seeded fresh
    /// via [`initial`](Self::initial) and the limit is silently bypassed (a client resets its
    /// allowance merely by pausing). [`LimiterBuilder`] uses this to pick a safe **default**
    /// TTL so the out-of-the-box configuration cannot be bypassed that way.
    ///
    /// A **provided** method (added without breaking existing out-of-crate impls, ADR-013):
    /// it defaults to [`Duration::ZERO`] (no coupling; the builder then falls back to its
    /// own default TTL). Concrete algorithms override it — e.g. Token Bucket returns the
    /// time to refill from empty to full, GCRA the maximum a schedule can sit ahead.
    ///
    /// [`LimiterBuilder`]: crate::limiter::LimiterBuilder
    fn min_ttl(&self) -> Duration {
        Duration::ZERO
    }
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
