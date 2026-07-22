//! Integration tests for the Valkey backend against a **real** server (ADR-006, FR-022).
//!
//! Provisioning is via `docker-compose` (Valkey on port 6380). Reachability-gated:
//! skips when absent, hard-fails when `FLOWSHIELD_REQUIRE_VALKEY` is set.
//!
//!   docker compose up -d
//!   cargo test -p flowshield-valkey

use flowshield_core::algorithm::gcra::Gcra;
use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::key::Key;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_core::resp::RespScripted;
use flowshield_core::time::TestClock;
use flowshield_memory::MemoryStore;
use flowshield_valkey::{ClockSource, ValkeyStore};
use std::sync::Arc;
use std::time::Duration;

fn valkey_url() -> String {
    std::env::var("FLOWSHIELD_VALKEY_URL").unwrap_or_else(|_| "redis://127.0.0.1:6380".into())
}

// Keyed on `FLOWSHIELD_REQUIRE_VALKEY` only — deliberately NOT on `CI` (which GitHub sets
// in every job, including the plain `test` job that has no Valkey service). The dedicated
// `integration` job sets `FLOWSHIELD_REQUIRE_VALKEY=1` to force the hard failure.
fn valkey_required() -> bool {
    std::env::var("FLOWSHIELD_REQUIRE_VALKEY").is_ok()
}

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

macro_rules! gate {
    ($url:expr) => {{
        if !reachable(&$url).await {
            if valkey_required() {
                panic!("Valkey required but unreachable at {}", $url);
            }
            eprintln!(
                "skipping: no Valkey at {} (run `docker compose up -d`)",
                $url
            );
            return;
        }
    }};
}

fn unique_key(tag: &str) -> Key {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Key::from(format!("fs-test:{tag}:{n}"))
}

/// Same oracle cross-check as the Redis suite (ADR-019): the shared Lua, run on
/// Valkey, must match the Rust `step` decision for decision.
async fn cross_check_against_oracle<A>(url: &str, algo: A, tag: &str)
where
    A: RespScripted + Clone,
{
    let clock = Arc::new(TestClock::default());
    let store = ValkeyStore::builder(url)
        .clock_source(ClockSource::Client)
        .build()
        .expect("build valkey store");
    let valkey_limiter = LimiterBuilder::new(algo.clone(), store)
        .clock(clock.clone())
        .build()
        .unwrap();
    let mem_limiter = LimiterBuilder::new(algo, MemoryStore::<A::State>::new())
        .clock(clock.clone())
        .build()
        .unwrap();

    let vkey = unique_key(tag);
    let mkey = Key::from(format!("{tag}-mem"));

    for step in 0..40u32 {
        for _ in 0..2 {
            let v = valkey_limiter.check(&vkey).await.unwrap();
            let m = mem_limiter.check(&mkey).await.unwrap();
            assert_eq!(
                v, m,
                "{tag}: Valkey Lua vs Rust diverged at step {step}: valkey={v:?} memory={m:?}"
            );
        }
        clock.advance(Duration::from_millis(100));
    }
}

#[tokio::test]
async fn token_bucket_lua_matches_the_rust_oracle() {
    let url = valkey_url();
    gate!(url);
    cross_check_against_oracle(&url, TokenBucket::per_second(5, 5).unwrap(), "vk-tb").await;
}

#[tokio::test]
async fn gcra_lua_matches_the_rust_oracle() {
    let url = valkey_url();
    gate!(url);
    cross_check_against_oracle(&url, Gcra::new(5.0, 5).unwrap(), "vk-gcra").await;
}

#[tokio::test]
async fn server_time_admits_burst_then_rejects() {
    let url = valkey_url();
    gate!(url);
    let store = ValkeyStore::builder(&url)
        .build()
        .expect("build valkey store");
    let limiter = LimiterBuilder::new(TokenBucket::per_second(3, 1).unwrap(), store)
        .build()
        .unwrap();
    let key = unique_key("vk-burst");
    for i in 0..3 {
        assert!(
            limiter.check(&key).await.unwrap().is_allowed(),
            "request {i} of the burst should be admitted"
        );
    }
    assert!(
        !limiter.check(&key).await.unwrap().is_allowed(),
        "the 4th request must be rejected"
    );
}
