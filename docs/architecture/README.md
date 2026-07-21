# Architecture Documentation

Narrative, long-form architecture documentation for FlowShield: how the pieces fit together, the request lifecycle, module boundaries, and cross-cutting concerns.

This complements — it does not duplicate — the [Architecture Decision Records](../adr/README.md). ADRs capture *decisions* (point-in-time, immutable); the documents here describe the *current system* (living, updated as the code evolves).

## Contents

- **[overview.md](overview.md)** — the architecture map at three zoom levels (system context, containers/crates, request lifecycle), with the consistency/correctness posture and links to the governing ADRs. Diagrams are rendered from the sources in [`../diagrams/`](../diagrams/README.md).
- **[algorithms.md](algorithms.md)** — how the two v1 rate-limiting algorithms (Token Bucket, GCRA) work: mental models, exact per-key state, worked examples, decision-flow diagrams, and animated GIFs. Companion to [ADR-008](../adr/ADR-008-algorithms.md) and the `algorithm/` module.

As implementation proceeds, expect deeper documents here (e.g., the extension-author guide, per-subsystem notes) linked from this index.
