# Contributing to FlowShield

Thank you for your interest in FlowShield! This document explains how to contribute.

> **Phase 0 note:** FlowShield is currently in the **research & architecture** phase. The most valuable contributions right now are **review and discussion** of the design docs — not code. Implementation contributions open up as Phase 1 begins. See the [roadmap](docs/design/Plan.md).

## Start here

- **[Documentation index](docs/README.md)** — the map of everything.
- **[Charter](docs/design/charter.md)** — why the project exists and its scope.
- **[Requirements (SRS)](docs/design/Requirements.md)** — what FlowShield must do.
- **[Plan](docs/design/Plan.md)** — the phased roadmap and what's in flight.
- **[ADRs](docs/adr/README.md)** — the decisions and their rationale.

## How we make decisions

Significant architectural or API decisions are recorded as **Architecture Decision Records** under [`docs/adr/`](docs/adr/README.md). If you propose a change that alters an established decision, add a new ADR (using [`docs/adr/_template.md`](docs/adr/_template.md)) that supersedes the old one — don't silently edit an Accepted ADR. Smaller changes can reference an existing ADR.

## Workflow (from Phase 1 onward)

1. **Open an issue** describing the problem or proposal before large changes, so design can be discussed early.
2. **Branch** from the default branch; keep changes focused.
3. **Match the surrounding code** — idioms, naming, and comment density.
4. **Test** your change: unit and property tests land with the code they cover; see the testing strategy in the [Requirements](docs/design/Requirements.md) (NFR-034) and the [Plan](docs/design/Plan.md).
5. **Keep the public API stable** — respect the extension-API stability rules in [ADR-013](docs/adr/ADR-013-public-extension-api-stability.md) and Semantic Versioning ([ADR-015](docs/adr/ADR-015-versioning-and-msrv.md)).
6. **Document** public items and update the changelog where relevant.
7. **Open a pull request** describing what changed and why, linking the issue/ADR.

## Quality gates

Once CI exists, all changes must pass formatting (`cargo fmt`), linting (`cargo clippy`), and the test suite across the supported feature combinations, per [ADR-010](docs/adr/ADR-010-feature-flags.md) and the CI requirements (NFR-033). Please run these locally before pushing.

## Internal / AI-assisted artifacts

If your workflow uses AI tooling, keep prompts and scratch in [`.ai/`](.ai/README.md), not in `docs/`. Promote anything with lasting value into `docs/research/` or an ADR.

## Licensing of contributions

Unless you state otherwise, contributions are dual-licensed under **MIT OR Apache-2.0**, consistent with the project license. See [`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).

## Code of Conduct

Participation is governed by our [Code of Conduct](CODE_OF_CONDUCT.md). Please read it.
