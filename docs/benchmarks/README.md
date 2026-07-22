# Benchmarks

Benchmark **methodology** and published **results** for FlowShield.

This directory documents *how* we measure and *what* we measured. The benchmark **code** lives with the crates (e.g., `benches/` or a dedicated bench harness) from Phase 6; this directory holds the human-readable methodology and result write-ups so claims are reproducible and reviewable.

See [ADR-014 — Benchmarking](../adr/ADR-014-benchmarking.md) for the governing decision: **no numeric performance targets are set until the suite produces data.**

Contents:
- **[`methodology.md`](./methodology.md)** — environment, tools, scenarios, workload
  parameters, statistical treatment, how to reproduce, and the regression-gating workflow.
- **Results snapshots** — dated, machine-scoped narrative summaries: the in-process microbenchmarks
  ([`results-2026-07-22-apple-m1.md`](./results-2026-07-22-apple-m1.md)) and the whole-stack
  HTTP→Redis latency/throughput run ([`e2e-load-2026-07-22-apple-m1.md`](./e2e-load-2026-07-22-apple-m1.md));
  Criterion's machine-readable `estimates.json` lives under `target/criterion/` per run.

The benchmark **code** is in the non-published [`flowshield-benchmarks`](../../crates/flowshield-benchmarks)
crate. The regression gate is [`scripts/bench_regression.py`](../../scripts/bench_regression.py).

**Status:** Phase 6 suite implemented (throughput, latency/tail, allocations, contention,
10k-concurrent, Redis RTT). Official numeric targets (DV-03) and the multi-node
over-admission bound (DV-01) await a dedicated benchmark host.
