//! Axum integration test: the layer rate-limits a real router by client IP taken from
//! `ConnectInfo`, returning `429 + Retry-After` on rejection (ADR-009, FR-031).

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::{Router, routing::get};
use flowshield_axum::per_client_ip;
use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_memory::MemoryStore;
use http::{Request, StatusCode, header};
use std::net::SocketAddr;
use tower::ServiceExt; // for `oneshot`

fn app() -> Router {
    let limiter = LimiterBuilder::new(TokenBucket::per_second(2, 1).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    Router::new()
        .route("/", get(|| async { "ok" }))
        .layer(per_client_ip(limiter))
}

fn request(addr: SocketAddr) -> Request<Body> {
    let mut req = Request::builder().uri("/").body(Body::empty()).unwrap();
    // What `into_make_service_with_connect_info::<SocketAddr>()` would insert per
    // connection; we set it directly so the router can be driven with `oneshot`.
    req.extensions_mut().insert(ConnectInfo(addr));
    req
}

#[tokio::test]
async fn rate_limits_by_client_ip_with_429_and_retry_after() {
    let app = app();
    let addr: SocketAddr = "9.9.9.9:40000".parse().unwrap();

    // Capacity 2 → two admits.
    for _ in 0..2 {
        let resp = app.clone().oneshot(request(addr)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
    // Third rejected with a Retry-After header.
    let resp = app.clone().oneshot(request(addr)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        resp.headers().contains_key(header::RETRY_AFTER),
        "429 must carry Retry-After"
    );
}

#[tokio::test]
async fn distinct_client_ips_are_independent() {
    let app = app();
    let a: SocketAddr = "10.0.0.1:1".parse().unwrap();
    let b: SocketAddr = "10.0.0.2:2".parse().unwrap();

    // Drain A's bucket (capacity 2).
    for _ in 0..2 {
        assert_eq!(
            app.clone().oneshot(request(a)).await.unwrap().status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        app.clone().oneshot(request(a)).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // B is untouched.
    assert_eq!(
        app.clone().oneshot(request(b)).await.unwrap().status(),
        StatusCode::OK
    );
}
