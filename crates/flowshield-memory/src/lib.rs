#![forbid(unsafe_code)]
//! # FlowShield — in-memory storage backend
//!
//! `flowshield-memory` is the default store for FlowShield (ADR-005, ADR-008): an
//! in-process store of per-key limiter state with **exact** semantics and no external
//! dependencies. It is what a single-node deployment uses, and the behavioural
//! reference every family-A backend is measured against via the shared conformance
//! harness (ADR-006, ADR-019).
//!
//! It is a **family-A** backend (ADR-019): it implements [`LocalStore`] (running the
//! algorithm's pure transition under a shard lock) and bridges to the limiter via
//! [`Evaluate`].
//!
//! Three properties define it:
//!
//! - **Bounded.** The store never holds more than a configured number of keys.
//!   High-cardinality or adversarial key spaces cannot grow memory without limit
//!   (FR-071, R-04); when full, the **least-recently-used** key is evicted. LRU is
//!   a correctness choice, not just a cache heuristic — see [`MemoryStore`].
//! - **Expiring.** Each entry carries a TTL (supplied per call by the limiter). An
//!   entry past its TTL presents as absent, so an idle key behaves exactly like a
//!   fresh one and is reclaimed (ADR-005).
//! - **Sharded.** State is partitioned across many independently-locked shards, so
//!   unrelated keys rarely contend on the same lock (NFR-004).
//!
//! All time is the injected monotonic clock (ADR-017); the store reads no wall
//! clock. The `transact` future is always ready on first poll, so the backend needs
//! no async runtime (ADR-016).
//!
//! ```
//! use flowshield_memory::{MemoryStore, MemoryStoreBuilder};
//! use flowshield_core::algorithm::token_bucket::TokenBucketState;
//!
//! // A store bounded to 50k keys, spread over the default number of shards.
//! let store: MemoryStore<TokenBucketState> =
//!     MemoryStoreBuilder::new().capacity(50_000).build()?;
//! # Ok::<(), flowshield_core::error::Error>(())
//! ```

mod lru;

use flowshield_core::algorithm::Algorithm;
use flowshield_core::decision::Decision;
use flowshield_core::error::{Error, Result};
use flowshield_core::key::Key;
use flowshield_core::store::{Evaluate, LocalStore, evaluate_local};
use lru::LruShard;
use std::future::Future;
use std::hash::BuildHasher;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// Default key bound when the caller does not specify one. Finite by design — a
/// memory backend must always be bounded (FR-071) — and generous enough for most
/// single-node services. Tune it with [`MemoryStoreBuilder::capacity`].
pub const DEFAULT_CAPACITY: usize = 100_000;

/// Upper clamp on the shard count. More shards buy concurrency but cost a `Mutex`
/// and a map each; past this there is no realistic benefit.
const MAX_SHARDS: usize = 4096;

/// In-memory family-A store ([`LocalStore`] + [`Evaluate`]): bounded, expiring,
/// sharded per-key state.
///
/// # Why LRU eviction
///
/// When the store is full, inserting a new key evicts the least-recently-used one.
/// For a *cache* the policy is a performance detail; for a *rate limiter* it is a
/// correctness one. A limiter's per-key state is what enforces the limit; dropping
/// it resets the key to fresh — a full allowance. Evicting a key that is *actively
/// sending traffic* would therefore briefly lift its limit exactly when it matters.
/// LRU sheds the cold keys (which are not being limited anyway) and protects the
/// hot ones, so eviction never loosens a limit that is actually in force. Under
/// memory pressure the store degrades by forgetting idle keys, never by
/// under-limiting active ones.
///
/// # Concurrency
///
/// Keys are hashed to one of `shards` independently-locked partitions. The
/// ADR-005 atomic check-and-consume is the shard `Mutex` held across the whole
/// load → `step` → store sequence, so two calls for the same key can never
/// interleave, while calls to keys in different shards proceed in parallel. This is
/// the [`LocalStore::transact`] contract.
pub struct MemoryStore<S> {
    shards: Box<[Mutex<LruShard<S>>]>,
    hasher: std::collections::hash_map::RandomState,
    /// `shards.len() - 1`; `shards.len()` is always a power of two, so masking is a
    /// correct and cheap replacement for `%`.
    mask: usize,
    /// Per-shard bound; `per_shard_cap * shards.len()` is the effective total.
    per_shard_cap: usize,
}

