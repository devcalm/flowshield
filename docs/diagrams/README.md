# Diagrams

Source-controlled architecture diagrams for FlowShield.

**Conventions:**
- Diagrams are authored as **text-based, diffable Mermaid sources** (`.mmd`) so they review like code.
- The **rendered copies** are embedded in [`../architecture/overview.md`](../architecture/overview.md), which is where humans read them (GitHub renders Mermaid). When you change a `.mmd` file, update its embedded copy in the overview.
- Name files by subject and level.

## Current diagrams

| Source | Level | Rendered in |
|--------|-------|-------------|
| [`c4-context.mmd`](c4-context.mmd) | C4 L1 — System context | [overview §1](../architecture/overview.md#1-system-context-c4-level-1) |
| [`c4-container.mmd`](c4-container.mmd) | C4 L2 — Containers / crates | [overview §2](../architecture/overview.md#2-containers--crates-c4-level-2) |
| [`request-flow.mmd`](request-flow.mmd) | Sequence — Admission lifecycle | [overview §3](../architecture/overview.md#3-request-lifecycle) |
| [`token-bucket-flow.mmd`](token-bucket-flow.mmd) | Flowchart — Token Bucket admission step | [algorithms §1](../architecture/algorithms.md#decision-flow) |
| [`gcra-flow.mmd`](gcra-flow.mmd) | Flowchart — GCRA admission step | [algorithms §2](../architecture/algorithms.md#decision-flow-1) |

Additional diagrams (e.g., per-backend or per-integration detail) will be added alongside the code that introduces them.

## Animations

Simulation-driven GIF animations of the algorithms live in [`animations/`](animations/README.md); they are embedded in and explained by [`../architecture/algorithms.md`](../architecture/algorithms.md).
