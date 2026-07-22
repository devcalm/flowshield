# Changelog

All notable changes to FlowShield are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (see [ADR-015](docs/adr/ADR-015-versioning-and-msrv.md)).

## [Unreleased]

## [1.0.0] - 2026-07-22

First stable release. The public API — including the extension traits — is now covered by the
SemVer and deprecation guarantees in [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md).

### Fixed

Correctness fixes from the pre-1.0 adversarial review (each with a regression test):
- **Idle-reset limit bypass with the default TTL.** The limiter used a fixed 60s default TTL
  unrelated to the algorithm's decay time, so for any config where full decay takes >60s (e.g.
  `TokenBucket::per_second(1000, 10)`) an idle key would expire and reset to fully-permissive — a
  client could restore its allowance by pausing. `LimiterBuilder` now **derives** the default TTL
  from the algorithm (`Algorithm::min_ttl()`, a new provided method): `max(min_ttl, 60s)`. An
  explicit `.ttl()` is still honored verbatim (documented escape hatch).
- **GCRA `step()` could panic** ("overflow when adding durations") on a very large `check_n` cost —
  a `Duration + Duration` that should have been `saturating_add`. Now saturates (rejects), honoring
  the never-panic contract (ADR-011/FR-072).
- **RESP backends silently diverged from memory on out-of-`i64` values.** A `TokenBucket` capacity
  `> i64::MAX` wrapped negative in the Redis/Valkey Lua and rejected everything (memory admitted);
  a `cost > i64::MAX` inverted the decision (minted tokens). Capacity is now rejected at construction
  and oversized cost as a typed error, so backends agree and misuse fails fast.
- **`Retry-After` header could overflow to `0`** (or panic in debug) for a near-`u64::MAX`-seconds
  wait in the Tower and Actix integrations — `Retry-After: 0` invites an immediate retry. Now
  saturates.

### Added
- **End-to-end test suite** — a new non-published `flowshield-e2e` crate drives **real HTTP over
  a real socket** (reqwest → Axum and Actix servers on ephemeral ports) all the way through to a
  **real Redis** backend, asserting `429`/`Retry-After`/`X-RateLimit-Remaining`. Headline test:
  two independent Axum servers sharing one Redis enforce a **single global limit** (remaining
  descends across both, then `429` on either) — the distributed guarantee proven end-to-end.
  Redis-gated tests skip locally and hard-fail in the `integration` CI job; always-run tests cover
  the in-memory path, a `Retry-After`-honoring refill, and fail-open/closed against a dead backend.
  Adds **local-only stress tests** (`#[ignore]`d, never in CI) — a full **2×2 matrix** of
  {Axum, Actix} × {Redis, in-memory} — that fire a concurrent HTTP herd and assert **no
  over-admission**: a capacity-`N` bucket admits exactly `N` under tens of thousands of requests
  (verified at 20k req → exactly 500 admitted per combo). In-flight concurrency is bounded so the
  load scales without exhausting file descriptors; total load is tunable via
  `FLOWSHIELD_STRESS_REQUESTS` / `FLOWSHIELD_STRESS_CAPACITY`.
- **End-to-end load benchmark** (`flowshield-e2e/tests/load.rs`, `#[ignore]`d/local-only) — measures
  **whole-stack latency (p50…p99.9) and throughput** over real HTTP → Axum → real Redis on the
  always-admit path, with an in-memory baseline for contrast. A dated Apple-M1 snapshot is published
  in `docs/benchmarks/e2e-load-2026-07-22-apple-m1.md` (in-memory ~96k req/s @ p50 0.5 ms; Redis
  ~16k req/s @ p50 3.8 ms — the loopback round-trip cost). Per ADR-014 these are dated data points,
  not targets.
- **Phase 7 — 1.0 readiness (hardening, docs, policy).**
  - **Committed MSRV of Rust 1.85** (edition-2024 floor; resolves DV-02). `rust-version` is
    declared workspace-wide, a CI `msrv` job verifies `flowshield-core`, `flowshield-memory`, and
    the default `flowshield` build on 1.85. The effective MSRV of the heavy optional features
    (`redis`/`valkey`/`axum`/`actix`) is governed by their upstream ecosystems (currently ~1.88) —
    documented in `docs/COMPATIBILITY.md`.
  - **`#![forbid(unsafe_code)]` added to `flowshield-core`** — the one library crate that lacked it;
    every first-party crate now forbids `unsafe`.
  - **Documentation set:** a getting-started [tutorial](docs/tutorial.md), an
    [extension-author guide](docs/extension-guide.md) with per-trait stability tiers, a
    [compatibility & versioning policy](docs/COMPATIBILITY.md) (SemVer, MSRV, deprecation window,
    extension-API tiers), and a [security review & posture](docs/security-review.md).
  - **README rewritten** to reflect the shipped state (was still marked "Phase 0 — no code written").

