//! Actix + real Redis, over a real socket (Redis-gated).
//!
//! Scenario C: the Axum burst-then-429 test, but through the standalone Actix middleware
//! and its own HTTP server — proving the second, independent integration end-to-end.

mod common;

use flowshield::{LimiterBuilder, RedisStore, TokenBucket};
use reqwest::StatusCode;

#[tokio::test]
async fn actix_redis_burst_then_429() {
    let url = common::redis_url();
    if !common::redis_ready(&url).await {
        return;
    }

    let store = RedisStore::builder(&url)
        .build()
        .expect("build redis store");
    let limiter = LimiterBuilder::new(TokenBucket::per_second(5, 1).unwrap(), store)
        .build()
        .unwrap();
    let server = common::spawn_actix(limiter);
    let client = reqwest::Client::new();
    let key = common::unique_api_key("actix-burst");

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

    let resp = common::hit(&client, server.addr, &key).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        resp.headers().contains_key("retry-after"),
        "429 must carry Retry-After"
    );
    assert_eq!(common::remaining(&resp), Some(0));

    server.stop().await;
}
