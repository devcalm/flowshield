//! Behavioural tests for the Tower layer: allow → 429 + Retry-After, per-key isolation,
//! and header keying (ADR-009, FR-011, FR-030, FR-034).

use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_memory::MemoryStore;
use flowshield_tower::{ForwardedForKeyExtractor, FlowShieldLayer, HeaderKeyExtractor};
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
async fn allowed_and_rejected_responses_carry_x_ratelimit_remaining() {
    // capacity 2, 1 token/sec → remaining ticks 1, 0, then 429 with remaining 0.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(2, 1).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    let svc = FlowShieldLayer::per_peer(limiter).layer(service_fn(ok));
    let addr: SocketAddr = "1.2.3.4:5678".parse().unwrap();

    let remaining = |resp: &Response<String>| -> Option<u64> {
        resp.headers()
            .get("x-ratelimit-remaining")
            .map(|v| v.to_str().unwrap().parse().unwrap())
    };

    // First admit: one token left after consuming.
    let r1 = svc.clone().oneshot(req_from(addr)).await.unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    assert_eq!(remaining(&r1), Some(1), "first allow leaves 1 token");

    // Second admit: bucket now drained.
    let r2 = svc.clone().oneshot(req_from(addr)).await.unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(remaining(&r2), Some(0), "second allow leaves 0 tokens");

    // Rejected: still reports remaining 0 alongside the 429.
    let r3 = svc.clone().oneshot(req_from(addr)).await.unwrap();
    assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(remaining(&r3), Some(0), "429 reports 0 remaining");
}

#[tokio::test]
async fn forwarded_for_keys_on_the_left_most_xff_ip_not_the_rfc7239_header() {
    // One token per key: a request with `X-Forwarded-For: <client>, <proxy>` must key on
    // the left-most client IP, and a *different* client IP must get its own bucket.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    let svc = FlowShieldLayer::new(limiter, ForwardedForKeyExtractor).layer(service_fn(ok));

    let with_headers = |xff: Option<&str>, forwarded: Option<&str>| {
        let mut req = Request::new(());
        if let Some(v) = xff {
            req.headers_mut().insert("x-forwarded-for", v.parse().unwrap());
        }
        if let Some(v) = forwarded {
            req.headers_mut().insert("forwarded", v.parse().unwrap());
        }
        req
    };

    // Client 203.0.113.7 (via a proxy hop) — admitted, then rejected on the second hit.
    let first = with_headers(Some("203.0.113.7, 70.41.3.18"), None);
    assert_eq!(
        svc.clone().oneshot(first).await.unwrap().status(),
        StatusCode::OK
    );
    let again = with_headers(Some("203.0.113.7, 70.41.3.18"), None);
    assert_eq!(
        svc.clone().oneshot(again).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS,
        "same left-most XFF client shares a bucket"
    );

    // A different client IP is a different bucket.
    let other = with_headers(Some("198.51.100.9"), None);
    assert_eq!(
        svc.clone().oneshot(other).await.unwrap().status(),
        StatusCode::OK,
        "a distinct XFF client must not share the first client's bucket"
    );

    // A request carrying only an RFC 7239 `Forwarded: for=203.0.113.7` header must NOT
    // be parsed as XFF: it has no `x-forwarded-for`, so it falls to the shared
    // `unknown-forwarded` bucket rather than mis-keying on `for=203.0.113.7`.
    let rfc = with_headers(None, Some("for=203.0.113.7;proto=https"));
    assert_eq!(
        svc.clone().oneshot(rfc).await.unwrap().status(),
        StatusCode::OK,
        "the RFC 7239 Forwarded header must not be mis-parsed as an XFF IP"
    );
    // The unknown-forwarded bucket now has one token spent; a second such request rejects.
    let rfc2 = with_headers(None, Some("for=198.51.100.9"));
    assert_eq!(
        svc.clone().oneshot(rfc2).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS,
        "requests without XFF share one fallback bucket"
    );
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