### Changed
- **All published crates bumped to `1.0.0`** with synchronized versions; inter-crate dependency
  requirements updated to `1`.

### Notes
- **Deferred beyond 1.0 (tracked):** empirical multi-node over-admission bound (DV-01) and ratified
  numeric performance targets (DV-03) await a dedicated benchmark host; both guarantees are
  documented qualitatively.

<!-- Everything below shipped during the 0.x cycle and is consolidated into 1.0.0. -->

### Added
- **Phase 6 — Benchmarking suite** (ADR-014). New non-published `flowshield-benchmarks`
  crate measuring the public facade, with reproducible methodology and machine-readable
  output:
  - **Criterion microbenchmarks** — `algorithm` (pure `Algorithm::step` floor), `decision`
    (end-to-end `Limiter::check` throughput + full latency distribution, swept over
    `{1, 1k, 100k}` keys, both algorithms, accept and reject branches), and `contention`
    (shared store under a thread sweep, hot-key vs. spread regimes). Criterion emits the
    tail (p50…p99.9) ADR-014 requires and per-benchmark `estimates.json`.
  - **End-to-end HTTP-middleware benchmark** — `http_middleware` drives the
    `flowshield-tower` `Layer`/`Service` **in-process** (no TCP, no server) over the
    in-memory backend, measuring the layer's own per-request cost vs. a bare downstream
    service (`flowshield-axum` builds on this same layer). First finding: the admit path
    adds ~455 ns and, unlike the raw decision, is **not** allocation-free — the peer-IP →
    `String` key extraction and the boxed service future dominate.
  - **Scenario examples emitting JSON** — `allocations` (counting global allocator →
    allocations-per-decision) and `concurrency_10k` (10k Tokio tasks × 100 decisions,
    NFR-004 no-deadlock/no-panic gate). First results: the in-memory hot path is
    **allocation-free in steady state**, and 10k-concurrent completes cleanly with a ~5×
    sharding payoff (per-task key vs. shared hot key).
  - **Reachability-gated `redis_rtt`** benchmark (behind the `redis` feature) for
    network round-trip overhead.
  - **Regression gate** — `scripts/bench_regression.py` (stdlib-only) flattens Criterion
    output and fails a candidate whose median regresses beyond tolerance (±15% default). A
    new per-PR `bench` CI job compiles the whole suite and runs the scenario binaries.
  - Methodology and a dated Apple-M1 results snapshot under `docs/benchmarks/`. Official
    numeric targets (DV-03) and the multi-node over-admission bound (DV-01) remain open,
    pending a dedicated benchmark host.

### Added
- **`X-RateLimit-Remaining` response header** across all three HTTP integrations
  (Tower/Axum/Actix). Allowed responses now carry the caller's remaining headroom when the
  algorithm reports it (Token Bucket / GCRA always do); a `429` reports `0`. Backed by a new
  `Decision::remaining()` accessor in `flowshield-core`.

### Fixed
- **`ForwardedForKeyExtractor` (Tower/Axum) mis-parsed the RFC 7239 `Forwarded` header as
  `X-Forwarded-For`.** It preferred the `Forwarded` header (grammar `for=1.2.3.4;proto=…`)
  and parsed it as a bare comma-separated IP list, so a present `Forwarded` header produced a
  garbage key like `for=1.2.3.4;proto=https` and scattered one client across buckets. It now
  reads **only** `X-Forwarded-For` (left-most entry), matching its documented behaviour;
  empty values fall to the `unknown-forwarded` bucket. Added regression tests.

### Added
- **Phase 5 — Framework integrations: Tower, Axum, Actix** (ADR-009). Each returns a
  standards-correct `429 Too Many Requests` + `Retry-After` (rounded up to whole seconds)
  on rejection and passes allowed requests through; all hold an `Arc<dyn DynLimiter>` so one
  middleware type serves every algorithm/backend.
  - `flowshield-tower` — the foundational Tower `Layer`/`Service`, pinned to the small
    `tower-service`/`tower-layer` trait crates to resist Tower churn. Extractors:
    `PeerAddrKeyExtractor` (default, direct peer IP — spoof-resistant), `HeaderKeyExtractor`
    (e.g. API key), `ForwardedForKeyExtractor` (**trusted-proxy-only**; client-spoofable).
  - `flowshield-axum` — Axum ergonomics over the Tower layer: `ConnectInfoKeyExtractor` +
    `per_client_ip`, runnable `axum_server` example.
  - `flowshield-actix` — standalone Actix `Transform`/`Service` (not Tower-based;
    `EitherBody` short-circuit, `!Send` per-worker futures). Extractors: peer-IP (default),
    header, and `RealIpKeyExtractor` (**trusted-proxy-only**). Runnable `actix_server` example.
  - Facade `tower` / `axum` / `actix` features, re-exported as namespaced modules
    (`flowshield::tower`, `::axum`, `::actix`) since the integrations share type names.
  - **Independence** (FR-033) verified via `cargo tree`: Tower and Actix pull in no other
    integration; Axum's only framework edge is the deliberate Tower reuse. Both examples
    validated over real HTTP (200s then 429s with `Retry-After`).

