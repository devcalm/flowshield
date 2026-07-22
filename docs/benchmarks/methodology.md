# FlowShield Benchmark Methodology

> Governing decision: [ADR-014 — Benchmarking](../adr/ADR-014-benchmarking.md).
> **No numeric performance targets are committed until the suite produces data.**
> This document describes *how* FlowShield is measured so every published number is
> reproducible and every regression is caught the same way twice.

## Where the benchmark code lives

All benchmark code is in the non-published [`flowshield-benchmarks`](../../crates/flowshield-benchmarks)
crate (`publish = false`). It depends on the `flowshield` facade with the same features a
user would enable, so it measures the public surface, not internal shortcuts.

| Target | Kind | Scenario (ADR-014) |
|---|---|---|
| `benches/algorithm.rs` | Criterion | Pure `Algorithm::step` cost (the decision-cost floor) |
| `benches/decision.rs` | Criterion | End-to-end `Limiter::check` throughput + latency/tail, cardinality sweep |
| `benches/contention.rs` | Criterion | Shared-store contention (hot key vs. spread), thread sweep |
| `benches/http_middleware.rs` | Criterion | End-to-end HTTP-middleware overhead (Tower layer, in-process, no server) |
| `benches/redis_rtt.rs` | Criterion | Redis/Valkey network round-trip overhead (reachability-gated) |
| `examples/concurrency_10k.rs` | JSON scenario | 10k concurrent tasks complete without deadlock/panic (NFR-004) |
| `examples/allocations.rs` | JSON scenario | Heap allocations per decision on the hot path |

## Tools and why

- **[Criterion](https://github.com/bheisler/criterion.rs)** for the microbenchmarks. It
  reports the **full latency distribution** (p50…p99.9), not just the mean — ADR-014
  requires the tail — and writes **machine-readable** `estimates.json` per benchmark under
  `target/criterion/<id>/new/`, which is the archive/diff format the regression gate reads.
- **A counting global allocator** (in `examples/allocations.rs`) for allocations-per-decision.
  It wraps the system allocator, is single-threaded during measurement, and reports exact
  deterministic counts — no sampling.
- **A real multi-threaded Tokio runtime** for the 10k-concurrent scenario, because that
  scenario is about the async/contention behavior a service actually sees (ADR-016).

The in-memory backend's `evaluate` future resolves synchronously (family A, ADR-019), so the
in-memory microbenchmarks drive it with a no-op waker rather than a runtime — the measurement
is the limiter's own cost, free of executor scheduling noise. The Redis benchmark, whose
future genuinely pends on the network, uses a Tokio runtime via Criterion's `to_async`.

## Reproducing a run

Two of these look network-adjacent but only one needs a server. `http_middleware`
measures the **HTTP frontend** (the Tower/Axum rate-limit layer) driven *in-process* over
the in-memory backend — no socket, no server. `redis_rtt` measures the **storage backend's
network round-trip** and genuinely needs a live Redis/Valkey. They sit on opposite ends of
the request: middleware in front, store behind.

```sh
# In-memory suite (no server needed) — includes the in-process HTTP-middleware bench:
cargo bench -p flowshield-benchmarks \
  --bench algorithm --bench decision --bench contention --bench http_middleware

# Scenario binaries (JSON on stdout — redirect to archive them):
cargo run -p flowshield-benchmarks --release --example allocations
cargo run -p flowshield-benchmarks --release --example concurrency_10k

# Distributed round-trip (requires a live server; see docker-compose.yml):
FLOWSHIELD_REDIS_URL=redis://127.0.0.1:6379 \
  cargo bench -p flowshield-benchmarks --features redis --bench redis_rtt
```

### Workload parameters

- **Cardinality sweep:** `{1, 1_000, 100_000}` keys (`CARDINALITIES` in `src/lib.rs`) — a
  single hot key, a comfortable working set, and a large set that stresses the sharded map
  and LRU bookkeeping (FR-071, NFR-010). Store capacity is set above the working set so the
  accept path is measured in steady state, not while evicting.
- **Algorithms:** Token Bucket (default) and GCRA, both configured with ample capacity so the
  sweep measures the accept path; a dedicated `decision_reject` case measures the reject branch.
- **Threads (contention):** `{1, 2, 4, 8, machine width}`, in two regimes — *hot key* (all
  threads on one shard, worst-case lock contention) and *spread* (fanned across shards).
- **10k-concurrent:** 10,000 tasks × 100 decisions = 1,000,000 decisions, in a shared-hot-key
  and a per-task-key regime.

### Statistical treatment

- Criterion warms up, then collects samples and reports a 95% confidence interval per
  benchmark. Default warm-up/measurement windows are used unless a run overrides them.
- The regression gate compares the **median** (robust to the outliers ADR-014 warns about),
  not the mean.
- Deterministic by construction: workload generators use no RNG and read no wall clock, so a
  run is a pure function of its inputs and diffs cleanly across revisions.

## Regression gating

`scripts/bench_regression.py` (stdlib-only) turns Criterion output into a flat results map and
gates a candidate against a baseline:

```sh
# On the base revision, on the SAME runner:
cargo bench -p flowshield-benchmarks
python3 scripts/bench_regression.py extract > base.json

# On the candidate revision:
cargo bench -p flowshield-benchmarks
python3 scripts/bench_regression.py extract > cur.json

# Fail if any benchmark's median grew beyond the tolerance:
python3 scripts/bench_regression.py compare --baseline base.json --current cur.json --tolerance 1.15
```

The default tolerance is **±15% on the median**, sized to absorb shared-CI-runner variance.
Per ADR-014 this is a *living parameter*: on a dedicated, quiesced benchmark host the tolerance
should be tightened. Both runs must happen on the **same hardware** — comparing numbers across
machines is meaningless.

**Why the per-PR CI job does not run the numeric gate.** GitHub's shared runners are too noisy
to gate a 15% (let alone tighter) tolerance without false alarms, which would erode trust in the
signal. The per-PR [`bench` CI job](../../.github/workflows/ci.yml) therefore does the two things
that *are* reliable on shared hardware: it **compiles the whole suite** (so benchmarks never
bit-rot) and **runs the scenario binaries** (so a deadlock, panic, or a blown allocation budget
fails the build — NFR-004). Numeric regression gating is intended for a dedicated runner using
the script above.

## Results archive

Dated result snapshots live alongside this file (e.g. `results-2026-07-22-apple-m1.md`). Each
records the exact machine, toolchain, and command, so a claim can be reproduced or challenged.
See [ADR-014](../adr/ADR-014-benchmarking.md) for why measurement precedes any target.
