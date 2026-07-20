//! Property-based conformance suite for the shipped algorithms (ADR-008).
//!
//! These tests assert the *behavioural contract* every rate-limiting algorithm
//! must honour, over randomised arrival patterns rather than hand-picked cases:
//!
//! - **Safety (never over-admit).** A limiter with burst `σ` and one token per
//!   interval `T` admits at most `σ + L/T` requests over any span `L` — the
//!   classic (σ, ρ) leaky-bucket bound. This is the property that guarantees the
//!   configured rate is actually enforced.
//! - **Liveness (never over-restrict).** If requests are spaced at least one
//!   interval apart, every one is admitted — the limiter does not spuriously
//!   reject conforming traffic.
//! - **Retry contract.** A rejection's `retry_after` is strictly positive and,
//!   waited out with no competing traffic, is *sufficient*: the retried request
//!   is then admitted (FR-011, ADR-017 #4).
//! - **Determinism.** Identical inputs yield identical decisions — the
//!   reproducibility guarantee the integer-state design exists to provide
//!   (ADR-005/017). A regression to floating-point or ambient time would break
//!   this.
//!
//! The algorithms are pure ([`Algorithm::step`] takes `now` explicitly), so the
//! suite drives them directly with synthetic time — no clock, store, or runtime.
//!
//! **On generator design.** Arrival gaps are drawn *relative to each run's
//! emission interval* (in thousandths of `T`), not as raw nanoseconds. A raw gap
//! is a burst at a fast rate but a long idle at a slow one; scaling to `T` gives
//! every rate the same burst pressure, so the safety bound stays tight and an
//! over-admission bug can't hide behind an inflated span. (Verified by mutation
//! testing: a phantom-refill bug and a GCRA under-advance are both caught.)

use flowshield_core::algorithm::{Algorithm, gcra::Gcra, token_bucket::TokenBucket};
use flowshield_core::decision::Decision;
use proptest::prelude::*;
use std::time::Duration;

const NANOS_PER_SEC: u128 = 1_000_000_000;

fn nanos(n: u128) -> Duration {
    Duration::new((n / NANOS_PER_SEC) as u64, (n % NANOS_PER_SEC) as u32)
}

/// Turn a list of nanosecond gaps into monotonically non-decreasing arrival
/// instants (a cumulative sum) from the origin. A gap of 0 means a simultaneous
/// arrival — a burst at the same instant.
fn arrivals_from_gaps(gaps: &[u64]) -> Vec<Duration> {
    let mut t = 0u128;
    gaps.iter()
        .map(|&g| {
            t += g as u128;
            nanos(t)
        })
        .collect()
}

/// Drive a pure algorithm over unit-cost arrivals, threading state, and collect
/// the per-request decisions.
fn run<A: Algorithm>(algo: &A, arrivals: &[Duration]) -> Vec<Decision> {
    let mut state = algo.initial(Duration::ZERO);
    arrivals
        .iter()
        .map(|&now| {
            let (next, decision) = algo.step(state, now, 1);
            state = next;
            decision
        })
        .collect()
}

fn admitted(decisions: &[Decision]) -> u128 {
    decisions.iter().filter(|d| d.is_allowed()).count() as u128
}

// Emission interval Token Bucket uses for `per_second` — matches the constructor.
fn token_bucket_interval_nanos(rate: u64) -> u128 {
    NANOS_PER_SEC / rate as u128
}

// Emission interval GCRA derives from a rate — matches `Gcra::new`.
fn gcra_interval_nanos(rate: u64) -> u128 {
    Duration::from_secs_f64(1.0 / rate as f64).as_nanos()
}

/// Gaps expressed in *thousandths of the emission interval*. Weighted toward
/// sub-interval values (bursts) with a tail out to 3× the interval (idle and
/// refill). Convert to nanoseconds against a concrete interval with [`scale`].
fn gap_permilles() -> impl Strategy<Value = Vec<u64>> {
    let permille = prop_oneof![
        4 => 0u64..=500,       // 0 – 0.5·T : sub-interval burst pressure
        1 => 500u64..=3000,    // 0.5 – 3·T : spacing, idle, refill
    ];
    prop::collection::vec(permille, 1..=256)
}

/// Scale interval-relative gaps (thousandths of `T`) into nanosecond gaps.
fn scale(permilles: &[u64], interval_nanos: u128) -> Vec<u64> {
    permilles
        .iter()
        .map(|&p| (interval_nanos * p as u128 / 1000) as u64)
        .collect()
}

