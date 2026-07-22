# Benchmark results — 2026-07-22 (Apple M1)

First baseline snapshot from the Phase 6 suite. Numbers here are a **data point on this
machine**, not a committed target (ADR-014): targets are ratified separately once
multi-machine data exists. Reproduce with the commands in [`methodology.md`](./methodology.md).

## Environment

| | |
|---|---|
| CPU | Apple M1 (8 cores: 4 performance + 4 efficiency) |
| Memory | 16 GB |
| OS | macOS 26.5.2 |
| Toolchain | `rustc 1.95.0` (release profile) |
| Criterion | warm-up 1s, measurement 3s (microbenchmarks) |

> A laptop under a normal desktop load is a *noisy* environment. Treat the microbenchmark
> medians as ±a few percent and the contention/throughput figures as order-of-magnitude.
> A quiesced, dedicated host is required before tightening the regression tolerance.

## Pure algorithm step (`benches/algorithm.rs`)

The decision-cost floor — algorithm transition only, no store, no lock, no async.

| Algorithm | Median |
|---|---|
| Token Bucket | **7.5 ns** |
| GCRA | **15.2 ns** |

## End-to-end decision (`benches/decision.rs`)

Full `Limiter::check` against the in-memory backend (hashing + shard lock + LRU + decision).

| Scenario | Keys | Median | Throughput |
|---|---|---|---|
| `decision_allow` / token_bucket | 1 | 137.7 ns | 7.26 M/s |
| `decision_allow` / token_bucket | 1 000 | 144.3 ns | 6.93 M/s |
| `decision_allow` / token_bucket | 100 000 | 178.1 ns | 5.62 M/s |
| `decision_allow` / gcra | 1 | ~148 ns | ~6.8 M/s |
| `decision_allow` / gcra | 1 000 | 148.8 ns | 6.72 M/s |
| `decision_allow` / gcra | 100 000 | 179.3 ns | 5.58 M/s |
| `decision_reject` / token_bucket | 1 | 139.7 ns | 7.16 M/s |

**Reading it.** The full `check` costs ~130 ns on top of the ~7.5 ns algorithm step — that
overhead is the shard hashing, lock acquire/release, LRU bookkeeping, and async plumbing.
Cardinality costs ~40 ns going from 1 to 100 000 keys (cache/LRU pressure), and the picture
is essentially identical for both algorithms and for the accept vs. reject branch.

## Allocations per decision (`examples/allocations.rs`)

| Scenario | Allocations / 1M decisions | Per decision |
|---|---|---|
| token_bucket / hot key | 0 | **0** |
| gcra / hot key | 0 | **0** |
| token_bucket / high cardinality (100k) | 8 | **~0** |

**The hot path is allocation-free in steady state** — `Copy` per-key state (ADR-008) and a
resident LRU entry mean a repeat decision touches no heap. This directly satisfies the
bounded-memory intent of FR-071 / NFR-010 at the allocation level.

## HTTP middleware overhead (`benches/http_middleware.rs`)

The Tower rate-limit layer driven in-process over the in-memory backend — no server, so
this is FlowShield's own per-request contribution (`flowshield-axum` builds on this layer).

| Case | Keys | Median | Throughput |
|---|---|---|---|
| `http_inner_only` (baseline downstream) | — | 84.1 ns | ~11.9 M/s |
| `http_wrapped_allow` | 1 | 539.4 ns | 1.85 M/s |
| `http_wrapped_allow` | 1 000 | 584.5 ns | 1.71 M/s |
| `http_wrapped_allow` | 100 000 | 692.1 ns | 1.44 M/s |
| `http_wrapped_reject` (429 build) | 1 | ~611 ns | ~1.6 M/s |

**Reading it.** The layer adds **~455 ns** over the bare service on the admit path — much
more than the ~138 ns raw decision, and the gap is the HTTP-integration cost, not the
limiter: boxing the service future (`Box::pin`), the `Arc` clones of limiter/extractor/inner,
the peer-IP → `String` key extraction (an allocation per request), and the
`X-RateLimit-Remaining` header write. Unlike the raw decision path, **the HTTP path is not
allocation-free** — the key-extraction string and the boxed future dominate. Still ~1.8 M
req/s single-threaded, so the limiter is not the bottleneck in a typical service; but this
is the clearest target for a future zero-alloc key-extraction pass.

## 10k concurrent (`examples/concurrency_10k.rs`)

10,000 Tokio tasks × 100 decisions = 1,000,000 decisions, shared limiter. Completes with **no
deadlock or panic** (NFR-004 acceptance gate).

| Regime | Aggregate throughput |
|---|---|
| shared hot key (one shard) | ~1.9 M/s |
| per-task key (fanned across shards) | ~9.4 M/s |

The ~5× gap between the two regimes is the sharding payoff: a single hot key serializes on one
shard `Mutex`, while a fanned-out workload proceeds in parallel across shards.

## Contention (`benches/contention.rs`)

Ran and captured, but the medians are dominated by scheduler/thermal noise at laptop scale;
they show the expected shape — hot-key throughput flattens as threads pile onto one shard while
spread scales further — but are **not** reported as figures here. Re-run on a dedicated host for
publishable contention numbers.

## Redis round-trip (`benches/redis_rtt.rs`)

Not run in this snapshot (no server was attached). Run against `docker-compose.yml` to populate;
expect the network round-trip to dominate — orders of magnitude above the in-memory path, which
is exactly why the distributed guarantee is server-side atomic (ADR-006) rather than chatty.

## Open items carried forward

- **DV-01** (numeric multi-node over-admission bound) still needs a multi-node run against real
  Redis/Valkey — the `redis_rtt` harness is the starting point; the multi-node coordination
  measurement is the remaining piece.
- **DV-03 / official targets** await a dedicated benchmark host so the medians are stable enough
  to commit to.
