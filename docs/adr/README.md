# Architecture Decision Records (ADRs)

This directory records the significant architectural decisions for **FlowShield**. Each ADR captures the context, the decision, the alternatives that were considered and rejected, the consequences, and the anticipated future evolution.

ADRs are immutable once **Accepted**: to change a decision, add a new ADR that supersedes the old one (and mark the old one `Superseded by ADR-0NN`). This preserves the reasoning history.

## Conventions

- **File name:** `ADR-0NN-kebab-title.md`
- **IDs are stable and never reused.** Numbering is chronological/logical, not priority.
- **Status:** `Proposed` → `Accepted` → (`Superseded` | `Deprecated`). `Deferred` marks a decision intentionally postponed.
- Each ADR follows [`_template.md`](./_template.md).
- ADRs are **design-level**. They contain no production code — only prose, diagrams (Mermaid/C4), and tables.

## Index

| ADR | Title | Status | Resolves |
|-----|-------|--------|----------|
| [001](./ADR-001-vision-and-scope.md) | Vision & Scope | Accepted | — |
| [002](./ADR-002-core-architecture.md) | Core Architecture | Accepted | — |
| [003](./ADR-003-crate-structure.md) | Crate Structure | Accepted | — |
| [004](./ADR-004-public-api-design.md) | Public API Design | Accepted | OQ-10 |
| [005](./ADR-005-storage-abstraction.md) | Storage Abstraction | Accepted | OQ-04 |
| [006](./ADR-006-redis-and-valkey-strategy.md) | Redis & Valkey Strategy | Accepted | OQ-03 |
| [007](./ADR-007-backend-degradation-policy.md) | Backend Degradation Policy | Accepted | OQ-01 |
| [008](./ADR-008-algorithms.md) | Algorithms | Accepted | OQ-02 |
| [009](./ADR-009-middleware-and-integration.md) | Middleware & Framework Integration | Accepted | — |
| [010](./ADR-010-feature-flags.md) | Feature Flags | Accepted | OQ-09 |
| [011](./ADR-011-error-handling.md) | Error Handling | Accepted | — |
| [012](./ADR-012-observability.md) | Observability | Accepted | — |
| [013](./ADR-013-public-extension-api-stability.md) | Public Extension API Stability | Accepted | OQ-13 |
| [014](./ADR-014-benchmarking.md) | Benchmarking | Accepted | OQ-11 |
| [015](./ADR-015-versioning-and-msrv.md) | Versioning, MSRV & Compatibility | Accepted (MSRV value deferred) | OQ-05 |
| [016](./ADR-016-async-runtime.md) | Async Runtime Strategy | Accepted | OQ-06 |
| [017](./ADR-017-time-source.md) | Time Source | Accepted | — |
| [018](./ADR-018-facade-crate.md) | Public Facade Crate (`flowshield`) | Accepted | — |
| [019](./ADR-019-storage-seam-revision.md) | Storage Seam Revision (two families + `Evaluate`) | Accepted | — |

## Relationship to other documents

- [`README.md`](../../README.md) — the public project front door.
- [`docs/design/charter.md`](../design/charter.md) — the founding Phase-0 research & architecture charter.
- [`docs/design/Requirements.md`](../design/Requirements.md) — the Software Requirements Specification these decisions satisfy.
- [`docs/design/Plan.md`](../design/Plan.md) — the phased roadmap; each phase references the ADRs it depends on.
