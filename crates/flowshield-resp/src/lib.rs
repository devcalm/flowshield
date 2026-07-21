#![forbid(unsafe_code)]
//! # FlowShield — shared RESP (Redis / Valkey) backend
//!
//! `flowshield-resp` is the vendor-neutral, shared implementation behind the
//! `flowshield-redis` and `flowshield-valkey` backends (ADR-006). Redis and Valkey
//! speak the same RESP protocol and server-side Lua scripting, so the rate-limiting
//! logic — the atomic check-and-consume, key layout, TTL handling, and result
//! decoding — is written **once** here and reused by both; the two vendor crates are
//! thin, independently-selectable wrappers that can diverge later without breaking
//! each other.
//!
//! It is a **family-B** backend (ADR-019): it cannot run the Rust
//! [`Algorithm::step`](flowshield_core::algorithm::Algorithm::step) closure, so it
//! runs an algorithm's [`RespScripted`] Lua program server-side, atomically, in a
//! single round trip (NFR-001), and implements the limiter's [`Evaluate`] bridge
//! directly.
//!
//! ## Guarantees (ADR-005)
//!
//! - **Atomic & exact on the executing primary.** The whole check-and-consume runs
//!   inside one Lua script, so concurrent requests on a key never interleave.
//! - **Bounded over-admission only under replication/failover topology** — a
//!   documented property of the deployment, not of the operation.
//! - **Microsecond resolution.** Lua numbers are IEEE-754 doubles; the encoding works
//!   in microseconds so time arithmetic stays exact (see
//!   [`flowshield_core::resp`]). This is the distributed backend's precision. A
//!   consequence: **sustained rates above 1,000,000/s are pinned to that ceiling**
//!   on RESP backends (the per-token interval is clamped to 1µs), whereas the memory
//!   backend keeps nanosecond exactness. Configure limits below 1e6/s for identical
//!   behaviour across backends.
//!
//! ## Runtime coupling (ADR-016)
//!
//! This crate depends on `redis` + `deadpool-redis` and is **Tokio-coupled**, as
//! disclosed here at the crate boundary. Only `tokio::time::timeout` is used; no
//! runtime is started by the library.
//!
//! ## Degradation (ADR-007)
//!
//! Every operation resolves to a [`Decision`] or a typed [`Error`] (`Timeout` /
//! `Backend`). The limiter applies the configured degradation policy to those errors;
//! this crate only surfaces them. A per-operation timeout is mandatory and always
//! enforced.

use flowshield_core::decision::Decision;
use flowshield_core::error::{Error, Result};
use flowshield_core::key::Key;
use flowshield_core::resp::RespScripted;
use flowshield_core::store::Evaluate;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

pub use deadpool_redis::Pool;

/// Default per-operation timeout (ADR-006/007). Deliberately conservative; tune it
/// with [`RespStoreBuilder::timeout`].
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

/// Where a decision's time comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClockSource {
    /// **Server-authoritative** time, read via `redis.call('TIME')` inside the
    /// atomic script (ADR-017): the production default. It removes inter-node clock
    /// skew because every node's decision is timed by the one store, in the same
    /// round trip.
    Server,
    /// The limiter's injected clock, passed into the script. **Single-node or test
    /// use only** — across multiple nodes it reintroduces the clock skew `Server`
    /// exists to remove. Its purpose is deterministic testing (driving the script
    /// from a controllable clock to cross-check it against the Rust oracle).
    Client,
}

/// The shared RESP backend: a pooled Redis/Valkey connection that evaluates any
/// [`RespScripted`] algorithm's Lua program atomically.
///
/// Construct it with [`RespStore::builder`]. The `flowshield-redis` /
/// `flowshield-valkey` crates wrap this with their own identity and `kind` label.
pub struct RespStore {
    pool: Pool,
    kind: &'static str,
    timeout: Duration,
    clock: ClockSource,
    // Prepared scripts cached by their `&'static` Lua pointer, so the SHA-1 digest is
    // computed once per distinct script instead of on every request (`Script::new`
    // hashes eagerly — a real hot-path cost). The pointer is a stable, cheap key
    // because a `RespProgram` always returns `&'static` Lua. Poison-tolerant: the
    // critical sections are trivial and panic-free.
    scripts: RwLock<HashMap<usize, Arc<redis::Script>>>,
}

impl RespStore {
    /// Start building a store connecting to `url` (e.g. `redis://127.0.0.1:6379`).
    pub fn builder(url: impl Into<String>) -> RespStoreBuilder {
        RespStoreBuilder {
            url: url.into(),
            kind: "resp",
            timeout: DEFAULT_TIMEOUT,
            clock: ClockSource::Server,
            max_connections: None,
        }
    }

    /// Build directly from an existing pool — used by the vendor wrapper crates and
    /// by callers who manage their own [`Pool`].
    pub fn from_pool(
        pool: Pool,
        kind: &'static str,
        timeout: Duration,
        clock: ClockSource,
    ) -> Self {
        Self {
            pool,
            kind,
            timeout,
            clock,
            scripts: RwLock::new(HashMap::new()),
        }
    }

