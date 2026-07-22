# FlowShield 1.0.0 — Release Notes

**FlowShield 1.0 is the first stable release** of traffic management for Rust: a fast, correct rate
limiter that runs single-node or distributed, behind one storage abstraction, with thin web-framework
integrations — and a committed compatibility contract to build on.

## What you get

- **Two algorithms** — Token Bucket (default) and GCRA, allocation-free on the hot path
  (benchmarked: ~7.5 ns / ~15 ns per pure step; ~140 ns per full in-memory decision).
- **Three backends behind one seam** — in-memory (default, bounded + LRU), Redis, and Valkey. The
  distributed backends are **server-side atomic** (one Lua round-trip), not read-modify-write races.
- **Three framework integrations** — Tower, Axum, Actix; each returns `429 Too Many Requests` with
  `Retry-After`, and `X-RateLimit-Remaining` on allowed responses.
- **Safe by default** — fail-closed on backend outage (fail-open opt-in), a mandatory operation
  timeout, bounded memory, construction-time config validation, key redaction in telemetry.
- **Observability off by default** — Prometheus metrics and `tracing` spans behind feature flags.
- **`#![forbid(unsafe_code)]`** on every library crate.

## Stability & support

- **SemVer** across the `flowshield-*` family, covering the public **extension traits** too.
- **MSRV: Rust 1.85** for the core/default build (heavier optional features track their upstream
  ecosystems). See [COMPATIBILITY.md](COMPATIBILITY.md).
- **Deprecation policy**: compiler-visible, with a transition window of at least one minor release.

## Upgrading from 0.x

Update the version requirement to `1`. The public API is unchanged from the late 0.2 line; the 1.0
bump is a **stability commitment**, not an API break. `#[non_exhaustive]` types still require a
wildcard match arm.

## Known follow-ups (post-1.0, tracked)

- Empirical multi-node over-admission bound (DV-01) and ratified numeric performance targets (DV-03),
  both pending a dedicated benchmark host.

---

## Maintainer publish checklist (not yet executed)

> The crates are staged at `1.0.0` but **not published**. Run these steps to ship.

1. **Green CI on `main`** — fmt, clippy (`--all-features`), test matrix (incl. `--no-default-features`),
   docs, the `msrv` (1.85) job, the `bench` job, and the `integration` (Redis/Valkey) job.
2. **Dry-run each crate** (respecting the dependency order below):
   ```sh
   cargo publish -p <crate> --dry-run
   ```
3. **Publish in dependency order** (a crate can only be published after its path-deps exist on
   crates.io):
   ```
   flowshield-core
   flowshield-memory
   flowshield-resp
   flowshield-redis      flowshield-valkey
   flowshield-tower
   flowshield-axum       flowshield-actix
   flowshield            # the facade, last
   ```
   (`flowshield-benchmarks` is `publish = false` and is never released.)
4. **Tag the release:**
   ```sh
   git tag -a v1.0.0 -m "FlowShield 1.0.0 — first stable release"
   git push origin v1.0.0
   ```
5. **Update the status badge** in `README.md` from "1.0 readiness" to a release badge, and open the
   next `## [Unreleased]` section in `CHANGELOG.md` (already present).
