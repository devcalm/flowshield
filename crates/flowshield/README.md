# flowshield

**Ergonomic traffic management and rate limiting for Rust.**

`flowshield` is the public facade for the FlowShield ecosystem — the crate most users should install. It re-exports the stable, user-facing API from [`flowshield-core`](https://crates.io/crates/flowshield-core) and, as the ecosystem grows, hosts the feature flags that pull in storage backends and web-framework integrations.

> **Status: Phase 0 (design).** The public API is under construction and **not yet implemented**. This crate currently ships as a documented facade shell; user-facing re-exports arrive in Phase 1. See the [design docs](https://github.com/devcalm/flowshield/tree/main/docs).

```toml
[dependencies]
flowshield = "0.1"
```

Once the API lands, typical usage will be:

```rust,ignore
use flowshield::Limiter;
```

## Facade vs. core

| Crate | Role |
|-------|------|
| **`flowshield`** (this crate) | The ergonomic entry point: curated public API, top-level docs, and the user-facing feature flags. Depend on this. |
| [`flowshield-core`](https://crates.io/crates/flowshield-core) | The engineering foundation: algorithms, storage abstraction, extension traits, internal logic. Remains published and supported; advanced users and ecosystem crates may depend on it directly. |

This is the same facade/core split used by `tokio`/`tokio-*`, `tracing`/`tracing-core`, and `tower`/`tower-service`. See [ADR-018](https://github.com/devcalm/flowshield/blob/main/docs/adr/ADR-018-facade-crate.md) and the [architecture overview](https://github.com/devcalm/flowshield/blob/main/docs/architecture/overview.md).

## License

Licensed under either of [Apache-2.0](https://github.com/devcalm/flowshield/blob/main/LICENSE-APACHE) or [MIT](https://github.com/devcalm/flowshield/blob/main/LICENSE-MIT) at your option.
