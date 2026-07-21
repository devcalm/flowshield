#![forbid(unsafe_code)]
//! # FlowShield — Valkey backend
//!
//! `flowshield-valkey` is FlowShield's distributed backend for **Valkey**: atomic,
//! server-side rate-limit state over RESP. It is a thin, distinctly-identified
//! wrapper over the shared [`flowshield-resp`](flowshield_resp) logic (ADR-006) — the
//! same logic [`flowshield-redis`] wraps — so Valkey and Redis behave identically
//! today while remaining independently selectable and free to diverge later.
//! FlowShield treats the two symmetrically and takes no side in the Redis/Valkey
//! split (ADR-006 vendor neutrality).
//!
//! [`flowshield-redis`]: https://crates.io/crates/flowshield-redis
//!
//! ```no_run
//! use flowshield_core::algorithm::token_bucket::TokenBucket;
//! use flowshield_core::key::Key;
//! use flowshield_core::limiter::LimiterBuilder;
//! use flowshield_valkey::ValkeyStore;
//!
//! # async fn demo() -> flowshield_core::error::Result<()> {
//! let store = ValkeyStore::builder("redis://127.0.0.1:6379").build()?;
//! let limiter = LimiterBuilder::new(TokenBucket::per_second(10, 5)?, store).build()?;
//! let _ = limiter.check(&Key::from("client-42")).await?;
//! # Ok(())
//! # }
//! ```
//!
//! See [`flowshield-resp`](flowshield_resp) for guarantees and the Tokio coupling.

use flowshield_core::decision::Decision;
use flowshield_core::error::Result;
use flowshield_core::key::Key;
use flowshield_core::resp::RespScripted;
use flowshield_core::store::Evaluate;
use std::future::Future;
use std::time::Duration;

pub use flowshield_resp::{ClockSource, DEFAULT_TIMEOUT, Pool, RespStore};

/// The telemetry `kind` label for this backend (ADR-012).
const KIND: &str = "valkey";

/// A FlowShield storage backend backed by Valkey.
///
/// A newtype over the shared [`RespStore`] with Valkey's own identity (ADR-006), so
/// Valkey-specific behaviour can be added here later without touching Redis. Build it
/// with [`ValkeyStore::builder`].
pub struct ValkeyStore(RespStore);

impl ValkeyStore {
    /// Start building a store connecting to `url` (Valkey speaks RESP, so the URL
    /// scheme is `redis://…`).
    pub fn builder(url: impl Into<String>) -> ValkeyStoreBuilder {
        ValkeyStoreBuilder(RespStore::builder(url).kind(KIND))
    }

    /// The underlying shared RESP store (e.g. to reach its [`Pool`]).
    pub fn inner(&self) -> &RespStore {
        &self.0
    }
}

impl<A: RespScripted> Evaluate<A> for ValkeyStore {
    fn kind(&self) -> &'static str {
        <RespStore as Evaluate<A>>::kind(&self.0)
    }

    fn evaluate(
        &self,
        algo: &A,
        key: &Key,
        now: Duration,
        ttl: Duration,
        cost: u64,
    ) -> impl Future<Output = Result<Decision>> + Send {
        self.0.evaluate(algo, key, now, ttl, cost)
    }
}

/// Builder for a [`ValkeyStore`]. Forwards to the shared [`flowshield_resp`] builder
/// with the `kind` fixed to `"valkey"`.
pub struct ValkeyStoreBuilder(flowshield_resp::RespStoreBuilder);

impl ValkeyStoreBuilder {
    /// Set the mandatory per-operation timeout (ADR-006/007).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.0 = self.0.timeout(timeout);
        self
    }

    /// Choose the decision clock source. Defaults to [`ClockSource::Server`].
    pub fn clock_source(mut self, clock: ClockSource) -> Self {
        self.0 = self.0.clock_source(clock);
        self
    }

    /// Bound the connection pool to at most `max` connections (ADR-006).
    pub fn max_connections(mut self, max: usize) -> Self {
        self.0 = self.0.max_connections(max);
        self
    }

    /// Validate configuration and build the store (creates a Tokio connection pool).
    pub fn build(self) -> Result<ValkeyStore> {
        Ok(ValkeyStore(self.0.build()?))
    }
}
