# FlowShield — Implementation Roadmap (Plan.md)

> **Version:** v0.1 (Phase 0 output)
> **Status:** DRAFT — pending review. Implementation may begin only after the CI/Quality gate in the README is satisfied (Requirements approved, Plan approved, ADRs reviewed, diagrams complete, risks documented).
> **Related:** [`README.md`](../../README.md) · [`Requirements.md`](./Requirements.md) · [`docs/adr/`](../adr/README.md)

## How to read this document

Work is organized into sequential **phases**. Each phase lists an Objective, Motivation, Scope, Tasks (small and actionable), Deliverables, Dependencies, Risks, Acceptance Criteria, Exit Criteria, Estimated Complexity, and the **ADRs it implements**. Phases are ordered so each builds on a proven predecessor; within a phase, tasks may run in parallel unless a dependency is noted.

**Complexity scale:** S (≤ a few days) · M (~1–2 weeks) · L (~2–4 weeks) · XL (> 4 weeks), for a single focused maintainer; indicative only.

**Requirement traceability:** each phase's Acceptance Criteria trace to `FR-`/`NFR-` IDs in `Requirements.md`.

### Cross-cutting workstreams (active in every phase)

- **Testing** (NFR-034): unit + property tests land with the code they cover; integration/stress/concurrency tests grow per phase.
- **CI/Quality gates** (NFR-033): format, lint (clippy), test, feature-combination matrix, and — once benchmarks exist — regression checks, enforced from Phase 1 onward.
- **Documentation** (NFR-041): every public item is documented as it is added; examples land with the feature they demonstrate.
- **Versioning** (ADR-015): the workspace stays in the `0.x` range through Phase 6; 1.0 is declared in Phase 7.

### Phase overview

| Phase | Title | Primary ADRs | Complexity |
|-------|-------|--------------|------------|
| 0 | Research & Architecture | all | — (in progress) |
| 1 | Core Abstractions & Public API | 002, 004, 005, 007, 011, 013, 016, 017, 018 | L |
| 2 | Memory Backend + Algorithms (single-node MVP) | 005, 008, 010 | L |
| 3 | Observability | 010, 012 | M |
| 4 | Distributed Backends (Redis + Valkey) | 005, 006, 007 | L |
| 5 | Framework Integrations (Tower, Axum, Actix) | 003, 009 | L |
| 6 | Benchmarking & Performance | 014, 006 | M |
| 7 | Hardening, Docs & 1.0 Readiness | 013, 015 | L |

---

## Phase 0 — Research & Architecture

- **Objective:** Produce a complete engineering blueprint before any production code.
- **Motivation:** The README mandates architecture-before-implementation; a wrong public API or storage abstraction is expensive to change after release.
- **Scope:** Requirements, ADRs, architecture diagrams, roadmap, open-question resolution. No production code; no crate scaffolding beyond the existing placeholder.
- **Tasks:**
  1. ✅ Author `Requirements.md` (SRS with FR/NFR, personas, use cases, risks, metrics).
  2. ✅ Resolve open questions via maintainer review; record in the Decision Log (§16).
  3. ✅ Author ADR-001 … ADR-016 under `docs/adr/`.
  4. ✅ Author this roadmap.
  5. ✅ Consolidate architecture diagrams (C4 context/container + request-flow) into `docs/diagrams/` (`.mmd` sources) and a narrative `docs/architecture/overview.md`.
- **Deliverables:** `Requirements.md` (v0.2), `docs/adr/` (16 ADRs + index + template), `Plan.md`, architecture diagrams.
- **Dependencies:** None.
- **Risks:** R-03 (API instability) and R-01 (distributed correctness) are addressed here by design review; residual risk carried into later phases.
- **Acceptance Criteria:** All Phase-0 deliverables exist and are internally consistent; every OQ is resolved or explicitly deferred with an owner (DV-01…DV-03).
- **Exit Criteria:** Requirements, Plan, and ADRs approved in review; diagrams complete; risks documented (README CI/Quality gate satisfied).
- **Estimated Complexity:** — (in progress).
- **Implements ADRs:** all.

---

## Phase 1 — Core Abstractions & Public API

