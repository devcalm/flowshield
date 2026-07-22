# FlowShield Extension Author Guide

FlowShield is designed to be an *ecosystem*: you can ship your own **algorithm**, **storage
backend**, or **key extractor** in your own crate, without patching FlowShield. This guide shows
how, and states the stability you can rely on.

> Governing decisions: [ADR-013 (Extension API Stability)](adr/ADR-013-public-extension-api-stability.md),
> [ADR-005 (Storage Abstraction)](adr/ADR-005-storage-abstraction.md),
> [ADR-008 (Algorithms)](adr/ADR-008-algorithms.md). Stability tiers per trait are in
> [COMPATIBILITY.md](COMPATIBILITY.md).

All extension traits live in `flowshield-core` and are re-exported from the `flowshield` facade.
Depend on whichever you prefer; the types are identical.

---

## 1. A custom `Algorithm`

An algorithm is a **pure** state transition. Given the current per-key state, the current time, and
a request cost, it returns the new state and a [`Decision`]. Purity (no I/O, no clock reads) is what
lets the same algorithm run under an in-memory lock *or* be re-run by a transactional backend
(ADR-008).

```rust
use flowshield::{Algorithm, Decision};
use std::time::Duration;

/// A trivial "allow N per fixed window from first-seen" algorithm (illustrative).
pub struct FixedWindow {
    limit: u64,
    window: Duration,
}

#[derive(Clone, Copy)]
pub struct WindowState {
    window_start: Duration,
    used: u64,
}

impl Algorithm for FixedWindow {
    type State = WindowState; // must be `Copy + Send + 'static` — keep it small (FR-071)

    fn initial(&self, now: Duration) -> Self::State {
        WindowState { window_start: now, used: 0 }
    }

    fn step(&self, mut state: Self::State, now: Duration, cost: u64) -> (Self::State, Decision) {
        // Roll the window if it has elapsed.
        if now.saturating_sub(state.window_start) >= self.window {
            state = WindowState { window_start: now, used: 0 };
        }
        if state.used + cost <= self.limit {
            state.used += cost;
            let remaining = self.limit - state.used;
            (state, Decision::allowed(Some(remaining)))
        } else {
            let elapsed = now.saturating_sub(state.window_start);
            let retry = self.window.saturating_sub(elapsed);
            (state, Decision::rejected(retry))
        }
    }

    fn name(&self) -> &'static str {
        "fixed_window" // low-cardinality telemetry label (ADR-012) — never per-key data
    }
}
```

Rules:
- **`State: Copy`** and small — it is stored per key; large state defeats the bounded-memory goal.
- **`step` must be pure and deterministic.** No `Instant::now()`, no locks, no randomness. All time
  comes in as the `now` argument (ADR-017).
- Build a decision with `Decision::allowed(Some(remaining))` (or `None` if you can't cheaply report
  headroom) and `Decision::rejected(retry_after)` — the fields are `#[non_exhaustive]`, so these
  constructors are the supported way to create them from another crate.

Your algorithm works with any family-A backend immediately (in-memory, and any transactional store).
To also run **server-side** on Redis/Valkey, you additionally implement the `RespScripted` capability
trait (a Lua encoding of `step`) — that seam is *Evolving* (see COMPATIBILITY.md) and documented in
the `flowshield-resp` crate.

---

## 2. A custom storage backend

A backend makes the check-and-consume **atomic**. There are two families (ADR-019); pick the one that
matches how your store achieves atomicity.

### Family A — you run the Rust transition under your own isolation

Implement [`LocalStore<S>`]: load the state at `key`, apply the caller's `step`, store the result, and
return the [`Decision`] — with no two operations for the same key interleaving. A lock, or a
transaction with retry, both qualify. `step` is `Fn` (not `FnOnce`) precisely so a transactional
backend may re-run it on a write conflict; that is sound because the transition is pure.

