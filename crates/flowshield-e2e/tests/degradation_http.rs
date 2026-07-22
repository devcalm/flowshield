//! Degradation over real HTTP against a DEAD Redis (always runs — no live server needed).
//!
//! Points the store at a port nothing listens on, with a short timeout, and drives the
//! decision through the full HTTP stack. This proves the configured degradation policy is
//! what the client actually sees: fail-closed → 429, fail-open → 200.

mod common;

use flowshield::{DegradationPolicy, LimiterBuilder, RedisStore, TokenBucket};
use reqwest::StatusCode;
use std::time::Duration;

/// Nothing listens here; the store will always time out.
const DEAD_REDIS: &str = "redis://127.0.0.1:6390";

fn dead_store() -> RedisStore {
    RedisStore::builder(DEAD_REDIS)
        .timeout(Duration::from_millis(200))
        .build()
        .expect("build redis store")
}

#[tokio::test]
async fn axum_fail_closed_dead_redis_returns_429() {
    // Default degradation is FailClosed: a backend outage must reject.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), dead_store())
        .build()
        .unwrap();
    let server = common::spawn_axum(limiter).await;
    let client = reqwest::Client::new();

    let resp = common::hit(&client, server.addr, "any-key").await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "fail-closed must surface 429 when the backend is unreachable"
    );
    assert!(resp.headers().contains_key("retry-after"));
}

#[tokio::test]
async fn axum_fail_open_dead_redis_returns_200() {
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), dead_store())
        .degradation(DegradationPolicy::FailOpen)
        .build()
        .unwrap();
    let server = common::spawn_axum(limiter).await;
    let client = reqwest::Client::new();

    let resp = common::hit(&client, server.addr, "any-key").await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "fail-open must admit when the backend is unreachable"
    );
}

#[tokio::test]
async fn actix_fail_closed_dead_redis_returns_429() {
    // Same guarantee through the independent Actix integration.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), dead_store())
        .build()
        .unwrap();
    let server = common::spawn_actix(limiter);
    let client = reqwest::Client::new();

    let resp = common::hit(&client, server.addr, "any-key").await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().contains_key("retry-after"));

    server.stop().await;
}