- **Objective:** Establish the storage/framework-agnostic core: the public traits, the limiter builder, the decision and error types, and the degradation-policy type — with no concrete backend logic yet.
- **Motivation:** The public API and extension seams are the longest-lived artifacts (ADR-004, ADR-013); getting the shape right before backends depend on it prevents costly churn.
- **Scope:** `flowshield-core` plus the `flowshield` facade wiring (ADR-018). Public traits `StorageBackend`, `Algorithm`, `KeyExtractor`; the limiter builder; the `Decision` (allow/reject + retry-after) type; the error taxonomy; the extensible degradation-policy type; construction-time validation; and the facade's curated re-exports. Async boundary defined per ADR-016.
- **Tasks:**
  1. ☐ Define the `StorageBackend` extension trait and its documented semantics (atomic check-and-consume, TTL/expiry, typed errors) — no implementation. (ADR-005)
  2. ☐ Define the `Algorithm` extension trait. (ADR-008)
  3. ☐ Define the `KeyExtractor` extension trait. (ADR-004, ADR-009)
  4. ☐ Define the `Decision` outcome type (allowed/rejected + retry-after metadata), `#[non_exhaustive]`. (FR-011)
  5. ☐ Define the error taxonomy (config/backend/timeout), `#[non_exhaustive]`, no panics. (ADR-011)
  6. ☐ Define the extensible `DegradationPolicy` type (FailClosed default, FailOpen opt-in; room for FailToLocal). (ADR-007)
  6a. ☐ Define the injectable, monotonic **time-source abstraction** (production monotonic clock + controllable test clock); algorithm math reads time through it. (ADR-017)
  7. ☐ Design the limiter builder with construction-time validation. (ADR-004, FR-010, FR-072)
  8. ✅ Establish CI: fmt, clippy (`-D warnings`), test matrix (stable/beta × ubuntu/macOS), doc build, weekly security audit, and Dependabot. (NFR-033)
  9. ☐ Write the backend **conformance test-suite harness** (contract tests any backend can run) — behavior only, run against an in-crate test double. (FR-025)
  10. ☐ Wire the **`flowshield` facade**: re-export the stable public API from core via `#[doc(inline)]`, and establish the user-facing feature-flag host (flags added as backends/integrations land). (ADR-018)
- **Deliverables:** `flowshield-core` public API (documented, unit-tested against test doubles); conformance harness; CI pipeline.
- **Dependencies:** Phase 0 approved.
- **Risks:** Premature API lock-in → mitigate by validating the surface against the Phase-2 backends before any `0.x` stabilization; keep everything `#[non_exhaustive]`.
- **Acceptance Criteria:** Core compiles with no backend/framework deps (NFR-021); traits are implementable by an out-of-crate type (NFR-020); config validation rejects invalid limits (FR-072); errors are typed, never panics (FR-080, NFR-012).
- **Exit Criteria:** Public API reviewed and documented; conformance harness runs green against a test double; CI green.
- **Estimated Complexity:** L.
- **Implements ADRs:** 002, 004, 005, 007, 011, 013, 016, 017, 018.

---

## Phase 2 — Memory Backend + Algorithms (single-node MVP)

- **Objective:** First working end-to-end limiter: in-memory backend with Token Bucket (default) and GCRA, usable programmatically (no HTTP yet).
- **Motivation:** Proves the core abstractions against real algorithm and backend implementations before distributed complexity; delivers immediate single-node value (UC-01, UC-05).
- **Scope:** `flowshield-memory`; Token Bucket and GCRA algorithms; bounded per-key state with expiry/eviction; the optional composition helper.
- **Tasks:**
  1. ✅ Implement the in-memory backend with bounded, expiring per-key state — `flowshield-memory` (sharded LRU + lazy TTL expiry). (FR-020, FR-071)
  2. ✅ Implement Token Bucket (default). (ADR-008, FR-012)
  3. ✅ Implement GCRA. (ADR-008, FR-012)
  4. ✅ Run both algorithms + memory backend through the conformance suite. (FR-025) — memory backend passes the `StorageBackend` contract harness; Token Bucket and GCRA are driven end-to-end through the `Limiter` + `MemoryStore` in `tests/limiter_behaviour.rs`.
  5. ✅ Implement the thin composition helper (evaluate-all, binding-limit, max retry-after) — `core::composite`: pure `combine` + type-erased `LimiterSet`. (ADR-004, FR-013)
  6. ✅ Property tests for admission invariants; stress test for high-cardinality/adversarial keys. (NFR-010, NFR-034) — (σ, ρ) safety + liveness proven end-to-end through the store in `tests/admission_properties.rs`; adversarial high-cardinality + concurrency stress in `tests/stress.rs` / `tests/concurrency.rs`.
  7. ✅ Example: rate-limit a background/non-HTTP workload. (FR-004, UC-05) — `flowshield/examples/background_worker.rs` (throttled worker loop) and `layered_quotas.rs` (composite limits); both runnable via `cargo run --example`.
