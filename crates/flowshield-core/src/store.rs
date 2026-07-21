//! The storage seam (ADR-005, revised by ADR-019).
//!
//! A rate-limit decision is a read-modify-write on per-key state that must be
//! atomic, or concurrent requests race and over-admit. Backends achieve that
//! atomicity in radically different ways, which fall into **two execution
//! families**:
//!
//! - **Family A — [`LocalStore`]:** the backend provides isolation (a lock) or
//!   transactional retry *around a pure Rust transition it runs itself*. Memory
//!   holds a mutex; a transactional store (FoundationDB, Postgres-in-transaction,
//!   DynamoDB with optimistic concurrency) runs the transition and re-runs it on
//!   conflict. Because the transition is pure (ADR-008) it is safe to run more
//!   than once, so the closure is [`Fn`], not `FnOnce`.
//! - **Family B — server-native transitions:** the backend cannot run Rust, so the
//!   *algorithm* supplies a backend-native encoding of its transition behind a
//!   capability trait specific to that encoding (e.g. a `RespScripted` Lua program
//!   for Redis/Valkey — added in Phase 4). There is deliberately no single
//!   "server transition" trait, because Lua, SQL, and DynamoDB expressions share
//!   no common language.
//!
//! Both families are unified for the limiter by one bridge trait, [`Evaluate`]:
//! *"store `S` can evaluate algorithm `A`."* [`Limiter`] depends only on
//! `S: Evaluate<A>`; adding a future backend is additive on one of the two
//! families and never rewrites this seam.
//!
//! [`Limiter`]: crate::limiter::Limiter

use crate::algorithm::Algorithm;
use crate::{decision::Decision, error::Result, key::Key};
use std::future::Future;
use std::time::Duration;

/// **Family A.** A store that makes the check-and-consume atomic by running a pure
/// Rust transition under its own isolation or transactional retry (ADR-005, ADR-019).
///
/// The atomic unit is: load the state at `key` (or `None` if absent or expired),
/// apply `step`, store the result, and return the resulting [`Decision`], with no
/// two operations for the same key ever interleaving. How that isolation is
/// achieved is the backend's business — a lock for the in-memory store; a
/// transaction for a database backend.
///
/// `step` is [`Fn`], not `FnOnce`: a transactional backend may run it **more than
/// once** (re-reading state and recomputing on a write conflict) and commit the
/// final attempt. This is sound precisely because the algorithm transition is pure
/// (ADR-008) — re-running it has no side effects.
///
/// `S` is the algorithm's per-key state. It need only be `Send` (it is moved into
/// and out of the store); it is deliberately **not** required to be `Sync`, since
/// it is only ever touched while the store holds it exclusively.
pub trait LocalStore<S>: Send + Sync
where
    S: Send + 'static,
{
    /// Atomically transition the state stored at `key`.
    ///
    /// The explicit `+ Send` future is deliberate: a plain `async fn` in a trait
    /// does not guarantee a `Send` future, which would break `tokio::spawn`.
    /// Spelling out the return type keeps the limiter spawnable across threads.
    fn transact(
        &self,
        key: &Key,
        now: Duration,
        ttl: Duration,
        step: impl Fn(Option<S>) -> (S, Decision) + Send,
    ) -> impl Future<Output = Result<Decision>> + Send;
}

/// The bridge seam the limiter depends on: *store `Self` can evaluate algorithm `A`*
/// (ADR-019).
///
/// This is the single contract [`Limiter`] requires of a backend. It is satisfied
/// two ways:
///
/// - **Family A** backends (any [`LocalStore`]) implement it with a small forwarding
///   impl that calls [`evaluate_local`]; see [`MemoryStore`] for the pattern.
/// - **Family B** backends (e.g. Redis, Phase 4) implement it **directly** for the
///   algorithms that expose a server-native encoding (`impl<A: RespScripted>
///   Evaluate<A> for RedisStore`), running that encoding on the server.
///
/// There is intentionally no blanket `impl<A, T: LocalStore<A::State>> Evaluate<A>
/// for T`: Rust cannot prove a concrete family-B store does not also implement
/// `LocalStore`, so such a blanket would conflict with family-B impls (ADR-019).
///
/// [`Limiter`]: crate::limiter::Limiter
/// [`MemoryStore`]: https://docs.rs/flowshield-memory
pub trait Evaluate<A: Algorithm>: Send + Sync {
    /// Evaluate one request of `cost` for `key` against `algo`, atomically
    /// transitioning the stored state and returning the [`Decision`].
    fn evaluate(
        &self,
        algo: &A,
        key: &Key,
        now: Duration,
        ttl: Duration,
        cost: u64,
    ) -> impl Future<Output = Result<Decision>> + Send;

    /// A short, static, low-cardinality backend-kind name for telemetry (ADR-012),
    /// e.g. `"memory"` or `"redis"`. Provided so adding it breaks no existing
    /// backend (ADR-013); defaults to `"unknown"` and concrete backends override it.
    /// It must never encode per-key or connection-specific data.
    fn kind(&self) -> &'static str {
        "unknown"
    }
}

/// The family-A adapter: drive a [`LocalStore`] with an algorithm's pure transition.
///
/// Every family-A backend's [`Evaluate`] impl is a one-line forward to this helper,
/// so the "build the closure, run it under isolation" logic lives in exactly one
/// place. It constructs the pure `step` (seeding a fresh key via
/// [`Algorithm::initial`]) and hands it to [`LocalStore::transact`].
pub fn evaluate_local<A, S>(
    store: &S,
    algo: &A,
    key: &Key,
    now: Duration,
    ttl: Duration,
    cost: u64,
) -> impl Future<Output = Result<Decision>> + Send
where
    A: Algorithm,
    S: LocalStore<A::State>,
{
    store.transact(key, now, ttl, move |state| {
        let state = state.unwrap_or_else(|| algo.initial(now));
        algo.step(state, now, cost)
    })
}
