# ADR-017: Time Source

- **Status:** Accepted
- **Date:** 2026-07-20
- **Deciders:** FlowShield maintainers
- **Related:** ADR-002 (Core Architecture), ADR-005 (Storage Abstraction), ADR-006 (Redis & Valkey Strategy), ADR-007 (Backend Degradation Policy), ADR-008 (Algorithms), ADR-013 (Extension API Stability), ADR-014 (Benchmarking), ADR-016 (Async Runtime); FR-011, NFR-002, NFR-034

## Context

Every rate-limiting algorithm FlowShield ships is a function of **elapsed time**: Token Bucket refills proportionally to time since the last update; GCRA compares the current instant against a stored theoretical arrival time (ADR-008). Time is therefore not an incidental detail — it is a load-bearing input to correctness. Three properties make it delicate:

1. **Monotonicity.** Elapsed-time math must never see time move backward. Wall-clock sources (`SystemTime` / civil time) can jump — NTP steps, manual clock changes, leap-second smearing — which would corrupt token replenishment or make a stored theoretical-arrival-time nonsensical, silently admitting or denying traffic incorrectly.
2. **Distribution.** In a shared-backend deployment, many nodes evaluate the same key. If each trusts its **own** wall clock, clock skew between nodes becomes a correctness variable: the same request stream yields different decisions depending on which node served it and how far its clock drifted.
3. **Determinism for testing.** Correctness and conformance (ADR-005) demand that "given identical inputs and conditions, decisions are identical." Time is one of those conditions. If time is read from a hidden global clock, tests cannot reproduce a condition or assert behavior across an exact elapsed interval — property tests, concurrency tests, and benchmarks (ADR-014) all become flaky or impossible.

The framework must decide, before implementation, what clock its algorithms read, whether that clock is injectable, and how time is sourced in the distributed case.

## Decision

FlowShield adopts an explicit, injectable **time-source abstraction**, with the following rules:

1. **Monotonic clock for elapsed-time math.** All algorithm math uses a **monotonic, non-decreasing** time source with `Instant`-like semantics (measuring elapsed durations), never a wall-clock source. Wall-clock time is used only where an *absolute civil time* is genuinely required and is documented as such at each use site; it is never used to compute elapsed intervals.

2. **Time is injected, not ambient.** The core reads time through the abstraction rather than calling a global clock directly. Production supplies a real monotonic clock; **tests supply a controllable fake** that can be created at a fixed origin and advanced by exact durations. This makes the deterministic-behavior requirement of the conformance suite (ADR-005) achievable, and lets property/concurrency tests assert exact behavior across simulated time.

3. **The store is the time authority for shared keys.** For distributed backends (Redis/Valkey, ADR-006), the time used in a shared-key decision is sourced from the **store**, inside the same atomic check-and-consume (e.g., the server's clock), not from each client's local clock. This removes inter-node clock skew as a correctness factor: every decision on a key is evaluated against one clock. Client-side time is not trusted for shared-key decisions; it remains valid for node-local backends (memory).

4. **Caller-facing time is relative, not absolute.** Retry-after / reset information returned on rejection (FR-011) is expressed as a **duration** (relative), computed from the authoritative clock, not as an absolute wall-clock timestamp. This prevents client/server clock skew from leaking into the public API contract.

5. **Runtime-independence preserved.** The abstraction does not mandate a specific async runtime's clock (ADR-016). A runtime timer (e.g., a pausable test clock) MAY back the abstraction where convenient, but the core depends on the FlowShield abstraction, not on the runtime, so the core stays runtime-independent.

The injectable time source is a **public extension seam** and its stability is governed by ADR-013 (evolved additively, `#[non_exhaustive]` where applicable).

## Alternatives Considered

- **Read `SystemTime` / wall-clock directly in algorithms.** Rejected: susceptible to backward jumps and NTP steps that corrupt elapsed-time math; unfit for a correctness-focused limiter.
- **Read a global monotonic clock (`Instant::now`) directly, no abstraction.** Rejected: monotonic and correct in production, but untestable — time becomes a hidden input, defeating deterministic conformance (ADR-005) and reproducible benchmarks (ADR-014).
- **Trust each node's local clock in the distributed case.** Rejected: makes cross-node clock skew a silent correctness variable; violates the single-clock-per-key property that keeps distributed decisions well-defined.
- **Mandate a specific runtime's time (e.g., a single async runtime's timer) in core.** Rejected: couples the core to a runtime, contradicting ADR-016. Such a timer may *back* the abstraction in tests, but must not be the core's dependency.
- **Return absolute wall-clock reset timestamps to callers.** Rejected: exposes clock skew across the client/server boundary; relative durations are unambiguous.