```rust
use flowshield::{Algorithm, Decision, Key};
use flowshield_core::store::{Evaluate, LocalStore, evaluate_local};
use flowshield_core::error::Result;
use std::future::Future;
use std::time::Duration;

pub struct MyStore { /* connection pool / map / ... */ }

impl<S: Send + 'static> LocalStore<S> for MyStore {
    fn transact(
        &self,
        key: &Key,
        now: Duration,
        ttl: Duration,
        step: impl Fn(Option<S>) -> (S, Decision) + Send,
    ) -> impl Future<Output = Result<Decision>> + Send {
        async move {
            // 1. begin isolation (lock / transaction) for `key`
            // 2. let current = load(key, now);  // treat expired as None
            // 3. let (new_state, decision) = step(current);
            // 4. store(key, new_state, expires_at = now + ttl);
            // 5. commit; return Ok(decision)
            todo!()
        }
    }
}

// One-line bridge to the limiter: forward to the shared helper.
impl<A: Algorithm> Evaluate<A> for MyStore {
    fn kind(&self) -> &'static str { "my_store" } // telemetry backend label
    fn evaluate(
        &self,
        algo: &A,
        key: &Key,
        now: Duration,
        ttl: Duration,
        cost: u64,
    ) -> impl Future<Output = Result<Decision>> + Send {
        evaluate_local(self, algo, key, now, ttl, cost)
    }
}
```

### Family B — the server runs the transition (no Rust on the server)

If your store can't run Rust (Redis, a SQL `UPDATE … RETURNING`, a DynamoDB conditional write), you
implement [`Evaluate<A>`] **directly** for the algorithms that expose a backend-native encoding, and
run that encoding on the server. There is deliberately no single "server transition" trait — Lua, SQL,
and DynamoDB expressions share no language. See `flowshield-resp` for the Redis/Valkey pattern.

### Prove it with the conformance harness

Any family-A backend must pass the shared behavioural contract (ADR-005, FR-025). Enable the
`conformance` feature as a dev-dependency and run the harness against a fresh store:

```rust,ignore
// tests/conformance.rs
#[tokio::test]
async fn my_store_satisfies_the_contract() {
    flowshield_core::conformance::verify_local_store(|| MyStore::new()).await;
}
```

The harness checks that an absent/expired key presents `None`, that stored state is handed to the next
`step`, that `transact` returns exactly the decision `step` produced, and more — panicking with a named
property on any violation.

---

## 3. A custom `KeyExtractor`

A key extractor derives the limiting [`Key`] from your context type (an HTTP request, a gRPC metadata
map, a job descriptor — anything). It is generic over the context `C`.

```rust
use flowshield::{Key, KeyExtractor};

pub struct TenantHeader;

impl KeyExtractor<http::Request<()>> for TenantHeader {
    fn extract_key(&self, req: &http::Request<()>) -> Key {
        req.headers()
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(Key::from)
            // Missing credential → one shared bucket, limited rather than unlimited.
            .unwrap_or_else(|| Key::from("unknown-tenant"))
    }
}
```

Security note: prefer keys the client **cannot forge**. Peer IP (from the connection) is
spoof-resistant; `X-Forwarded-For` is client-controlled and safe only behind a trusted proxy that
overwrites it. FlowShield's built-in extractors document this explicitly.

---

## 4. What you can rely on (stability)

`Algorithm`, `KeyExtractor`, `Decision`, `Error`, and the limiter types are **Stable** — covered by
SemVer, evolved only additively (default methods / capability traits / `#[non_exhaustive]`). The
storage seam (`LocalStore`, `Evaluate`, `evaluate_local`) is **Stable** too; the server-native
encoding seam (`RespScripted`) is **Evolving**. Full tier table: [COMPATIBILITY.md](COMPATIBILITY.md).

Because public enums, error types, and the decision/outcome types are `#[non_exhaustive]`, always
include a wildcard arm when matching them so a future added variant doesn't break your build.

[`Decision`]: https://docs.rs/flowshield-core/latest/flowshield_core/decision/enum.Decision.html
[`Key`]: https://docs.rs/flowshield-core/latest/flowshield_core/key/struct.Key.html
[`LocalStore<S>`]: https://docs.rs/flowshield-core/latest/flowshield_core/store/trait.LocalStore.html
[`Evaluate<A>`]: https://docs.rs/flowshield-core/latest/flowshield_core/store/trait.Evaluate.html
