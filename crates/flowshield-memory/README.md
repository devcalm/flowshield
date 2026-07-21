# flowshield-memory

The in-memory storage backend for [FlowShield](https://github.com/devcalm/flowshield) — the default, zero-external-dependency store for single-node rate limiting.

It implements `flowshield-core`'s `StorageBackend` trait with per-key limiter state that is:

- **Bounded** — never holds more than a configured number of keys; when full, the **least-recently-used** key is evicted, so a high-cardinality or adversarial key space cannot grow memory without limit.
- **Expiring** — each entry carries a TTL; an entry past its TTL presents as absent and is reclaimed, so an idle key behaves exactly like a fresh one.
- **Sharded** — state is partitioned across many independently-locked shards, so unrelated keys rarely contend on the same lock.

## Why LRU, not FIFO or random

For a cache, the eviction policy is a performance detail. For a **rate limiter** it is a correctness one: a key's state is what enforces its limit, and dropping it resets the key to a full allowance. Evicting a key that is *actively sending traffic* would briefly lift its limit exactly when it is needed. LRU sheds cold keys — which are not being limited anyway — and protects the hot ones, so eviction never loosens a limit that is actually in force.

## Usage

Most users get this backend transitively through the `flowshield` facade's default `memory` feature and never name this crate directly. To use it explicitly:

```rust
use flowshield_memory::{MemoryStore, MemoryStoreBuilder};
use flowshield_core::algorithm::token_bucket::TokenBucketState;

let store: MemoryStore<TokenBucketState> = MemoryStoreBuilder::new()
    .capacity(50_000) // approximate key bound
    .build()?;
# Ok::<(), flowshield_core::error::Error>(())
```

`MemoryStore::new()` uses a sensible default bound; `MemoryStoreBuilder::new().shards(1)` gives an exact capacity with no rounding.

## Conformance

This backend is validated against the shared `StorageBackend` contract harness in `flowshield-core` (behind its `conformance` feature), the same harness every distributed backend must pass — see `tests/conformance.rs`. Bounded-memory and atomicity-under-contention are covered in `tests/stress.rs`.

See [ADR-005 (Storage Abstraction)](../../docs/adr/ADR-005-storage-abstraction.md), [ADR-008 (Algorithms)](../../docs/adr/ADR-008-algorithms.md), and [ADR-010 (Feature Flags)](../../docs/adr/ADR-010-feature-flags.md).

## License

MIT OR Apache-2.0.
