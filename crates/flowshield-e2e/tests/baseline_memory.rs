//! Real-HTTP baseline over the in-memory backend (always runs — no Redis needed).
//!
//! Gives the plain `test` CI job genuine end-to-end coverage of the request → middleware →
//! decision → 429/Retry-After path on every OS, with zero external services and zero
//! flakiness for the burst test. A second, time-dependent test proves `Retry-After` is
//! actually honored once a token refills.

mod common;

use flowshield::{LimiterBuilder, MemoryStore, TokenBucket};
use reqwest::StatusCode;
use std::time::Duration;

/// The burst-then-429 shape, in-memory. Deterministic and always green.
#[tokio::test]
async fn axum_memory_burst_then_429() {
    let limiter = LimiterBuilder::new(TokenBucket::per_second(5, 1).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    let server = common::spawn_axum(limiter).await;
    let client = reqwest::Client::new();
    let key = common::unique_api_key("mem-burst");

    for expected in (0..5u64).rev() {
        let resp = common::hit(&client, server.addr, &key).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(common::remaining(&resp), Some(expected));
    }

    let resp = common::hit(&client, server.addr, &key).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().contains_key("retry-after"));
}

/// After a rejection, waiting past the advertised refill window admits again. This is the
/// one wall-clock-dependent test; it runs on the in-memory backend (single-process,
/// monotonic clock) with generous slack over a 1s refill quantum to stay non-flaky.
#[tokio::test]
async fn axum_memory_retry_after_is_honored() {
    // Capacity 1, refill 1/sec: one admit, then rejections until a token refills.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    let server = common::spawn_axum(limiter).await;
    let client = reqwest::Client::new();
    let key = common::unique_api_key("mem-retry");

    let first = common::hit(&client, server.addr, &key).await;
    assert_eq!(first.status(), StatusCode::OK, "first request admitted");

    let second = common::hit(&client, server.addr, &key).await;
    assert_eq!(
        second.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "second request rejected (bucket drained)"
    );

    // Wait past the 1s refill window (1.5s slack) and try again.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let third = common::hit(&client, server.addr, &key).await;
    assert_eq!(
        third.status(),
        StatusCode::OK,
        "after the refill window, a token is available again"
    );
}
