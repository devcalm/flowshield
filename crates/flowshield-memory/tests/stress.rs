//! Adversarial memory stress for `flowshield-memory` (R-04, NFR-010, NFR-034):
//! high-cardinality key floods and LRU-under-pressure. Atomicity under concurrency
//! is covered separately in `concurrency.rs`. These go through the full async
//! [`Limiter`] → [`MemoryStore`] path — the same way production traffic does.
//!
//! No async runtime is involved (ADR-016): the store's futures are ready on first
//! poll, so a no-op-waker `block_on` completes each `check` on the calling thread.

use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::key::Key;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_core::time::TestClock;
use flowshield_memory::{MemoryStore, MemoryStoreBuilder};
use std::future::Future;
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

/// R-04, availability half: a flood of unique keys — the classic memory-exhaustion
/// attack on a rate limiter — must not take the limiter down. Every request through
/// the full limiter path keeps getting a decision while a fresh key space streams
/// past the store's bound.
#[test]
fn limiter_stays_available_under_a_unique_key_flood() {
    let store: MemoryStore<_> = MemoryStoreBuilder::new().capacity(4_096).build().unwrap();
    let limiter = LimiterBuilder::new(TokenBucket::per_second(5, 5).unwrap(), store)
        .clock(TestClock::default())
        .ttl(Duration::from_secs(3600))
        .build()
        .unwrap();

    for i in 0..250_000u64 {
        let key = Key::from(format!("attacker-{i}"));
        // Fresh key each time → always admitted, never an error, no matter how many
        // keys have already churned through the bounded store.
        assert!(block_on(limiter.check(&key)).unwrap().is_allowed());
    }
}

/// R-04, memory half: the same flood asserted directly on the store — after a
/// million distinct keys the live-entry count is pinned at or below the bound. This
/// is the concrete memory guarantee.
#[test]
fn store_never_exceeds_its_bound_under_churn() {
    use flowshield_core::decision::Decision;
    use flowshield_core::store::StorageBackend;

    let store: MemoryStore<u64> = MemoryStoreBuilder::new().capacity(1_000).build().unwrap();
    let bound = store.capacity();
    let ttl = Duration::from_secs(3600);
    let allow = || Decision::allowed(None);

    for i in 0..1_000_000u64 {
        let key = Key::from(format!("k-{i}"));
        block_on(store.update(&key, Duration::ZERO, ttl, |_| (i, allow()))).unwrap();
    }
    assert!(
        store.len() <= bound,
        "store held {} entries, over its bound {bound}",
        store.len()
    );
}

/// LRU is a correctness property here, not just a cache heuristic: a key that keeps
/// sending traffic must not have its (drained) limit state evicted and reset to a
/// full allowance by unrelated cold keys churning through the store. Single shard so
/// there is no hashing to hide behind — the hot key competes with *every* cold key.
#[test]
fn actively_limited_key_survives_cold_key_churn() {
    // Bucket of 2, frozen time (no refill): once drained, the hot key must stay
    // rejected for the whole test — unless its state is wrongly evicted.
    let store: MemoryStore<_> = MemoryStoreBuilder::new()
        .capacity(2)
        .shards(1)
        .build()
        .unwrap();
    let limiter = LimiterBuilder::new(TokenBucket::per_second(2, 1).unwrap(), store)
        .clock(TestClock::default())
        .build()
        .unwrap();

    let hot = Key::from("hot");
    assert!(block_on(limiter.check(&hot)).unwrap().is_allowed()); // 2 -> 1
    assert!(block_on(limiter.check(&hot)).unwrap().is_allowed()); // 1 -> 0
    assert!(!block_on(limiter.check(&hot)).unwrap().is_allowed()); // drained

    // Hammer far more distinct cold keys than the store can hold, touching the hot
    // key right before each cold insert so it is always the most-recently-used.
    for i in 0..10_000u64 {
        assert!(
            !block_on(limiter.check(&hot)).unwrap().is_allowed(),
            "hot key's drained state was lost to eviction at i={i} — LRU failed to \
             protect an actively-limited key, resetting its limit"
        );
        let cold = Key::from(format!("cold-{i}"));
        let _ = block_on(limiter.check(&cold));
    }
}