- **Deliverables:** `flowshield-memory`; two algorithms; composition helper; property + stress tests; a runnable non-HTTP example.
- **Dependencies:** Phase 1.
- **Risks:** R-04 (unbounded memory) → enforced eviction + adversarial stress test as acceptance gate.
- **Acceptance Criteria:** Single-node limiting works for both algorithms (FR-001, FR-002); Token Bucket is the default (FR-012); high-cardinality input does not grow memory unbounded (FR-071, NFR-010); composite limits enforce all-must-admit (FR-013).
- **Exit Criteria:** Conformance + property + stress suites green; example runs; docs for both algorithms published.
- **Estimated Complexity:** L.
- **Implements ADRs:** 005, 008, 010.

---

## Phase 3 — Observability

- **Objective:** Feature-gated metrics, tracing, and structured logging, off by default with negligible overhead when disabled.
- **Motivation:** Operators (P-04) require visibility before distributed rollout; degradation events (Phase 4) must be observable when they land.
- **Scope:** `metrics` and `tracing` features on `flowshield-core`; Prometheus-compatible metrics; OpenTelemetry-compatible spans; redaction of sensitive keys.
- **Tasks:**
  1. ✅ Add allowed/rejected counters and decision-latency instrumentation. (FR-050) — `flowshield_decisions_total{outcome,algorithm,backend}` counter + `flowshield_backend_duration_seconds` histogram, emitted once from the limiter's orchestration layer (`core::observe`).
  2. ✅ Add tracing spans around decisions and (forward-looking) backend calls. (FR-052) — a `flowshield.check` span wraps each decision via `.instrument`, so the backend await and all events nest under it (OTel-correlatable).
  3. ✅ Add structured logging for significant events with key redaction by default. (FR-051, FR-073) — degradation emits a `warn` event + `flowshield_degradations_total{policy,…}` counter; keys appear on spans only as a non-reversible FNV digest (`k#…`), never raw. A test asserts no raw key reaches output.
  4. ✅ Verify zero/negligible overhead when features are disabled (micro-benchmark or compile-out check). (NFR-023) — all emission is behind `#[cfg(feature = …)]` and the latency `Timer` is a ZST no-op when `metrics` is off, so no clock is even read; the `--no-default-features` build proves the compile-out.
- **Deliverables:** Observability behind `metrics`/`tracing` features; a Prometheus + OTel example. — ✅ `metrics`/`tracing` features on the facade forward to `flowshield-core`; `flowshield/examples/observability.rs` installs a Prometheus recorder + `tracing` subscriber. OTel export is host-side layering over the emitted `tracing` spans (ADR-012), documented in the example rather than pulling OTel deps into the library.
- **Dependencies:** Phase 1 (Phase 2 recommended for meaningful signals).
- **Risks:** R-12 (key leakage) → redaction is a default and a test asserts no raw key in output.
- **Acceptance Criteria:** Counters and latency exposable to Prometheus (FR-050); spans exportable via OTel (FR-052); features off by default and near-zero cost when disabled (NFR-023); no sensitive key emitted by default (FR-073).
- **Exit Criteria:** Observability example runs; overhead check documented; CI covers feature-on and feature-off builds. — ✅ example runs; overhead posture documented in `core::observe`; CI exercises features-on (`--all-features`) and features-off (`--no-default-features`).
- **Estimated Complexity:** M.
- **Implements ADRs:** 010, 012.
- **Status:** ✅ Complete.

---

## Phase 4 — Distributed Backends (Redis + Valkey)

