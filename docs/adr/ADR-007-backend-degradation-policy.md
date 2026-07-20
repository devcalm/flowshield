# ADR-007: Backend Degradation Policy

- **Status:** Accepted
- **Date:** 2026-07-20
- **Deciders:** FlowShield maintainers
- **Related:** ADR-005 (Storage Abstraction), ADR-006 (Redis & Valkey Strategy), ADR-011 (Error Handling), ADR-012 (Observability); FR-023, FR-070, NFR-011; Risk R-07; OQ-01

## Context

A distributed rate limiter depends on an external store (Redis/Valkey) that can become slow or unreachable. When a backend operation fails or times out, the limiter must still return an admission decision. The behavior in that moment is a safety-vs-availability trade-off:

- **Fail Closed** — reject when the backend is unavailable. Protects the downstream service from overload/abuse, but converts a *backend* degradation into a *service* outage: every request is rejected while the store is down.
- **Fail Open** — admit when the backend is unavailable. Preserves availability, but removes protection exactly when a backend outage may coincide with a traffic surge, exposing the service to overload or abuse.

FlowShield markets itself as a *protection* tool. Users install it expecting their service to be shielded. A silent fail-open default would violate that expectation and could turn a dependency blip into an abuse window.

## Decision

**The default degradation behavior is Fail Closed.** Fail Open is available only via **explicit opt-in**.

The degradation policy is modeled as an explicit, extensible choice (not a boolean) so it can grow without breaking users. For v1 it exposes at least:

1. **Fail Closed (default)** — on backend failure/timeout, reject.
2. **Fail Open (opt-in)** — on backend failure/timeout, admit.

Every backend operation is subject to a **mandatory, configurable timeout**, so "fail closed" never means "hang the request thread waiting on a dead backend." A degradation event MUST emit observability signals (metric + structured log/trace) so operators can detect and alert on it (see ADR-012).

### Maintainer refinement (recorded disagreement / extension)

The reviewer's decision (Fail Closed default) is **accepted**. However, the maintainers note that a strict binary is a false dichotomy for production systems: failing an entire service closed on a brief Redis blip is, for many teams, worse than a small, bounded over-admission. We therefore **reserve a third policy for a future minor release**:

3. **Fail To Local (planned, non-breaking addition)** — on shared-backend failure, degrade to node-local limiting (using the in-memory backend as a fallback) rather than globally admitting or globally rejecting. This preserves *approximate* protection during an outage without a full service outage.

Because the policy is an extensible choice rather than a boolean, adding *Fail To Local* later is additive and does not break existing configurations. It is intentionally **out of v1 scope** to keep v1 correct and simple, but the API shape is chosen now so it can arrive without a breaking change.

## Alternatives Considered

- **Fail Open as default.** Rejected: contradicts the protection promise; a backend outage during a surge would leave the service unprotected by default. This is a footgun that fails *unsafely* and silently.
- **Boolean `fail_open: bool`.** Rejected: not extensible. Adding *Fail To Local* (or per-error-class policies) later would require a breaking change or an awkward second flag.
- **Fail To Local as the v1 default.** Rejected *for v1*: node-local fallback has real semantics to design (state divergence across nodes, re-convergence on recovery) and would delay a correct, simple v1. Deferred, not discarded.
- **No timeout (block until backend responds).** Rejected: a hung backend would exhaust request-handling resources — a self-inflicted DoS. Timeouts are mandatory.

## Consequences

- **Positive:** Safe-by-default; the framework protects the service out of the box. Explicit opt-in for Fail Open makes the availability trade-off a conscious, auditable choice. Mandatory timeouts prevent resource exhaustion during backend stalls.
- **Negative:** With the default, a full backend outage causes the protected surface to reject traffic (a visible availability hit) until the backend recovers or the operator switches policy. This is the accepted cost of a safe default, and is the specific motivation for the planned *Fail To Local* mode. Operators MUST wire the degradation metric to alerting, or a backend outage looks like an application outage.
- **Neutral:** The default posture is documented prominently in the getting-started material so users choose their policy deliberately.

## Future Evolution

- Add **Fail To Local** as a non-breaking policy option once its cross-node semantics are designed.
- Consider **per-error-class policies** (e.g., timeout vs. connection-refused vs. auth-error) if operators need finer control.
- Consider a **circuit-breaker / half-open probing** wrapper so the limiter stops hammering a dead backend and recovers gracefully.
