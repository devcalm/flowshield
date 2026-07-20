# flowshield-core

Core traits and types for **[FlowShield](https://github.com/devcalm/flowshield)** — storage- and framework-agnostic traffic management and rate limiting for Rust.

> **Status: Phase 0 (design).** This crate is a placeholder; the public API is under design and **not yet implemented**. See the [design docs](https://github.com/devcalm/flowshield/tree/main/docs) for the authoritative blueprint.

`flowshield-core` will define the framework's foundation — the `StorageBackend`, `Algorithm`, and `KeyExtractor` extension points, the limiter builder, the admission-decision and error types, and the backend-degradation policy — with **no** dependency on any storage backend or web framework. Backends (`flowshield-memory`, `flowshield-redis`, `flowshield-valkey`) and integrations (`flowshield-tower`, `flowshield-axum`, `flowshield-actix`) build on top of it.

## Documentation

Full design documentation lives in the repository:

- [Requirements (SRS)](https://github.com/devcalm/flowshield/blob/main/docs/design/Requirements.md)
- [Roadmap](https://github.com/devcalm/flowshield/blob/main/docs/design/Plan.md)
- [Architecture Decision Records](https://github.com/devcalm/flowshield/tree/main/docs/adr)

## License

Licensed under either of [Apache-2.0](https://github.com/devcalm/flowshield/blob/main/LICENSE-APACHE) or [MIT](https://github.com/devcalm/flowshield/blob/main/LICENSE-MIT) at your option.
