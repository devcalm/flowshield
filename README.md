# FlowShield

> Traffic management for Rust — distributed, pluggable rate limiting with clean framework integrations.

[![CI](https://github.com/devcalm/flowshield/actions/workflows/ci.yml/badge.svg)](https://github.com/devcalm/flowshield/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Status: 1.0 readiness](https://img.shields.io/badge/status-1.0%20readiness-brightgreen.svg)](docs/design/Plan.md)
[![MSRV: 1.85](https://img.shields.io/badge/MSRV-1.85-blue.svg)](docs/adr/ADR-015-versioning-and-msrv.md)

FlowShield aims to be the de facto Rust framework for **traffic management**. The first
deliverable is a **rate limiter** that runs single-node or distributed, behind one storage
abstraction, with thin web-framework integrations — architected so future modules
(concurrency limiting, quotas, adaptive throttling) can be added without breaking the public API.

Design priorities: **correctness · performance · stable APIs · extensibility · production-grade docs.**

---

## Highlights

- **Two algorithms** — Token Bucket (default) and GCRA, both allocation-free on the hot path.
- **Three backends behind one seam** — in-memory (default), Redis, and Valkey. The distributed
  backends are **server-side atomic** (a single Lua round-trip), not read-modify-write races.
- **Three framework integrations** — Tower, Axum, and Actix, each in an isolated crate, each
  returning a standards-correct `429 Too Many Requests` with `Retry-After`.
- **Safe by default** — fail-closed on backend outage (fail-open is opt-in), a mandatory
  operation timeout, bounded memory with LRU eviction, and construction-time config validation.
- **Observability, off by default** — Prometheus-compatible metrics and `tracing` spans behind
  feature flags, with non-reversible key redaction so raw keys never reach telemetry.
- **`#![forbid(unsafe_code)]`** across every library crate.

## Quick start

```toml
[dependencies]
flowshield = "1"
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use flowshield::{Key, LimiterBuilder, MemoryStore, TokenBucket};

#[tokio::main]
async fn main() -> flowshield::Result<()> {
    // Allow 10 requests, refilling 2 tokens/second, per key.
    let limiter = LimiterBuilder::new(
        TokenBucket::per_second(10, 2)?,
        MemoryStore::new(),
    )
    .build()?;

    let decision = limiter.check(&Key::from("client-42")).await?;
    if decision.is_allowed() {
        // ... serve the request
    } else {
        // 429; tell the caller when to come back
        let retry = decision.retry_after().unwrap();
        eprintln!("rate limited, retry after {retry:?}");
    }
    Ok(())
}
```

### Rate-limit an Axum route (per client IP)

```toml
flowshield = { version = "1", features = ["axum"] }
```

```rust,ignore
use axum::{Router, routing::get};
use flowshield::axum::per_client_ip;
use flowshield::{LimiterBuilder, MemoryStore, TokenBucket};

let limiter = LimiterBuilder::new(TokenBucket::per_second(5, 2)?, MemoryStore::new()).build()?;
let app = Router::new()
    .route("/", get(|| async { "ok" }))
    .layer(per_client_ip(limiter)); // 429 + Retry-After on rejection
```

### Go distributed (Redis or Valkey)

```toml
flowshield = { version = "1", features = ["redis"] }
```

```rust,ignore
use flowshield::{LimiterBuilder, RedisStore, TokenBucket};

let store = RedisStore::builder("redis://127.0.0.1:6379").build()?;
let limiter = LimiterBuilder::new(TokenBucket::per_second(100, 20)?, store).build()?;
// Multiple processes now coordinate on the same keys, atomically, server-side.
```

More runnable examples live in each crate's `examples/` directory (`cargo run --example …`).

## Feature flags

The facade is lean by default (in-memory only); everything else is additive.

| Feature | Pulls in | Purpose |
|---------|----------|---------|
| `memory` *(default)* | `flowshield-memory` | In-memory bounded store |
| `redis` | `flowshield-redis` | Distributed Redis backend |
| `valkey` | `flowshield-valkey` | Distributed Valkey backend |
| `tower` | `flowshield-tower` | Tower `Layer`/`Service` |
| `axum` | `flowshield-axum` (+`tower`) | Axum integration |
| `actix` | `flowshield-actix` | Actix Web integration |
| `metrics` | — | Prometheus-compatible metrics (off by default) |
| `tracing` | — | `tracing` spans (off by default) |

## Backend guarantees at a glance

| Backend | Consistency | Notes |
|---------|-------------|-------|
| in-memory | exact, single-process | bounded memory, LRU eviction, lazy TTL |
| Redis / Valkey | exact on the primary | server-side atomic; bounded over-admission only under replica failover |

See [ADR-005](docs/adr/ADR-005-storage-abstraction.md) and the `flowshield-resp` docs for the
full consistency story.

## Documentation

- **[Getting started tutorial](docs/tutorial.md)** — from zero to a limited service.
- **[Extension author guide](docs/extension-guide.md)** — write your own backend, algorithm, or key extractor.
- **[Compatibility & versioning policy](docs/COMPATIBILITY.md)** — SemVer, MSRV, deprecations.
- **[Security posture](docs/security-review.md)** — safe defaults, resource bounds, threat notes.
- **[Architecture overview](docs/architecture/overview.md)** and the **[18 ADRs](docs/adr/README.md)**.
- **[Requirements (SRS)](docs/design/Requirements.md)** and the **[roadmap](docs/design/Plan.md)**.

## Project status

FlowShield has completed its core, backends, integrations, observability, and benchmark
phases and is in **1.0 readiness** (Phase 7): hardening, documentation, and a committed
compatibility policy. The public API is stabilizing; see the [roadmap](docs/design/Plan.md)
and [CHANGELOG](CHANGELOG.md).

## Repository layout

| Path | Contents |
|------|----------|
| [`crates/`](crates/) | The library crates: `flowshield` (facade), `flowshield-core`, backends, and integrations. |
| [`docs/`](docs/README.md) | All project documentation — start at the docs index. |
| [`docs/adr/`](docs/adr/README.md) | Architecture Decision Records. |
| [`docs/benchmarks/`](docs/benchmarks/README.md) | Benchmark methodology and results. |

> The Git repository is a **superset** of what is published to crates.io: design history, ADRs,
> planning, and benchmarks live here for contributors; the published crates ship only what
> library users need.

## Contributing

Contributions are welcome — please read [`CONTRIBUTING.md`](CONTRIBUTING.md) and our
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). Security issues: see [`SECURITY.md`](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as
above, without any additional terms or conditions.
