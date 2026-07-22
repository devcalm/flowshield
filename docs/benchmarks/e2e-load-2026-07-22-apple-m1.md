# End-to-end load results — 2026-07-22 (Apple M1)

Latency and throughput of the **whole HTTP stack** — real requests over loopback TCP →
Axum → limiter → backend → `200` — from the `flowshield-e2e` load benchmark
(`crates/flowshield-e2e/tests/load.rs`). Unlike the [in-process microbenchmarks](./results-2026-07-22-apple-m1.md),
this includes hyper, the TCP round-trip, and (for Redis) the network hop.

Per [ADR-014](../adr/ADR-014-benchmarking.md), these are a **dated, machine-scoped data point,
not a committed target**. Client and server share the machine (localhost), so the Redis figures
reflect a loopback round-trip, *not* real cross-host network latency (which would be higher).

## Environment

| | |
|---|---|
| CPU | Apple M1 (8 cores) |
| Memory | 16 GB |
| OS | macOS 26.5.2 |
| Toolchain | `rustc 1.95.0`, `--release` |
| Redis | `redis:7-alpine` in Docker, published on `127.0.0.1:6379` |
| Load | 20,000 requests, 64 in-flight, always-admit limiter |

## Results

Always-admitting limiter (capacity ≫ request count), so every request is served on the `200`
path — this measures the serving path, not rejections. Latencies in **microseconds**.

| Scenario | Throughput | p50 | p90 | p95 | p99 | p99.9 | max | mean |
|----------|-----------:|----:|----:|----:|----:|------:|----:|-----:|
| Axum + in-memory | **95,840 req/s** | 540 | 854 | 1047 | 1233 | 2754 | 2953 | 580 |
| Axum + Redis | **16,100 req/s** | 3796 | 4943 | 5441 | 6833 | 16638 | 17899 | 3894 |

## Reading it

- **The network round-trip dominates the distributed path.** Going from in-memory to Redis drops
  throughput ~6× (95.8k → 16.1k req/s) and raises p50 from ~0.5 ms to ~3.8 ms. That gap is the
  loopback RTT + RESP/EVALSHA cost per decision — exactly why FlowShield makes the distributed
  decision a *single* server-side atomic round-trip (ADR-006) rather than a chatty read-modify-write.
- **The in-memory hot path over real HTTP is ~0.5 ms p50 at ~96k req/s** on one machine — the limiter
  itself is a tiny fraction of that (the [in-process bench](./results-2026-07-22-apple-m1.md) puts a
  full decision at ~140 ns and the middleware overhead at ~455 ns); the rest is hyper + loopback TCP
  under 64-way concurrency.
- **Tails are wider under Redis** (p99.9 ~17 ms vs ~2.8 ms): connection-pool contention and occasional
  scheduler/GC-style hiccups on a shared laptop show up in the far tail. A dedicated host would tighten
  these; run-to-run variance on this laptop is roughly ±15% on throughput and larger on p99.9.

## Reproduce

```sh
docker compose up -d
cargo test -p flowshield-e2e --release --test load -- --ignored --nocapture
# tune the load:
FLOWSHIELD_LOAD_REQUESTS=50000 FLOWSHIELD_LOAD_CONCURRENCY=128 \
  cargo test -p flowshield-e2e --release --test load -- --ignored --nocapture
docker compose down
```

The benchmark also emits a machine-readable `JSON {…}` line per scenario for archival. It is
`#[ignore]`d (local-only) and never runs in CI. See [methodology.md](./methodology.md) for the
governing approach.

## Caveats

- **Localhost only** — no real network; treat the Redis numbers as a *lower bound* on distributed
  latency.
- **Single dated run** — not averaged across trials; not a target (ADR-014). Re-run on a quiesced,
  dedicated host before quoting figures anywhere load-bearing.
- **In-process servers** — the Axum server runs on a background task inside the test process, so the
  client and server contend for the same cores; a separate server process would shift the numbers.
