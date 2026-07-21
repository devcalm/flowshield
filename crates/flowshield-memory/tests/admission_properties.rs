//! Property-based admission invariants driven through the real [`Limiter`] →
//! [`MemoryStore`] path (Phase 2 task 6, NFR-010).
//!
//! `flowshield-core`'s property suite proves these same invariants on the *pure*
//! algorithms. Re-proving them end-to-end matters because the store adds a load →
//! apply → persist round-trip and clock sampling between the caller and the
//! algorithm; a lost or duplicated state write there would break the guarantee
//! without touching the algorithm. The bounds are stated identically to the core
//! suite (the classic (σ, ρ) leaky-bucket bound) so a divergence is easy to spot.
//!
//! Time is a shared [`TestClock`] advanced to each synthetic arrival; the store's
//! futures are ready on first poll, so a no-op-waker `block_on` needs no runtime
//! (ADR-016). The per-run TTL is set above the whole arrival span, so no key expires
//! mid-run — expiry would reset a key to fresh and is out of scope for these bounds
//! (it has its own behavioural test).

use flowshield_core::algorithm::Algorithm;
use flowshield_core::algorithm::gcra::Gcra;
use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::key::Key;
use flowshield_core::limiter::{Limiter, LimiterBuilder};
use flowshield_core::store::Evaluate;
use flowshield_core::time::TestClock;
use flowshield_memory::MemoryStore;
use proptest::prelude::*;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

const NANOS_PER_SEC: u128 = 1_000_000_000;

fn block_on<F: Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll};
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn nanos(n: u128) -> Duration {
    Duration::new((n / NANOS_PER_SEC) as u64, (n % NANOS_PER_SEC) as u32)
}

/// Cumulative-sum nanosecond gaps into monotonically non-decreasing arrival instants.
fn arrivals_from_gaps(gaps: &[u64]) -> Vec<Duration> {
    let mut t = 0u128;
    gaps.iter()
        .map(|&g| {
            t += g as u128;
            nanos(t)
        })
        .collect()
}

/// Drive unit-cost arrivals through a limiter on a single key, advancing the shared
/// clock to each arrival instant, and count how many were admitted.
fn admitted_through_store<A, S>(
    limiter: &Limiter<A, S, Arc<TestClock>>,
    clock: &TestClock,
    arrivals: &[Duration],
) -> u128
where
    A: Algorithm,
    S: Evaluate<A>,
{
    let key = Key::from("k");
    let mut current = Duration::ZERO;
    let mut admitted = 0u128;
    for &t in arrivals {
        clock.advance(t.saturating_sub(current)); // arrivals are non-decreasing
        current = t;
        if block_on(limiter.check(&key)).unwrap().is_allowed() {
            admitted += 1;
        }
    }
    admitted
}

/// A TTL strictly greater than the whole run, so no key expires mid-run.
fn ttl_covering(arrivals: &[Duration]) -> Duration {
    arrivals.last().copied().unwrap_or(Duration::ZERO) + Duration::from_secs(1)
}

// Emission intervals, matching each algorithm's constructor exactly.
fn token_bucket_interval_nanos(rate: u64) -> u128 {
    NANOS_PER_SEC / rate as u128
}
fn gcra_interval_nanos(rate: u64) -> u128 {
    Duration::from_secs_f64(1.0 / rate as f64).as_nanos()
}

/// Gaps in thousandths of the emission interval — weighted toward sub-interval bursts
/// with a tail out to 3× the interval. Matches the core suite's generator so the two
/// levels exert the same burst pressure.
fn gap_permilles() -> impl Strategy<Value = Vec<u64>> {
    let permille = prop_oneof![
        4 => 0u64..=500,    // 0 – 0.5·T : sub-interval burst pressure
        1 => 500u64..=3000, // 0.5 – 3·T : spacing, idle, refill
    ];
    prop::collection::vec(permille, 1..=256)
}

fn scale(permilles: &[u64], interval_nanos: u128) -> Vec<u64> {
    permilles
        .iter()
        .map(|&p| (interval_nanos * p as u128 / 1000) as u64)
        .collect()
}

