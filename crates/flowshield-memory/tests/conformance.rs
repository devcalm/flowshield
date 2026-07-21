//! `flowshield-memory` against the shared backend conformance harness (ADR-005,
//! FR-025). The behavioural half of the `LocalStore` contract is defined once
//! in `flowshield-core`; running it here is how this backend proves parity with the
//! reference and, in time, with the distributed backends — no new test logic.

use flowshield_core::conformance::verify_local_store;
use flowshield_memory::{MemoryStore, MemoryStoreBuilder};
use std::future::Future;

// The store's futures are ready on first poll, so a no-op-waker busy-poll drives the
// async harness without pulling in an async runtime (ADR-016).
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

#[test]
fn memory_backend_satisfies_the_local_store_contract() {
    // A fresh, empty, isolated store per invocation, as the harness requires.
    block_on(verify_local_store(MemoryStore::<u64>::new));
}

#[test]
fn contract_holds_with_a_single_shard() {
    // Exercise the same contract on the exact-capacity (single-shard) configuration,
    // where every key lands in one lock — no hash-based isolation to lean on.
    block_on(verify_local_store(|| {
        let store: MemoryStore<u64> = MemoryStoreBuilder::new()
            .shards(1)
            .build()
            .expect("valid config");
        store
    }));
}
