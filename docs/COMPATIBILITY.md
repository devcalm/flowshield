# FlowShield Compatibility, Versioning & MSRV Policy

> Governing decisions: [ADR-015 (Versioning, MSRV & Compatibility)](adr/ADR-015-versioning-and-msrv.md)
> and [ADR-013 (Public Extension API Stability)](adr/ADR-013-public-extension-api-stability.md).
> This document turns those decisions into the concrete promises FlowShield makes to its users
> at 1.0.

## Semantic Versioning

All published FlowShield crates follow [SemVer 2.0](https://semver.org/). Given a released
`MAJOR.MINOR.PATCH`:

- **PATCH** — backward-compatible bug fixes only.
- **MINOR** — backward-compatible additions (new items, new default-provided trait methods,
  new capability traits, new `#[non_exhaustive]` variants/fields).
- **MAJOR** — any breaking change to a stable public API, always with a documented migration path.

Pre-1.0 (`0.x`) releases followed the convention that a minor bump could break; **from 1.0
onward the full guarantees above are in force.**

### What is covered

The **stable public API** is everything reachable from the `flowshield` facade and the documented
public items of each crate, **including the public extension traits** (`Algorithm`,
`StorageBackend`/`LocalStore`/`Evaluate`, `KeyExtractor`) per ADR-013. Items marked
`#[doc(hidden)]`, modules documented as internal, and the `flowshield-benchmarks` crate are **not**
part of the contract.

## Extension-API stability tiers

Public extension points are labeled so implementers know how much churn to expect (ADR-013 §Future
Evolution). At 1.0:

| Tier | Meaning | Members |
|------|---------|---------|
| **Stable** | Covered by SemVer; evolves only additively (defaults / capability traits / `#[non_exhaustive]`). | `Algorithm`, `KeyExtractor`, `Decision`, `Error`, `DegradationPolicy`, `LimiterBuilder`, `Limiter` |
| **Stable (seam)** | Stable, but most authors use the `LocalStore` adapter rather than implementing `Evaluate` directly. | `Evaluate`, `LocalStore`, `evaluate_local` |
| **Evolving** | Public and usable, but may gain members via the additive rules more often as the ecosystem matures. | `RespScripted` / `RespProgram` (server-native encoding seam), framework integration surfaces |
| **Internal** | Not public contract; may change any time. | `observe`, `#[doc(hidden)]` items, test doubles |

Adding a **required** method to any Stable trait is a breaking change and is forbidden within a
major version — new behavior arrives as a default-provided method or a new capability trait (e.g.
a future `Peek`), never as a silent break. See the [extension-author guide](extension-guide.md).

## Minimum Supported Rust Version (MSRV)

**Committed MSRV: Rust 1.85** — the edition-2024 floor (resolves DV-02).

- FlowShield's **own source** compiles on 1.85 across every first-party crate.
- The **default build** (`flowshield` with the default `memory` feature, plus `flowshield-core`
  and `flowshield-memory`) is **CI-verified on 1.85** on every change.

### Effective MSRV is the max of the floor and your enabled features

Enabling the heavy optional features pulls large third-party dependency trees whose *own* MSRVs are
currently **higher than 1.85** (e.g. `time` requires 1.88; `icu`/`idna`, via `url`, require 1.86):

| Build | Effective MSRV |
|-------|----------------|
| `flowshield` default (`memory`), `flowshield-core`, `flowshield-memory` | **1.85** (guaranteed) |
| `+ redis` / `+ valkey` | governed by the `redis`/`url` ecosystem (currently ~1.88) |
| `+ axum` / `+ actix` / `+ tower` | governed by those framework ecosystems (currently ~1.88) |

This is a deliberate, common trade-off: we do not hold the whole ecosystem back to 1.85, but we
*do* guarantee the core rate-limiting experience there. If you need a hard 1.85 build with a
distributed backend, pin the offending transitive deps to their last 1.85-compatible releases with
`cargo update <dep> --precise <ver>`.

### Advancing the MSRV

Raising the committed MSRV is done **only in a minor release**, announced in the
[CHANGELOG](../CHANGELOG.md), and is treated as a **minor (non-breaking) change** — consistent with
common Rust-ecosystem practice. It never happens in a patch release. It will be advanced
conservatively, driven by concrete need, not by chasing the newest toolchain.

## Deprecation policy

When a stable item is slated for removal:

1. It is marked `#[deprecated(since = "…", note = "…")]` so the compiler surfaces it.
2. The deprecation is recorded in the CHANGELOG under the release that introduces it, with the
   replacement and the earliest version it may be removed in.
3. The item keeps working for **at least one full minor release** and is removed **no earlier than
   the next major version**. Users always get a compiler-visible transition window, never a surprise
   break within a major line.

## Cross-crate versioning

The `flowshield-*` family shares a synchronized major/minor line so cross-crate upgrades stay
predictable. Backend and integration crates may publish patch releases independently for fixes, but
a breaking change anywhere in the public surface advances the shared major version.

## Reporting compatibility issues

If an upgrade breaks you in a way this policy says it shouldn't, please open an issue — that's a bug
in our adherence to this policy, and we want to fix it. Security-sensitive reports: see
[`SECURITY.md`](../SECURITY.md).
