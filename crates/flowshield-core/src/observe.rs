//! Internal observability hooks (ADR-012).
//!
//! Every emission in this module is behind a `#[cfg(feature = …)]`. With both
//! `metrics` and `tracing` disabled — the default — the functions here are empty
//! and the arguments they would compute (timestamps, hashes, label lookups) are
//! never evaluated, so the decision path pays negligible overhead (NFR-023).
//!
//! Observability is emitted from exactly one place, the limiter's orchestration
//! layer (ADR-002), so that every algorithm × backend combination produces the
//! same, uniformly-labelled signals without instrumenting each one.
//!
//! # Signals (ADR-012)
//!
//! | Metric | Type | Labels |
//! |---|---|---|
//! | `flowshield_decisions_total` | counter | `outcome`, `algorithm`, `backend` |
//! | `flowshield_backend_duration_seconds` | histogram | `algorithm`, `backend` |
//! | `flowshield_degradations_total` | counter | `policy`, `algorithm`, `backend` |
//!
//! Labels are always low-cardinality (`&'static str`); the rate-limit **key is
//! never a label**. Keys appear only on `tracing` spans, and there only in
//! redacted form (see [`redact`]).

// These appear in the function signatures below regardless of features, so they
// are imported unconditionally; the *bodies* that use their contents are gated.
use crate::decision::Decision;
use crate::error::Error;
use crate::limiter::DegradationPolicy;

/// Static, low-cardinality labels identifying the limiter that produced a signal.
///
/// Computed once at [`build`](crate::limiter::LimiterBuilder::build) time from the
/// algorithm's [`name`](crate::algorithm::Algorithm::name) and the backend's
/// [`kind`](crate::store::StorageBackend::kind), then copied onto each signal.
#[derive(Debug, Clone, Copy)]
// The fields are read only by feature-gated emission below; with both features off
// the struct is still constructed and threaded through, but its fields go unread.
#[cfg_attr(not(any(feature = "metrics", feature = "tracing")), allow(dead_code))]
pub(crate) struct Labels {
    pub algorithm: &'static str,
    pub backend: &'static str,
}

/// A backend round-trip stopwatch that only exists when `metrics` is enabled.
///
/// When the feature is off this is a zero-sized type whose methods are no-ops, so
/// no clock is read on the decision path at all — the timing itself is part of the
/// "zero cost when disabled" contract, not just its reporting.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timer {
    #[cfg(feature = "metrics")]
    start: std::time::Instant,
}

impl Timer {
    #[inline]
    pub(crate) fn start() -> Self {
        Self {
            #[cfg(feature = "metrics")]
            start: std::time::Instant::now(),
        }
    }

    #[cfg(feature = "metrics")]
    #[inline]
    fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

/// Record the outcome of a decision: a counter increment (`metrics`) and, when a
/// decision span is active, an event carrying the outcome (`tracing`).
#[inline]
pub(crate) fn record_decision(_labels: Labels, _decision: &Decision) {
    #[cfg(any(feature = "metrics", feature = "tracing"))]
    {
        let outcome = if _decision.is_allowed() {
            "allowed"
        } else {
            "rejected"
        };

        #[cfg(feature = "metrics")]
        metrics::counter!(
            "flowshield_decisions_total",
            "outcome" => outcome,
            "algorithm" => _labels.algorithm,
            "backend" => _labels.backend,
        )
        .increment(1);

        #[cfg(feature = "tracing")]
        tracing::trace!(
            outcome,
            algorithm = _labels.algorithm,
            backend = _labels.backend,
            "rate-limit decision"
        );
    }
}

/// Record the backend round-trip latency as a histogram sample (`metrics` only).
#[inline]
pub(crate) fn record_backend_latency(_labels: Labels, _timer: Timer) {
    #[cfg(feature = "metrics")]
    metrics::histogram!(
        "flowshield_backend_duration_seconds",
        "algorithm" => _labels.algorithm,
        "backend" => _labels.backend,
    )
    .record(_timer.elapsed_secs());
}

/// Record that the backend failed and a degradation fallback was applied: a counter
/// increment (`metrics`) and a warn-level event (`tracing`). This is the one truly
/// "significant event" on the path (FR-051) — a limiter running on its fallback
/// policy rather than real state — so it is logged at `warn`, not `trace`.
#[inline]
pub(crate) fn record_degradation(_labels: Labels, _error: &Error, _policy: DegradationPolicy) {
    #[cfg(any(feature = "metrics", feature = "tracing"))]
    {
        let policy = _policy.telemetry_name();

        #[cfg(feature = "metrics")]
        metrics::counter!(
            "flowshield_degradations_total",
            "policy" => policy,
            "algorithm" => _labels.algorithm,
            "backend" => _labels.backend,
        )
        .increment(1);

        #[cfg(feature = "tracing")]
        tracing::warn!(
            policy,
            algorithm = _labels.algorithm,
            backend = _labels.backend,
            error = %_error,
            "backend unavailable; applied degradation fallback"
        );
    }
}

/// A non-reversible, low-cardinality descriptor for a key, safe to attach to a
/// `tracing` span (ADR-012, FR-073).
///
/// Redaction is the **default and only** behaviour in v1: a raw key — often an IP,
/// user id, or API token — must never leak into telemetry. The digest is a 64-bit
/// FNV-1a hash rendered as hex; it is stable within a running process (so the same
/// key correlates across spans) but cannot be reversed to the original.
///
/// This is only compiled with the `tracing` feature, because a key reaches a sink
/// only through a span; metrics never carry it.
#[cfg(feature = "tracing")]
pub(crate) fn redact(key: &crate::key::Key) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in key.as_str().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("k#{hash:016x}")
}

#[cfg(all(test, feature = "tracing"))]
mod redaction_tests {
    use super::redact;
    use crate::key::Key;

    #[test]
    fn digest_never_contains_the_raw_key() {
        let secret = "192.168.1.7:super-secret-token";
        let digest = redact(&Key::from(secret));
        assert!(
            !digest.contains("super-secret-token"),
            "redacted digest must not leak key material: {digest}"
        );
        assert!(digest.starts_with("k#"));
    }

    #[test]
    fn digest_is_stable_and_distinguishing() {
        let a = redact(&Key::from("alice"));
        let b = redact(&Key::from("bob"));
        assert_eq!(a, redact(&Key::from("alice")), "same key → same digest");
        assert_ne!(a, b, "distinct keys → distinct digests");
    }
}
