# FlowShield

> Traffic management for Rust — starting with distributed, pluggable rate limiting.

[![CI](https://github.com/devcalm/flowshield/actions/workflows/ci.yml/badge.svg)](https://github.com/devcalm/flowshield/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Status: Phase 0 — Design](https://img.shields.io/badge/status-Phase%200%20%E2%80%94%20design-orange.svg)](docs/design/Plan.md)

FlowShield aims to be the de facto Rust framework for **traffic management**. The initial deliverable is a **distributed rate limiter** with pluggable storage backends and clean web-framework integrations, architected so that future modules — concurrency limiting, quotas, adaptive throttling, request budgeting — can be added without breaking the public API.

Design priorities: **correctness · performance · stable APIs · extensibility · production-grade docs.**

---

## ⚠️ Project status

FlowShield is in **Phase 0 (Research & Architecture)**. The design blueprint is complete and under review; **no production code has been written yet**. The public API, crate names, and features described in the design docs are provisional until the first release.

If you are evaluating FlowShield: start with the [design charter](docs/design/charter.md) and the [roadmap](docs/design/Plan.md).

## Planned capabilities (v1)

- **Algorithms:** Token Bucket (default) and GCRA.
- **Backends:** in-memory, Redis, and Valkey — all first-class, behind one storage abstraction.
- **Integrations:** Tower, Axum, and Actix, each in an isolated crate.
- **Operations:** safe defaults (fail-closed on backend outage), first-class observability (tracing, Prometheus, OpenTelemetry), and bounded resource usage.

See the [Software Requirements Specification](docs/design/Requirements.md) for the authoritative scope.

## Installation

Most users install the facade crate, **`flowshield`**:

```toml
[dependencies]
flowshield = "0.1"
```

Once the API lands (Phase 1), typical usage will be:

```rust,ignore
use flowshield::Limiter;
```

`flowshield` re-exports the stable API from [`flowshield-core`](https://crates.io/crates/flowshield-core) — the already-published foundation of the ecosystem. Advanced users and ecosystem crates may depend on `flowshield-core` directly. See [ADR-018](docs/adr/ADR-018-facade-crate.md) for the facade/core split.

## Repository layout

| Path | Contents |
|------|----------|
| [`crates/`](crates/) | The published library crates: **`flowshield`** (public facade) and **`flowshield-core`** (foundation). |
| [`examples/`](examples/) | Runnable examples (added from Phase 2). |
| [`docs/`](docs/README.md) | All project documentation — start at the docs index. |
| [`docs/adr/`](docs/adr/README.md) | Architecture Decision Records. |
| [`docs/design/`](docs/design/) | Charter, Requirements (SRS), and the phased Plan. |
| [`.ai/`](.ai/README.md) | Internal, non-authoritative development artifacts (see its README). |

> The Git repository is a **superset** of what is published to crates.io. Design history, ADRs, planning, and research live here for contributors and maintainers; the published crates ship only what library users need.

## Documentation

- **[Documentation index](docs/README.md)** — the map of everything.
- **[Charter](docs/design/charter.md)** — the founding research & architecture brief.
- **[Requirements (SRS)](docs/design/Requirements.md)** — goals, functional/non-functional requirements, personas, risks.
- **[Roadmap (Plan)](docs/design/Plan.md)** — the phased implementation plan.
- **[Architecture Decision Records](docs/adr/README.md)** — the 18 ADRs behind the design.

## Contributing

Contributions are welcome once Phase 1 begins. Please read [`CONTRIBUTING.md`](CONTRIBUTING.md) and our [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). Security issues: see [`SECURITY.md`](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
