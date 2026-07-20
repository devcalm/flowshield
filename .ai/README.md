# `.ai/` — Internal Development Artifacts

This directory holds **internal, non-authoritative** engineering artifacts: AI prompts, brainstorming, design-review inputs, and experiments. It is deliberately separated from the curated project documentation in [`docs/`](../docs/README.md).

## Why this exists

The repository preserves **design history** — including the provenance of AI-assisted work — for future maintainers. Keeping these artifacts here (rather than scattered through `docs/`) means the authoritative documentation stays clean while the raw material remains available for context.

## Rules

- **Nothing here is authoritative.** The source of truth is always `docs/` (charter, Requirements, Plan, ADRs) and the code. If an artifact here disagrees with `docs/`, `docs/` wins.
- **Never referenced by published crates.** This directory lives at the workspace root, outside every crate's package directory, so it is never included in a crates.io package.
- **Tool-agnostic by design.** The directory is named `.ai/`, not after any specific tool, so it stays meaningful even if the tooling changes. Avoid tool-specific subdirectory names.
- **Promote, don't leave.** When an artifact gains lasting reference value (a formalized research finding, a finalized design rationale), promote it into [`docs/research/`](../docs/research/README.md) or an ADR and link back if useful.

## Layout

| Subdirectory | Contents |
|--------------|----------|
| [`prompts/`](prompts/) | Prompts and briefs used to drive AI-assisted design/engineering. |
| [`design-reviews/`](design-reviews/) | Design-review inputs and their outcomes (e.g., decision briefs that fed the ADRs). |
| [`experiments/`](experiments/) | Throwaway experiments, spikes, and exploratory notes. |

## Committed vs. ignored

Curated, meaningful artifacts (prompts and design-reviews with lasting provenance value) **are committed**. Truly ephemeral scratch goes in a `scratch/` subfolder anywhere under `.ai/`, which is git-ignored (see the repository `.gitignore`).
