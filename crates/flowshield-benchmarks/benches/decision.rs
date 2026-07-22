//! End-to-end single-threaded decision benchmark (ADR-014: throughput + latency/tail).
//!
//! Measures the full `Limiter::check` path against the in-memory backend — algorithm
//! transition, shard hashing, lock acquire/release, LRU bookkeeping, and decision
//! construction — for both algorithms and across a cardinality sweep (FR-071, NFR-010).
//! Criterion reports the full latency distribution (p50…p99.9), not just the mean, as
//! ADR-014 requires; throughput is reported per element so the number reads as
//! decisions/second.
//!
//! Uninstrumented on purpose: built with `--no-default-features` on the observability
//! side, this is the near-zero-overhead path (NFR-023). The observability cost is a
//! separate concern measured by feature-on builds.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use flowshield::{Gcra, Key, LimiterBuilder, MemoryStore, TokenBucket};
use flowshield_benchmarks::{CARDINALITIES, block_on_ready, key_pool};
use std::hint::black_box;

/// Round-robin over a key pool, timing one `check` per iteration. The pool and limiter
/// are built once, outside the timed closure, so only the decision is measured.
fn bench_allow_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("decision_allow");

    for &n in CARDINALITIES {
        let keys = key_pool(n);
        // Capacity comfortably above the working set so we measure steady-state hits,
        // not eviction (eviction under pressure is a separate, deliberate scenario).
        let cap = (n * 2).max(1024);

        // --- Token Bucket (default). Huge capacity ⇒ always admits ⇒ accept path. ---
        {
            let store = MemoryStore::with_capacity(cap).unwrap();
            let limiter = LimiterBuilder::new(
                TokenBucket::per_second(i64::MAX as u64, 1_000_000).unwrap(),
                store,
            )
            .build()
            .unwrap();
            let mut i = 0usize;
            group.throughput(Throughput::Elements(1));
            group.bench_with_input(BenchmarkId::new("token_bucket", n), &n, |b, _| {
                b.iter(|| {
                    let key: &Key = &keys[i % keys.len()];
                    i = i.wrapping_add(1);
                    black_box(block_on_ready(limiter.check(black_box(key))).unwrap())
                })
            });
        }

        // --- GCRA. Large burst + high rate ⇒ accept path across the pool. ---
        {
            let store = MemoryStore::with_capacity(cap).unwrap();
            let limiter = LimiterBuilder::new(Gcra::new(1_000_000.0, 1_000_000).unwrap(), store)
                .build()
                .unwrap();
            let mut i = 0usize;
            group.throughput(Throughput::Elements(1));
            group.bench_with_input(BenchmarkId::new("gcra", n), &n, |b, _| {
                b.iter(|| {
                    let key: &Key = &keys[i % keys.len()];
                    i = i.wrapping_add(1);
                    black_box(block_on_ready(limiter.check(black_box(key))).unwrap())
                })
            });
        }
    }
    group.finish();
}

/// The reject branch on a single saturated key: capacity 1, immediately drained, so
/// every subsequent decision is a rejection. Distinct code path (retry-after math) worth
/// measuring separately from the accept path.
fn bench_reject_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("decision_reject");
    let store = MemoryStore::with_capacity(1024).unwrap();
    // Capacity 1, refills 1/sec: after the first admit the bucket stays empty for the
    // whole (sub-second) measurement window, so we exercise the rejection path.
    let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), store)
        .build()
        .unwrap();
    let key = Key::from("saturated");
    // Drain the single token so we are firmly on the reject branch.
    let _ = block_on_ready(limiter.check(&key));

    group.throughput(Throughput::Elements(1));
    group.bench_function("token_bucket", |b| {
        b.iter(|| {
            let d = block_on_ready(limiter.check(black_box(&key))).unwrap();
            debug_assert!(!d.is_allowed());
            black_box(d)
        })
    });
    group.finish();
}

criterion_group!(benches, bench_allow_path, bench_reject_path);
criterion_main!(benches);
