pub mod algorithm;
pub mod decision;
pub mod error;
pub mod key;
pub mod limiter;
pub mod memory;
pub mod store;
pub mod time;

// The backend conformance harness. Compiled for the crate's own tests, and for
// external consumers who opt in via the `conformance` feature — it is not part
// of the default (shipping) build.
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
