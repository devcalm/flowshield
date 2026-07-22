//! Concurrency stress tests over the full HTTP stack (local-only; `#[ignore]`d).
//!
//! These fire a *thundering herd* of real concurrent HTTP requests through the whole stack
//! and assert the one property a rate limiter must never violate under load: **no
//! over-admission**. A capacity-`N` bucket, hit by `M > N` simultaneous requests, must admit
//! *exactly* `N` — not one more (server-side atomic for Redis; shard-lock atomic for
//! in-memory) — and every server must survive without a panic, deadlock, or error (NFR-004).
//!
//! They are deliberately excluded from CI (`#[ignore]`) — too expensive to run there. Run
//! them locally:
//!
//! ```text
//! docker compose up -d                                   # for the Redis case
//! cargo test -p flowshield-e2e --release -- --ignored
//! # scale it up:
//! FLOWSHIELD_STRESS_REQUESTS=5000 FLOWSHIELD_STRESS_CAPACITY=250 \
//!   cargo test -p flowshield-e2e --release -- --ignored
//! ```

mod common;

use flowshield::{LimiterBuilder, MemoryStore, RedisStore, TokenBucket};
use reqwest::StatusCode;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Total concurrent requests (the herd size). Override with `FLOWSHIELD_STRESS_REQUESTS`.
fn stress_requests() -> u64 {
    env_u64("FLOWSHIELD_STRESS_REQUESTS", 1000)
}

/// Bucket capacity — the exact number that must be admitted. Override with
/// `FLOWSHIELD_STRESS_CAPACITY`.
fn stress_capacity() -> u64 {
    env_u64("FLOWSHIELD_STRESS_CAPACITY", 100)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Refill interval long enough that no token is replenished during the herd, so the
/// admission count is exact regardless of how long the herd takes to drain.
const NO_REFILL: Duration = Duration::from_secs(3600);

/// Cap on simultaneously in-flight requests per herd. High enough to race hundreds of
/// requests on one key (a serious concurrency test for the atomic check-and-consume), but
/// bounded so socket/fd usage stays sane — `cargo test` runs the four matrix tests in
/// parallel, and an unbounded herd would open `requests × 4` connections at once and
/// exhaust file descriptors. Total requests (`FLOWSHIELD_STRESS_REQUESTS`) can therefore be
/// cranked arbitrarily high without connection exhaustion; only the burst width is capped.
const MAX_IN_FLIGHT: usize = 512;

/// Outcome tallies from a herd.
#[derive(Default)]
struct Tally {
    allowed: AtomicU64,
    rejected: AtomicU64,
    other: AtomicU64,
}

/// Fire `total` GETs at `addr` (all with the same key), at most [`MAX_IN_FLIGHT`] in flight
/// at once, and tally the outcomes. A panic/deadlock in any task fails the test via
/// `.expect` — the NFR-004 gate.
async fn herd(addr: SocketAddr, key: &str, total: u64) -> Tally {
    let client = reqwest::Client::new();
    let tally = Arc::new(Tally::default());
    let key = Arc::new(key.to_string());
    let permits = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));

    let mut handles = Vec::with_capacity(total as usize);
    for _ in 0..total {
        // Acquire before spawning: the loop blocks here once MAX_IN_FLIGHT are in flight,
        // which is what bounds the concurrent socket count.
        let permit = Arc::clone(&permits)
            .acquire_owned()
            .await
            .expect("semaphore closed");
        let client = client.clone();
        let key = Arc::clone(&key);
        let tally = Arc::clone(&tally);
        handles.push(tokio::spawn(async move {
            let _permit = permit; // released when the request completes
            let resp = common::hit(&client, addr, &key).await;
            match resp.status() {
                StatusCode::OK => tally.allowed.fetch_add(1, Ordering::Relaxed),
                StatusCode::TOO_MANY_REQUESTS => tally.rejected.fetch_add(1, Ordering::Relaxed),
                _ => tally.other.fetch_add(1, Ordering::Relaxed),
            };
        }));
    }
    for h in handles {
        h.await.expect("request task panicked");
    }

    Arc::try_unwrap(tally).ok().expect("tally still shared")
}

