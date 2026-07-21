pub mod algorithm;
pub mod composite;
pub mod decision;
pub mod error;
pub mod key;
pub mod limiter;
pub mod store;
pub mod time;

// A minimal in-crate `StorageBackend` used only to test core itself — the limiter,
// the conformance harness self-test, and the algorithms — since core cannot depend
// on a backend crate (the inward dependency rule, ADR-002/003). It is compiled only
// for core's own tests and is NOT public API; the production in-memory backend is
// the separate `flowshield-memory` crate.
#[cfg(test)]
mod memory;

// The backend conformance harness. Compiled for the crate's own tests, and for
// external consumers who opt in via the `conformance` feature — it is not part
// of the default (shipping) build.
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
