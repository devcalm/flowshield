//! Actix Web integration test: the middleware rate-limits by client IP and by header,
//! returning `429 + Retry-After` on rejection (ADR-009, FR-032).

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpResponse, test, web};
use flowshield_actix::{FlowShield, HeaderKeyExtractor};
use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_memory::MemoryStore;
use std::net::SocketAddr;

fn limiter(
    capacity: u64,
) -> flowshield_core::limiter::Limiter<
    TokenBucket,
    MemoryStore<flowshield_core::algorithm::token_bucket::TokenBucketState>,
> {
    LimiterBuilder::new(
        TokenBucket::per_second(capacity, 1).unwrap(),
        MemoryStore::new(),
    )
    .build()
    .unwrap()
}

#[actix_web::test]
async fn rate_limits_by_peer_ip_with_429_and_retry_after() {
    let app = test::init_service(
        App::new()
            .wrap(FlowShield::per_peer(limiter(2)))
            .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
    )
    .await;

    let addr: SocketAddr = "1.2.3.4:5678".parse().unwrap();
    let make = || {
        test::TestRequest::get()
            .uri("/")
            .peer_addr(addr)
            .to_request()
    };

    // Capacity 2 → two admits, each reporting its remaining headroom (1 then 0).
    let r1 = test::call_service(&app, make()).await;
    assert_eq!(r1.status(), StatusCode::OK);
    assert_eq!(remaining(&r1), Some(1), "first allow leaves 1 token");
    let r2 = test::call_service(&app, make()).await;
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(remaining(&r2), Some(0), "second allow leaves 0 tokens");

    // Third rejected with Retry-After and X-RateLimit-Remaining: 0.
    let resp = test::call_service(&app, make()).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        resp.headers().contains_key(header::RETRY_AFTER),
        "429 must carry Retry-After"
    );
    assert_eq!(remaining(&resp), Some(0), "429 reports 0 remaining");
}

/// Read `X-RateLimit-Remaining` off a response, whatever its body type.
fn remaining<B>(resp: &actix_web::dev::ServiceResponse<B>) -> Option<u64> {
    resp.headers()
        .get("x-ratelimit-remaining")
        .map(|v| v.to_str().unwrap().parse().unwrap())
}

#[actix_web::test]
async fn distinct_peers_are_independent() {
    let app = test::init_service(
        App::new()
            .wrap(FlowShield::per_peer(limiter(1)))
            .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
    )
    .await;

    let a: SocketAddr = "10.0.0.1:1".parse().unwrap();
    let b: SocketAddr = "10.0.0.2:2".parse().unwrap();

    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::get().uri("/").peer_addr(a).to_request()
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::get().uri("/").peer_addr(a).to_request()
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // Peer B unaffected.
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::get().uri("/").peer_addr(b).to_request()
        )
        .await
        .status(),
        StatusCode::OK
    );
}

#[actix_web::test]
async fn keys_on_a_header_value() {
    let extractor = HeaderKeyExtractor::new(header::HeaderName::from_static("x-api-key"));
    let app = test::init_service(
        App::new()
            .wrap(FlowShield::new(limiter(1), extractor))
            .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
    )
    .await;

    let make = |k: &'static str| {
        test::TestRequest::get()
            .uri("/")
            .insert_header(("x-api-key", k))
            .to_request()
    };

    assert_eq!(
        test::call_service(&app, make("alice")).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        test::call_service(&app, make("alice")).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        test::call_service(&app, make("bob")).await.status(),
        StatusCode::OK
    );
}
