# Benchmarks

Benchmark **methodology** and published **results** for FlowShield.

This directory documents *how* we measure and *what* we measured. The benchmark **code** lives with the crates (e.g., `benches/` or a dedicated bench harness) from Phase 6; this directory holds the human-readable methodology and result write-ups so claims are reproducible and reviewable.

See [ADR-014 — Benchmarking](../adr/ADR-014-benchmarking.md) for the governing decision: **no numeric performance targets are set until the suite produces data.**

Planned contents:
- Methodology: environment, warm-up, sample sizes, statistical treatment, how to reproduce.
- Scenarios: throughput, latency (incl. tail), allocations, contention, 10k concurrent, Redis RTT, multi-node.
- Results: dated, machine-readable + narrative summaries; regression history.

**Status:** to be populated during Phase 6.