## Consequences

**Positive**
- Correctness is robust to clock adjustments (monotonic math) and to inter-node skew (store-authoritative time).
- Deterministic, reproducible tests and benchmarks become possible; the ADR-005 determinism requirement is actually testable.
- The core stays runtime-independent (ADR-016) while still being fully time-controllable in tests.

**Negative**
- A time source must be threaded through the algorithm and backend paths rather than reading an ambient clock — a small amount of plumbing and one more concept in the public surface.
- Sourcing time from the store adds a dependency on the store's clock facility and must be folded into the atomic operation (a design constraint on ADR-006), rather than read locally for free.

**Neutral**
- Memory backend and distributed backends source time differently (local monotonic vs. store-authoritative); this is expected and documented, mirroring the per-backend guarantee split of ADR-005.

## Future Evolution

- If future strongly-consistent or multi-key features (ADR-005 Future Evolution) need cross-node ordering beyond a single store clock, the abstraction can grow toward hybrid logical clocks under the additive rules of ADR-013.
- A **clock-skew / drift observability** signal (ADR-012) can be added so operators can detect when node or store clocks diverge beyond expected bounds.
- Optional capability traits may expose richer time semantics (e.g., deadline propagation) without changing the base seam.

## Addendum — 2026-07-20: Time is sampled outside the atomic section; algorithm anchors must be monotonic

*This addendum records an implementation invariant discovered while building the Phase-1 algorithms and their concurrency tests. It sharpens decision point #1 (monotonicity); it does not change the decision.*

**The hazard.** A monotonic *clock* is necessary but not sufficient. In the current pipeline the limiter reads `now` from the time source **before** it enters the store's atomic check-and-consume, then passes that value in. The read and the apply are therefore not a single atomic step. Under concurrent load on one key, two callers can each read a monotonic, non-decreasing timestamp — say 10ms and 50ms — yet acquire the store's atomic section in the **opposite** order, so the *older* sample is applied *after* the newer one. The clock never went backward; the ordering of two valid samples did.

**Why it bites.** Every algorithm stores a time-based anchor against which elapsed time is measured (a last-refill instant for Token Bucket, a theoretical-arrival-time for GCRA). If an algorithm writes that anchor back **verbatim** from the incoming `now`, the reordered older sample rewinds the anchor. The next request then measures elapsed time from the rewound anchor and **double-counts** the intervening span — minting allowance that time did not actually earn, i.e., silent over-admission. This was observed directly: a concurrency test admitted 143 where the rate permitted at most 100.

**The invariant.** Every algorithm MUST advance its stored time anchor **monotonically** — it takes the maximum of the previous anchor and the incoming `now`, never the incoming value alone. A reordered stale sample already contributes no elapsed time (elapsed-time math floors a backward delta to zero, per point #1); the additional rule is that it must also never *move the anchor back*. GCRA satisfied this for free because its transition is expressed as `max(tat, now) + increment`; Token Bucket had to be corrected to clamp its last-refill anchor the same way. This is a **conformance requirement** for any algorithm added under the ADR-013 extension seam, and belongs in the algorithm conformance suite.

**Relationship to the distributed case (decision point #3).** For shared-backend deployments where the store is the time authority, sourcing `now` from **inside** the atomic operation removes the read/apply gap at its root, so the reordering cannot arise for shared keys. The monotonic-anchor invariant remains mandatory regardless — it is the node-local (memory-backend) guarantee and defense-in-depth for every backend, and it keeps each algorithm correct without depending on how a particular store sources time. A future option is to fold the time read into the store contract (`update` sourcing `now` itself) so client-sampled time is never used for a shared-key decision; that is an additive ADR-005/006 change, not a prerequisite for the invariant above.
