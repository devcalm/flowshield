#![forbid(unsafe_code)]
//! # FlowShield — Redis backend
//!
//! `flowshield-redis` is FlowShield's distributed backend for **Redis**: atomic,
//! server-side rate-limit state over RESP. It is a thin, distinctly-identified
//! wrapper over the shared [`flowshield-resp`](flowshield_resp) logic (ADR-006) — the
//! same logic [`flowshield-valkey`] wraps — so Redis and Valkey behave identically
//! today while remaining independently selectable and free to diverge later.
//!
//! [`flowshield-valkey`]: https://crates.io/crates/flowshield-valkey
//!
//! ```no_run
//! use flowshield_core::algorithm::token_bucket::TokenBucket;
//! use flowshield_core::key::Key;
//! use flowshield_core::limiter::LimiterBuilder;
//! use flowshield_redis::RedisStore;
//!
//! # async fn demo() -> flowshield_core::error::Result<()> {
//! let store = RedisStore::builder("redis://127.0.0.1:6379").build()?;
//! let limiter = LimiterBuilder::new(TokenBucket::per_second(10, 5)?, store).build()?;
//!
//! let decision = limiter.check(&Key::from("client-42")).await?;
//! if decision.is_allowed() { /* proceed */ }
//! # Ok(())
//! # }
//! ```
//!
//! See [`flowshield-resp`](flowshield_resp) for the guarantees (exact on the
//! executing primary; bounded over-admission only under replication/failover;
//! microsecond resolution) and the Tokio coupling this crate inherits.

use flowshield_core::decision::Decision;
use flowshield_core::error::Result;
use flowshield_core::key::Key;
use flowshield_core::resp::RespScripted;
use flowshield_core::store::Evaluate;
use std::future::Future;
use std::time::Duration;

pub use flowshield_resp::{ClockSource, DEFAULT_TIMEOUT, Pool, RespStore};

/// The telemetry `kind` label for this backend (ADR-012).
const KIND: &str = "redis";

/// A FlowShield storage backend backed by Redis.
///
/// A newtype over the shared [`RespStore`] with Redis's own identity (ADR-006), so
/// Redis-specific behaviour can be added here later without touching Valkey. Build it
/// with [`RedisStore::builder`].
pub struct RedisStore(RespStore);

impl RedisStore {
    /// Start building a store connecting to `url` (e.g. `redis://127.0.0.1:6379`).
    pub fn builder(url: impl Into<String>) -> RedisStoreBuilder {
        RedisStoreBuilder(RespStore::builder(url).kind(KIND))
    }

    /// The underlying shared RESP store (e.g. to reach its [`Pool`]).
    pub fn inner(&self) -> &RespStore {
        &self.0
    }
}

impl<A: RespScripted> Evaluate<A> for RedisStore {
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

/// Builder for a [`RedisStore`]. Forwards to the shared [`flowshield_resp`] builder
/// with the `kind` fixed to `"redis"`.
pub struct RedisStoreBuilder(flowshield_resp::RespStoreBuilder);

impl RedisStoreBuilder {
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
    ///
    /// # Errors
    ///
    /// [`Error::Config`](flowshield_core::error::Error::Config) on a zero timeout or
    /// invalid connection configuration.
    pub fn build(self) -> Result<RedisStore> {
        Ok(RedisStore(self.0.build()?))
    }
}
