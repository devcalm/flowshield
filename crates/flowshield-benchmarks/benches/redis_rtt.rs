//! Redis/Valkey round-trip benchmark (ADR-014: network-backed store overhead vs. memory).
//!
//! Measures the added cost of a network-backed, server-atomic decision (ADR-006): one
//! EVALSHA round-trip to Redis versus the in-memory path measured by `decision.rs`. This
//! is the number that dominates distributed deployments (ADR-005), so it is measured
//! directly rather than inferred.
//!
//! **Reachability-gated.** Like the integration tests, this runs only against a live
//! server. Point it at one with `FLOWSHIELD_REDIS_URL` (default `redis://127.0.0.1:6379`);
//! if the server is unreachable the benchmark logs a skip and does no work rather than
//! failing — a benchmark cannot manufacture a network. Enable with `--features redis`.
//!
//! ```text
//! FLOWSHIELD_REDIS_URL=redis://127.0.0.1:6379 \
//!   cargo bench -p flowshield-benchmarks --features redis --bench redis_rtt
//! ```

use criterion::{Criterion, criterion_group, criterion_main};
use flowshield::{Key, LimiterBuilder, TokenBucket};
use flowshield_redis::RedisStore;
use std::hint::black_box;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

fn redis_url() -> String {
    std::env::var("FLOWSHIELD_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

/// Cheap std-only TCP probe: parse `redis://host:port` and try to connect. Avoids
/// pulling a client dependency in just to decide whether to run.
fn reachable(url: &str) -> bool {
    let hostport = url.trim_start_matches("redis://").trim_end_matches('/');
    // Strip any auth / path components, keep `host:port`.
    let hostport = hostport.split('/').next().unwrap_or(hostport);
    let hostport = hostport.rsplit('@').next().unwrap_or(hostport);
    let Ok(mut addrs) = hostport.to_socket_addrs() else {
        return false;
    };
    addrs.any(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok())
}

fn bench_redis_rtt(c: &mut Criterion) {
    let url = redis_url();
    if !reachable(&url) {
        eprintln!(
            "redis_rtt: skipping — no server reachable at {url} \
             (set FLOWSHIELD_REDIS_URL to a live server to run this benchmark)"
        );
        return;
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let store = RedisStore::builder(&url)
        .build()
        .expect("build redis store");
    let limiter = LimiterBuilder::new(TokenBucket::per_second(u64::MAX, 1_000_000).unwrap(), store)
        .build()
        .expect("build limiter");
    let key = Key::from("bench-rtt");

    let mut group = c.benchmark_group("redis_rtt");
    // One decision == one server round trip. Fewer samples than the in-memory suite:
    // network latency makes each sample far slower, so a smaller sample size keeps the
    // run bounded while still capturing the distribution.
    group.sample_size(50);
    group.bench_function("token_bucket", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(limiter.check(black_box(&key)).await.unwrap()) })
    });
    group.finish();
}

criterion_group!(benches, bench_redis_rtt);
criterion_main!(benches);
