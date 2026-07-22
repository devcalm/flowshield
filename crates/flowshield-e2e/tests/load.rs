//! End-to-end **load** benchmark (local-only; `#[ignore]`d) — latency + throughput.
//!
//! Unlike `stress.rs` (which asserts *correctness* under a herd, and whose responses are
//! mostly `429`s), this measures the **real serving path**: an always-admitting limiter, so
//! every request goes limiter → backend → `200`, and we record the per-request latency of
//! the full HTTP round-trip. It reports p50/p90/p95/p99/p99.9/max and throughput, for an
//! in-memory baseline and for real Redis, so the network cost of the distributed backend is
//! visible against the local floor.
//!
//! Per ADR-014, the numbers this prints are **dated, machine-scoped data points, not
//! targets**; capture a run into `docs/benchmarks/` (see the e2e-load results snapshot).
//! Client and server share the machine (localhost), so these reflect stack + loopback cost,
//! not real network latency.
//!
//! ```text
//! docker compose up -d
//! cargo test -p flowshield-e2e --release --test load -- --ignored --nocapture
//! # tune it:
//! FLOWSHIELD_LOAD_REQUESTS=50000 FLOWSHIELD_LOAD_CONCURRENCY=128 \
//!   cargo test -p flowshield-e2e --release --test load -- --ignored --nocapture
//! ```

mod common;

use flowshield::{LimiterBuilder, MemoryStore, RedisStore, TokenBucket};
use reqwest::StatusCode;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn load_requests() -> u64 {
    env_u64("FLOWSHIELD_LOAD_REQUESTS", 20_000)
}

fn load_concurrency() -> usize {
    env_u64("FLOWSHIELD_LOAD_CONCURRENCY", 64) as usize
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// An always-admitting token bucket: a capacity far larger than any local run's request
/// count, so every request is served on the `200` path and we measure real serving latency,
/// never the reject branch. The value stays within `i64` (Redis stores it as a Redis integer
/// server-side, so `u64::MAX` would overflow there and spuriously reject).
fn always_admit() -> TokenBucket {
    TokenBucket::per_second(1_000_000_000, 1_000_000_000).unwrap()
}

/// Fire `total` requests at `addr`, at most `concurrency` in flight, and return each
/// request's latency plus the total wall-clock elapsed (for throughput).
async fn measure(
    addr: SocketAddr,
    key: &str,
    total: u64,
    concurrency: usize,
) -> (Vec<Duration>, Duration) {
    let client = reqwest::Client::new();
    let key = Arc::new(key.to_string());
    let permits = Arc::new(tokio::sync::Semaphore::new(concurrency));

    let start = Instant::now();
    let mut handles = Vec::with_capacity(total as usize);
    for _ in 0..total {
        let permit = Arc::clone(&permits)
            .acquire_owned()
            .await
            .expect("semaphore closed");
        let client = client.clone();
        let key = Arc::clone(&key);
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let t = Instant::now();
            let resp = common::hit(&client, addr, &key).await;
            let elapsed = t.elapsed();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "load benchmark uses an always-admit limiter; every response must be 200"
            );
            elapsed
        }));
    }
    let mut latencies = Vec::with_capacity(total as usize);
    for h in handles {
        latencies.push(h.await.expect("request task panicked"));
    }
    (latencies, start.elapsed())
}

/// Nearest-rank percentile over a sorted slice.
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// Run one scenario and print a formatted line + a JSON object for archival.
async fn report(name: &str, addr: SocketAddr, total: u64, concurrency: usize) {
    let (mut lat, wall) = measure(addr, &common::unique_api_key("load"), total, concurrency).await;
    lat.sort_unstable();

    let n = lat.len() as f64;
    let mean = micros(lat.iter().sum::<Duration>()) / n;
    let throughput = n / wall.as_secs_f64();
    let p = |q| micros(percentile(&lat, q));

    println!(
        "  {name:<12} thrpt={throughput:>9.0} req/s  \
         p50={:>7.1}  p90={:>7.1}  p95={:>7.1}  p99={:>8.1}  p99.9={:>8.1}  max={:>8.1}  mean={:>7.1}  (µs)",
        p(50.0),
        p(90.0),
        p(95.0),
        p(99.0),
        p(99.9),
        micros(*lat.last().unwrap()),
        mean,
    );
    // Machine-readable line for archival.
    println!(
        "  JSON {{\"scenario\":\"{name}\",\"requests\":{},\"concurrency\":{concurrency},\
         \"throughput_req_s\":{throughput:.0},\"p50_us\":{:.1},\"p90_us\":{:.1},\
         \"p95_us\":{:.1},\"p99_us\":{:.1},\"p99_9_us\":{:.1},\"max_us\":{:.1},\"mean_us\":{:.1}}}",
        lat.len(),
        p(50.0),
        p(90.0),
        p(95.0),
        p(99.0),
        p(99.9),
        micros(*lat.last().unwrap()),
        mean,
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "e2e load benchmark: local-only, run with --ignored --nocapture"]
async fn e2e_load_latency_report() {
    let total = load_requests();
    let conc = load_concurrency();
    println!(
        "\n# FlowShield e2e load — {total} requests, concurrency {conc}, always-admit (Axum)\n"
    );

    // In-memory baseline (the local floor: no network).
    {
        let limiter = LimiterBuilder::new(always_admit(), MemoryStore::new())
            .build()
            .unwrap();
        let server = common::spawn_axum(limiter).await;
        report("axum+memory", server.addr, total, conc).await;
    }

    // Real Redis (adds the network round-trip), if a server is reachable.
    let url = common::redis_url();
    if common::redis_ready(&url).await {
        let store = RedisStore::builder(&url)
            .max_connections(conc.max(16))
            .build()
            .expect("build redis store");
        let limiter = LimiterBuilder::new(always_admit(), store).build().unwrap();
        let server = common::spawn_axum(limiter).await;
        report("axum+redis", server.addr, total, conc).await;
    } else {
        println!("  (skipped axum+redis: no server — run `docker compose up -d`)");
    }
    println!();
}
