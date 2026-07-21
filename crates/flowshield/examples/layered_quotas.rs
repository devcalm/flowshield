//! Layered quotas with a `LimiterSet` — FR-013 / UC-06.
//!
//! A request often has to satisfy several limits at once: a short-term burst limit
//! *and* a longer-window quota. FlowShield expresses this by composing independent
//! limiters — no rule DSL — and a [`LimiterSet`] evaluates them together, admitting
//! only when every layer admits and, on rejection, reporting the *binding* limit and
//! the longest wait across the layers.
//!
//! Run it:
//!
//! ```text
//! cargo run --example layered_quotas
//! ```
//!
//! This demo fires requests as fast as it can (no pacing), so you can watch the
//! binding limit change: an initial burst is allowed, then the short-term limit binds
//! (a sub-second retry), and finally the hard quota binds (a long retry).

use flowshield::{Key, LimiterBuilder, LimiterSet, MemoryStore, TokenBucket};
use std::time::Duration;

#[tokio::main]
async fn main() -> flowshield::Result<()> {
    // Layer 1 — short-term: a burst of 5, refilling 5/sec.
    let per_second =
        LimiterBuilder::new(TokenBucket::per_second(5, 5)?, MemoryStore::new()).build()?;

    // Layer 2 — the hard quota: 10 events total, refilling only once per hour (so it
    // is effectively fixed for this short demo).
    let quota = LimiterBuilder::new(
        TokenBucket::new(10, Duration::from_secs(3600))?,
        MemoryStore::new(),
    )
    .build()?;

    let limits = LimiterSet::new()
        .with("per-second", per_second)
        .with("quota", quota);

    let key = Key::from("tenant-7");

    // Note the consumption semantics (see `flowshield::composite`): a `LimiterSet`
    // evaluates *every* member to find the maximum retry-after, so a layer that admits
    // consumes even when another layer rejects the request overall. That is why the
    // quota below is drawn down during the phase where the per-second layer is the one
    // rejecting — v1 has no reserve-then-commit.
    for i in 1..=14 {
        let outcome = limits.check(&key).await?;
        if outcome.is_allowed() {
            println!("request {i:>2}  ✓ allowed");
        } else {
            let binding = outcome
                .binding()
                .expect("a rejection names its binding limit");
            let retry = outcome
                .retry_after()
                .expect("a rejection carries a retry_after");
            // In a real worker you would `sleep(retry)` and retry; here we just report.
            println!("request {i:>2}  ✗ rejected by '{binding}'  (retry after {retry:?})");
        }
    }

    println!(
        "\nThe binding limit moves from 'per-second' (short wait) to 'quota' (long wait) \n\
         as the hard cap is reached — the caller always learns which limit to wait on."
    );
    Ok(())
}
