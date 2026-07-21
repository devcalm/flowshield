//! A runnable Axum server rate-limited by client IP (FR-030, FR-031, UC-01).
//!
//! ```text
//! cargo run -p flowshield-axum --example axum_server
//! # then, in another shell — the first few succeed (200), then 429s appear:
//! for i in $(seq 1 10); do curl -s -o /dev/null -w "%{http_code} " http://127.0.0.1:3000/; done; echo
//! ```
//!
//! The limiter is a Token Bucket of capacity 5 refilling 2/sec, keyed per client IP via
//! [`ConnectInfo`]. A burst of 5 is admitted, then requests are paced; rejected requests
//! get `429 Too Many Requests` with a `Retry-After` header.

use axum::{Router, routing::get};
use flowshield_axum::per_client_ip;
use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_memory::MemoryStore;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // capacity 5, refill 2/sec — a small burst then a steady pace.
    let limiter =
        LimiterBuilder::new(TokenBucket::per_second(5, 2)?, MemoryStore::new()).build()?;

    let app = Router::new()
        .route("/", get(|| async { "ok\n" }))
        .layer(per_client_ip(limiter));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("listening on http://{addr}");

    // `into_make_service_with_connect_info` makes the peer address available to the
    // extractor — without it, all clients would share the `"unknown-peer"` bucket.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
