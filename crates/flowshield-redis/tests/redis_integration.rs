//! Integration tests for the Redis backend against a **real** server (ADR-006, FR-021).
//!
//! Provisioning is via `docker-compose` (see `docker-compose.yml` at the repo root):
//!
//! ```text
//! docker compose up -d
//! cargo test -p flowshield-redis
//! ```
//!
//! The tests are **reachability-gated**: if no server answers they skip (so a plain
//! `cargo test` without the services stays green), unless `CI` or
//! `FLOWSHIELD_REQUIRE_REDIS` is set, in which case an unreachable server is a hard
//! failure — that is how CI guarantees the distributed path is actually exercised.

use flowshield_core::algorithm::gcra::Gcra;
use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::decision::Decision;
use flowshield_core::key::Key;
use flowshield_core::limiter::{DegradationPolicy, LimiterBuilder};
use flowshield_core::resp::RespScripted;
use flowshield_core::time::TestClock;
use flowshield_memory::MemoryStore;
use flowshield_redis::{ClockSource, RedisStore};
use std::sync::Arc;
use std::time::Duration;

fn redis_url() -> String {
    std::env::var("FLOWSHIELD_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

fn redis_required() -> bool {
    std::env::var("CI").is_ok() || std::env::var("FLOWSHIELD_REQUIRE_REDIS").is_ok()
}

/// `true` if a RESP server answers `PING` at `url`.
async fn reachable(url: &str) -> bool {
    let Ok(client) = redis::Client::open(url) else {
        return false;
    };
    let Ok(mut conn) = client.get_multiplexed_async_connection().await else {
        return false;
    };
    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .is_ok()
}

/// Skip helper: returns `false` (caller should `return`) when the server is absent and
/// not required; panics when it is required.
macro_rules! gate {
    ($url:expr) => {{
        if !reachable(&$url).await {
            if redis_required() {
                panic!(
                    "Redis required (CI/FLOWSHIELD_REQUIRE_REDIS) but unreachable at {}",
                    $url
                );
            }
            eprintln!(
                "skipping: no Redis at {} (run `docker compose up -d`)",
                $url
            );
            return;
        }
    }};
}

/// A per-test-unique key so runs never collide on shared server state (no FLUSHDB).
fn unique_key(tag: &str) -> Key {
    // A monotonic-ish disambiguator; test isolation only, not correctness-critical.
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Key::from(format!("fs-test:{tag}:{n}"))
}

/// The heart of the phase: a RESP algorithm's Lua transition must produce **exactly**
/// the decisions its Rust `step` oracle produces (ADR-019). We drive the *same*
/// arrival trace, against the *same* controllable clock, through both the Redis
/// backend (client-time mode) and the memory backend, and assert every decision
/// matches — including `remaining` and `retry_after`.
///
/// `algo` is cloned so both limiters run an identically-configured instance. The
/// trace advances in whole 100ms steps so the nanosecond (memory) and microsecond
/// (Redis) encodings land on identical arithmetic.
async fn cross_check_against_oracle<A>(url: &str, algo: A, tag: &str)
where
    A: RespScripted + Clone,
{
    let clock = Arc::new(TestClock::default());

    let redis_store = RedisStore::builder(url)
        .clock_source(ClockSource::Client) // deterministic: use the injected clock
        .build()
        .expect("build redis store");
    let redis_limiter = LimiterBuilder::new(algo.clone(), redis_store)
        .clock(clock.clone())
        .build()
        .unwrap();

    let mem_limiter = LimiterBuilder::new(algo, MemoryStore::<A::State>::new())
        .clock(clock.clone())
        .build()
        .unwrap();

    // Distinct keys per backend (different stores) but the same clock/trace.
    let rkey = unique_key(tag);
    let mkey = Key::from(format!("{tag}-mem"));

    for step in 0..40u32 {
        // Hammer twice per step to force both bursts and rejections.
        for _ in 0..2 {
            let r = redis_limiter.check(&rkey).await.unwrap();
            let m = mem_limiter.check(&mkey).await.unwrap();
            assert_eq!(
                r, m,
                "{tag}: Lua vs Rust diverged at step {step}: redis={r:?} memory={m:?}"
            );
        }
        clock.advance(Duration::from_millis(100));
    }
}

#[tokio::test]
async fn token_bucket_lua_matches_the_rust_oracle() {
    let url = redis_url();
    gate!(url);
    cross_check_against_oracle(&url, TokenBucket::per_second(5, 5).unwrap(), "xcheck-tb").await;
}

#[tokio::test]
async fn gcra_lua_matches_the_rust_oracle() {
    let url = redis_url();
    gate!(url);
    cross_check_against_oracle(&url, Gcra::new(5.0, 5).unwrap(), "xcheck-gcra").await;
}

/// Regression test for the H1 audit finding: a GCRA key that is *continuously
/// rejected* must not let its state expire mid-limiting and reset to a fresh (more
/// permissive) schedule (ADR-005 §3). The bug was that the GCRA reject path did not
/// refresh the key TTL, unlike the memory backend and the Token Bucket script.
///
/// This must use **real time**, not the injected clock: Redis key expiry (`PEXPIRE`)
/// is governed by the server's wall-clock, so only real elapsed time exercises it. A
/// slow limiter (1 admit / 5s) with a short 300ms TTL: after the first admit, hammer
/// every 100ms past the TTL — every request must stay rejected. Before the fix the
/// key expired at ~300ms and a fresh request was wrongly admitted around 400ms.
#[tokio::test]
async fn gcra_reject_refreshes_ttl_and_does_not_over_admit() {
    let url = redis_url();
    gate!(url);

    let store = RedisStore::builder(&url).build().unwrap(); // server (real) time
    let limiter = LimiterBuilder::new(Gcra::new(0.2, 1).unwrap(), store) // T = 5s, burst 1
        .ttl(Duration::from_millis(300))
        .build()
        .unwrap();
    let key = unique_key("gcra-ttl");

    // Fresh key: the first request is admitted; the schedule is now ~5s ahead.
    assert!(limiter.check(&key).await.unwrap().is_allowed());

    // Hammer every 100ms out to 700ms — well past the 300ms TTL, far before 5s. Every
    // request must be rejected; a resurrected key would admit here (the H1 bug).
    for i in 1..=7 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !limiter.check(&key).await.unwrap().is_allowed(),
            "request {i} (~{}ms) must be rejected — an actively-rejected key must not \
             expire and over-admit",
            i * 100
        );
    }
}

