#![forbid(unsafe_code)]
//! # FlowShield
//!
//! Ergonomic traffic management and rate limiting for Rust.
//!
//! `flowshield` is the **public facade** for the FlowShield ecosystem. It re-exports the
//! stable, user-facing API from [`flowshield-core`] and is the crate most users should
//! depend on:
//!
//! ```toml
//! [dependencies]
//! flowshield = "0.1"
//! ```
//!
//! Once the API lands (see the project roadmap), typical usage will be:
//!
//! ```ignore
//! use flowshield::Limiter;
//! ```
//!
//! rather than reaching into the core crate directly.
//!
//! ## Facade vs. core
//!
//! - **`flowshield`** (this crate) — the ergonomic entry point: a curated public API,
//!   top-level documentation, and (as they land) the user-facing feature flags that pull
//!   in backends and framework integrations.
//! - [`flowshield-core`] — the engineering foundation: algorithms, the storage
//!   abstraction, extension traits, and internal logic. It remains published and
//!   supported, and advanced users or ecosystem crates may depend on it directly.
//!
//! See the [architecture overview] and ADR-018 for the rationale behind this split.
//!
//! [`flowshield-core`]: https://docs.rs/flowshield-core
//! [architecture overview]: https://github.com/devcalm/flowshield/blob/main/docs/architecture/overview.md
//!
//! ## Composite / layered limits
//!
//! Enforce several limits at once by composing independent limiters; a request is
//! admitted only when every one admits it, and on rejection the [`LimiterSet`]
//! reports the binding limit and the maximum retry-after (FR-013).
//!
//! ```no_run
//! # // The hidden wrapper is gated on `memory` so this example compiles cleanly under
//! # // `--no-default-features` (where `MemoryStore` is not re-exported): it then reduces
//! # // to an empty program. With the default `memory` feature on, the real code compiles.
//! # #[cfg(feature = "memory")]
//! # async fn demo() -> flowshield::Result<()> {
//! use flowshield::{LimiterBuilder, LimiterSet, MemoryStore, Key};
//! use flowshield::TokenBucket;
//!
//! // Two layers on the same client key: a burst-per-second and a daily quota.
//! let per_second = LimiterBuilder::new(TokenBucket::per_second(10, 10)?, MemoryStore::new()).build()?;
//! let per_day    = LimiterBuilder::new(TokenBucket::per_second(1000, 1000)?, MemoryStore::new()).build()?;
//!
//! let limits = LimiterSet::new()
//!     .with("per-second", per_second)
//!     .with("per-day", per_day);
//!
//! let outcome = limits.check(&Key::from("client-42")).await?;
//! if !outcome.is_allowed() {
//!     let limit = outcome.binding().unwrap();       // which layer bound
//!     let retry = outcome.retry_after().unwrap();    // the longer of the waits
//!     eprintln!("rejected by {limit}, retry after {retry:?}");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Status
//!
//! FlowShield is in early development (Phase 1). The core API is taking shape; the
//! re-exports below track `flowshield-core`'s public surface and grow as backends and
//! integrations land.

/// The stable, user-facing API, re-exported from `flowshield-core`.
#[doc(inline)]
pub use flowshield_core::{
    algorithm::{Algorithm, gcra::Gcra, token_bucket::TokenBucket},
    composite::{Combined, DynLimiter, LimiterSet, combine},
    decision::{Allowed, Decision, Rejected},
    error::{Error, Result},
    key::{Key, KeyExtractor},
    limiter::{DegradationPolicy, Limiter, LimiterBuilder},
    store::StorageBackend,
    time::{MonotonicClock, TimeSource},
};

/// The default in-memory storage backend (bounded, expiring, sharded), enabled by
/// the default `memory` feature (ADR-010). Depend on `flowshield-memory` directly for
/// its builder and tuning knobs.
#[cfg(feature = "memory")]
#[doc(inline)]
pub use flowshield_memory::MemoryStore;