proptest! {
    /// Safety: Token Bucket never admits more than `capacity + span/interval`.
    #[test]
    fn token_bucket_never_exceeds_configured_rate(
        capacity in 1u64..=64,
        rate in 1u64..=1000,
        permilles in gap_permilles(),
    ) {
        let interval = token_bucket_interval_nanos(rate);
        let arrivals = arrivals_from_gaps(&scale(&permilles, interval));
        let got = admitted(&run(&TokenBucket::per_second(capacity, rate).unwrap(), &arrivals));

        let span = arrivals.last().unwrap().as_nanos();
        // Tight bound, no slack: the real (σ, ρ) limit is `capacity + span/T`,
        // and since `admitted` is an integer it floors cleanly. This catches even
        // a single-token over-admission.
        let bound = capacity as u128 + span / interval;
        prop_assert!(
            got <= bound,
            "admitted {got} > bound {bound} (cap {capacity}, {rate}/s, span {span}ns)"
        );
    }

    /// Safety: GCRA never admits more than `burst + span/interval`.
    #[test]
    fn gcra_never_exceeds_configured_rate(
        burst in 1u64..=64,
        rate in 1u64..=1000,
        permilles in gap_permilles(),
    ) {
        let interval = gcra_interval_nanos(rate);
        let arrivals = arrivals_from_gaps(&scale(&permilles, interval));
        let got = admitted(&run(&Gcra::new(rate as f64, burst).unwrap(), &arrivals));

        let span = arrivals.last().unwrap().as_nanos();
        // Tight bound (see the Token Bucket case): `burst + span/T`, no slack.
        let bound = burst as u128 + span / interval;
        prop_assert!(
            got <= bound,
            "admitted {got} > bound {bound} (burst {burst}, {rate}/s, span {span}ns)"
        );
    }

    /// Liveness: Token Bucket admits every request spaced ≥ one interval apart.
    #[test]
    fn token_bucket_admits_all_when_spaced(
        capacity in 1u64..=64,
        rate in 1u64..=1000,
        extra in prop::collection::vec(0u64..=2_000_000_000, 1..=128),
    ) {
        let interval = token_bucket_interval_nanos(rate) as u64;
        let gaps: Vec<u64> = extra.iter().map(|e| interval + e).collect();
        let tb = TokenBucket::per_second(capacity, rate).unwrap();
        for d in run(&tb, &arrivals_from_gaps(&gaps)) {
            prop_assert!(d.is_allowed(), "conforming request was rejected");
        }
    }

    /// Liveness: GCRA admits every request spaced ≥ one interval apart.
    #[test]
    fn gcra_admits_all_when_spaced(
        burst in 1u64..=64,
        rate in 1u64..=1000,
        extra in prop::collection::vec(0u64..=2_000_000_000, 1..=128),
    ) {
        let interval = gcra_interval_nanos(rate) as u64;
        let gaps: Vec<u64> = extra.iter().map(|e| interval + e).collect();
        let gcra = Gcra::new(rate as f64, burst).unwrap();
        for d in run(&gcra, &arrivals_from_gaps(&gaps)) {
            prop_assert!(d.is_allowed(), "conforming request was rejected");
        }
    }

    /// Retry contract: after draining the burst, a rejection's `retry_after` is
    /// positive and exactly sufficient — waiting it out admits the next request.
    #[test]
    fn token_bucket_retry_after_is_sufficient(
        capacity in 1u64..=64,
        rate in 1u64..=1000,
    ) {
        let tb = TokenBucket::per_second(capacity, rate).unwrap();
        let t0 = Duration::ZERO;
        let mut s = tb.initial(t0);
        for _ in 0..capacity {
            let (next, d) = tb.step(s, t0, 1);
            prop_assert!(d.is_allowed());
            s = next;
        }
        let (s_rejected, d) = tb.step(s, t0, 1);
        let retry = d.retry_after().expect("bucket should be drained");
        prop_assert!(retry > Duration::ZERO, "retry_after must be positive");

        let (_, after_wait) = tb.step(s_rejected, t0 + retry, 1);
        prop_assert!(after_wait.is_allowed(), "retry_after {retry:?} was not enough");
    }

    /// Retry contract for GCRA.
    #[test]
    fn gcra_retry_after_is_sufficient(
        burst in 1u64..=64,
        rate in 1u64..=1000,
    ) {
        let gcra = Gcra::new(rate as f64, burst).unwrap();
        let t0 = Duration::ZERO;
        let mut s = gcra.initial(t0);
        for _ in 0..burst {
            let (next, d) = gcra.step(s, t0, 1);
            prop_assert!(d.is_allowed());
            s = next;
        }
        let (s_rejected, d) = gcra.step(s, t0, 1);
        let retry = d.retry_after().expect("burst should be exhausted");
        prop_assert!(retry > Duration::ZERO, "retry_after must be positive");

        let (_, after_wait) = gcra.step(s_rejected, t0 + retry, 1);
        prop_assert!(after_wait.is_allowed(), "retry_after {retry:?} was not enough");
    }

    /// Determinism: identical inputs yield identical decision streams, for both
    /// algorithms. This is what integer state buys us (ADR-005/017).
    #[test]
    fn decisions_are_deterministic(
        capacity in 1u64..=64,
        burst in 1u64..=64,
        rate in 1u64..=1000,
        gaps in prop::collection::vec(0u64..=2_000_000_000, 1..=256),
    ) {
        let arrivals = arrivals_from_gaps(&gaps);

        let tb = TokenBucket::per_second(capacity, rate).unwrap();
        prop_assert_eq!(run(&tb, &arrivals), run(&tb, &arrivals));

        let gcra = Gcra::new(rate as f64, burst).unwrap();
        prop_assert_eq!(run(&gcra, &arrivals), run(&gcra, &arrivals));
    }
}
