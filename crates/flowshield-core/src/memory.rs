use crate::algorithm::Algorithm;
use crate::decision::Decision;
use crate::error::Result;
use crate::key::Key;
use crate::store::{Evaluate, LocalStore, evaluate_local};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

struct Slot<S> {
    state: S,
    expires_at: Duration,
}

/// A minimal in-crate [`StorageBackend`] (ADR-005), compiled **only for core's own
/// tests** — it is not public API.
///
/// Core cannot depend on a backend crate (the inward dependency rule, ADR-002/003),
/// yet its unit tests, the conformance-harness self-test, and the algorithm tests
/// all need a concrete `StorageBackend` to run against. This single-mutex, unbounded
/// map is the simplest thing that satisfies the contract, and exists purely to serve
/// that role. The production in-memory backend — bounded, sharded, LRU-evicting — is
/// the separate `flowshield-memory` crate; reach for it in real use.
pub struct MemoryStore<S> {
    slots: Mutex<HashMap<Key, Slot<S>>>,
}

impl<S> MemoryStore<S> {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
        }
    }
}

impl<S> Default for MemoryStore<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Copy + Send + 'static> LocalStore<S> for MemoryStore<S> {
    fn transact(
        &self,
        key: &Key,
        now: Duration,
        ttl: Duration,
        step: impl Fn(Option<S>) -> (S, Decision) + Send,
    ) -> impl Future<Output = Result<Decision>> + Send {
        let mut slots = self.slots.lock().expect("MemoryStore mutex poisoned");
        // Treat an expired slot as absent so idle keys are reclaimable (FR-071); this
        // must never make a limit *more* permissive than a fresh key would be.
        let current = slots
            .get(key)
            .filter(|slot| slot.expires_at > now)
            .map(|slot| slot.state);
        // A mutex gives exclusive access, so a single `step` application suffices; the
        // `Fn` bound only *permits* re-runs (for transactional backends), it does not
        // require them.
        let (new_state, decision) = step(current);
        slots.insert(
            key.clone(),
            Slot {
                state: new_state,
                expires_at: now + ttl,
            },
        );
        // The work is synchronous; hand back an already-ready future. The mutex guard is
        // dropped here, before the future is returned, so it is never held across await.
        std::future::ready(Ok(decision))
    }
}

// Family-A bridge: forward to the shared `evaluate_local` helper (ADR-019).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::{Allowed, Rejected};
    use std::future::Future;

    // Store futures are immediately ready; a busy-poll with the no-op waker avoids an
    // async-runtime dependency for tests.
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
        Decision::Allowed(Allowed { remaining: None })
    }
    fn reject() -> Decision {
        Decision::Rejected(Rejected {
            retry_after: Duration::from_secs(1),
        })
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

        // Past the TTL, the slot must look absent.
        block_on(store.transact(&key, Duration::from_secs(11), ttl, |prev| {
            assert!(prev.is_none());
            (0, allow())
        }))
        .unwrap();
    }
}