proptest! {
    /// Safety: Token Bucket through the store never admits more than
    /// `capacity + span/interval`.
    #[test]
    fn token_bucket_never_over_admits_through_store(
        capacity in 1u64..=64,
        rate in 1u64..=1000,
        permilles in gap_permilles(),
    ) {
        let interval = token_bucket_interval_nanos(rate);
        let arrivals = arrivals_from_gaps(&scale(&permilles, interval));
        let span = arrivals.last().unwrap().as_nanos();

        let clock = Arc::new(TestClock::default());
        let limiter = LimiterBuilder::new(
            TokenBucket::per_second(capacity, rate).unwrap(),
            MemoryStore::new(),
        )
        .clock(clock.clone())
        .ttl(ttl_covering(&arrivals))
        .build()
        .unwrap();

        let got = admitted_through_store(&limiter, &clock, &arrivals);
        let bound = capacity as u128 + span / interval;
        prop_assert!(
            got <= bound,
            "admitted {got} > bound {bound} (cap {capacity}, {rate}/s, span {span}ns)"
        );
    }

    /// Safety: GCRA through the store never admits more than `burst + span/interval`.
    #[test]
    fn gcra_never_over_admits_through_store(
        burst in 1u64..=64,
        rate in 1u64..=1000,
        permilles in gap_permilles(),
    ) {
        let interval = gcra_interval_nanos(rate);
        let arrivals = arrivals_from_gaps(&scale(&permilles, interval));
        let span = arrivals.last().unwrap().as_nanos();

        let clock = Arc::new(TestClock::default());
        let limiter = LimiterBuilder::new(
            Gcra::new(rate as f64, burst).unwrap(),
            MemoryStore::new(),
        )
        .clock(clock.clone())
        .ttl(ttl_covering(&arrivals))
        .build()
        .unwrap();

        let got = admitted_through_store(&limiter, &clock, &arrivals);
        let bound = burst as u128 + span / interval;
        prop_assert!(
            got <= bound,
            "admitted {got} > bound {bound} (burst {burst}, {rate}/s, span {span}ns)"
        );
    }

    /// Liveness: Token Bucket through the store admits every request spaced at least
    /// one interval apart.
    #[test]
    fn token_bucket_admits_all_when_spaced_through_store(
        capacity in 1u64..=64,
        rate in 1u64..=1000,
        extra in prop::collection::vec(0u64..=2_000_000_000, 1..=128),
    ) {
        let interval = token_bucket_interval_nanos(rate) as u64;
        let gaps: Vec<u64> = extra.iter().map(|e| interval + e).collect();
        let arrivals = arrivals_from_gaps(&gaps);

        let clock = Arc::new(TestClock::default());
        let limiter = LimiterBuilder::new(
            TokenBucket::per_second(capacity, rate).unwrap(),
            MemoryStore::new(),
        )
        .clock(clock.clone())
        .ttl(ttl_covering(&arrivals))
        .build()
        .unwrap();

        let admitted = admitted_through_store(&limiter, &clock, &arrivals);
        prop_assert_eq!(admitted, arrivals.len() as u128, "a conforming request was rejected");
    }

    /// Liveness: GCRA through the store admits every request spaced at least one
    /// interval apart.
    #[test]
    fn gcra_admits_all_when_spaced_through_store(
        burst in 1u64..=64,
        rate in 1u64..=1000,
        extra in prop::collection::vec(0u64..=2_000_000_000, 1..=128),
    ) {
        let interval = gcra_interval_nanos(rate) as u64;
        let gaps: Vec<u64> = extra.iter().map(|e| interval + e).collect();
        let arrivals = arrivals_from_gaps(&gaps);

        let clock = Arc::new(TestClock::default());
        let limiter = LimiterBuilder::new(
            Gcra::new(rate as f64, burst).unwrap(),
            MemoryStore::new(),
        )
        .clock(clock.clone())
        .ttl(ttl_covering(&arrivals))
        .build()
        .unwrap();

        let admitted = admitted_through_store(&limiter, &clock, &arrivals);
        prop_assert_eq!(admitted, arrivals.len() as u128, "a conforming request was rejected");
    }
}
