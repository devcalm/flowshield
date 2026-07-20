# ADR-013: Public Extension API Stability

- **Status:** Accepted
- **Date:** 2026-07-20
- **Deciders:** FlowShield maintainers
- **Related:** ADR-004 (Public API), ADR-005 (Storage Abstraction), ADR-008 (Algorithms), ADR-009 (Middleware), ADR-015 (Versioning/MSRV); NFR-020, NFR-032; OQ-13

## Context

FlowShield intends to be an *ecosystem*, not a closed library: third parties should be able to ship their own storage backends, algorithms, key-extraction strategies, and framework integrations. That ambition creates an obligation. The moment an extension point is public and a third party implements it, that trait's shape becomes part of our SemVer contract — every method we later add is a breaking change for downstream implementers.

We must decide, **before** any extension trait exists, which points are public, which are sealed, and what compatibility we guarantee. Getting this wrong is expensive: either we freeze ourselves out of evolving the traits, or we repeatedly break the ecosystem we are trying to grow.

## Decision

### 1. The public extension points (v1)

| Extension point | Purpose | Third parties may implement? | Sealed? |
|-----------------|---------|------------------------------|---------|
| **StorageBackend** | Provide a store for limiter state | **Yes** | No |
| **Algorithm** | Provide a rate-limiting strategy | **Yes** | No |
| **KeyExtractor** | Derive the limiting key from a request/context | **Yes** | No |
| **Framework integration** (middleware) | Adapt the limiter to a web framework | **Yes** | No (thin, per-crate) |

The traits third parties are *meant* to implement (StorageBackend, Algorithm, KeyExtractor) are **NOT sealed** — sealing them would defeat the entire extensibility goal.

### 2. What we seal instead

Sealing is used only where implementation by third parties would be a mistake we must prevent: internal capability/marker traits, and any trait that exists purely to wire internal machinery. These are documented as `#[doc(hidden)]`/internal and are explicitly **not** part of the public contract.

### 3. Evolution without breakage

To evolve public extension traits without breaking downstream implementers, we adopt these rules as binding policy (enforced in review and, where possible, in CI):

- **Additive-only within a major version.** New required trait methods are a breaking change and are forbidden within a major version. New behavior is added via **default-provided methods** (a new method with a default keeps existing impls valid) or via **new, separate capability traits** an implementer can opt into.
- **`#[non_exhaustive]` on public enums, error types, config structs, and decision/outcome types** so that adding a variant/field is non-breaking.
- **Capability traits over god-traits.** Optional behaviors (e.g., non-consuming inspection / "peek", batch operations, runtime reconfiguration) live in separate opt-in traits rather than being baked into the base trait. A backend advertises what it supports; the core degrades or reports gracefully when a capability is absent. (This is precisely why "peek" is deferred — see ADR-004 — it becomes an opt-in capability trait later, not a v1 change to StorageBackend.)
- **Semantics are part of the contract.** The required semantics of each extension point (e.g., atomicity, expiry, error reporting for StorageBackend) are documented and validated by a **conformance test suite** (see ADR-005), so "implements the trait" and "behaves correctly" are both checkable.

### 4. SemVer coverage

Public extension traits are covered by SemVer exactly like the rest of the public API (ADR-015). Any breaking change to them requires a major-version bump and a documented migration path. Internal/sealed items are explicitly excluded from the SemVer contract.

## Alternatives Considered

- **Seal all extension traits.** Rejected: prevents third-party backends/algorithms — the opposite of the project's goal.
- **Leave everything open with no policy.** Rejected: guarantees painful ecosystem breakage the first time we need to add a method; erodes the trust that drives adoption.
- **One large trait per concern with all optional behavior included.** Rejected: forces every implementer to handle capabilities they don't support, and makes every new capability a breaking change. Capability traits are the extensible alternative.
- **Defer the whole question until traits exist.** Rejected: the reviewer correctly flagged that this decision *shapes* the traits; deciding after the fact means retrofitting stability onto an unstable surface.

## Consequences

- **Positive:** Third parties can safely build on FlowShield; the maintainers retain room to evolve via defaults, capability traits, and `#[non_exhaustive]`; the SemVer contract is explicit about what is and isn't covered.
- **Negative:** Discipline cost — every public-trait change must be reviewed against the additive-only rule, and some evolutions must take the longer "new capability trait" path rather than the quick "add a method" path. Capability-trait proliferation must be watched so the surface stays comprehensible.
- **Neutral:** The conformance suite becomes a first-class deliverable, not an afterthought.

## Future Evolution

- Introduce optional **capability traits** as needs emerge (peek/inspection, batch decisions, runtime reconfiguration, hierarchical limits).
- Once the trait surface is proven in the wild, publish an **extension-author guide** and a **stability tier** label per trait (stable / evolving / experimental).
- Revisit sealing decisions only via a superseding ADR; never silently.