- **Objective:** First-class distributed rate limiting via a shared RESP-based backend, with server-side atomicity, timeouts, and the degradation policy.
- **Motivation:** Distributed limiting (UC-02) is the project's headline capability and the hardest correctness problem (R-01).
- **Scope:** `flowshield-redis` and `flowshield-valkey` sharing a common RESP implementation; atomic check-and-consume server-side; connection pooling; mandatory operation timeout; degradation policy wired in.
- **Tasks:**
  1. ✅ Implement the shared RESP backend logic with server-side atomic check-and-consume. (ADR-006, FR-021) — `flowshield-resp`: `RespStore` runs an algorithm's Lua program via `redis::Script` (EVALSHA + EVAL fallback), one atomic round trip.
  2. ✅ Expose it as `flowshield-redis` and `flowshield-valkey` (separate crates/features). (ADR-006, FR-022) — thin newtypes over `flowshield-resp` with distinct identities/`kind` labels; facade `redis`/`valkey` features forward to them.
  3. ✅ Wire the mandatory backend timeout and the degradation policy (FailClosed default). (ADR-007, FR-023) — every op wrapped in `tokio::time::timeout` → typed `Error::Timeout`; the limiter applies the configured policy (FailClosed default).
  3a. ✅ Source decision time from the **store** (server-authoritative time) inside the atomic operation, not client clocks, to remove inter-node skew. (ADR-017, ADR-006) — `ClockSource::Server` (default) reads `redis.call('TIME')` inside the script; `Client` mode exists only for deterministic testing.
  4. ✅ Run both backends through the conformance suite. (FR-025) — the family-A `LocalStore` harness does **not** apply to server-side (family-B) backends (they don't run the Rust `step`); parity is instead proven by the **Rust-vs-Lua oracle cross-check** — both algorithms, both servers, decision-for-decision (ADR-019).
  5. ✅ Integration tests against real Redis and Valkey instances in CI. (NFR-034) — `docker-compose.yml` (Redis 6379 / Valkey 6380); reachability-gated tests; a dedicated CI `integration` job with `FLOWSHIELD_REQUIRE_*` set so unreachable ⇒ hard failure.
  6. ✅ Fault-injection tests: backend slow/unreachable → verify FailClosed and (opt-in) FailOpen behavior + observability. (UC-03, NFR-011) — unreachable-server tests assert FailClosed rejects and FailOpen admits, via the timeout path; degradation observability (`flowshield_degradations_total` + warn event) is covered by the Phase-3 core tests.
  7. ✅ Document each backend's consistency guarantee and bounded over-admission. (ADR-005, NFR-002) — ADR-005 per-backend table + a "Guarantees" section in the `flowshield-resp` docs (exact on primary; bounded only under replication/failover; microsecond resolution).
- **Deliverables:** `flowshield-redis`, `flowshield-valkey`; real-backend integration tests; fault-injection tests; documented guarantees. — ✅ plus the shared `flowshield-resp` crate (ADR-006) and `RespScripted` family-B capability in core (ADR-019).
- **Dependencies:** Phases 1–3.
- **Risks:** R-01 (distributed correctness), R-02 (backend latency dominates), R-07 (fail-open misconfig) → conformance + fault-injection + timeout as gates; degradation observable.
- **Acceptance Criteria:** Multiple processes coordinate on shared keys (FR-021, UC-02); Valkey is a supported v1 backend (FR-022); degradation matches configured policy with mandatory timeout (FR-023, NFR-011); per-backend guarantees documented (NFR-002, OQ-04).
- **Exit Criteria:** Conformance + integration + fault-injection suites green against real instances; guarantees documented; DV-01 (numeric over-admission bound) flagged for Phase 6 characterization. — ✅ oracle cross-check + integration + fault-injection green against real Redis & Valkey; guarantees documented; DV-01 remains flagged for Phase 6.
- **Estimated Complexity:** L.
- **Implements ADRs:** 005, 006, 007 (and 019, the seam revision this phase is built on).
- **Status:** ✅ Complete (Redis + Valkey, both algorithms). Multi-node over-admission characterization (DV-01) deferred to Phase 6 as planned.

---

## Phase 5 — Framework Integrations (Tower, Axum, Actix)

- **Objective:** Ship middleware integrations, each in its own crate, independent of one another and of the core's evolution.
- **Motivation:** Most users (P-01) adopt via middleware (UC-01); isolation protects the core from web-framework churn (R-06).
- **Scope:** `flowshield-tower` (foundational), `flowshield-axum` (builds on Tower where practical), `flowshield-actix` (own integration); key extraction from requests; HTTP 429 + `Retry-After` on rejection.
- **Tasks:**
  1. ✅ Implement the Tower integration (layer/service). (ADR-009, FR-030) — `flowshield-tower`: `FlowShieldLayer`/`FlowShieldService` holding an `Arc<dyn DynLimiter>`; pinned to `tower-service`/`tower-layer` (not all of `tower`) to insulate from churn.
  2. ✅ Implement the Axum integration + a default client-address `KeyExtractor`. (FR-031, FR-034) — `flowshield-axum` reuses the Tower layer and adds `ConnectInfoKeyExtractor` + `per_client_ip`; safe peer-IP default (`PeerAddrKeyExtractor` / `ConnectInfoKeyExtractor`), with `HeaderKeyExtractor` and a clearly-warned `ForwardedForKeyExtractor` for other keying.
  3. ✅ Implement the Actix integration. (FR-032) — `flowshield-actix`: standalone `Transform`/`Service` (not Tower-based), `EitherBody` short-circuit to 429, `!Send` per-worker futures; peer-IP + header + realip extractors.
  4. ✅ Map rejection → HTTP 429 with correct `Retry-After` from the decision metadata. (FR-011) — `429 Too Many Requests` + `Retry-After` (seconds, rounded up); tested in all three crates and verified over real HTTP in the axum + actix examples.
  5. ✅ Verify crate independence (Axum build excludes Actix and vice versa). (FR-033, NFR-021) — `cargo tree` confirms: `flowshield-tower` and `flowshield-actix` pull in no other integration; `flowshield-axum`'s only framework edge is the deliberate Tower reuse. Actix depends on neither Tower nor Axum.
  6. ✅ Examples per framework. (NFR-041) — runnable `flowshield-axum/examples/axum_server.rs` and `flowshield-actix/examples/actix_server.rs` (both validated over real HTTP); the Tower layer is exercised by its own tests and, end-to-end, by the Axum example (which wraps it).
- **Deliverables:** Three integration crates; per-framework examples; independence test. — ✅ `flowshield-tower`, `flowshield-axum`, `flowshield-actix`, each with tests + facade `tower`/`axum`/`actix` features (namespaced module re-exports to avoid type-name clashes).
- **Dependencies:** Phases 1–2 (4 for distributed demos).
- **Risks:** R-06 (framework churn) → thin integrations pinned to stable seams; integrations versioned independently.
- **Acceptance Criteria:** Each framework can rate-limit a route with documented setup (FR-030–FR-032); rejections carry `Retry-After` (FR-011); integrations are mutually independent (FR-033).
- **Exit Criteria:** All three examples run; independence verified in CI; docs published.
- **Estimated Complexity:** L.
- **Implements ADRs:** 003, 009.
- **Status:** ✅ Complete — `flowshield-tower`, `flowshield-axum`, and `flowshield-actix` all shipped (Layer/Service & Transform/Service, peer-IP + header + forwarded/realip extractors, 429/Retry-After, runnable examples, tests, facade features). Cross-framework independence verified.

---

## Phase 6 — Benchmarking & Performance

- **Objective:** Build the reproducible benchmark suite, gather empirical data, and set official performance targets (previously deferred).
- **Motivation:** Targets must follow measurement, not precede it (ADR-014); performance is an adoption gate for P-05.
- **Scope:** Benchmark suite (throughput, latency incl. tail, allocations, contention, 10k concurrent, Redis RTT, multi-node); CI regression gating; characterization of DV-01/DV-03.
- **Tasks:**
  1. ✅ Implement the benchmark scenarios with documented, reproducible methodology and machine-readable output. (ADR-014, FR-060) — `flowshield-benchmarks` (non-published): Criterion benches for pure-algorithm, end-to-end decision (throughput + tail, cardinality sweep), contention, and end-to-end HTTP-middleware overhead (the `flowshield-tower` layer driven in-process — the Phase-5 dependency for HTTP benchmarks); JSON-emitting scenario examples for allocations-per-decision and 10k-concurrent; reachability-gated Redis RTT bench. Methodology + a dated results snapshot in `docs/benchmarks/`.
  2. ✅ Run the 10k-concurrent and contention scenarios. (NFR-004) — `examples/concurrency_10k.rs` runs 1M decisions across 10k Tokio tasks (no deadlock/panic); `benches/contention.rs` sweeps threads × {hot-key, spread}.
  3. ☐ Characterize the Redis/Valkey over-admission bound via multi-node runs → resolve DV-01. (NFR-002) — the `redis_rtt` harness is in place; the multi-node coordination run needs dedicated infra and remains open.
  4. ☐ Establish official latency/allocation/throughput targets → resolve DV-03; set SM-01/SM-02 baselines. — deferred until a dedicated, quiesced benchmark host makes the medians stable enough to commit; a first Apple-M1 snapshot is published as a data point, not a target.
  5. ✅ Add CI performance-regression gating against tolerance. (NFR-033, NFR-035) — `scripts/bench_regression.py` (median-based, ±15% default tolerance) is the numeric gate for a dedicated runner; the per-PR `bench` CI job compiles the whole suite (no bit-rot) and runs the scenario binaries (panic/deadlock/blown-allocation ⇒ CI failure).
- **Deliverables:** Benchmark suite; published baseline results; official targets; CI regression gate. — ✅ suite + first results snapshot + regression gate delivered; **official targets (DV-03) deferred** pending dedicated hardware.
- **Dependencies:** Phases 2 and 4 (and 5 for end-to-end HTTP benchmarks).
- **Risks:** R-08 (non-reproducibility) → documented methodology + machine-readable output as acceptance gate.
- **Acceptance Criteria:** All benchmark scenarios exist and run reproducibly (FR-060); 10k-concurrent completes without deadlock/panic (NFR-004); DV-01 and DV-03 resolved and documented.
- **Exit Criteria:** Baselines published; targets ratified; regression gate active in CI.
- **Estimated Complexity:** M.
- **Implements ADRs:** 014, 006.
- **Status:** 🟡 In progress — suite, scenarios, machine-readable output, first results snapshot, and CI/regression tooling complete. Remaining: multi-node over-admission characterization (DV-01) and ratifying official targets (DV-03), both gated on a dedicated benchmark host.

---

## Phase 7 — Hardening, Documentation & 1.0 Readiness

- **Objective:** Bring FlowShield to a stable, documented, secure 1.0 with a committed MSRV and compatibility policy.
- **Motivation:** 1.0 is a promise of stability (ADR-013, ADR-015); it must be earned with test depth, security review, docs, and a versioning commitment.
- **Scope:** Test-depth completion; security review; full documentation set; MSRV decision; deprecation/compat policy; release.
- **Tasks:**
  1. ☐ Complete property/stress/concurrency coverage across algorithms and backends. (NFR-034)
  2. ☐ Security review: safe defaults, DoS/resource-exhaustion bounds, config validation, no `unsafe` footguns. (FR-070–FR-072, P-06)
  3. ☐ Documentation set: tutorial, example, API docs, design notes per public feature. (NFR-041)
  4. ☐ Fix the concrete **MSRV** and update cadence → resolve DV-02. (ADR-015)
  5. ☐ Publish the deprecation and backward-compatibility policy. (NFR-032)
  6. ☐ Finalize extension-API stability tiers and an extension-author guide. (ADR-013)
  7. ☐ Cut the 1.0 release across the crate set with a changelog. (NFR-030)
- **Deliverables:** Hardened test suite; security review report; complete docs; MSRV + compat policy; 1.0 release.
- **Dependencies:** Phases 1–6.
- **Risks:** R-03 (post-1.0 breaking changes), R-10 (maintainer bandwidth), R-11 (guarantee misunderstanding) → freeze API review before 1.0; document guarantees prominently.
- **Acceptance Criteria:** Security review passes with documented posture (Security §17); every public feature documented with tutorial/example/API/design notes (NFR-041); MSRV and deprecation policies published (NFR-031, NFR-032); SemVer honored across crates (NFR-030).
- **Exit Criteria:** 1.0 released; changelog published; extension-author guide available; Definition of Done for 1.0 met.
- **Estimated Complexity:** L.
- **Implements ADRs:** 013, 015.

---

## Deferred beyond 1.0 (tracked, not scheduled here)

These are recorded in `Requirements.md` Future Vision and are **not** part of the 1.0 roadmap:

- **Peek / non-consuming inspection** as an opt-in capability trait (FV-10, OQ-07).
- **Runtime reconfiguration** of limits (FV-11, OQ-08 → v2).
- **Fail-To-Local** degradation mode (FV-12, ADR-007).
- **Additional backends** — PostgreSQL, DynamoDB, FoundationDB (FV-04, OQ-12).
- **Additional algorithms** — Sliding Window, Fixed Window, Leaky Bucket, Sliding Log (FV-06, ADR-008).
- **Additional integrations** — e.g., Warp (FV-05).
- **Additional traffic-control modules** — concurrency limiting, quotas, adaptive throttling, request budgeting (FV-01…FV-03).

---

*End of Plan.md (v0.1, DRAFT). Awaiting review together with Requirements.md and the ADRs before implementation begins.*
