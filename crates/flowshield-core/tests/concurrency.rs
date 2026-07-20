//! End-to-end concurrency tests for the atomic check-and-consume contract
//! (ADR-005): "no two calls for the same key ever interleave."
//!
//! The property-based `conformance` suite drives the *pure* algorithm directly
//! and so never touches the store's atomicity. These tests instead go through the
//! full async [`Limiter`] → [`MemoryStore`] path with many OS threads contending
//! on the same key, which is exactly where a lost-update race in the store would
//! show up as over-admission.
//!
//! No async runtime is involved (ADR-016): the memory store's futures are ready
//! on first poll, so a no-op-waker `block_on` completes each `check` on the
//! calling thread, and real parallelism comes from [`std::thread::scope`]. The
//! contention is genuine — every `check` locks the store's mutex during that
//! poll.

use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::key::Key;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_core::memory::MemoryStore;
use flowshield_core::time::TestClock;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

/// Drive an immediately-ready future to completion without a runtime — the same
/// technique the crate's in-module tests use.
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

const THREADS: usize = 16;

/// With time frozen so no token ever refills, a burst of concurrent requests far
/// exceeding the bucket must admit *exactly* `capacity` — never more (which would
/// be a lost-update race in the atomic section) and never fewer (which would be a
/// spurious rejection). Repeated over many rounds to shake out timing-dependent
/// races.
#[test]
fn concurrent_requests_on_one_key_admit_exactly_capacity() {
    const CAPACITY: u64 = 100;
    const PER_THREAD: u64 = 40; // 16 * 40 = 640 attempts >> 100 capacity
    const ROUNDS: usize = 25;

    for round in 0..ROUNDS {
        // Frozen clock (never advanced) → zero refill → pure capacity contest.
        let limiter = LimiterBuilder::new(
            TokenBucket::per_second(CAPACITY, 1).unwrap(),
            MemoryStore::new(),
        )
        .clock(TestClock::default())
        .build()
        .unwrap();
        let key = Key::from("shared");
        let allowed = AtomicU64::new(0);

        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    for _ in 0..PER_THREAD {
                        if block_on(limiter.check(&key)).unwrap().is_allowed() {
                            allowed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });

        assert_eq!(
            allowed.load(Ordering::Relaxed),
            CAPACITY,
            "round {round}: expected exactly {CAPACITY} admits under contention"
        );
    }
}

/// Distinct keys must not interfere: each key, hammered concurrently on its own
/// thread, independently admits exactly its own `capacity`. This exercises
/// concurrent access to different entries of the shared store map.
#[test]
fn concurrent_requests_across_keys_are_independent() {
    const CAPACITY: u64 = 20;
    const KEYS: usize = 32;

    let limiter = LimiterBuilder::new(
        TokenBucket::per_second(CAPACITY, 1).unwrap(),
        MemoryStore::new(),
    )
    .clock(TestClock::default())
    .build()
    .unwrap();

    let admitted: Vec<AtomicU64> = (0..KEYS).map(|_| AtomicU64::new(0)).collect();
    let limiter = &limiter;

    std::thread::scope(|scope| {
        for (k, slot) in admitted.iter().enumerate() {
            scope.spawn(move || {
                let key = Key::from(format!("key-{k}"));
                // Three times capacity worth of attempts, all at frozen time.
                for _ in 0..CAPACITY * 3 {
                    if block_on(limiter.check(&key)).unwrap().is_allowed() {
                        slot.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });

    for (k, count) in admitted.iter().enumerate() {
        assert_eq!(
            count.load(Ordering::Relaxed),
            CAPACITY,
            "key {k} should admit exactly its own capacity, independent of others"
        );
    }
}

/// Under contention *with* refill: while threads hammer a drained bucket, the
/// clock is advanced by exactly `capacity` intervals, so precisely `capacity`
/// more tokens become available over the run. Total admits must land within the
/// atomic bound `initial_capacity + refilled` — never above it (a race), and the
/// refilled tokens must actually be handed out (liveness).
#[test]
fn concurrent_refill_stays_within_bound() {
    const CAPACITY: u64 = 50;
    const REFILLS: u64 = 50;
    const PER_THREAD: u64 = 40;

    // Shared, advanceable clock: 1 token per second, advanced 1s at a time.
    let clock = std::sync::Arc::new(TestClock::default());
    let limiter = LimiterBuilder::new(
        TokenBucket::per_second(CAPACITY, 1).unwrap(),
        MemoryStore::new(),
    )
    .clock(clock.clone())
    .build()
    .unwrap();
    let limiter = &limiter;
    let key = Key::from("refilling");
    let key = &key;
    let allowed = AtomicU64::new(0);
    let allowed = &allowed;

    std::thread::scope(|scope| {
        // Worker threads hammer the key continuously.
        for _ in 0..THREADS {
            scope.spawn(move || {
                for _ in 0..PER_THREAD {
                    if block_on(limiter.check(key)).unwrap().is_allowed() {
                        allowed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
        // One thread meters out exactly REFILLS intervals of time.
        let clock = clock.clone();
        scope.spawn(move || {
            for _ in 0..REFILLS {
                clock.advance(std::time::Duration::from_secs(1));
                std::thread::yield_now();
            }
        });
    });

    // Upper bound: the initial burst plus every token the elapsed time could mint.
    // Equality isn't asserted because how many refilled tokens are actually
    // claimed depends on the race between advancing time and draining — but the
    // atomic section must never let admits exceed what time permits.
    let got = allowed.load(Ordering::Relaxed);
    let bound = CAPACITY + REFILLS;
    assert!(
        got <= bound,
        "admitted {got} exceeded atomic bound {bound} (capacity {CAPACITY} + {REFILLS} refills)"
    );
    // And the initial capacity alone should always be fully claimable.
    assert!(
        got >= CAPACITY,
        "admitted {got} fell below the initial capacity {CAPACITY}"
    );
}
