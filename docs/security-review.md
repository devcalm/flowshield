# FlowShield Security Review & Posture

> Phase 7 deliverable (Plan.md task 2; FR-070–FR-072, P-06). This is a design-level security
> review of FlowShield's own code and defaults. For reporting vulnerabilities, see
> [`SECURITY.md`](../SECURITY.md).

FlowShield sits on the request hot path and is often the component that decides whether traffic is
admitted. Its security properties are therefore about **safe failure, bounded resources, and not
leaking sensitive data** — not cryptography. This review records the posture at 1.0.

## Summary

| Area | Posture |
|------|---------|
| Memory safety | `#![forbid(unsafe_code)]` on **every** library crate; no `unsafe` in library source. |
| Failure mode | **Fail-closed by default** (ADR-007); fail-open is explicit opt-in. |
| Backend latency | **Mandatory** per-operation timeout on distributed backends; a slow store cannot hang the caller. |
| Resource bounds | In-memory state is **bounded** (capacity + LRU eviction + lazy TTL); adversarial high-cardinality input cannot grow memory without bound. |
| Config safety | Invalid limits are rejected at **construction time**, not silently accepted. |
| Data leakage | Rate-limit **keys are redacted** in telemetry by default; raw keys never emitted. |
| Correctness under concurrency | Distributed decisions are **server-side atomic**; no client-side read-modify-write race. |
| Error handling | Typed error taxonomy; **no panics** on the decision path. |

## 1. Memory safety

No library crate uses `unsafe`. Each declares `#![forbid(unsafe_code)]`, so a future `unsafe` block
is a compile error, not a review miss. (The sole `unsafe` in the repository is a counting global
allocator inside the non-published `flowshield-benchmarks` crate, which never ships.)

## 2. Safe failure (degradation)

When a backend is unreachable or times out, the limiter applies its `DegradationPolicy`. The default
is **`FailClosed`**: a backend failure results in a *rejection*, so an outage cannot silently disable
rate limiting and expose the protected service to a flood (ADR-007). `FailOpen` exists but is an
explicit, deliberate opt-in for callers who prefer availability over protection. Degradation is
observable (`flowshield_degradations_total` + a `warn` event) so operators can see it happening.

A configuration error (e.g. an invalid algorithm parameter surfacing at runtime) is **never**
degraded — it is returned as `Error::Config`, because masking a setup bug as a rate-limit decision
would hide the real problem (ADR-011).

## 3. Backend latency cannot hang the caller

Every distributed operation is wrapped in `tokio::time::timeout` and resolves to a `Decision` or a
typed `Error::Timeout` — the timeout is **mandatory**, with a conservative default of 1s
(`DEFAULT_TIMEOUT`) tunable via the builder (ADR-006/007). A pathologically slow or half-open
connection therefore degrades per policy instead of blocking the request indefinitely — closing a
denial-of-service vector where a stalled backend stalls the whole service.

## 4. Bounded resource usage (DoS resistance)

The in-memory backend stores per-key state in a **sharded, capacity-bounded LRU** with lazy TTL
expiry (FR-071, NFR-010). An attacker sending a flood of distinct keys (high cardinality) cannot grow
memory without bound: once capacity is reached, the least-recently-used entry is evicted. This is
exercised by an adversarial high-cardinality stress test and concurrency tests (Phase 2). Per-key
state is `Copy` and small by contract, and the hot path is **allocation-free in steady state**
(benchmarked, Phase 6), so sustained load does not create allocator pressure.

On rejection, the `Decision` carries a **relative** `Retry-After`, which lets clients back off
correctly instead of hot-looping — reducing self-inflicted load amplification.

## 5. Construction-time configuration validation

Invalid configurations are rejected when the limiter/algorithm is built, not at first use:
`TokenBucket`/`Gcra` constructors return `Error::Config` for zero capacity, zero/absurd refill
intervals, etc., and `LimiterBuilder::build` rejects a zero TTL. This turns a class of latent
runtime footguns into eager, typed errors (FR-072).

## 6. No sensitive-data leakage in telemetry

Rate-limit keys are frequently sensitive — client IPs, user ids, API tokens. FlowShield **never**
emits raw keys. In telemetry a key appears only as a **non-reversible 64-bit FNV-1a digest**
(`k#…`), sufficient to correlate events for one key without disclosing it (ADR-012, FR-073). A test
asserts the raw key never reaches output. Static telemetry labels (algorithm name, backend kind) are
low-cardinality and carry no per-key data.

## 7. Correctness under concurrency

Single-node atomicity is provided by holding the shard lock across the whole check-and-consume, so
concurrent requests for one key cannot interleave and over-admit. Distributed atomicity is provided
**server-side**: the algorithm's transition runs as a single atomic Redis/Valkey script per decision
(EVALSHA, one round trip), sourcing time from the server to remove inter-node clock skew (ADR-006,
ADR-017). There is no client-side read-then-write window to race. Residual multi-node over-admission
is bounded and only occurs under replica failover; it is documented as a backend guarantee.

## Threats explicitly out of scope

- **Key spoofing** is the *integration's* responsibility: FlowShield limits whatever key it is given.
  The built-in extractors default to the spoof-resistant peer IP and loudly warn on
  `X-Forwarded-For`/`Forwarded` (trusted-proxy-only). See the [extension guide](extension-guide.md).
- **Transport security / auth** to Redis/Valkey is delegated to the connection URL and the operator's
  network posture; FlowShield does not add its own crypto.
- **Cryptographic key hashing** — the telemetry digest is for redaction/correlation, not a security
  boundary, and is not represented as one.

## Follow-ups tracked

- Multi-node over-admission bound (DV-01) awaits empirical characterization on dedicated hardware
  (Phase 6 follow-up); the qualitative guarantee is documented, the numeric bound is pending.
