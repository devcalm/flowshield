# flowshield-valkey

The **Valkey** distributed backend for [FlowShield](https://github.com/devcalm/flowshield):
atomic, server-side rate-limit state over RESP.

A thin wrapper over the shared [`flowshield-resp`](https://crates.io/crates/flowshield-resp)
logic (ADR-006) — the same logic [`flowshield-redis`](https://crates.io/crates/flowshield-redis)
wraps — so Valkey and Redis behave identically today while staying independently
selectable. FlowShield takes no side in the Redis/Valkey split.

```rust,ignore
use flowshield_valkey::ValkeyStore;
use flowshield_core::{algorithm::token_bucket::TokenBucket, key::Key, limiter::LimiterBuilder};

let store = ValkeyStore::builder("redis://127.0.0.1:6379").build()?;
let limiter = LimiterBuilder::new(TokenBucket::per_second(10, 5)?, store).build()?;
let decision = limiter.check(&Key::from("client-42")).await?;
```

**Guarantees:** exact on the executing primary; bounded over-admission only under
replication/failover; microsecond resolution. Tokio-coupled (`redis` + `deadpool-redis`).

## License

Licensed under either of Apache-2.0 or MIT at your option.