/// End-to-end behavioural check on the production path: **server** time, a fresh key
/// admits a full burst, then rejects once drained.
#[tokio::test]
async fn server_time_admits_burst_then_rejects() {
    let url = redis_url();
    gate!(url);

    let store = RedisStore::builder(&url)
        .build()
        .expect("build redis store");
    let limiter = LimiterBuilder::new(TokenBucket::per_second(3, 1).unwrap(), store)
        .build()
        .unwrap();
    let key = unique_key("burst");

    // Capacity 3: three immediate admits, then a rejection carrying a retry-after.
    for i in 0..3 {
        assert!(
            limiter.check(&key).await.unwrap().is_allowed(),
            "request {i} of the burst should be admitted"
        );
    }
    let denied = limiter.check(&key).await.unwrap();
    assert!(!denied.is_allowed(), "the 4th request must be rejected");
    match denied {
        Decision::Rejected(r) => assert!(
            r.retry_after > Duration::ZERO && r.retry_after <= Duration::from_secs(1),
            "retry_after should be ~1s (1 token/sec), got {:?}",
            r.retry_after
        ),
        _ => unreachable!(),
    }
}

/// A backend pointed at a dead server must surface a typed error (which the limiter
/// turns into its degradation fallback) rather than hang — the timeout contract
/// (ADR-006/007). This one needs no live server, so it always runs.
#[tokio::test]
async fn unreachable_server_times_out_and_degrades() {
    // Port 6390 is not part of docker-compose; nothing listens there.
    let store = RedisStore::builder("redis://127.0.0.1:6390")
        .timeout(Duration::from_millis(200))
        .build()
        .expect("build");
    // Default degradation is fail-closed, so an unreachable backend rejects rather
    // than hanging or erroring out of `check`.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), store)
        .build()
        .unwrap();
    let decision = limiter.check(&Key::from("k")).await.unwrap();
    assert!(
        !decision.is_allowed(),
        "fail-closed degradation must reject when the backend is unreachable"
    );
}

/// The opt-in inverse: with `FailOpen`, the same unreachable backend must *admit*
/// rather than reject — the explicit availability-over-safety choice (ADR-007). No
/// live server needed.
#[tokio::test]
async fn unreachable_server_fails_open_when_opted_in() {
    let store = RedisStore::builder("redis://127.0.0.1:6390")
        .timeout(Duration::from_millis(200))
        .build()
        .expect("build");
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), store)
        .degradation(DegradationPolicy::FailOpen)
        .build()
        .unwrap();
    let decision = limiter.check(&Key::from("k")).await.unwrap();
    assert!(
        decision.is_allowed(),
        "fail-open degradation must admit when the backend is unreachable"
    );
}