impl<S: Send + 'static> MemoryStore<S> {
    /// A store with the default capacity ([`DEFAULT_CAPACITY`]) and a shard count
    /// derived from the machine's parallelism. Infallible: the defaults are always
    /// valid.
    pub fn new() -> Self {
        Self::from_parts(DEFAULT_CAPACITY, default_shards())
    }

    /// A store bounded to roughly `capacity` keys, with a default shard count.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] if `capacity` is zero — a store that can hold nothing
    /// would reject every request (ADR-011, FR-072).
    pub fn with_capacity(capacity: usize) -> Result<Self> {
        MemoryStoreBuilder::new().capacity(capacity).build()
    }

    /// Construct from already-validated parameters. `capacity` is clamped to at
    /// least 1 so this cannot produce a zero-capacity shard; public entry points do
    /// the real validation.
    fn from_parts(capacity: usize, requested_shards: usize) -> Self {
        // Clamp *before* rounding up: `next_power_of_two` panics on overflow (e.g. a
        // `shards(usize::MAX)` request), and the result is bounded by MAX_SHARDS anyway.
        let num_shards = requested_shards.clamp(1, MAX_SHARDS).next_power_of_two();
        let per_shard_cap = capacity.max(1).div_ceil(num_shards).max(1);
        let shards = (0..num_shards)
            .map(|_| Mutex::new(LruShard::with_capacity(per_shard_cap)))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            shards,
            hasher: std::collections::hash_map::RandomState::new(),
            mask: num_shards - 1,
            per_shard_cap,
        }
    }

    /// The effective total key bound. Because capacity is divided across a
    /// power-of-two number of shards (each holding `⌈capacity / shards⌉`), the
    /// realised bound is rounded *up* to a shard multiple and may slightly exceed
    /// the requested capacity — never fall below it.
    pub fn capacity(&self) -> usize {
        self.per_shard_cap * self.shards.len()
    }

    /// Number of shards the store is partitioned across (a power of two).
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    /// Total live entries across all shards. O(shards); intended for tests,
    /// diagnostics, and metrics rather than the hot path.
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| lock(s).len()).sum()
    }

    /// Whether the store currently holds no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn shard_index(&self, key: &Key) -> usize {
        (self.hasher.hash_one(key) as usize) & self.mask
    }
}

/// Lock a shard, recovering from poisoning. A poisoned lock means a previous caller
/// panicked *inside* `step` (the algorithm is `#[forbid]`-clean and panic-free by
/// contract, ADR-011, so this should not happen). The shard's structure is intact
/// at that point — the only casualty is one key's state, which simply resets to
/// fresh — so recovering keeps the limiter available rather than wedging it.
fn lock<S>(m: &Mutex<LruShard<S>>) -> std::sync::MutexGuard<'_, LruShard<S>> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<S: Send + 'static> Default for MemoryStore<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Send + 'static> LocalStore<S> for MemoryStore<S> {
    fn transact(
        &self,
        key: &Key,
        now: Duration,
        ttl: Duration,
        step: impl Fn(Option<S>) -> (S, Decision) + Send,
    ) -> impl Future<Output = Result<Decision>> + Send {
        let mut shard = lock(&self.shards[self.shard_index(key)]);
        // Atomic check-and-consume (ADR-005): load (treating expired as absent),
        // apply the pure transition, and write the result back — all under the one
        // lock, so no two calls for this key can interleave. The shard lock gives
        // exclusive access, so a single `step` application suffices; the `Fn` bound
        // only permits re-runs (which transactional backends need), it does not
        // require them. Writing via `insert_front` also reinserts the key at the MRU
        // position, which is the LRU bookkeeping.
        let current = shard.take_current(key, now);
        let (new_state, decision) = step(current);
        shard.insert_front(key.clone(), new_state, now.saturating_add(ttl));
        // Synchronous work; hand back an already-ready future. The guard drops here,
        // before the future is returned, so it is never held across an await point.
        std::future::ready(Ok(decision))
    }
}

// Family-A bridge to the limiter (ADR-019): forward to the shared `evaluate_local`
// helper, which builds the pure transition and drives `transact`.
impl<A: Algorithm> Evaluate<A> for MemoryStore<A::State> {
    fn evaluate(
        &self,
        algo: &A,
        key: &Key,
        now: Duration,
        ttl: Duration,
        cost: u64,
    ) -> impl Future<Output = Result<Decision>> + Send {
        evaluate_local(self, algo, key, now, ttl, cost)
    }

    fn kind(&self) -> &'static str {
        "memory"
    }
}

/// Builder for [`MemoryStore`] with construction-time validation (ADR-004, FR-072).
#[derive(Debug, Clone)]
pub struct MemoryStoreBuilder {
    capacity: usize,
    shards: usize,
}

