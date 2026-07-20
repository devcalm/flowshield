# Changelog

All notable changes to FlowShield are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (see [ADR-015](docs/adr/ADR-015-versioning-and-msrv.md)).

> While the project is in Phase 0 (design), this changelog tracks design milestones. Once code is released, entries will track per-crate library changes.

## [Unreleased]

### Added
- Phase 0 design blueprint: project charter, Software Requirements Specification, phased Plan, and ADR-001 … ADR-016.
- Repository organization: `docs/` documentation tree (`adr/`, `architecture/`, `benchmarks/`, `diagrams/`, `design/`, `research/`), `.ai/` internal-artifacts workspace, and community-health files.
- Architecture diagrams (C4 context/container + request-flow) and `docs/architecture/overview.md`.
- CI: GitHub Actions for fmt/clippy/test/doc, a weekly security audit, Dependabot, and issue/PR templates.
- `flowshield` facade crate (public entry point) over the already-published `flowshield-core` foundation, and ADR-018 documenting the facade/core architecture.

[Unreleased]: https://github.com/devcalm/flowshield/commits/main
