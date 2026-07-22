//! Shared helpers for the FlowShield benchmark suite (ADR-014, Phase 6).
//!
//! The in-memory backend is a **family-A** store: its `evaluate` future is already
//! resolved when returned (the work happens synchronously under a shard lock, then a
//! ready future is handed back — see `flowshield-memory`). Microbenchmarks of the hot
//! path therefore drive that future to completion with a no-op waker instead of a real
//! async runtime, so the measurement reflects the limiter's own cost and not executor
//! scheduling overhead. Scenarios that genuinely exercise concurrency (contention,
//! 10k-concurrent, Redis round-trips) use a real Tokio runtime instead.
//!
//! Reproducibility (ADR-014): the workload generators here are deterministic — no
//! `rand`, no clock reads — so a given benchmark run is a pure function of its inputs
//! and can be diffed across machines and revisions.

use std::future::Future;
use std::task::{Context, Poll};

/// Drive an already-ready future to completion with a no-op waker.
///
/// Sound only for futures that never actually pend — which is exactly the in-memory
/// backend's contract. If a future ever returns `Poll::Pending` here it would spin, so
/// this is used *only* against the memory store, never a network backend.
#[inline]
pub fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!(
            "block_on_ready polled a future that pended; it is only valid for the \
             in-memory backend, whose futures are always ready"
        ),
    }
}

/// A deterministic pool of distinct keys, sized to `n`, for high-cardinality workloads.
///
/// Keys are plain decimal strings (`"k0"`, `"k1"`, …) so the generator adds no hashing
/// or formatting cost to the measured section — the caller builds the `Vec` once,
/// outside the timed loop, and indexes into it.
pub fn key_pool(n: usize) -> Vec<flowshield::Key> {
    (0..n)
        .map(|i| flowshield::Key::from(format!("k{i}")))
        .collect()
}

/// The standard cardinalities the suite sweeps, chosen to span the regimes that matter:
/// a single hot key (pure lock/algorithm cost, no eviction), a small working set that
/// fits comfortably, and a large set that stresses the sharded map and LRU bookkeeping
/// (FR-071, NFR-010).
pub const CARDINALITIES: &[usize] = &[1, 1_000, 100_000];
