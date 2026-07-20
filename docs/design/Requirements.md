# FlowShield — Software Requirements Specification (SRS)

> **Version:** v0.2 (Phase 0 — Research & Architecture)
> **Status:** DRAFT — architectural decisions incorporated (see §16 Decision Log); pending final review
> **Scope:** Requirements only. This document deliberately contains **no implementation details** (no code, no type signatures, no concrete data structures). Design and implementation choices belong in `Plan.md`, the ADRs (`docs/adr/`), and the source code.
> **Related documents:** [`README.md`](../../README.md) (project front door) · [`charter.md`](./charter.md) (founding charter) · [`docs/adr/`](../adr/README.md) (Architecture Decision Records) · [`Plan.md`](./Plan.md) (roadmap).

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Goals](#2-goals)
3. [Non-Goals](#3-non-goals)
4. [Personas](#4-personas)
5. [User Stories](#5-user-stories)
6. [Use Cases](#6-use-cases)
7. [Functional Requirements](#7-functional-requirements-fr)
8. [Non-Functional Requirements](#8-non-functional-requirements-nfr)
9. [Constraints](#9-constraints)
10. [Assumptions](#10-assumptions)
11. [Risks](#11-risks)
12. [Success Metrics](#12-success-metrics)
13. [Future Vision](#13-future-vision)
14. [Requirement Conventions](#14-requirement-conventions)
15. [Traceability Notes](#15-traceability-notes)
16. [Decision Log (Resolved Open Questions)](#16-decision-log-resolved-open-questions)

---

## 1. Executive Summary

FlowShield aims to become the de facto Rust framework for **traffic management**. The initial deliverable is a **distributed rate limiter** with pluggable storage backends and framework integrations, architected so that additional traffic-control modules — concurrency limiting, quotas, adaptive throttling, request budgeting — can be added later without breaking the public API.

The project is being developed maintainer-first: correctness, a stable and idiomatic public API, high performance under contention, first-class observability, and production-grade documentation are treated as primary requirements rather than aspirations. The architecture separates a storage- and framework-agnostic **core** from optional backend and integration crates, so that a user pays (in dependencies, compile time, and binary size) only for what they use.

This document specifies **what** FlowShield must do and the qualities it must exhibit. It does not prescribe **how**. It establishes the shared contract that the roadmap (`Plan.md`), the architecture decision records (`docs/adr/`), and eventually the code must satisfy. It is the gate for Phase 0: implementation may not begin until the requirements herein are reviewed and approved.

The target audience for the framework is Rust backend and platform engineers building services (single-node and distributed) that must protect resources, enforce fairness, and shape traffic. The framework must be usable both as an embedded library (direct API) and as middleware in the dominant Rust web stacks.

---

## 2. Goals

The following are the primary objectives of the FlowShield project. Each maps to one or more requirements in Sections 7 and 8.

| ID | Goal | Rationale |
|----|------|-----------|
| G-01 | **Correct rate limiting** under concurrency and in distributed deployments. | A limiter that miscounts under load is worse than no limiter; correctness is non-negotiable. |
| G-02 | **Pluggable storage backends** behind a single stable abstraction. | Users must be able to start in-memory and scale to Redis/Valkey without rewriting application code. |
| G-03 | **Multiple, well-characterized algorithms** (token bucket, sliding window, GCRA, etc.). | Different workloads demand different burst/smoothness/memory trade-offs. |
| G-04 | **Clean framework integrations** (Tower, Axum, Actix) kept independent of the core. | Web-framework churn must not destabilize the core; users of one stack must not pay for another. |
| G-05 | **Stable, idiomatic, builder-based public API.** | Long-lived infrastructure requires API stability and low cognitive load. |
| G-06 | **High performance**: low latency, minimal allocations, low contention. | The limiter sits on the hot path of every request. |
| G-07 | **First-class observability** (tracing, metrics, structured logs). | Operators must be able to see limiter behavior in production. |
| G-08 | **Production-grade documentation** (tutorials, examples, API docs, design notes). | Adoption and correct usage depend on documentation quality. |
| G-09 | **Extensibility** via traits and composition, without inheritance or magic. | New backends, algorithms, and modules must be addable by third parties. |
| G-10 | **Safe defaults and DoS resistance.** | Misconfiguration should fail safe; the limiter must not become an attack amplifier. |
| G-11 | **Predictable versioning & compatibility** (SemVer, MSRV, deprecation policy). | Downstream users must be able to upgrade with confidence. |

---

## 3. Non-Goals

The following are explicitly **out of scope** for FlowShield, at least for the initial major version. Declaring them prevents scope creep and clarifies the architecture's boundaries.

| ID | Non-Goal | Reasoning |
|----|----------|-----------|
| NG-01 | Being a full API gateway or reverse proxy (routing, TLS termination, load balancing). | FlowShield is a library/middleware, not a network appliance; it composes into gateways, it is not one. |
| NG-02 | Being a general-purpose distributed coordination or consensus system. | Rate limiting requires bounded coordination, not general consensus; those concerns belong to the storage backends. |
| NG-03 | Providing a hosted service, control plane, or management UI. | The project ships libraries; operational tooling is a downstream/future concern. |
| NG-04 | Language bindings outside Rust (FFI, WASM host bindings) in the initial scope. | Focus on an excellent Rust-native API first. |
| NG-05 | Guaranteeing perfectly global, strongly-consistent counters with zero over-admission across all backends. | Some backends (e.g., eventually-consistent stores) can only offer bounded approximations; the framework will document the guarantee each backend provides rather than promise the impossible. |
| NG-06 | Web-framework feature parity for every Rust framework in existence. | The initial integration set is Tower, Axum, and Actix; others may follow but are not required for v1. |
| NG-07 | Bundled persistence/migration tooling for SQL/NoSQL backends. | Where a database backend is offered, schema/operational concerns are documented, not automated by the core. |
| NG-08 | Built-in authentication, authorization, or identity resolution. | The framework consumes a caller-supplied key/identity; deciding *who* the caller is belongs to the application. |
| NG-09 | A synchronous/blocking-first API. | The framework is async-first by design (see Constraints); a blocking facade, if any, is a future consideration. |

---

## 4. Personas

Personas ground the requirements in concrete user needs. Each user story and use case references one or more.

### P-01 — "Priya", Backend Service Engineer
Builds HTTP/gRPC services in Rust (Axum/Actix). Wants to add per-client rate limiting to an endpoint in minutes, using middleware, without understanding distributed-systems theory. Values good defaults, copy-pasteable examples, and clear error messages.

### P-02 — "Marco", Platform / Infrastructure Engineer
Owns shared platform libraries across many services and multiple nodes. Needs *distributed* rate limiting backed by Redis/Valkey, consistent behavior across nodes, and strong observability (metrics, tracing) to feed dashboards and alerts. Cares about failure modes when the backend is slow or unreachable.

### P-03 — "Dana", Library Author / Integrator
Embeds FlowShield inside another framework or product. Consumes the **core** API directly (not middleware), needs a stable trait surface to implement a custom backend or algorithm, and demands strict SemVer discipline and a clear MSRV policy.

### P-04 — "Sam", Site Reliability Engineer (Operator)
Does not write the code but runs the service. Needs to understand limiter behavior in production: which keys are throttled, rejection rates, backend latency, and how the system behaves during a backend outage. Wants safe, documented degradation.

### P-05 — "Alex", Performance Engineer
Evaluates FlowShield against alternatives and internal SLAs. Runs benchmarks for throughput, tail latency, allocations, and contention. Will only adopt if the hot path is demonstrably cheap and results are reproducible.

### P-06 — "Robin", Security Engineer
Reviews FlowShield for use in a security-sensitive context. Cares about DoS resistance, resource-exhaustion bounds, safe defaults, configuration validation, and the absence of `unsafe` foot-guns. Needs the security posture documented.

### P-07 — "Jordan", Open-Source Contributor / Maintainer
Contributes backends, algorithms, or integrations. Needs clear architecture docs, contribution guidelines, stable internal seams, and a test/CI harness that makes contributions safe to merge.

---

## 5. User Stories

User stories are written in the standard "As a … I want … so that …" form and are traceable to functional/non-functional requirements. Priorities use the MoSCoW scale defined in Section 14.

| ID | Persona | Story | Priority | Traces To |
|----|---------|-------|----------|-----------|
| US-01 | P-01 | As a service engineer, I want to add per-client rate limiting to an endpoint via middleware with a few builder calls, so that I can protect it without boilerplate. | Must | FR-001, FR-030, FR-031 |
| US-02 | P-01 | As a service engineer, I want a limiter to tell me *when* to retry when a request is rejected, so that I can return a correct `Retry-After` response. | Must | FR-011 |
| US-03 | P-02 | As a platform engineer, I want the same application code to run against an in-memory backend in tests and Redis in production, so that I don't rewrite logic per environment. | Must | FR-020, FR-021, FR-002 |
| US-04 | P-02 | As a platform engineer, I want distributed rate limiting to behave consistently across nodes, so that a client's global quota is respected regardless of which node serves it. | Must | FR-022, NFR-002 |
| US-05 | P-02 | As a platform engineer, I want a documented, configurable behavior when the storage backend is unreachable (fail-open vs. fail-closed), so that an outage degrades predictably. | Must | FR-023, FR-041, NFR-011 |
| US-06 | P-03 | As a library author, I want a stable trait to implement my own storage backend, so that I can integrate a store FlowShield doesn't ship. | Must | FR-024, NFR-020 |
| US-07 | P-03 | As a library author, I want a stable trait to implement my own algorithm, so that I can add a strategy FlowShield doesn't ship. | Should | FR-012, NFR-020 |
| US-08 | P-04 | As an operator, I want metrics for allowed/rejected counts and backend latency, so that I can build dashboards and alerts. | Must | FR-050, FR-051 |
| US-09 | P-04 | As an operator, I want tracing spans around limiter decisions and backend calls, so that I can debug latency in a distributed trace. | Should | FR-052 |
| US-10 | P-05 | As a performance engineer, I want a documented, reproducible benchmark suite, so that I can validate FlowShield meets my latency and throughput SLAs. | Must | FR-060, NFR-001, NFR-003 |
| US-11 | P-06 | As a security engineer, I want safe defaults and bounded memory usage, so that FlowShield cannot be turned into a resource-exhaustion vector. | Must | FR-070, FR-071, NFR-010 |
| US-12 | P-06 | As a security engineer, I want all configuration validated at construction time with clear errors, so that invalid limits are rejected before serving traffic. | Must | FR-072, FR-010 |
| US-13 | P-01 | As a service engineer, I want to choose a rate-limiting algorithm (e.g., token bucket vs. sliding window) via configuration, so that I can match burst behavior to my workload. | Should | FR-012 |
| US-14 | P-01 | As a service engineer, I want to define multiple named limits (e.g., per-second and per-day) that all apply, so that I can enforce layered quotas. | Should | FR-013 |
| US-15 | P-02 | As a platform engineer, I want to inspect a client's remaining quota without consuming it, so that I can expose quota headers or dashboards. | Could | FR-014 |
| US-16 | P-03 | As a library author, I want to use the core limiter directly (no web framework), so that I can rate-limit non-HTTP work such as queue consumers or background jobs. | Must | FR-001, FR-004 |
| US-17 | P-07 | As a contributor, I want architecture docs and a conformance test suite for backends, so that my new backend is provably correct. | Should | FR-025, NFR-021, NFR-032 |
| US-18 | P-01 | As a service engineer, I want to enable only the backend and integration I use via feature flags, so that my build stays lean. | Must | FR-003, NFR-023 |
| US-19 | P-04 | As an operator, I want to atomically update limit configuration where the design allows, so that I can tighten limits during an incident. | Could | FR-015 |
| US-20 | P-02 | As a platform engineer, I want limiter keys to be arbitrary caller-defined identities (IP, API key, tenant, composite), so that I control the granularity of limiting. | Must | FR-005 |

---

## 6. Use Cases

Use cases describe end-to-end interactions at a behavioral level (no implementation detail).

### UC-01 — Rate-limit an HTTP endpoint (single node)
- **Primary actor:** P-01 (Priya)
- **Precondition:** A running Axum/Actix service; FlowShield core + memory backend + framework integration available.
- **Main flow:** Engineer configures a limiter (algorithm, limit, key extractor) via a builder and attaches it as middleware. For each request, the middleware derives the key, asks the limiter for a decision, and either forwards the request or returns a rejection with retry metadata.
- **Postcondition:** Requests exceeding the limit are rejected; allowed requests pass through unchanged.
- **Traces to:** FR-001, FR-005, FR-011, FR-030, FR-031.

### UC-02 — Distributed rate limiting across N nodes
- **Primary actor:** P-02 (Marco)
- **Precondition:** Multiple service instances share a Redis/Valkey backend.
- **Main flow:** Each node uses the identical limiter configuration pointed at the shared backend. Decisions for a given key are coordinated through the backend so the aggregate rate respects the configured limit within the backend's documented guarantee.
- **Postcondition:** A client's global rate is bounded regardless of node affinity.
- **Traces to:** FR-020, FR-021, FR-022, NFR-002.

### UC-03 — Backend degradation
- **Primary actor:** P-04 (Sam) / P-02 (Marco)
- **Precondition:** Distributed backend becomes slow or unreachable.
- **Main flow:** The limiter enforces a configured timeout on backend operations; on failure it applies the configured degradation policy (fail-open or fail-closed) and emits observability signals indicating degraded operation.
- **Postcondition:** The service continues under a documented, predictable policy; operators are alerted via metrics.
- **Traces to:** FR-023, FR-041, NFR-011, FR-050.

### UC-04 — Implementing a custom backend
- **Primary actor:** P-03 (Dana)
- **Precondition:** A store FlowShield does not ship (e.g., a bespoke KV service).
- **Main flow:** The author implements the published storage abstraction and validates it against the provided backend conformance test suite, then uses it via the same core API.
- **Postcondition:** The custom backend is interchangeable with shipped backends.
- **Traces to:** FR-024, FR-025, NFR-020, NFR-021.

### UC-05 — Rate-limiting non-HTTP work
- **Primary actor:** P-03 (Dana)
- **Precondition:** A background worker or queue consumer.
- **Main flow:** The application calls the core limiter API directly (no middleware) to gate units of work by key.
- **Postcondition:** Non-HTTP workloads are limited using the same core.
- **Traces to:** FR-001, FR-004.

### UC-06 — Layered / composite limits
- **Primary actor:** P-01 (Priya)
- **Precondition:** A client must satisfy several limits simultaneously (e.g., 10/s and 1000/day).
- **Main flow:** The application configures multiple named limits; a request is admitted only if all applicable limits admit it, and rejection reports which limit was exceeded.
- **Postcondition:** Layered quotas are enforced.
- **Traces to:** FR-013.

### UC-07 — Observing limiter behavior
- **Primary actor:** P-04 (Sam)
- **Precondition:** Metrics/tracing wired to the service.
- **Main flow:** The limiter emits metrics (allowed/rejected, backend latency, degradation events) and tracing spans; the operator builds dashboards/alerts.
- **Postcondition:** Limiter behavior is observable in production.
- **Traces to:** FR-050, FR-051, FR-052.

### UC-08 — Benchmark-driven evaluation
- **Primary actor:** P-05 (Alex)
- **Precondition:** Benchmark suite available.
- **Main flow:** The engineer runs the documented benchmarks (throughput, latency, allocations, contention, 10k concurrent, Redis RTT, multi-node) and compares against SLAs.
- **Postcondition:** Adoption decision is backed by reproducible data.
- **Traces to:** FR-060, NFR-001, NFR-003.

---

## 7. Functional Requirements (FR)

Each requirement has an **ID**, **Description**, **Priority**, and **Acceptance Criteria**. Requirements describe observable behavior and capabilities only — not how they are implemented.

### 7.1 Core Limiting

#### FR-001 — Admission decision
- **Description:** The framework shall provide a core capability to decide whether a unit of demand identified by a key is admitted or rejected under a configured limit, independent of any web framework.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Given a configured limit and a key, the framework returns a decision of *allowed* or *rejected*.
  2. The decision is usable in a non-HTTP context (e.g., direct programmatic call).
  3. Repeated demand beyond the configured limit within its window/rate is rejected; demand within the limit is allowed.

#### FR-002 — Backend-agnostic behavior
- **Description:** The core admission behavior shall be identical (within each backend's documented consistency guarantee) regardless of which storage backend is configured.
- **Priority:** Must
- **Acceptance Criteria:**
  1. The same application code, run against two different backends, produces the same allow/reject outcomes for the same single-node input sequence.
  2. Backend selection does not require changes to application-level limiting logic.

#### FR-003 — Feature-gated composition
- **Description:** Backends and framework integrations shall be independently selectable so that only chosen components are compiled in.
- **Priority:** Must
- **Acceptance Criteria:**
  1. A build enabling only the core and the memory backend does not pull in Redis/Valkey/framework dependencies.
  2. Enabling an integration or backend is a documented, additive configuration step.

#### FR-004 — Synchronous programmatic usage without middleware
- **Description:** The core shall be usable directly by application code without any middleware layer.
- **Priority:** Must
- **Acceptance Criteria:**
  1. A background/non-HTTP workload can obtain admission decisions using only the core API.
  2. No web-framework crate is required to use the core.

#### FR-005 — Caller-defined keys
- **Description:** The framework shall limit based on an arbitrary caller-supplied key/identity (e.g., IP, API key, tenant ID, composite), and shall not impose an identity model.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Any caller-provided key value is accepted and limited independently.
  2. Two distinct keys are limited independently of one another.
  3. The framework does not resolve or authenticate identity itself.

### 7.2 Configuration & Limits

#### FR-010 — Builder-based configuration
- **Description:** Limiters shall be configured through a builder-style, discoverable configuration surface.
- **Priority:** Must
- **Acceptance Criteria:**
  1. A limiter can be fully configured (algorithm, limit, backend, key strategy, policies) via a builder.
  2. Required parameters missing at build time produce a clear, actionable error rather than a silent default that changes safety posture.

#### FR-011 — Retry / wait metadata on rejection
- **Description:** On rejection, the framework shall provide metadata sufficient to compute when the caller may retry.
- **Priority:** Must
- **Acceptance Criteria:**
  1. A rejected decision carries information from which a retry-after duration can be derived.
  2. The metadata is expressed in units suitable for producing standard HTTP retry semantics.

#### FR-012 — Selectable algorithm
- **Description:** The framework shall allow selection among rate-limiting algorithms via configuration, and shall define a stable extension point for additional algorithms. For v1 the shipped algorithms are **Token Bucket** (the default) and **GCRA**; further algorithms are deferred (see Future Vision). *Decision:* ADR-008; resolves OQ-02.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Token Bucket and GCRA are both selectable at configuration time in v1, with Token Bucket as the documented default.
  2. Each shipped algorithm's burst/smoothing/memory characteristics and distributed suitability are documented.
  3. A third party can supply an additional algorithm through the published extension point without forking (see NFR-020, ADR-013).

#### FR-013 — Composite / layered limits by explicit composition
- **Description:** The framework shall support enforcing more than one limit simultaneously (e.g., global, per-tenant, per-user, per-API-key), admitting only if all applicable limits admit. This is achieved by **explicit composition of independent limiters**, not by a configuration DSL. The framework shall additionally provide an **optional, thin composition helper** that evaluates a set of limiters and returns a combined decision. *Decision:* ADR-004; resolves OQ-10.
- **Priority:** Should
- **Acceptance Criteria:**
  1. An application can compose two or more independent limiters such that a request is admitted only when every one admits it.
  2. The optional composition helper reports which limit was binding on rejection and the correct (maximum) retry-after across the set.
  3. Each limiter remains usable independently; the helper adds nothing to the single-limiter hot path and can be ignored.
  4. No bespoke rule/configuration language is introduced.

#### FR-014 — Non-consuming inspection ("peek")
- **Description:** Non-consuming inspection of remaining allowance is **out of scope for v1**. Not every algorithm/backend can report accurate inspection information, and a partially-accurate API would create inconsistent behavior. When introduced later, it shall arrive as an **opt-in capability trait** that a backend/algorithm advertises, never as a silent approximation. *Decision:* ADR-004 / ADR-013; resolves OQ-07. See Future Vision FV-10.
- **Priority:** Won't (v1) — deferred
- **Acceptance Criteria:**
  1. v1 does not expose a non-consuming inspection API.
  2. The deferral and its rationale are documented; any future inspection support is gated behind an advertised capability, not assumed.

#### FR-015 — Runtime reconfiguration of limits
- **Description:** Runtime reconfiguration of limits without a restart is **deferred to v2**. Changing policies while requests are in flight raises bucket-migration, active-window, and distributed-synchronization questions that require dedicated research; v1 prioritizes correctness and simplicity. *Decision:* resolves OQ-08. See Future Vision FV-11.
- **Priority:** Won't (v1) — deferred to v2
- **Acceptance Criteria:**
  1. v1 does not claim runtime reconfiguration support.
  2. The deferral is documented, and the v1 configuration surface (ADR-004) is designed so that runtime reconfiguration can be added later without breaking existing configuration.

### 7.3 Storage Backends

#### FR-020 — In-memory backend
- **Description:** The framework shall provide an in-memory, single-process storage backend suitable for single-node use, tests, and development.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Limiting works with no external dependency using the memory backend.
  2. The memory backend enforces the same admission semantics as other backends for single-node input.
  3. Memory usage is bounded per the eviction/expiry requirements (see FR-071).

#### FR-021 — Redis backend
- **Description:** The framework shall provide a Redis-based storage backend for distributed rate limiting.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Multiple processes sharing one Redis instance coordinate limiting for shared keys.
  2. The backend's consistency and atomicity guarantees are documented.
  3. Backend operation failures are surfaced to the degradation policy (FR-023).

#### FR-022 — Valkey backend
- **Description:** The framework shall provide a Valkey-based storage backend for distributed rate limiting as a **first-class, vendor-neutral peer to Redis**. Redis and Valkey are RESP-compatible today and share a common backend implementation while remaining separately selectable so they can diverge later. *Decision:* ADR-006; resolves OQ-03.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Distributed limiting works against Valkey with the same admission semantics as the Redis backend for equivalent input.
  2. Valkey is a supported v1 backend (alongside Memory and Redis), independently selectable via feature flag (see NFR-023, ADR-010).
  3. Any behavioral differences from the Redis backend are documented.

#### FR-023 — Backend degradation policy
- **Description:** The framework shall support a configurable, extensible policy for how admission decisions behave when a backend operation times out or fails. The **default is Fail Closed** (reject when the backend is unavailable); **Fail Open is available only via explicit opt-in**. The policy is modeled as an extensible choice (not a boolean) so additional modes (e.g., a planned *Fail-To-Local*) can be added without breaking changes. *Decision:* ADR-007; resolves OQ-01.
- **Priority:** Must
- **Acceptance Criteria:**
  1. The degradation policy is configurable per limiter, defaulting to Fail Closed.
  2. Fail Open (admit on backend unavailability) requires an explicit, documented opt-in.
  3. Backend operations are subject to a mandatory, configurable timeout so Fail Closed never blocks indefinitely.
  4. Degradation events are observable via metrics/logs (see FR-050, FR-051).
  5. The policy type is extensible so a future *Fail-To-Local* mode can be added additively.

#### FR-024 — Storage abstraction / extension point
- **Description:** The framework shall expose a stable storage abstraction so that third parties can implement additional backends.
- **Priority:** Must
- **Acceptance Criteria:**
  1. A third party can implement a new backend against the published abstraction without modifying FlowShield.
  2. The abstraction's required semantics (atomicity, expiry, error reporting) are documented.

#### FR-025 — Backend conformance suite
- **Description:** The framework shall provide a reusable conformance test suite that any backend (shipped or third-party) can run to validate correct semantics, including **deterministic logical behavior**: given identical inputs and conditions, conforming backends produce identical logical decisions within their documented consistency guarantee. *Decision:* ADR-005 (determinism), ADR-017 (controllable time source).
- **Priority:** Should
- **Acceptance Criteria:**
  1. All shipped backends pass the conformance suite.
  2. The suite is runnable against a third-party backend.
  3. The suite covers admission correctness, expiry, and error/degradation reporting.
  4. The suite verifies deterministic logical decisions against a reference oracle — exact match for exact backends, and match within the declared over-admission bound for bounded-staleness backends — using an injectable time source (ADR-017) so time-dependent conditions are reproducible.

### 7.4 Framework Integrations

#### FR-030 — Tower integration
- **Description:** The framework shall provide a Tower-compatible integration usable across the Tower ecosystem.
- **Priority:** Must
- **Acceptance Criteria:**
  1. FlowShield can be applied as a Tower layer/service to gate requests.
  2. The integration lives in a crate separate from the core.
  3. Rejections carry retry metadata (FR-011) to the caller.

#### FR-031 — Axum integration
- **Description:** The framework shall provide an Axum integration (directly or via the Tower integration) for HTTP services.
- **Priority:** Must
- **Acceptance Criteria:**
  1. An Axum route/router can be rate-limited with documented setup.
  2. Rejected requests produce an appropriate HTTP response including retry metadata.

#### FR-032 — Actix integration
- **Description:** The framework shall provide an Actix Web integration for HTTP services.
- **Priority:** Should
- **Acceptance Criteria:**
  1. An Actix handler/scope can be rate-limited with documented setup.
  2. Rejected requests produce an appropriate HTTP response including retry metadata.

#### FR-033 — Integration independence
- **Description:** Each framework integration shall be independent of the others and of unneeded backends.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Depending on the Axum integration does not pull in Actix (and vice versa).
  2. A breaking change in one web framework does not force a breaking change in the core.

#### FR-034 — Key extraction in HTTP context
- **Description:** HTTP integrations shall allow the application to define how the limiting key is extracted from a request.
- **Priority:** Must
- **Acceptance Criteria:**
  1. The application can supply a strategy to derive the key from request attributes.
  2. A sensible, documented default is available for common cases (e.g., client address).

### 7.5 Observability

#### FR-050 — Metrics
- **Description:** The framework shall emit metrics covering admission outcomes and backend health.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Counters exist for allowed and rejected decisions.
  2. Backend operation latency and failure/degradation events are measurable.
  3. Metrics are exposable to a Prometheus-compatible pipeline.
  4. Emitting metrics is optional/feature-gated and imposes negligible overhead when disabled.

#### FR-051 — Structured logging
- **Description:** The framework shall support structured, leveled logging of significant events (e.g., degradation, configuration errors).
- **Priority:** Should
- **Acceptance Criteria:**
  1. Significant events are logged with structured fields.
  2. Logging integrates with the standard Rust tracing/logging ecosystem.
  3. Logging never emits secrets or full key values where those may be sensitive (see FR-073).

#### FR-052 — Tracing spans
- **Description:** The framework shall provide tracing instrumentation around admission decisions and backend operations, compatible with OpenTelemetry export.
- **Priority:** Should
- **Acceptance Criteria:**
  1. Admission decisions and backend calls can be represented as spans in a distributed trace.
  2. Instrumentation is feature-gated and adds negligible overhead when disabled.

### 7.6 Performance & Benchmarking

#### FR-060 — Benchmark suite
- **Description:** The framework shall provide a documented, reproducible benchmark suite.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Benchmarks exist for throughput, latency, allocations, and contention.
  2. Scenarios include high concurrency (target: 10k concurrent requests), Redis round-trip cost, and a multi-node scenario.
  3. Benchmarks are runnable via a documented command and report comparable results across runs.

### 7.7 Security & Safety

#### FR-070 — Safe defaults
- **Description:** Default configurations shall be safe: conservative limits and a documented default degradation posture.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Out-of-the-box defaults do not silently disable limiting.
  2. The default degradation policy is documented and justified with its trade-offs.

#### FR-071 — Bounded resource usage
- **Description:** The framework shall bound the memory used to track keys (e.g., via expiry/eviction) to prevent unbounded growth from a large or adversarial key space.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Per-key state is subject to expiry/eviction.
  2. A high-cardinality key stream does not cause unbounded memory growth in the memory backend.
  3. The bounding mechanism and its trade-offs are documented.

#### FR-072 — Configuration validation
- **Description:** The framework shall validate configuration at construction time and reject invalid configurations with clear errors.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Invalid limit parameters (e.g., zero/negative rates where nonsensical) are rejected before serving traffic.
  2. Validation errors are descriptive and actionable.

#### FR-073 — Sensitive-value handling
- **Description:** The framework shall avoid leaking sensitive key material in logs, errors, or telemetry by default.
- **Priority:** Should
- **Acceptance Criteria:**
  1. Keys that may contain sensitive data are not emitted verbatim by default in logs/telemetry.
  2. Any opt-in to emit raw keys is explicit and documented.

### 7.8 Error Handling

#### FR-080 — Explicit, typed error reporting
- **Description:** The framework shall report operational failures (backend errors, configuration errors, timeouts) through explicit, inspectable error reporting rather than panics.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Recoverable failures are surfaced as errors, not panics.
  2. Errors are distinguishable by category (e.g., configuration vs. backend vs. timeout) so callers can react appropriately.
  3. The distinction between "rejected by limit" (a normal outcome) and "operation failed" (an error) is unambiguous.

---

## 8. Non-Functional Requirements (NFR)

### 8.1 Performance

#### NFR-001 — Hot-path latency (local backend)
- **Description:** An admission decision against the in-memory backend shall add minimal latency to the request hot path.
- **Priority:** Must
- **Acceptance Criteria:**
  1. A documented benchmark reports memory-backend decision latency, and a target threshold is established and tracked over releases.
  2. Regressions beyond an agreed tolerance are flagged by CI (see NFR-033).

#### NFR-002 — Distributed correctness bound
- **Description:** Distributed backends shall respect the configured limit within a documented over-/under-admission bound.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Each distributed backend documents its consistency guarantee and any bounded over-admission.
  2. A multi-node test demonstrates aggregate admission stays within the documented bound for defined workloads.

#### NFR-003 — Low allocation on the hot path
- **Description:** The admission hot path shall minimize heap allocations.
- **Priority:** Should
- **Acceptance Criteria:**
  1. An allocation benchmark reports per-decision allocations for the memory backend.
  2. A target is set and tracked; regressions are flagged.

#### NFR-004 — Concurrency scalability
- **Description:** Throughput shall scale acceptably under high concurrency without pathological contention.
- **Priority:** Must
- **Acceptance Criteria:**
  1. A 10k-concurrent-request benchmark completes without deadlock, unbounded latency growth, or panics.
  2. Contention behavior is documented.

### 8.2 Reliability & Availability

#### NFR-010 — Robustness under adversarial input
- **Description:** The framework shall remain stable under adversarial input (high key cardinality, hostile retry storms).
- **Priority:** Must
- **Acceptance Criteria:**
  1. Adversarial key/traffic patterns do not cause panics or unbounded resource growth (ties to FR-071).
  2. A stress test covering these patterns exists and passes.

#### NFR-011 — Predictable degradation
- **Description:** Backend failures shall degrade according to the documented, configured policy rather than in an undefined manner.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Under injected backend failure/latency, behavior matches the configured degradation policy (FR-023).
  2. Degradation is observable via metrics/logs.

#### NFR-012 — No data-corruption panics
- **Description:** The framework shall not panic on recoverable operational errors.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Backend, timeout, and configuration failures are returned as errors (ties to FR-080), not panics.
  2. Fuzz/property tests do not surface panics on valid API usage.

### 8.3 Maintainability & Extensibility

#### NFR-020 — Stable extension seams
- **Description:** The public extension points — **StorageBackend, Algorithm, KeyExtractor, and framework integration (middleware)** — shall be stable across a major version so third-party implementations remain valid. Traits intended for third-party implementation are **not sealed**; internal machinery may be sealed. Evolution is **additive-only** within a major version (default-provided methods, new opt-in capability traits, and `#[non_exhaustive]` public enums/errors/config). *Decision:* ADR-013; resolves OQ-13.
- **Priority:** Must
- **Acceptance Criteria:**
  1. StorageBackend, Algorithm, and KeyExtractor are implementable by third-party crates without forking FlowShield.
  2. Extension-point changes within a major version are backward compatible (no new required trait methods; additive via defaults/capability traits).
  3. Breaking changes to public extension points require a major-version bump and a documented migration path.
  4. Optional capabilities (e.g., peek/inspection, batch, runtime reconfiguration) are expressed as separate opt-in capability traits, not added to base traits.

#### NFR-021 — Core/integration separation
- **Description:** The core shall have no dependency on any web framework; integrations shall depend on the core, not vice versa.
- **Priority:** Must
- **Acceptance Criteria:**
  1. The core crate compiles and tests without any web-framework dependency.
  2. Dependency direction is enforced and documented.

#### NFR-022 — Documented architecture
- **Description:** The architecture and significant decisions shall be documented (ADRs, diagrams) and kept in sync with the code.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Each significant decision has an ADR with context, decision, alternatives, consequences.
  2. Architecture diagrams exist and are referenced from the docs.

#### NFR-023 — Minimal default footprint
- **Description:** Default builds shall be lean, with optional functionality behind a **minimal, additive feature-flag set** hosted on the **`flowshield` facade crate** (ADR-018): `default = ["memory"]`, plus `memory`, `redis`, `valkey`, `tower`, `axum`, `actix`, `metrics`, `tracing`. A small flag set is chosen deliberately to bound the combinatorial test matrix. *Decision:* ADR-010, ADR-018; resolves OQ-09.
- **Priority:** Must
- **Acceptance Criteria:**
  1. A minimal build includes only the core and the memory backend (`default = ["memory"]`).
  2. Enabling one backend or integration does not pull in the others; features are additive.
  3. `metrics` and `tracing` are off by default and impose negligible overhead when disabled.
  4. CI exercises a documented set of feature combinations.

### 8.4 Compatibility & Versioning

#### NFR-030 — Semantic Versioning
- **Description:** The project shall follow Semantic Versioning for all published crates.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Breaking API changes require a major-version bump.
  2. A public changelog records notable changes per release.

#### NFR-031 — MSRV policy
- **Description:** During the pre-1.0 research and development phase, the project shall **not commit to a Minimum Supported Rust Version** and shall develop against the latest stable Rust. **Before the first stable (1.0) release**, a dedicated decision shall fix the MSRV, its update cadence, and the compatibility guarantee. *Decision:* ADR-015; resolves OQ-05.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Pre-1.0, no MSRV is advertised; CI builds against current stable Rust.
  2. An MSRV and its update policy are defined and documented before the 1.0 release.
  3. Once set, MSRV increases are treated as notable changes governed by the documented policy.

#### NFR-032 — Deprecation & backward-compatibility policy
- **Description:** The project shall define how APIs are deprecated and how backward compatibility is maintained.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Deprecations are announced with a documented removal timeline.
  2. Within a major version, previously stable public APIs continue to work.

### 8.5 Quality Gates & Testing

#### NFR-033 — CI quality gates
- **Description:** Continuous integration shall enforce build, lint, format, test, MSRV, and performance-regression gates.
- **Priority:** Must
- **Acceptance Criteria:**
  1. CI runs formatting, linting, and the test suite on every change.
  2. CI checks MSRV and flags performance regressions beyond tolerance.
  3. Merges are blocked when gates fail.

#### NFR-034 — Test coverage across strategies
- **Description:** The framework shall be tested with unit, integration, property, stress, and concurrency tests, plus real-backend integration tests.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Each shipped algorithm and backend has unit and integration coverage.
  2. Property tests cover admission invariants; stress/concurrency tests cover high load.
  3. Redis/Valkey integration tests run against real instances in CI.

#### NFR-035 — Reproducible benchmarks
- **Description:** Benchmarks shall be reproducible and comparable across runs and environments.
- **Priority:** Should
- **Acceptance Criteria:**
  1. Benchmark methodology and environment assumptions are documented.
  2. Results are emitted in a comparable, machine-readable form.

### 8.6 Usability & Documentation

#### NFR-040 — Idiomatic, discoverable API
- **Description:** The public API shall be idiomatic, async-first, builder-based, and discoverable.
- **Priority:** Must
- **Acceptance Criteria:**
  1. Common tasks are achievable through a builder without reading source.
  2. The API follows established Rust API guidelines.

#### NFR-041 — Documentation completeness
- **Description:** Every public feature shall eventually ship with a tutorial, an example, API docs, and design notes.
- **Priority:** Should
- **Acceptance Criteria:**
  1. Public items carry API documentation.
  2. At least one runnable example exists per major feature/integration.
  3. Design notes/ADRs are linked from the docs.

#### NFR-042 — Actionable errors and diagnostics
- **Description:** Errors and misconfigurations shall be actionable, guiding the user toward a fix.
- **Priority:** Should
- **Acceptance Criteria:**
  1. Configuration and operational errors include enough context to diagnose the cause.
  2. Common misconfigurations have documented remedies.

### 8.7 Portability

#### NFR-050 — Platform support
- **Description:** The core and memory backend shall build and pass tests on the primary supported platforms (Linux, macOS; Windows on a best-effort basis).
- **Priority:** Should
- **Acceptance Criteria:**
  1. CI builds and tests the core on Linux and macOS.
  2. Platform-specific limitations, if any, are documented.

#### NFR-051 — Async runtime posture
- **Description:** **Tokio is the primary, officially-supported async runtime.** The core shall remain runtime-independent as far as practical; where a backend's client library couples it to a runtime, that coupling shall be explicit and documented. Additional runtimes may be supported later via separate integration crates. *Decision:* ADR-016; resolves OQ-06.
- **Priority:** Should
- **Acceptance Criteria:**
  1. Tokio is documented as the primary supported runtime.
  2. The core's runtime-independence boundary is documented; any runtime coupling in backends is explicit.
  3. Adding another runtime later does not require breaking the core.

---

## 9. Constraints

Constraints are fixed conditions the solution must respect. They are requirements-level facts, not design choices.

| ID | Constraint | Source |
|----|------------|--------|
| C-01 | The framework is **async-first**; the public API is designed around asynchronous execution. | Project Philosophy (README) |
| C-02 | The project uses a **multi-crate workspace** separating core, backends, and integrations. | Architecture Guidelines (README) |
| C-03 | Framework integrations must be **independent of the core** and of one another. | Architecture Guidelines / FR-033 |
| C-04 | The baseline toolchain is the **workspace-declared Rust edition and version** (currently edition 2024, Rust ≥ 1.88); MSRV is governed by NFR-031. | `Cargo.toml` workspace settings |
| C-05 | Crates are published under a **permissive dual license (MIT OR Apache-2.0)**. | `Cargo.toml` workspace license |
| C-06 | **No production/implementation code** may be written during Phase 0; only research and design artifacts. | README "Important Rules" |
| C-07 | The public API must be **stable** and governed by SemVer once released. | API Principles / NFR-030 |
| C-08 | Distributed backends are limited by their underlying store's **consistency and network characteristics**; the framework cannot exceed those guarantees. | Distributed-systems reality / NG-05 |
| C-09 | The crate naming convention is the **`flowshield-` prefix** for workspace members. | Existing workspace layout |
| C-10 | Observability integrations target the **standard Rust ecosystem** (tracing, OpenTelemetry, Prometheus). | Observability (README) |

---

## 10. Assumptions

Assumptions are conditions believed true; if any proves false, associated requirements/risks must be revisited.

| ID | Assumption | If False |
|----|------------|----------|
| A-01 | Users primarily operate in an async Rust environment (e.g., Tokio-based services). | A blocking facade or alternative runtime support may be required, expanding scope. |
| A-02 | Redis/Valkey are acceptable and available as the primary distributed backends for most users. | Additional first-class backends may need to be prioritized earlier. |
| A-03 | Most users can tolerate a small, bounded over-admission in distributed mode in exchange for performance and availability. | Strong-consistency backends/algorithms would need higher priority (affects NG-05, FR-022). |
| A-04 | The Tower/Axum/Actix ecosystems remain the dominant integration targets during the initial development horizon. | The integration set (FR-030–FR-032) may need revision. |
| A-05 | Providers of custom backends can meet the documented storage semantics (atomicity/expiry). | The storage abstraction (FR-024) may need to relax or split into capability tiers. |
| A-06 | Reproducible benchmarking is feasible in the target CI environment. | NFR-035/NFR-001 may need environment-normalized methodology. |
| A-07 | The maintainer team can sustain SemVer/MSRV/deprecation discipline over the long term. | Versioning policy (NFR-030–NFR-032) may need automation or narrowing. |
| A-08 | Application code, not the framework, is responsible for identity resolution and key derivation. | Scope grows if identity handling must move into the framework (contradicts NG-08). |

---

## 11. Risks

Risks are rated by **Likelihood** (L/M/H) and **Impact** (L/M/H) with a mitigation strategy. Each risk references the requirements it threatens.

| ID | Risk | L | I | Threatens | Mitigation |
|----|------|---|---|-----------|------------|
| R-01 | **Distributed correctness is hard**: naive coordination over-admits or over-rejects under load. | H | H | FR-002, FR-022, NFR-002 | Choose algorithms with known distributed behavior; document guarantees; enforce with multi-node tests and a conformance suite (FR-025). |
| R-02 | **Backend latency dominates the hot path**, negating performance goals in distributed mode. | H | M | NFR-001, G-06 | Enforce timeouts and degradation (FR-023); document local vs. distributed latency separately; benchmark Redis RTT (FR-060). |
| R-03 | **API instability** forces breaking changes after release, eroding trust. | M | H | NFR-020, NFR-030, G-05 | Invest in API design/ADRs before coding; ship builder-based, minimal surface; gate 1.0 on API review. |
| R-04 | **Unbounded memory** from high-cardinality/adversarial keys. | M | H | FR-071, NFR-010 | Mandatory expiry/eviction; stress tests with adversarial cardinality; document bounds. |
| R-05 | **Feature-flag / crate matrix complexity** causes broken build combinations. | M | M | FR-003, NFR-023, NFR-033 | CI matrix over feature combinations; keep the flag taxonomy small and documented. |
| R-06 | **Web-framework churn** (Axum/Actix/Tower breaking changes) destabilizes integrations. | M | M | FR-030–FR-033, NFR-021 | Keep integrations thin and isolated; depend on stable seams; version integrations independently. |
| R-07 | **Fail-open misconfiguration** silently disables limiting during an outage, enabling abuse. | M | H | FR-023, FR-070, NG-05 | Safe documented default; loud observability on degradation (FR-050); explicit opt-in for fail-open. |
| R-08 | **Benchmark non-reproducibility** undermines performance claims. | M | M | NFR-035, FR-060 | Documented methodology; machine-readable output; environment normalization. |
| R-09 | **Scope creep** into gateway/identity/consensus territory. | M | M | Non-Goals (§3) | Enforce non-goals in review; require ADRs for scope changes. |
| R-10 | **Maintainer bandwidth** insufficient to uphold docs/versioning/testing standards. | M | M | G-08, G-11, NFR-041 | Automate quality gates; prioritize Must requirements; keep surface small. |
| R-11 | **Consistency-guarantee misunderstanding** by users who expect exact global limits. | M | M | NG-05, NFR-002 | Prominent documentation of each backend's guarantee; explicit guarantee labeling in APIs/docs. |
| R-12 | **Sensitive key leakage** via logs/telemetry. | L | H | FR-073, FR-051 | Redact by default; explicit opt-in for raw keys; security review. |

---

## 12. Success Metrics

Success metrics make the goals measurable. Targets marked *TBD-in-Plan* are to be finalized with baselines during roadmap planning; the requirement here is that the metric is tracked.

| ID | Metric | Target / Definition of Success | Traces To |
|----|--------|-------------------------------|-----------|
| SM-01 | Memory-backend decision latency | Meets an established, documented threshold; no unexplained regression across releases. | NFR-001, FR-060 |
| SM-02 | Per-decision allocations (hot path) | Established target met and tracked over time. | NFR-003 |
| SM-03 | 10k-concurrent benchmark | Completes without deadlock/panic; latency stays within a documented envelope. | NFR-004, FR-060 |
| SM-04 | Distributed over-admission | Stays within each backend's documented bound in multi-node tests. | NFR-002 |
| SM-05 | Backend/algorithm conformance | 100% of shipped backends and algorithms pass the conformance/test suites. | FR-025, NFR-034 |
| SM-06 | CI gate pass rate on `main` | `main` is always green across format/lint/test/MSRV gates. | NFR-033 |
| SM-07 | Documentation coverage | Every public item documented; ≥1 runnable example per major feature/integration. | NFR-041 |
| SM-08 | API stability | Zero unplanned breaking changes within a major version post-1.0. | NFR-030, NFR-020 |
| SM-09 | Minimal build footprint | Core + one backend build excludes all unused backend/integration deps. | NFR-023, FR-003 |
| SM-10 | Degradation observability | 100% of injected backend-failure scenarios emit the expected metrics/logs. | FR-050, NFR-011 |
| SM-11 | Adoption signal (long-term) | Growth in downloads/dependents and community-contributed backends/integrations. | G-01, G-09 |

---

## 13. Future Vision

The following are anticipated future directions. They are **not** requirements for the initial scope but inform the architecture so that adding them later does not require breaking changes. They align with the extension points mandated by FR-012, FR-024, and NFR-020.

| ID | Future Capability | Notes |
|----|-------------------|-------|
| FV-01 | **Concurrency limiting** (in-flight request caps) as an additional traffic-control module. | Shares the storage/observability substrate. |
| FV-02 | **Quotas & request budgeting** (longer-horizon allowances, credits). | Builds on layered-limits (FR-013). |
| FV-03 | **Adaptive / dynamic throttling** reacting to system signals (load, latency). | Requires a feedback input seam; keep core decisions pluggable. |
| FV-04 | **Additional storage backends** (PostgreSQL, DynamoDB, FoundationDB). | Evaluated in the Storage Strategy research; offered as optional, guarantee-labeled backends. |
| FV-05 | **Additional framework integrations** (e.g., Warp and others). | Kept independent per NFR-021; not required for v1. |
| FV-06 | **More algorithms** (e.g., leaky bucket, sliding log variants) beyond the initial set. | Enabled by the algorithm extension point (FR-012). |
| FV-07 | **Multi-tier / hierarchical limits** (per-user within per-tenant within global). | Extends composite limiting. |
| FV-08 | **Non-Rust consumption** (FFI/WASM) if demand emerges. | Explicit non-goal now (NG-04); revisited later. |
| FV-09 | **Operational tooling** (admin/inspection surfaces) as a separate downstream project. | Non-goal for the library (NG-03). |
| FV-10 | **Non-consuming inspection ("peek")** via an opt-in capability trait. | Deferred from v1 (OQ-07); see FR-014, ADR-004/ADR-013. |
| FV-11 | **Runtime reconfiguration of limits** without restart. | Deferred to v2 (OQ-08); see FR-015. |
| FV-12 | **Fail-To-Local degradation mode** (node-local fallback on shared-backend outage). | Additive to the degradation policy; see ADR-007. |

---

## 14. Requirement Conventions

- **ID scheme:**
  - `FR-XXX` — Functional Requirement
  - `NFR-XXX` — Non-Functional Requirement
  - `G-`, `NG-`, `P-`, `US-`, `UC-`, `C-`, `A-`, `R-`, `SM-`, `FV-` — Goals, Non-Goals, Personas, User Stories, Use Cases, Constraints, Assumptions, Risks, Success Metrics, Future Vision, respectively.
  - IDs are **stable**: once assigned, an ID is never reused for a different requirement. Retired requirements are marked deprecated, not deleted.
- **Priority (MoSCoW):**
  - **Must** — required for the first releasable version; its absence blocks release.
  - **Should** — important; targeted for the first release but not release-blocking.
  - **Could** — desirable; included if time/scope permit.
  - **Won't (this scope)** — captured in Non-Goals / Future Vision.
- **Acceptance Criteria** are written to be **verifiable** (testable or demonstrable). Vague criteria are treated as defects in this document.
- **No implementation detail:** requirements state observable behavior and qualities. Any "how" statements (types, data structures, specific libraries) belong in `Plan.md`, ADRs, or code.

---

## 15. Traceability Notes

- Every **Goal** (§2) is realized by at least one FR/NFR.
- Every **User Story** (§5) and **Use Case** (§6) traces to functional requirements.
- Every **Risk** (§11) names the requirements it threatens and a mitigation.
- Every **Success Metric** (§12) traces to the requirement(s) it measures.
- A formal traceability matrix (requirement ↔ test ↔ ADR) will be maintained alongside the test plan during implementation; this SRS establishes the requirement IDs that matrix will reference.

---

## 16. Decision Log (Resolved Open Questions)

The Open Questions raised in v0.1 have been resolved by maintainer review and captured in ADRs. Each decision below is authoritative; the linked ADR carries the full context, alternatives, and consequences.

| ID | Question | Decision | ADR |
|----|----------|----------|-----|
| OQ-01 | Default degradation policy? | **Fail Closed by default**; Fail Open is explicit opt-in; policy is an extensible choice (future Fail-To-Local). | [ADR-007](../adr/ADR-007-backend-degradation-policy.md) |
| OQ-02 | Which algorithms ship in v1, and the default? | **Token Bucket (default) + GCRA**; other algorithms deferred. | [ADR-008](../adr/ADR-008-algorithms.md) |
| OQ-03 | Is Valkey first-class in v1? | **Yes** — first-class, vendor-neutral peer to Redis; v1 backends = Memory, Redis, Valkey. | [ADR-006](../adr/ADR-006-redis-and-valkey-strategy.md) |
| OQ-04 | Universal over-admission guarantee? | **No universal guarantee**; consistency documented **per backend**, distinguishing node-level operation semantics from deployment topology (memory = exact; redis/valkey = atomic & exact on the executing primary, with bounded over-admission arising only under replication/failover; research backends = guarantee TBD). | [ADR-005](../adr/ADR-005-storage-abstraction.md) |
| OQ-05 | Concrete MSRV & cadence? | **No MSRV pre-1.0**; develop on latest stable; a dedicated decision fixes MSRV before 1.0. | [ADR-015](../adr/ADR-015-versioning-and-msrv.md) |
| OQ-06 | Async runtime support? | **Tokio primary**; core runtime-independent as practical; other runtimes later via integration crates. | [ADR-016](../adr/ADR-016-async-runtime.md) |
| OQ-07 | Peek/inspection in v1? | **No** — deferred; future support via an opt-in capability trait. | [ADR-004](../adr/ADR-004-public-api-design.md) / [ADR-013](../adr/ADR-013-public-extension-api-stability.md) |
| OQ-08 | Runtime reconfiguration in v1? | **Deferred to v2.** | [ADR-004](../adr/ADR-004-public-api-design.md) |
| OQ-09 | Feature-flag taxonomy & defaults? | **Minimal set**; `default = ["memory"]`; memory/redis/valkey/tower/axum/actix/metrics/tracing. | [ADR-010](../adr/ADR-010-feature-flags.md) |
| OQ-10 | How are composite limits surfaced? | **Explicit composition of independent limiters** + optional thin helper; no config DSL. | [ADR-004](../adr/ADR-004-public-api-design.md) |
| OQ-11 | Quantitative performance targets? | **None until benchmarked**; build the benchmark suite first, then set official targets from data. | [ADR-014](../adr/ADR-014-benchmarking.md) |
| OQ-12 | Additional storage backends? | **Research PostgreSQL/DynamoDB/FoundationDB; do not implement in v1.** | [ADR-005](../adr/ADR-005-storage-abstraction.md) |
| OQ-13 | Public extension-API stability? | Extension points (StorageBackend, Algorithm, KeyExtractor, middleware) are public, **not sealed**, SemVer-covered, additive-only evolution. | [ADR-013](../adr/ADR-013-public-extension-api-stability.md) |

### 16.1 Concrete values intentionally deferred

These follow from the decisions above but require empirical data or a later milestone to finalize. They are **not** blockers for Phase 0; each has an owning ADR and a milestone.

| ID | Deferred concrete value | Resolved when | Owning ADR |
|----|-------------------------|---------------|------------|
| DV-01 | The numeric **over-admission bound** for the Redis/Valkey backends. | Characterized empirically via multi-node benchmarks. | ADR-005 / ADR-014 |
| DV-02 | The concrete **MSRV** version and update cadence. | Fixed before the 1.0 release. | ADR-015 |
| DV-03 | The official **latency/allocation/throughput targets** (baselines for SM-01/SM-02/NFR-001/NFR-003). | Set after the benchmark suite produces data. | ADR-014 |

---

*End of Requirements.md (v0.2, DRAFT). Architectural decisions incorporated; ADRs authored under `docs/adr/`. Awaiting final review before implementation (Plan.md governs sequencing).*
