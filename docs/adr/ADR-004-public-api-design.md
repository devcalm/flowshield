# ADR-004: Public API Design

- **Status:** Accepted
- **Date:** 2026-07-20
- **Deciders:** FlowShield maintainers
- **Related:** ADR-002 (Core Architecture), ADR-007 (Degradation), ADR-011 (Error Handling), ADR-013 (Extension API Stability); FR-010, FR-011, FR-013, FR-014, FR-072, NFR-040; OQ-07, OQ-10

## Context

The public API is the longest-lived artifact FlowShield produces: once released and covered by SemVer (ADR-015), its shape constrains us for the life of the major version. The README mandates an API that is builder-based, idiomatic, async-first, generic without unnecessary complexity, stable, and easy to extend. Two open questions bear directly on the surface: how composite/layered limits are expressed (OQ-10) and whether non-consuming inspection ("peek") ships in v1 (OQ-07).

## Decision

1. **Builder-based construction.** A limiter is assembled through a discoverable builder that selects algorithm, limit(s), backend, key strategy, degradation policy, and observability. Required parameters missing at build time produce a clear error; construction **validates configuration** and rejects nonsensical limits before any traffic is served (FR-072).

2. **Async-first decision call.** The core admission decision is asynchronous (the storage seam may be remote). The returned decision distinguishes *allowed* from *rejected*, and a rejected decision carries **retry-after metadata** sufficient to produce a correct HTTP 429 (FR-011). "Rejected by the limit" is a normal return value, **not** an error; operational failures are separate (ADR-011).

3. **Composite limits by explicit composition (OQ-10).** The engine does **not** introduce a configuration DSL or a multi-limit rule language. Each limiter remains independent and single-purpose. Applications compose several limiters (e.g., global, per-tenant, per-user, per-API-key) and a request is admitted only if **all** admit.
   - To avoid every application re-implementing the same "evaluate all, reject on the binding limit, report the correct Retry-After" logic, FlowShield provides an **optional, thin composition helper** (a limiter *set*) that evaluates its members and returns a combined decision identifying which limit was binding and the maximum retry-after. This helper is a convenience layer over the public decision API — it holds no special powers, adds nothing to the single-limiter hot path, and can be ignored by users who prefer to compose manually.
   - **Maintainer note:** this is a small, deliberate extension of the reviewer's "compose explicitly" guidance. It keeps limiters independent (as directed) while preventing a subtly-wrong retry-after computation from being copy-pasted across every adopter.

4. **No "peek" / non-consuming inspection in v1 (OQ-07).** Not every algorithm/backend can report accurate remaining allowance without consuming, and a partially-accurate inspection API would set inconsistent expectations. Inspection is deferred and, when introduced, arrives as an **opt-in capability trait** (ADR-013) that a backend/algorithm advertises — never as a silent approximation baked into the base API.

5. **Framework-agnostic core.** The public core API is usable without any web framework (FR-004); HTTP concerns (key extraction from requests, 429 responses) live in the integration crates (ADR-009).

## Alternatives Considered

- **A composite-limit rule DSL / config language.** Rejected: high complexity, a new thing to learn, hard to keep stable, and unnecessary — explicit composition of independent limiters expresses the same intent with less surface. (Directly per OQ-10.)
- **No composition helper (pure manual composition).** Rejected as the *sole* option: correct multi-limit retry-after handling is easy to get wrong; a thin, ignorable helper prevents that without compromising independence.
- **Shipping "peek" in v1 across all backends.** Rejected: forces every backend to fake inspection it cannot do accurately (OQ-07).
- **Free-function / non-builder configuration.** Rejected: less discoverable, poor forward-compatibility (adding an option becomes a breaking signature change); a builder plus `#[non_exhaustive]` config types evolves cleanly (ADR-013).

## Consequences

- **Positive:** Small, learnable, discoverable surface; async-first matches the storage reality; composition stays simple yet safe; deferring peek avoids a cross-implementation inconsistency; the surface is designed to evolve additively.
- **Negative:** Applications wanting layered limits must wire multiple limiters (mitigated by the helper). No inspection API in v1 means quota-header use cases wait for the capability trait.
- **Neutral:** The builder and decision types must be designed with `#[non_exhaustive]` and default-bearing options from day one to honor ADR-013.

## Future Evolution

- Add an **inspection capability trait** ("peek") once backends can advertise accurate support (OQ-07 revisited).
- Consider richer composition (hierarchical/weighted limits) as an additive layer over the helper.
- Runtime reconfiguration of limits (deferred to v2 per OQ-08) will extend, not replace, the builder-produced configuration.
