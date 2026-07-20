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
//! ## Status
//!
//! FlowShield is in **Phase 0 (design)**. The public API is still being built, so this
//! facade does not yet re-export user-facing types — that happens in Phase 1 as
//! `flowshield-core`'s surface stabilizes.

// Public re-exports of the stable API are added here as flowshield-core's surface lands
// in Phase 1, for example:
//
//     #[doc(inline)]
//     pub use flowshield_core::Limiter;
//
// Nothing is re-exported yet, to avoid exposing placeholder API through the facade.
