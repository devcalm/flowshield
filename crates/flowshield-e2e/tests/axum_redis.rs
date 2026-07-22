//! Axum + real Redis, over a real socket (Redis-gated).
//!
//! Scenario A: one server, burst up to capacity then 429.
//! Scenario B: TWO servers sharing one Redis + one key — the combined traffic hits a single
//! global limit, which is the thing an in-memory backend can never demonstrate.

mod common;

use flowshield::{LimiterBuilder, RedisStore, TokenBucket};
use reqwest::StatusCode;

/// Scenario A — burst to capacity, then rejection with `Retry-After`.
#[tokio::test]
async fn axum_redis_burst_then_429() {
    let url = common::redis_url();
    if !common::redis_ready(&url).await {
        return;
    }

    // capacity 5, refill 1/sec: the whole burst runs in milliseconds, far under the 1s
    // refill quantum, so no token is replenished mid-test.
    let store = RedisStore::builder(&url)
        .build()
        .expect("build redis store");
    let limiter = LimiterBuilder::new(TokenBucket::per_second(5, 1).unwrap(), store)
        .build()
        .unwrap();
    let server = common::spawn_axum(limiter).await;
    let client = reqwest::Client::new();
    let key = common::unique_api_key("axum-burst");

    // Five admitted; `X-RateLimit-Remaining` counts 4,3,2,1,0.
    for expected in (0..5u64).rev() {
        let resp = common::hit(&client, server.addr, &key).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "burst request should be admitted"
        );
        assert_eq!(
            common::remaining(&resp),
            Some(expected),
            "X-RateLimit-Remaining should count down"
        );
    }

    // Sixth rejected, with Retry-After and zero remaining.
    let resp = common::hit(&client, server.addr, &key).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        resp.headers().contains_key("retry-after"),
        "429 must carry Retry-After"
    );
    assert_eq!(common::remaining(&resp), Some(0));
}

/// Scenario B — two servers, one Redis, one key: a single shared limit end-to-end.
#[tokio::test]
async fn axum_redis_distributed_shares_one_limit() {
    let url = common::redis_url();
    if !common::redis_ready(&url).await {
        return;
    }

    let cap = 6u64;
    let build = || {
        // Separate stores/connections (as two processes would have), same Redis URL.
        let store = RedisStore::builder(&url)
            .build()
            .expect("build redis store");
        LimiterBuilder::new(TokenBucket::per_second(cap, 1).unwrap(), store)
            .build()
            .unwrap()
    };
    let s1 = common::spawn_axum(build()).await;
    let s2 = common::spawn_axum(build()).await;
    let client = reqwest::Client::new();
    let key = common::unique_api_key("axum-dist");

    // Alternate across both servers. If they shared no state, each cap-6 bucket would show
    // remaining 5,5,4,4,3,3…; a single shared bucket shows a strict 5,4,3,2,1,0 descent.
    let addrs = [s1.addr, s2.addr];
    for i in 0..cap as usize {
        let resp = common::hit(&client, addrs[i % 2], &key).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "combined burst within capacity must be admitted"
        );
        assert_eq!(
            common::remaining(&resp),
            Some(cap - 1 - i as u64),
            "shared Redis state: remaining descends across BOTH servers"
        );
    }

    // The limit is now exhausted regardless of which server is asked.
    let r1 = common::hit(&client, s1.addr, &key).await;
    let r2 = common::hit(&client, s2.addr, &key).await;
    assert_eq!(
        r1.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "server 1 sees the shared limit exhausted"
    );
    assert_eq!(
        r2.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "server 2 sees the same shared limit exhausted"
    );
    assert!(r1.headers().contains_key("retry-after"));
    assert!(r2.headers().contains_key("retry-after"));
}