impl MemoryStoreBuilder {
    /// A builder seeded with the defaults ([`DEFAULT_CAPACITY`] and a
    /// parallelism-derived shard count).
    pub fn new() -> Self {
        Self {
            capacity: DEFAULT_CAPACITY,
            shards: default_shards(),
        }
    }

    /// Set the approximate maximum number of live keys. The realised bound is
    /// rounded up to a multiple of the shard count — see [`MemoryStore::capacity`].
    pub fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// Set the desired shard count. Rounded **up** to the next power of two and
    /// clamped to a sane maximum. More shards reduce lock contention on unrelated
    /// keys at the cost of a little memory per shard (NFR-004). One shard gives an
    /// exact `capacity` bound (no rounding).
    pub fn shards(mut self, shards: usize) -> Self {
        self.shards = shards;
        self
    }

    /// Validate and build the store.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] if `capacity` is zero (ADR-011, FR-072).
    pub fn build<S: Send + 'static>(self) -> Result<MemoryStore<S>> {
        if self.capacity == 0 {
            return Err(Error::Config("capacity must be greater than zero".into()));
        }
        Ok(MemoryStore::from_parts(self.capacity, self.shards))
    }
}

impl Default for MemoryStoreBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Default shard count: the machine's parallelism rounded up to a power of two and
/// clamped, falling back to a small constant when parallelism is unknown.
fn default_shards() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .next_power_of_two()
        .clamp(1, 64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    // The store's futures are ready on first poll, so a no-op-waker busy-poll drives
    // them without an async runtime (ADR-016).
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

    fn allow() -> Decision {
        Decision::allowed(None)
    }
    fn reject() -> Decision {
        Decision::rejected(Duration::from_secs(1))
    }

    #[test]
    fn stores_and_reloads_state() {
        let store: MemoryStore<u32> = MemoryStore::new();
        let key = Key::from("k");
        let ttl = Duration::from_secs(60);

        block_on(store.transact(&key, Duration::ZERO, ttl, |prev| {
            assert!(prev.is_none());
            (1, allow())
        }))
        .unwrap();
        block_on(store.transact(&key, Duration::ZERO, ttl, |prev| {
            assert_eq!(prev, Some(1));
            (2, reject())
        }))
        .unwrap();
    }

    #[test]
    fn expired_state_is_treated_as_absent() {
        let store: MemoryStore<u32> = MemoryStore::new();
        let key = Key::from("k");
        let ttl = Duration::from_secs(10);
        block_on(store.transact(&key, Duration::ZERO, ttl, |_| (7, allow()))).unwrap();
        block_on(store.transact(&key, Duration::from_secs(11), ttl, |prev| {
            assert!(prev.is_none());
            (0, allow())
        }))
        .unwrap();
    }

    #[test]
    fn zero_capacity_is_rejected() {
        let built = MemoryStoreBuilder::new().capacity(0).build::<u32>();
        assert!(matches!(built, Err(Error::Config(_))));
    }

    #[test]
    fn single_shard_gives_exact_capacity() {
        let store: MemoryStore<u32> = MemoryStoreBuilder::new()
            .capacity(10)
            .shards(1)
            .build()
            .unwrap();
        assert_eq!(store.capacity(), 10);
        assert_eq!(store.shard_count(), 1);
    }

    #[test]
    fn shard_count_is_rounded_to_power_of_two() {
        let store: MemoryStore<u32> = MemoryStoreBuilder::new().shards(5).build().unwrap();
        assert_eq!(store.shard_count(), 8);
    }

    #[test]
    fn effective_capacity_never_below_requested() {
        let store: MemoryStore<u32> = MemoryStoreBuilder::new()
            .capacity(1000)
            .shards(16)
            .build()
            .unwrap();
        assert!(store.capacity() >= 1000);
        assert!(store.capacity() < 1000 + store.shard_count());
    }

    #[test]
    fn bounded_under_high_cardinality() {
        // Far more distinct keys than the bound: memory must not grow past it.
        let store: MemoryStore<u32> = MemoryStoreBuilder::new()
            .capacity(64)
            .shards(4)
            .build()
            .unwrap();
        let ttl = Duration::from_secs(3600);
        for i in 0..100_000u64 {
            let key = Key::from(format!("k-{i}"));
            block_on(store.transact(&key, Duration::ZERO, ttl, |_| (i as u32, allow()))).unwrap();
        }
        assert!(
            store.len() <= store.capacity(),
            "store held {} entries, over its bound {}",
            store.len(),
            store.capacity()
        );
    }
}
