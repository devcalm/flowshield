//! Behavioural tests for the Tower layer: allow → 429 + Retry-After, per-key isolation,
//! and header keying (ADR-009, FR-011, FR-030, FR-034).

use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_memory::MemoryStore;
use flowshield_tower::{FlowShieldLayer, HeaderKeyExtractor};
use http::{HeaderName, Request, Response, StatusCode, header};
use std::convert::Infallible;
use std::net::SocketAddr;
use tower::{Layer, ServiceExt, service_fn};

/// A trivial inner service: always `200 OK` with an empty body.
async fn ok(_req: Request<()>) -> Result<Response<String>, Infallible> {
    Ok(Response::new(String::new()))
}

fn req_from(addr: SocketAddr) -> Request<()> {
    let mut req = Request::new(());
    req.extensions_mut().insert(addr); // what PeerAddrKeyExtractor reads
    req
}

#[tokio::test]
async fn admits_burst_then_429_with_retry_after() {
    // capacity 2, 1 token/sec.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(2, 1).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    let svc = FlowShieldLayer::per_peer(limiter).layer(service_fn(ok));
    let addr: SocketAddr = "1.2.3.4:5678".parse().unwrap();

    // The two-token burst is admitted.
    for _ in 0..2 {
        let resp = svc.clone().oneshot(req_from(addr)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
    // The third is rejected with a well-formed 429 + Retry-After.
    let resp = svc.clone().oneshot(req_from(addr)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry = resp
        .headers()
        .get(header::RETRY_AFTER)
        .expect("429 must carry Retry-After");
    let secs: u64 = retry.to_str().unwrap().parse().unwrap();
    assert!(secs >= 1, "Retry-After should be at least 1s, got {secs}");
}

#[tokio::test]
async fn distinct_peers_have_independent_limits() {
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    let svc = FlowShieldLayer::per_peer(limiter).layer(service_fn(ok));
    let a: SocketAddr = "1.1.1.1:1".parse().unwrap();
    let b: SocketAddr = "2.2.2.2:2".parse().unwrap();

    // Peer A: one token, then rejected.
    assert_eq!(
        svc.clone().oneshot(req_from(a)).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        svc.clone().oneshot(req_from(a)).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // Peer B is unaffected — separate bucket.
    assert_eq!(
        svc.clone().oneshot(req_from(b)).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn keys_on_a_header_value() {
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    let extractor = HeaderKeyExtractor::new(HeaderName::from_static("x-api-key"));
    let svc = FlowShieldLayer::new(limiter, extractor).layer(service_fn(ok));

    let with_key = |k: &str| {
        let mut req = Request::new(());
        req.headers_mut().insert("x-api-key", k.parse().unwrap());
        req
    };

    // Key "alice": one token then rejected; key "bob" independent.
    assert_eq!(
        svc.clone()
            .oneshot(with_key("alice"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        svc.clone()
            .oneshot(with_key("alice"))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        svc.clone().oneshot(with_key("bob")).await.unwrap().status(),
        StatusCode::OK
    );
}
