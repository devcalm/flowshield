//! A runnable Actix Web server rate-limited by client IP (FR-032, UC-01).
//!
//! ```text
//! cargo run -p flowshield-actix --example actix_server
//! # then, in another shell — the first few succeed (200), then 429s appear:
//! for i in $(seq 1 10); do curl -s -o /dev/null -w "%{http_code} " http://127.0.0.1:8080/; done; echo
//! ```
//!
//! Token Bucket of capacity 5 refilling 2/sec, keyed per client IP: a burst of 5 is
//! admitted, then requests are paced; rejected requests get `429 Too Many Requests`
//! with a `Retry-After` header.

use actix_web::{App, HttpResponse, HttpServer, web};
use flowshield_actix::FlowShield;
use flowshield_core::algorithm::token_bucket::TokenBucket;
use flowshield_core::limiter::LimiterBuilder;
use flowshield_memory::MemoryStore;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // capacity 5, refill 2/sec — a small burst then a steady pace.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(5, 2).unwrap(), MemoryStore::new())
        .build()
        .unwrap();
    // One shared limiter (Arc inside), cloned into each worker's App.
    let mw = FlowShield::per_peer(limiter);

    println!("listening on http://127.0.0.1:8080");
    HttpServer::new(move || {
        App::new().wrap(mw.clone()).route(
            "/",
            web::get().to(|| async { HttpResponse::Ok().body("ok\n") }),
        )
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
