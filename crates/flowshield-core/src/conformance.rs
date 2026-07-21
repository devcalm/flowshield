//! Backend conformance harness for the [`StorageBackend`] contract (ADR-005, FR-025).
//!
//! Every storage backend must honour the same behavioural contract so that a
//! limiter behaves identically no matter where per-key state lives. Rather than
//! re-testing that contract in each backend crate, the contract is expressed
//! *once*, here, as a reusable harness any backend can run against its own store.
//! The `flowshield-memory` backend is its first client; the distributed backends
//! (ADR-006) become later clients with no new test logic — which is how their
//! behavioural parity is demonstrated.
//!
//! # What it checks
//!
//! The **behavioural** half of the ADR-005 contract, deterministically:
//!
//! - `step` is invoked exactly once per `update`;
//! - an absent key presents `None` to `step`;
//! - state stored by one `update` is presented to the next;
//! - `update` returns exactly the [`Decision`] `step` produced;
//! - distinct keys never share state;
//! - state within its TTL persists, and state past its TTL presents as absent —
//!   an expired key behaves exactly like a fresh one, never retaining
//!   more-permissive state.
//!
//! It does **not** exercise atomicity under concurrency (that needs the
//! backend's own runtime to spawn contending tasks; the memory backend's is
//! covered by the crate's `concurrency` integration tests) nor backend-specific
//! error mapping (a backend tests its own timeout/error taxonomy).
//!
//! # Usage
//!
//! Enable the `conformance` feature as a dev-dependency, then call the harness
//! from a test, supplying a constructor that yields a fresh, empty, isolated
//! store on each call. The harness is async and runtime-agnostic — you await it
//! on whatever runtime your backend uses:
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_backend_is_conformant() {
//!     flowshield_core::conformance::verify_state_store(MyBackend::new).await;
//! }
//! ```
//!
//! On any contract violation the harness panics with a message naming the
//! property that failed — it is meant to be driven from a `#[test]`.

use crate::decision::{Allowed, Decision, Rejected};
use crate::key::Key;
use crate::store::StorageBackend;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn allow(remaining: u64) -> Decision {
    Decision::Allowed(Allowed {
        remaining: Some(remaining),
    })
}

fn reject(after: Duration) -> Decision {
    Decision::Rejected(Rejected { retry_after: after })
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Run a single `update`, capturing what `step` observed as the prior state and
/// the [`Decision`] the call returned. Also asserts the once-and-only-once
/// invocation invariant.
async fn observe<S: StorageBackend<u64>>(
    store: &S,
    key: &Key,
    now: Duration,
    ttl: Duration,
    new_state: u64,
    decision: Decision,
) -> (Option<u64>, Decision) {
    // The step closure runs inside the backend's atomic section; a shared sink
    // is the simplest way to smuggle its observation back out.
    let sink: Arc<Mutex<Vec<Option<u64>>>> = Arc::new(Mutex::new(Vec::new()));
    let observed = sink.clone();
    let produced = decision.clone();

    let returned = store
        .update(key, now, ttl, move |prev| {
            observed
                .lock()
                .expect("conformance sink poisoned")
                .push(prev);
            (new_state, produced)
        })
        .await
        .expect("contract violation: `update` failed on the conformance happy path");

    let mut calls = sink.lock().expect("conformance sink poisoned");
    assert_eq!(
        calls.len(),
        1,
        "contract violation: `update` must invoke `step` exactly once"
    );
    (calls.pop().unwrap(), returned)
}

/// Verify that `make_store` produces a [`StorageBackend`] satisfying the ADR-005
/// behavioural contract. `make_store` must return a fresh, empty, isolated store
/// on each call. Panics on the first violation, naming the property.
pub async fn verify_state_store<S, F>(make_store: F)
where
    S: StorageBackend<u64>,
    F: Fn() -> S,
{
    absent_key_presents_none(&make_store).await;
    stored_state_is_reloaded(&make_store).await;
    update_returns_the_produced_decision(&make_store).await;
    distinct_keys_are_isolated(&make_store).await;
    unexpired_state_persists(&make_store).await;
    expired_state_presents_as_absent(&make_store).await;
}

async fn absent_key_presents_none<S, F>(make_store: &F)
where
    S: StorageBackend<u64>,
    F: Fn() -> S,
{
    let store = make_store();
    let (seen, _) = observe(
        &store,
        &Key::from("fresh"),
        Duration::ZERO,
        secs(60),
        1,
        allow(0),
    )
    .await;
    assert_eq!(
        seen, None,
        "contract violation: a fresh/absent key must present `None` to `step`"
    );
}

async fn stored_state_is_reloaded<S, F>(make_store: &F)
where
    S: StorageBackend<u64>,
    F: Fn() -> S,
{
    let store = make_store();
    let key = Key::from("k");
    observe(&store, &key, Duration::ZERO, secs(60), 42, allow(0)).await;
    let (seen, _) = observe(&store, &key, Duration::ZERO, secs(60), 43, allow(0)).await;
    assert_eq!(
        seen,
        Some(42),
        "contract violation: state stored by a prior `update` must be presented to the next `step`"
    );
}

async fn update_returns_the_produced_decision<S, F>(make_store: &F)
where
    S: StorageBackend<u64>,
    F: Fn() -> S,
{
    let store = make_store();
    let (_, allowed) = observe(
        &store,
        &Key::from("a"),
        Duration::ZERO,
        secs(60),
        0,
        allow(7),
    )
    .await;
    assert_eq!(
        allowed,
        allow(7),
        "contract violation: `update` must return exactly the `Decision` `step` produced"
    );

    let (_, rejected) = observe(
        &store,
        &Key::from("b"),
        Duration::ZERO,
        secs(60),
        0,
        reject(secs(3)),
    )
    .await;
    assert_eq!(
        rejected,
        reject(secs(3)),
        "contract violation: `update` must return the rejection `step` produced"
    );
}

async fn distinct_keys_are_isolated<S, F>(make_store: &F)
where
    S: StorageBackend<u64>,
    F: Fn() -> S,
{
    let store = make_store();
    observe(
        &store,
        &Key::from("a"),
        Duration::ZERO,
        secs(60),
        100,
        allow(0),
    )
    .await;
    let (seen, _) = observe(
        &store,
        &Key::from("b"),
        Duration::ZERO,
        secs(60),
        0,
        allow(0),
    )
    .await;
    assert_eq!(
        seen, None,
        "contract violation: distinct keys must not share state"
    );
}

async fn unexpired_state_persists<S, F>(make_store: &F)
where
    S: StorageBackend<u64>,
    F: Fn() -> S,
{
    let store = make_store();
    let key = Key::from("k");
    observe(&store, &key, Duration::ZERO, secs(10), 7, allow(0)).await;
    // Well within the TTL window (1s ≪ 10s TTL).
    let (seen, _) = observe(&store, &key, secs(1), secs(10), 0, allow(0)).await;
    assert_eq!(
        seen,
        Some(7),
        "contract violation: state within its TTL must persist"
    );
}

async fn expired_state_presents_as_absent<S, F>(make_store: &F)
where
    S: StorageBackend<u64>,
    F: Fn() -> S,
{
    let store = make_store();
    let key = Key::from("k");
    observe(&store, &key, Duration::ZERO, secs(10), 7, allow(0)).await;
    // Well past the TTL. Deliberately not probing the exact expiry tick — a
    // backend with coarser time granularity (e.g. Redis second-resolution TTLs)
    // may round the boundary, and pinning it would cause false non-conformance.
    let (seen, _) = observe(&store, &key, secs(60), secs(10), 0, allow(0)).await;
    assert_eq!(
        seen, None,
        "contract violation: state past its TTL must present as absent — an expired \
         key must behave exactly like a fresh one, never retaining more-permissive \
         state (ADR-005)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;
    use std::future::Future;

    // The memory store's futures are ready on first poll, so a no-op-waker
    // busy-poll drives the async harness without an async runtime dependency.
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
    fn memory_store_satisfies_the_backend_contract() {
        block_on(verify_state_store(MemoryStore::<u64>::new));
    }
}
