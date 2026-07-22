//! 10k-concurrent scenario (ADR-014 / NFR-004: high concurrent load, no deadlock/panic).
//!
//! Spawns 10,000 concurrent Tokio tasks that share one limiter, each issuing a batch of
//! decisions, then joins them all. The acceptance bar (NFR-004) is that this completes
//! without deadlock or panic; we additionally report aggregate throughput and the
//! allowed/rejected split so the run is a data point, not just a smoke test.
//!
//! Two key regimes are run back to back:
//! - **shared hot key** — all tasks target one key (maximum shard-lock contention);
//! - **per-task key** — each task owns its key (fans out across shards).
//!
//! Machine-readable JSON on stdout. Run with:
//!
//! ```text
//! cargo run -p flowshield-benchmarks --release --bin concurrency-10k
//! ```

use flowshield::{Key, LimiterBuilder, MemoryStore, TokenBucket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const TASKS: usize = 10_000;
const PER_TASK: usize = 100;

struct Outcome {
    name: &'static str,
    tasks: usize,
    decisions: u64,
    allowed: u64,
    elapsed_secs: f64,
    throughput_per_sec: f64,
}

async fn run_regime(name: &'static str, per_task_key: bool) -> Outcome {
    // Always-admitting bucket so the run measures coordination, not the reject branch;
    // the allowed/rejected split is still reported for correctness confidence.
    let limiter = Arc::new(
        LimiterBuilder::new(
            TokenBucket::per_second(i64::MAX as u64, 1_000_000).unwrap(),
            MemoryStore::with_capacity(4 * TASKS).unwrap(),
        )
        .build()
        .unwrap(),
    );
    let hot = Arc::new(Key::from("shared-hot"));
    let allowed = Arc::new(AtomicU64::new(0));

    let start = Instant::now();
    let mut handles = Vec::with_capacity(TASKS);
    for t in 0..TASKS {
        let limiter = Arc::clone(&limiter);
        let allowed = Arc::clone(&allowed);
        let key = if per_task_key {
            Key::from(format!("task-{t}"))
        } else {
            (*hot).clone()
        };
        handles.push(tokio::spawn(async move {
            let mut local_allowed = 0u64;
            for _ in 0..PER_TASK {
                // A real await against the runtime: exercises the Send future and the
                // sharded lock under genuine concurrency.
                if limiter.check(&key).await.unwrap().is_allowed() {
                    local_allowed += 1;
                }
            }
            allowed.fetch_add(local_allowed, Ordering::Relaxed);
        }));
    }

    for h in handles {
        // A panic in any task propagates here and aborts the run — that is the
        // deadlock/panic acceptance gate (NFR-004).
        h.await.expect("task panicked");
    }
    let elapsed = start.elapsed();

    let decisions = (TASKS * PER_TASK) as u64;
    Outcome {
        name,
        tasks: TASKS,
        decisions,
        allowed: allowed.load(Ordering::Relaxed),
        elapsed_secs: elapsed.as_secs_f64(),
        throughput_per_sec: decisions as f64 / elapsed.as_secs_f64(),
    }
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let outcomes = rt.block_on(async {
        vec![
            run_regime("shared_hot_key", false).await,
            run_regime("per_task_key", true).await,
        ]
    });

    print_json(&outcomes);
}

fn print_json(outcomes: &[Outcome]) {
    println!("{{");
    println!("  \"scenario\": \"concurrency_10k\",");
    println!("  \"per_task_decisions\": {PER_TASK},");
    println!("  \"cases\": [");
    for (idx, o) in outcomes.iter().enumerate() {
        let comma = if idx + 1 < outcomes.len() { "," } else { "" };
        println!("    {{");
        println!("      \"name\": \"{}\",", o.name);
        println!("      \"tasks\": {},", o.tasks);
        println!("      \"decisions\": {},", o.decisions);
        println!("      \"allowed\": {},", o.allowed);
        println!("      \"elapsed_secs\": {:.6},", o.elapsed_secs);
        println!("      \"throughput_per_sec\": {:.1}", o.throughput_per_sec);
        println!("    }}{comma}");
    }
    println!("  ]");
    println!("}}");
}
