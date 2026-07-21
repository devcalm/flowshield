use crate::{decision::Decision, error::Result, key::Key};
use std::future::Future;
use std::time::Duration;

/// Atomic per-key state cell (ADR-005).
///
/// The atomic unit is check-and-consume: load the state at `key` (or `None` if absent
/// or expired), apply the pure transition `step`, store the result, and return the
/// resulting [`Decision`] — with no two calls for the same key ever interleaving. How
/// atomicity is achieved is the backend's business (a lock for the in-memory store; a
/// server-side script for Redis/Valkey, ADR-006).
///
/// `S` is the algorithm's per-key state. It need only be `Send` (it is moved into and
/// out of the store); it is deliberately **not** required to be `Sync`, since it is only
/// ever touched while the store holds it exclusively.
pub trait StorageBackend<S>: Send + Sync
where
    S: Send + 'static,
{
    /// Atomically transition the state stored at `key`.
    ///
    /// Note the explicit `+ Send` future: a plain `async fn` in a trait does not
    /// guarantee a `Send` future, which would break `tokio::spawn`. Spelling out the
    /// return type keeps the limiter spawnable across threads.
    fn update(
        &self,
        key: &Key,
        now: Duration,
        ttl: Duration,
        step: impl FnOnce(Option<S>) -> (S, Decision) + Send,
    ) -> impl Future<Output = Result<Decision>> + Send;

    /// A short, static, low-cardinality name used as the `backend` label on
    /// telemetry (ADR-012), e.g. `"memory"` or `"redis"`. A **provided** method so
    /// adding it breaks no existing out-of-crate backend (ADR-013); it defaults to
    /// `"unknown"` and concrete backends override it. It must never encode per-key
    /// or connection-specific (high-cardinality) data.
    fn kind(&self) -> &'static str {
        "unknown"
    }
}
