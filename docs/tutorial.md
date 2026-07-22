# FlowShield Tutorial — from zero to a rate-limited service

This walks from a single in-process limiter to a distributed, HTTP-integrated setup. Every snippet
uses the public `flowshield` facade. If you'd rather read code, each crate's `examples/` directory
has runnable versions (`cargo run --example …`).

## 1. Install

```toml
[dependencies]
flowshield = "1"                              # default: in-memory backend
tokio = { version = "1", features = ["full"] }
```

## 2. Your first limiter

A limiter is an **algorithm** + a **store**. Build it once, share it (it's cheap to clone behind an
`Arc`, or wrap it yourself), and call `check` per request.

```rust,no_run
use flowshield::{Key, LimiterBuilder, MemoryStore, TokenBucket};

#[tokio::main]
async fn main() -> flowshield::Result<()> {
    // Capacity 10, refilling 2 tokens/second, applied per key.
    let limiter = LimiterBuilder::new(
        TokenBucket::per_second(10, 2)?,
        MemoryStore::new(),
    )
    .build()?;

    let decision = limiter.check(&Key::from("user-42")).await?;
    assert!(decision.is_allowed());
    Ok(())
}
```

`Key::from` accepts anything string-like — a user id, an IP, a tenant, an API token.

## 3. Understand the decision

`check` returns a `Decision`. Handle both arms:

```rust,ignore
match limiter.check(&Key::from("user-42")).await? {
    d if d.is_allowed() => {
        // Serve the request. `d.remaining()` is the headroom left, when the
        // algorithm can report it (Token Bucket and GCRA both do).
        serve();
    }
    d => {
        // Rejected. `d.retry_after()` is the *relative* wait until it could succeed.
        let wait = d.retry_after().unwrap();
        reject_with_retry_after(wait);
    }
}
```

`check(&key)` consumes one unit. For a weighted request use `check_n(&key, cost)`.

## 4. Choose an algorithm

FlowShield ships two, both allocation-free on the hot path:

- **`TokenBucket`** (default) — allows bursts up to capacity, then refills at a steady rate. Best for
  most APIs.
- **`Gcra`** — the Generic Cell Rate Algorithm; smooths traffic to a precise rate with a bounded
  burst. Best when you want tight pacing.

```rust,ignore
use flowshield::Gcra;
let limiter = LimiterBuilder::new(Gcra::new(100.0, 20)?, MemoryStore::new()).build()?;
// 100 requests/second sustained, burst of 20.
```

## 5. Rate-limit an HTTP service

### Axum

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
    .layer(per_client_ip(limiter)); // rejects with 429 + Retry-After, keyed by client IP
```

### Actix

```toml
flowshield = { version = "1", features = ["actix"] }
```

The Actix integration is a standalone middleware; see `flowshield-actix/examples/actix_server.rs`.

Both emit a standards-correct `429 Too Many Requests` with a `Retry-After` header and, on allowed
responses, `X-RateLimit-Remaining`.

> **Keying safely:** the default extractors key on the **peer IP**, which a client cannot spoof. Only
> use an `X-Forwarded-For`/real-IP extractor behind a trusted proxy that overwrites the header —
> otherwise a client can pick any key and dodge its limit.

## 6. Go distributed

Swap the in-memory store for Redis or Valkey and multiple processes coordinate on the same keys,
atomically, with no code change to your decision logic:

```toml
flowshield = { version = "1", features = ["redis"] }
```

```rust,ignore
use flowshield::{LimiterBuilder, RedisStore, TokenBucket};

let store = RedisStore::builder("redis://127.0.0.1:6379")
    .max_connections(16)
    .build()?;
let limiter = LimiterBuilder::new(TokenBucket::per_second(100, 20)?, store).build()?;
```

By default a backend outage **fails closed** (rejects). If you'd rather stay available during an
outage, opt into fail-open explicitly:

```rust,ignore
use flowshield::DegradationPolicy;
let limiter = LimiterBuilder::new(algo, store)
    .degradation(DegradationPolicy::FailOpen)
    .build()?;
```

## 7. Layered / composite limits

Enforce several limits at once — e.g. a burst-per-second *and* a daily quota. A request is admitted
only if **every** layer admits it; on rejection you learn which layer bound and the longest wait.

```rust,ignore
use flowshield::{Key, LimiterBuilder, LimiterSet, MemoryStore, TokenBucket};

let per_second = LimiterBuilder::new(TokenBucket::per_second(10, 10)?, MemoryStore::new()).build()?;
let per_day    = LimiterBuilder::new(TokenBucket::per_second(1000, 1000)?, MemoryStore::new()).build()?;

let limits = LimiterSet::new()
    .with("per-second", per_second)
    .with("per-day", per_day);

let outcome = limits.check(&Key::from("client-42")).await?;
if !outcome.is_allowed() {
    let bound = outcome.binding().unwrap();       // which layer rejected
    let retry = outcome.retry_after().unwrap();   // the longest wait
    eprintln!("rejected by {bound}, retry after {retry:?}");
}
```

## 8. Observability (optional)

Metrics and tracing are **off by default** and cost nothing when disabled. Turn them on with features:

```toml
flowshield = { version = "1", features = ["metrics", "tracing"] }
```

You then get Prometheus-compatible counters (`flowshield_decisions_total`,
`flowshield_degradations_total`) and a backend-latency histogram, plus a `tracing` span per decision.
Keys are redacted to a non-reversible digest — raw keys never reach telemetry. See
`flowshield/examples/observability.rs` for a full Prometheus + tracing wiring.

## Where to go next

- [Extension author guide](extension-guide.md) — write your own algorithm, backend, or key extractor.
- [Compatibility & versioning](COMPATIBILITY.md) — SemVer, MSRV (1.85), deprecation policy.
- [Security posture](security-review.md) — safe defaults and resource bounds.
- [Architecture overview](architecture/overview.md) and the [ADRs](adr/README.md) — the "why".
