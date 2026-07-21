# Changelog

All notable changes to FlowShield are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (see [ADR-015](docs/adr/ADR-015-versioning-and-msrv.md)).

> While the project is in Phase 0 (design), this changelog tracks design milestones. Once code is released, entries will track per-crate library changes.

## [Unreleased]

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