    /// The connection pool, e.g. to share it across several stores.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// A prepared [`redis::Script`] for `lua`, computing its SHA-1 at most once and
    /// reusing it thereafter (M1). Keyed by the `&'static` Lua pointer.
    fn script_for(&self, lua: &'static str) -> Arc<redis::Script> {
        let key = lua.as_ptr() as usize;
        if let Some(script) = self
            .scripts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return script.clone();
        }
        self.scripts
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key)
            .or_insert_with(|| Arc::new(redis::Script::new(lua)))
            .clone()
    }
}

/// Run one prepared script against a pooled connection and decode the reply. Split
/// out of [`RespStore::evaluate`] so the timeout wrapper there reads as one line.
async fn run_script(
    pool: &Pool,
    script: &redis::Script,
    key: &Key,
    now_micros: i64,
    ttl_micros: i64,
    cost: u64,
    params: &[i64],
) -> Result<Decision> {
    let mut conn = pool.get().await.map_err(|e| Error::Backend(Box::new(e)))?;
    let mut invocation = script.prepare_invoke();
    invocation
        .key(key.as_str())
        .arg(now_micros)
        .arg(ttl_micros)
        .arg(cost as i64);
    for param in params {
        invocation.arg(*param);
    }
    let raw: (i64, i64, i64) = invocation
        .invoke_async(&mut conn)
        .await
        .map_err(|e| Error::Backend(Box::new(e)))?;
    Ok(decode(raw))
}

impl<A: RespScripted> Evaluate<A> for RespStore {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn evaluate(
        &self,
        algo: &A,
        key: &Key,
        now: Duration,
        ttl: Duration,
        cost: u64,
    ) -> impl Future<Output = Result<Decision>> + Send {
        let program = algo.resp_program();
        let script = self.script_for(program.lua); // cached SHA-1 (M1)
        let params = program.params;
        // `-1` tells the script to read server time; a non-negative value is used
        // verbatim (see `flowshield_core::resp`). Client time is only ever passed in
        // `ClockSource::Client` mode.
        let now_micros: i64 = match self.clock {
            ClockSource::Server => -1,
            ClockSource::Client => now.as_micros().min(i64::MAX as u128) as i64,
        };
        let ttl_micros = ttl.as_micros().min(i64::MAX as u128) as i64;
        let key = key.clone();
        let pool = self.pool.clone();
        let timeout = self.timeout;

        async move {
            // The mandatory per-operation timeout (ADR-006/007): a slow or unreachable
            // store surfaces as a typed `Timeout`, never an unbounded wait.
            let call = run_script(&pool, &script, &key, now_micros, ttl_micros, cost, &params);
            match tokio::time::timeout(timeout, call).await {
                Ok(result) => result,
                Err(_) => Err(Error::Timeout { elapsed: timeout }),
            }
        }
    }
}

/// Decode the script's `{allowed, remaining, retry_micros}` reply into a [`Decision`].
fn decode((allowed, remaining, retry_micros): (i64, i64, i64)) -> Decision {
    if allowed == 1 {
        // `-1` is the script's "remaining unknown" sentinel.
        let remaining = (remaining >= 0).then_some(remaining as u64);
        Decision::allowed(remaining)
    } else {
        Decision::rejected(Duration::from_micros(retry_micros.max(0) as u64))
    }
}

/// Builder for a [`RespStore`] (ADR-004: construction-time validation).
pub struct RespStoreBuilder {
    url: String,
    kind: &'static str,
    timeout: Duration,
    clock: ClockSource,
    max_connections: Option<usize>,
}

impl RespStoreBuilder {
    /// Set the low-cardinality backend `kind` label used in telemetry (ADR-012) and
    /// to give the vendor crates distinct identities (`"redis"` / `"valkey"`).
    pub fn kind(mut self, kind: &'static str) -> Self {
        self.kind = kind;
        self
    }

    /// Set the mandatory per-operation timeout (ADR-006/007).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Choose the decision clock source. Defaults to [`ClockSource::Server`].
    pub fn clock_source(mut self, clock: ClockSource) -> Self {
        self.clock = clock;
        self
    }

    /// Bound the connection pool to at most `max` connections (ADR-006: bounded
    /// acquisition). Defaults to the `deadpool` default when unset.
    pub fn max_connections(mut self, max: usize) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Validate configuration and build the store, creating a Tokio connection pool.
    ///
    /// # Errors
    ///
    /// [`Error::Config`] if `timeout` is zero (a zero timeout would reject every
    /// request) or if the URL/pool configuration is invalid (ADR-011, FR-072).
    pub fn build(self) -> Result<RespStore> {
        if self.timeout.is_zero() {
            return Err(Error::Config("timeout must be greater than zero".into()));
        }
        let mut cfg = deadpool_redis::Config::from_url(self.url);
        if let Some(max) = self.max_connections {
            cfg.pool = Some(deadpool_redis::PoolConfig::new(max));
        }
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| Error::Config(format!("invalid RESP connection config: {e}")))?;
        Ok(RespStore::from_pool(
            pool,
            self.kind,
            self.timeout,
            self.clock,
        ))
    }
}
