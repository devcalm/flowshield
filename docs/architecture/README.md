# Architecture Documentation

Narrative, long-form architecture documentation for FlowShield: how the pieces fit together, the request lifecycle, module boundaries, and cross-cutting concerns.

This complements — it does not duplicate — the [Architecture Decision Records](../adr/README.md). ADRs capture *decisions* (point-in-time, immutable); the documents here describe the *current system* (living, updated as the code evolves).

**Status:** to be populated during Phase 1. High-level diagrams currently live inline in [ADR-002](../adr/ADR-002-core-architecture.md) and [ADR-003](../adr/ADR-003-crate-structure.md); diagram sources belong in [`../diagrams/`](../diagrams/README.md).

Planned contents:
- System overview (C4 context & container level).
- Request lifecycle: key extraction → algorithm decision → storage → response.
- Module/crate boundaries and dependency rules.
- Extension model (StorageBackend, Algorithm, KeyExtractor).
