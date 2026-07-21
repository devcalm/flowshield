//! End-to-end behaviour of both shipped algorithms driven through the real
//! [`Limiter`] → [`MemoryStore`] path (Phase 2 task 4, FR-001/FR-002, FR-025).
//!
//! The property suite in `flowshield-core` drives the *pure* algorithms directly;
//! these tests instead exercise the full async path — clock sampling, the store's
//! load/apply/persist round-trip, and TTL expiry — with hand-picked scenarios that
//! mirror the algorithms' documented behaviour. This is where a bug in the wiring
//! between algorithm, store, and clock would show up.
//!
//! No async runtime is involved (ADR-016): the store's futures are ready on first
//! poll, so a no-op-waker `block_on` drives each `check` on the calling thread, and a
//! [`TestClock`] supplies deterministic time.

use flowshield_core::algorithm::gcra::Gcra;
use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::key::Key;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_core::time::TestClock;
use flowshield_memory::MemoryStore;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

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

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// --- Token Bucket ---------------------------------------------------------

#[test]
fn token_bucket_burst_drains_then_refills_through_store() {
    let clock = Arc::new(TestClock::default());
    let limiter = LimiterBuilder::new(TokenBucket::new(2, secs(1)).unwrap(), MemoryStore::new())
        .clock(clock.clone())
        .build()
        .unwrap();
    let k = Key::from("user");

    assert!(block_on(limiter.check(&k)).unwrap().is_allowed()); // 2 -> 1
    assert!(block_on(limiter.check(&k)).unwrap().is_allowed()); // 1 -> 0
    let rejected = block_on(limiter.check(&k)).unwrap(); // drained
    assert!(!rejected.is_allowed());
    assert_eq!(rejected.retry_after(), Some(secs(1)));

    clock.advance(secs(1)); // exactly one token refills
    assert!(block_on(limiter.check(&k)).unwrap().is_allowed());
}

#[test]
fn token_bucket_idle_refill_caps_at_capacity_through_store() {
    let clock = Arc::new(TestClock::default());
    // Long TTL so the idle stretch below does not expire the key — we are testing the
    // algorithm's cap, not the store's reclamation.
    let limiter = LimiterBuilder::new(TokenBucket::new(3, secs(1)).unwrap(), MemoryStore::new())
        .clock(clock.clone())
        .ttl(secs(10_000))
        .build()
        .unwrap();
    let k = Key::from("user");

    for _ in 0..3 {
        assert!(block_on(limiter.check(&k)).unwrap().is_allowed()); // drain
    }
    // Idle far longer than capacity·interval, then hammer: at most `capacity` admit.
    clock.advance(secs(1_000));
    let admitted = (0..10)
        .filter(|_| block_on(limiter.check(&k)).unwrap().is_allowed())
        .count();
    assert_eq!(admitted, 3, "idle refill must be capped at capacity");
}

#[test]
fn cost_n_consumes_n_through_store() {
    let clock = Arc::new(TestClock::default());
    let limiter = LimiterBuilder::new(TokenBucket::new(5, secs(1)).unwrap(), MemoryStore::new())
        .clock(clock.clone())
        .build()
        .unwrap();
    let k = Key::from("user");

    // A single cost-3 request draws down three tokens.
    assert!(block_on(limiter.check_n(&k, 3)).unwrap().is_allowed());
    // A cost-3 follow-up now exceeds the remaining 2 → rejected.
    assert!(!block_on(limiter.check_n(&k, 3)).unwrap().is_allowed());
    // But a cost-2 fits exactly.
    assert!(block_on(limiter.check_n(&k, 2)).unwrap().is_allowed());
}

// --- GCRA -----------------------------------------------------------------

#[test]
fn gcra_admits_burst_then_paces_one_per_interval_through_store() {
    let clock = Arc::new(TestClock::default());
    // 1 req/s sustained, burst of 3.
    let limiter = LimiterBuilder::new(Gcra::new(1.0, 3).unwrap(), MemoryStore::new())
        .clock(clock.clone())
        .build()
        .unwrap();
    let k = Key::from("user");

    for _ in 0..3 {
        assert!(block_on(limiter.check(&k)).unwrap().is_allowed()); // cold-key burst
    }
    assert!(!block_on(limiter.check(&k)).unwrap().is_allowed()); // paced out at t0

    // Exactly one admit becomes available each second thereafter.
    for sec in 1..=3 {
        clock.advance(secs(1));
        assert!(
            block_on(limiter.check(&k)).unwrap().is_allowed(),
            "should admit at second {sec}"
        );
        assert!(
            !block_on(limiter.check(&k)).unwrap().is_allowed(),
            "still paced within the same instant"
        );
    }
}

#[test]
fn gcra_idle_does_not_bank_beyond_burst_through_store() {
    let clock = Arc::new(TestClock::default());
    let limiter = LimiterBuilder::new(Gcra::new(1.0, 2).unwrap(), MemoryStore::new())
        .clock(clock.clone())
        .ttl(secs(10_000))
        .build()
        .unwrap();
    let k = Key::from("user");

    // Prime the schedule, then idle a long time and hammer: only `burst` admit.
    assert!(block_on(limiter.check(&k)).unwrap().is_allowed());
    clock.advance(secs(1_000));
    let admitted = (0..10)
        .filter(|_| block_on(limiter.check(&k)).unwrap().is_allowed())
        .count();
    assert_eq!(
        admitted, 2,
        "idle credit must be capped at the burst tolerance"
    );
}

// --- Store-specific behaviour ---------------------------------------------

#[test]
fn distinct_keys_are_limited_independently_through_store() {
    let clock = Arc::new(TestClock::default());
    let limiter = LimiterBuilder::new(TokenBucket::new(1, secs(60)).unwrap(), MemoryStore::new())
        .clock(clock.clone())
        .build()
        .unwrap();
    let (a, b) = (Key::from("a"), Key::from("b"));

    assert!(block_on(limiter.check(&a)).unwrap().is_allowed()); // drain a
    assert!(!block_on(limiter.check(&a)).unwrap().is_allowed()); // a rejected
    // b has its own fresh bucket and is unaffected.
    assert!(block_on(limiter.check(&b)).unwrap().is_allowed());
}

#[test]
fn ttl_expiry_reclaims_an_idle_key() {
    let clock = Arc::new(TestClock::default());
    // Refill interval (100s) far exceeds the TTL (5s): without expiry the drained key
    // would still be rejected at t = 10s, so an admission there proves the store
    // reclaimed the idle key and presented it as fresh (FR-071, ADR-005).
    //
    // In production, set the TTL at least as long as the limiting window so a client
    // cannot reset its allowance merely by idling; here the mismatch is deliberate,
    // to isolate the expiry path.
    let limiter = LimiterBuilder::new(TokenBucket::new(1, secs(100)).unwrap(), MemoryStore::new())
        .clock(clock.clone())
        .ttl(secs(5))
        .build()
        .unwrap();
    let k = Key::from("user");

    assert!(block_on(limiter.check(&k)).unwrap().is_allowed()); // drain the single token
    assert!(!block_on(limiter.check(&k)).unwrap().is_allowed()); // rejected; ~100s to refill

    clock.advance(secs(10)); // past the 5s TTL, but well short of the 100s refill
    assert!(
        block_on(limiter.check(&k)).unwrap().is_allowed(),
        "an idle key past its TTL must be reclaimed and behave as fresh"
    );
}
