//! FlowShield end-to-end test suite (not published).
//!
//! This crate has no library code of its own — it exists so the integration tests under
//! `tests/` have a home with the heavy, cross-cutting dev-dependencies (both web
//! frameworks, a real HTTP client, and the Redis probe) that don't belong in any single
//! integration crate. See `tests/common/mod.rs` for the shared harness and the individual
//! `tests/*.rs` files for the scenarios.
//!
//! Run `cargo test -p flowshield-e2e`; Redis-backed tests skip unless a server is
//! reachable (or `FLOWSHIELD_REQUIRE_REDIS=1` forces a hard failure). See the workspace
//! `docker-compose.yml`.
//!
//! ## Stress tests (local-only)
//!
//! `tests/stress.rs` fires a thundering herd of concurrent HTTP requests through the full
//! stack and asserts **no over-admission** (a capacity-`N` bucket admits exactly `N`),
//! across the full 2×2 matrix of {Axum, Actix} × {Redis, in-memory}. These are `#[ignore]`d
//! so they never run in CI (too expensive there); run them on demand:
//!
//! ```text
//! docker compose up -d                                   # for the Redis case
//! cargo test -p flowshield-e2e --release -- --ignored
//! ```
//!
//! Scale the load with `FLOWSHIELD_STRESS_REQUESTS` (default 1000) and
//! `FLOWSHIELD_STRESS_CAPACITY` (default 100).
//!
//! ## Load benchmark (local-only)
//!
//! `tests/load.rs` measures whole-stack **latency (p50…p99.9) and throughput** over real HTTP
//! → Axum → Redis on the always-admit path (in-memory baseline included). Also `#[ignore]`d;
//! run with `cargo test -p flowshield-e2e --release --test load -- --ignored --nocapture` and
//! see `docs/benchmarks/` for a dated results snapshot.
#![forbid(unsafe_code)]