- **Phase 4 — Distributed backends: Redis + Valkey** (ADR-006), built on the family-B seam:
  - `flowshield-resp` — shared, vendor-neutral RESP logic: runs an algorithm's Lua program server-side via `EVALSHA` (one atomic round trip, NFR-001), with a bounded connection pool (`deadpool-redis`), a **mandatory per-operation timeout**, and typed error surfacing for the limiter's degradation policy.
  - `flowshield-redis` / `flowshield-valkey` — thin, independently-selectable backends with distinct identities; facade `redis` / `valkey` features.
  - `flowshield_core::resp::{RespScripted, RespProgram}` — the family-B capability: an algorithm supplies a Lua transition. **Token Bucket and GCRA** both implement it, at microsecond resolution (Lua f64 keeps epoch-µs exact; see the module docs).
  - **Server-authoritative time** by default (`redis.call('TIME')` inside the script, ADR-017) to remove inter-node clock skew; a client-time mode enables deterministic testing.
  - **Oracle cross-check:** integration tests drive the same trace through the Lua backend and the memory backend and assert identical decisions — proving each Lua transition matches its Rust `step` exactly, for both algorithms on both servers.
  - `docker-compose.yml` (Redis + Valkey) and a reachability-gated integration suite; a dedicated CI job runs it against service containers.
  - Hardening from a pre-commit audit: GCRA now refreshes key TTL on rejection (parity with the memory backend and Token Bucket — prevents an actively-rejected key from expiring and over-admitting; covered by a real-time regression test); compiled scripts are cached (SHA-1 computed once, not per request); scripts use `redis.replicate_commands()` for Redis 3.2–6 compatibility; `Gcra::new` rejects rates whose interval rounds to zero; Token Bucket retry math is fully saturating; and `max_connections` pool sizing is exposed.

### Changed
- **Storage seam revision (ADR-019), a breaking change → `0.2.0`.** The single closure-based `StorageBackend<S>` seam is replaced by two execution families unified by a bridge, so the abstraction fits many providers rather than binding to Redis/Valkey:
  - `LocalStore<S>` — family A: the backend provides isolation/retry around a **pure, re-runnable** transition (`Fn`, not `FnOnce`), covering memory today and transactional/optimistic backends (FoundationDB, Postgres, DynamoDB-OCC) later with no server code.
  - `Evaluate<A>` — the single bridge trait the `Limiter` depends on ("store `S` can evaluate algorithm `A`"). Family-A stores satisfy it via the `evaluate_local` helper; server-side (family B) stores implement it directly.
  - The conformance harness now targets `LocalStore` (`verify_local_store`) and permits a transition to run more than once (retry-aware).
  - `flowshield-memory` re-implemented on `LocalStore` + `Evaluate`; behaviour unchanged. Server-native backends (Redis/Valkey via a `RespScripted` capability) build on this seam in Phase 4.

### Added
- **Phase 3 — Observability** (ADR-010, ADR-012), behind the additive, off-by-default `metrics` and `tracing` features:
  - Metrics: `flowshield_decisions_total` (labelled by `outcome`/`algorithm`/`backend`), `flowshield_backend_duration_seconds` (histogram), and `flowshield_degradations_total` (by `policy`), emitted from the limiter's single orchestration point.
  - Tracing: a `flowshield.check` span per decision plus a `warn` event on backend degradation; rate-limit keys are attached only as a non-reversible digest (`k#…`) — raw key material is never emitted (FR-073).
  - `Algorithm::name()` and `StorageBackend::kind()` — provided (non-breaking) methods supplying low-cardinality telemetry labels.
  - Zero-cost when disabled: all emission is `#[cfg]`-gated and the latency timer is a ZST no-op with observability off (NFR-023).
  - `flowshield/examples/observability.rs`: Prometheus recorder + `tracing` subscriber, with OTel wiring documented as host-side layering.
- Phase 0 design blueprint: project charter, Software Requirements Specification, phased Plan, and ADR-001 … ADR-016.
- Repository organization: `docs/` documentation tree (`adr/`, `architecture/`, `benchmarks/`, `diagrams/`, `design/`, `research/`), `.ai/` internal-artifacts workspace, and community-health files.
- Architecture diagrams (C4 context/container + request-flow) and `docs/architecture/overview.md`.
- CI: GitHub Actions for fmt/clippy/test/doc, a weekly security audit, Dependabot, and issue/PR templates.
- `flowshield` facade crate (public entry point) over the already-published `flowshield-core` foundation, and ADR-018 documenting the facade/core architecture.

[Unreleased]: https://github.com/devcalm/flowshield/commits/main
