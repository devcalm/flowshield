//! Shared-state contention benchmark (ADR-014: contention under concurrency).
//!
//! Many threads hammer one shared limiter. Two regimes bracket real behavior:
//!
//! - **hot key** — every thread hits the *same* key, so every decision serializes on a
//!   single shard `Mutex`. This is the worst case and isolates lock cost (NFR-004).
//! - **spread** — threads fan out across a large key pool, so decisions land on
//!   different shards and proceed in parallel. This shows the sharding payoff (the
//!   in-memory store partitions state across independently-locked shards).
//!
//! Measured with `iter_custom` + scoped threads: each timed unit is a fixed batch of
//! decisions divided evenly across the worker threads, and we report wall-clock
//! throughput (decisions/second aggregate). The memory backend's future is ready
//! immediately, so workers drive it with a no-op waker — no runtime scheduling noise.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use flowshield::{Key, LimiterBuilder, MemoryStore, TokenBucket};
use flowshield_benchmarks::{block_on_ready, key_pool};
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn thread_counts() -> Vec<usize> {
    let max = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    // 1 (baseline, no contention) plus a couple of points up to the machine width.
    let mut v = vec![1usize];
    for t in [2, 4, 8, max] {
        if t > 1 && t <= max && !v.contains(&t) {
            v.push(t);
        }
    }
    v.sort_unstable();
    v
}

fn run_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("contention");
    // A generous, always-admitting bucket so we measure coordination cost, not the
    // algorithm's accept/reject branch.
    let make_limiter = || {
        LimiterBuilder::new(
            TokenBucket::per_second(i64::MAX as u64, 1_000_000).unwrap(),
            MemoryStore::with_capacity(200_000).unwrap(),
        )
        .build()
        .unwrap()
    };

    for &threads in &thread_counts() {
        // --- Hot key: all threads contend on one shard. ---
        {
            let limiter = Arc::new(make_limiter());
            let key = Arc::new(Key::from("hot"));
            group.throughput(Throughput::Elements(1));
            group.bench_with_input(
                BenchmarkId::new("hot_key", threads),
                &threads,
                |b, &threads| {
                    b.iter_custom(|iters| {
                        drive(iters, threads, |_tid, worker_iters| {
                            for _ in 0..worker_iters {
                                black_box(block_on_ready(limiter.check(black_box(&key))).unwrap());
                            }
                        })
                    })
                },
            );
        }

        // --- Spread: threads fan out across many keys / shards. ---
        {
            let limiter = Arc::new(make_limiter());
            let keys = Arc::new(key_pool(100_000));
            group.throughput(Throughput::Elements(1));
            group.bench_with_input(
                BenchmarkId::new("spread", threads),
                &threads,
                |b, &threads| {
                    b.iter_custom(|iters| {
                        drive(iters, threads, |tid, worker_iters| {
                            // Give each thread a disjoint stride into the pool so workers
                            // mostly touch different keys (hence different shards).
                            let base = tid.wrapping_mul(97usize.wrapping_mul(keys.len() / 8 + 1));
                            for j in 0..worker_iters {
                                let key = &keys[(base + j as usize) % keys.len()];
                                black_box(block_on_ready(limiter.check(black_box(key))).unwrap());
                            }
                        })
                    })
                },
            );
        }
    }
    group.finish();
}

/// Split `iters` decisions evenly across `threads` scoped workers, run `body` on each,
/// and return the aggregate wall-clock elapsed. `body` receives that worker's share.
fn drive<F>(iters: u64, threads: usize, body: F) -> Duration
where
    F: Fn(usize, u64) + Sync,
{
    let per = iters / threads as u64;
    let remainder = iters % threads as u64;
    let body = &body;
    let start = Instant::now();
    std::thread::scope(|s| {
        for t in 0..threads {
            // Hand the remainder to the first worker so the total is exactly `iters`.
            let worker_iters = per + if (t as u64) < remainder { 1 } else { 0 };
            s.spawn(move || body(t, worker_iters));
        }
    });
    start.elapsed()
}

criterion_group!(benches, run_contention);
criterion_main!(benches);
