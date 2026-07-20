# ADR-001: Vision & Scope

- **Status:** Accepted
- **Date:** 2026-07-20
- **Deciders:** FlowShield maintainers
- **Related:** ADR-002 (Core Architecture), ADR-005 (Storage Abstraction), ADR-006 (Redis & Valkey Strategy), ADR-008 (Algorithms), ADR-013 (Public Extension API Stability); NFR-scope, NFR-portability, FR-ratelimit-core

## Context

FlowShield is an open-source Rust framework for **traffic management**: the disciplined shaping, admission, and protection of request flows against shared or scarce resources. The Rust ecosystem has capable point solutions (per-crate rate limiters, framework-specific middleware, Redis Lua snippets), but lacks a cohesive, storage- and framework-agnostic foundation that treats distributed rate limiting as a first-class concern while leaving room to grow into the wider traffic-management space.

The README frames the vision: a composable core that expresses *who* is being limited (key derivation), *by what rule* (algorithm), and *against what shared state* (backend), with framework integrations layered on top rather than baked in. This ADR fixes the boundaries of that vision so later ADRs can decide *how* without re-litigating *what* and *why*.

Phase 0 is design-only: this document commits to scope, not to any implementation.

## Decision

FlowShield's mission is to be **the reusable traffic-management substrate for Rust services**. The initial and primary focus is **distributed rate limiting**. The architecture must be designed so it extends — without rework of the core contracts — to:

- Concurrency limiting (in-flight caps)
- Quotas (long-window allowances)
- Adaptive throttling (load-responsive limits)
- Request budgeting (cost-weighted admission)

We commit to these scope pillars:

1. **Core is agnostic.** The core knows nothing about a specific storage engine or web framework. Behavior is expressed through extension points (StorageBackend, Algorithm, KeyExtractor, Middleware) governed by ADR-013.
2. **Distributed-first, local-capable.** Distribution is the hard case we design for; a single-process memory backend remains a first-class, exact-semantics option.
3. **Composability over configuration monolith.** Users assemble a limiter from a key strategy, an algorithm, and a backend rather than selecting from a fixed menu of presets.
4. **Rust-native, no hosted product.** FlowShield is a library ecosystem, not a service.

**Non-goals (explicit).** FlowShield is:

- **Not an API gateway or reverse proxy** — no routing, TLS termination, or L7 request forwarding.
- **Not a consensus or coordination system** — it does not provide strong distributed consensus; cross-node consistency is bounded/eventual per ADR-006.
- **Not an identity or authentication provider** — it derives keys from request context but does not issue, verify, or manage identity.
- **Not a hosted/SaaS service** — no managed control plane, no billing, no multi-tenant service offering.
- **No non-Rust bindings in v1** — no FFI, WASM-facing, or foreign-language SDKs in the first release line.

## Alternatives Considered

- **"Batteries-included gateway" (all-in-one).** Ship a full traffic proxy with rate limiting bundled. *Rejected:* enormous surface area, forces networking and TLS opinions on users, competes with mature proxies, and buries the reusable core inside a product.
- **Single-crate rate limiter only.** Do just in-process rate limiting, no distribution, no extension seams. *Rejected:* the differentiating need in Rust is *distributed* limiting; a single-crate scope would ossify around local assumptions and block the traffic-management roadmap.
- **Strongly-consistent distributed limiter.** Guarantee exact global limits across nodes via consensus. *Rejected:* the latency and operational cost of consensus on a per-request hot path is unacceptable for a general-purpose limiter; bounded over-admission is the right trade (ADR-006).
- **Framework-first design (e.g., Axum-only).** *Rejected:* couples the core to one framework's lifecycle and excludes Tower/Actix users; integrations must be peripheral, not central (ADR-002, ADR-003).

## Consequences

**Positive**
- Clear, defensible boundaries let every downstream ADR reason about a stable mission.
- Agnostic core maximizes reuse across storage engines and frameworks.
- Explicit non-goals prevent scope creep into gateway/identity/consensus territory.

**Negative**
- Users needing strong global consistency must look elsewhere or accept bounded over-admission.
- "Assemble-your-own" composability asks more of first-time users than a preset menu would.

**Neutral**
- The traffic-management roadmap (concurrency, quotas, throttling, budgeting) is acknowledged but deferred; only rate limiting is delivered first.
- Rust-only in v1 narrows the audience deliberately.

## Future Evolution

Post-v1, the same core contracts are expected to host concurrency limiting, quotas, adaptive throttling, and request budgeting as additional Algorithm/decision families. Non-Rust bindings, additional backends, and richer key-derivation strategies may be revisited once the extension API stabilizes (ADR-013). Any move toward a hosted service or gateway role would require a new vision ADR superseding this one.