/// Assert the no-over-admission invariant: exactly `capacity` admitted, the rest rejected,
/// and not a single malformed response (5xx / connection error).
fn assert_exact_admission(tally: &Tally, capacity: u64, total: u64) {
    let allowed = tally.allowed.load(Ordering::Relaxed);
    let rejected = tally.rejected.load(Ordering::Relaxed);
    let other = tally.other.load(Ordering::Relaxed);

    assert_eq!(
        other, 0,
        "every response must be a clean 200 or 429; got {other} malformed"
    );
    assert_eq!(
        allowed, capacity,
        "exactly capacity must be admitted under the herd (no over/under-admission)"
    );
    assert_eq!(
        rejected,
        total - capacity,
        "all non-admitted requests must be rejected"
    );
}

/// Number of Actix workers for the memory-backed cases — several threads contending on the
/// one shared shard-locked store is the point.
const ACTIX_WORKERS: usize = 4;

/// A capacity-`cap` limiter over **real Redis** (server-side atomic transition), refill
/// disabled so the herd's admission count is exact.
fn redis_limiter(url: &str, cap: u64) -> impl flowshield::DynLimiter + 'static {
    let store = RedisStore::builder(url)
        // Raise the pool so the herd isn't needlessly serialized by connection starvation.
        .max_connections(64)
        .build()
        .expect("build redis store");
    LimiterBuilder::new(TokenBucket::new(cap, NO_REFILL).unwrap(), store)
        .build()
        .unwrap()
}

/// A capacity-`cap` limiter over the **in-memory** store (shard-lock atomic transition).
fn memory_limiter(cap: u64) -> impl flowshield::DynLimiter + 'static {
    LimiterBuilder::new(
        TokenBucket::new(cap, NO_REFILL).unwrap(),
        MemoryStore::new(),
    )
    .build()
    .unwrap()
}

// --- The 2×2 matrix: {Axum, Actix} × {Redis, in-memory} -----------------------------
//
// Each fires a concurrent HTTP herd at one server and asserts exactly `cap` admissions.
// Redis cases are additionally gated on a reachable server; all four are `#[ignore]`d
// (local-only).

/// Axum + Redis: server-side atomic no-over-admission under a concurrent HTTP herd.
#[tokio::test]
#[ignore = "stress: local-only, run with --ignored (needs Redis via docker compose)"]
async fn axum_redis_no_over_admission_under_herd() {
    let url = common::redis_url();
    if !common::redis_ready(&url).await {
        return;
    }
    let (cap, total) = herd_params();
    let server = common::spawn_axum(redis_limiter(&url, cap)).await;
    let tally = herd(
        server.addr,
        &common::unique_api_key("stress-axum-redis"),
        total,
    )
    .await;
    assert_exact_admission(&tally, cap, total);
}

/// Axum + in-memory: shard-lock atomic no-over-admission under a concurrent HTTP herd.
#[tokio::test]
#[ignore = "stress: local-only, run with --ignored"]
async fn axum_memory_no_over_admission_under_herd() {
    let (cap, total) = herd_params();
    let server = common::spawn_axum(memory_limiter(cap)).await;
    let tally = herd(
        server.addr,
        &common::unique_api_key("stress-axum-memory"),
        total,
    )
    .await;
    assert_exact_admission(&tally, cap, total);
}

/// Actix + Redis: server-side atomic no-over-admission through the Actix stack under a herd.
#[tokio::test]
#[ignore = "stress: local-only, run with --ignored (needs Redis via docker compose)"]
async fn actix_redis_no_over_admission_under_herd() {
    let url = common::redis_url();
    if !common::redis_ready(&url).await {
        return;
    }
    let (cap, total) = herd_params();
    let server = common::spawn_actix_with_workers(redis_limiter(&url, cap), ACTIX_WORKERS);
    let tally = herd(
        server.addr,
        &common::unique_api_key("stress-actix-redis"),
        total,
    )
    .await;
    assert_exact_admission(&tally, cap, total);
    server.stop().await;
}

/// Actix + in-memory: shard-lock atomic no-over-admission under a herd, with several Actix
/// workers contending on the one shared store.
#[tokio::test]
#[ignore = "stress: local-only, run with --ignored"]
async fn actix_memory_no_over_admission_under_herd() {
    let (cap, total) = herd_params();
    let server = common::spawn_actix_with_workers(memory_limiter(cap), ACTIX_WORKERS);
    let tally = herd(
        server.addr,
        &common::unique_api_key("stress-actix-memory"),
        total,
    )
    .await;
    assert_exact_admission(&tally, cap, total);
    server.stop().await;
}

/// Resolve `(capacity, total)`, guaranteeing `total > capacity` so there is something to reject.
fn herd_params() -> (u64, u64) {
    let cap = stress_capacity().max(1);
    let total = stress_requests().max(cap + 1);
    (cap, total)
}
