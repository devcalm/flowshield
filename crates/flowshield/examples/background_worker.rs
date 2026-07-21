//! Rate-limiting a background (non-HTTP) workload — UC-05 / FR-004.
//!
//! A worker drains a queue of jobs, each of which calls a downstream that tolerates
//! only a bounded rate (say, a third-party API or a database). FlowShield gates every
//! job: admitted jobs run immediately, and when the limit is hit the worker waits
//! exactly as long as the decision's `retry_after` says before trying again. That
//! check → (run | wait retry_after → retry) loop is the canonical throttled-worker
//! pattern, and it is entirely transport-agnostic — no HTTP involved.
//!
//! Run it:
//!
//! ```text
//! cargo run --example background_worker
//! ```
//!
//! Expected shape of the output: the first ~10 jobs are admitted in a burst, then the
//! worker settles into a steady ~5 jobs/second, printing a short "throttled" wait
//! before each paced job. See `docs/architecture/algorithms.md` for why.

use flowshield::{Key, LimiterBuilder, MemoryStore, TokenBucket};
use std::time::Instant;

#[tokio::main]
async fn main() -> flowshield::Result<()> {
    // The downstream sustains 5 ops/sec, but we allow an initial burst of up to 10 so
    // a small backlog drains promptly before pacing takes over.
    //
    // To get even, un-bursty spacing instead of a burst-then-pace shape, swap in GCRA
    // — the rest of the code is identical:
    //
    //     use flowshield::Gcra;
    //     LimiterBuilder::new(Gcra::new(5.0, 10)?, MemoryStore::new())
    let limiter = LimiterBuilder::new(
        TokenBucket::per_second(10, 5)?, // capacity 10, refill 5/sec
        MemoryStore::new(),
    )
    .build()?;

    // One key per limited identity (downstream, tenant, API key, …). Here: a single
    // outbound integration shared by every job.
    let key = Key::from("outbound-api");

    let start = Instant::now();
    for job in 1..=20 {
        // Wait until the limiter admits this job, honouring the retry-after it reports.
        loop {
            let decision = limiter.check(&key).await?;
            if decision.is_allowed() {
                println!(
                    "[{:>5.2}s] job {job:>2}  ✓ admitted",
                    start.elapsed().as_secs_f64()
                );
                // ... perform the rate-limited work here (the API call, DB write, …) ...
                break;
            }

            // Rejected: a rejection always carries the relative wait until it can
            // succeed. Sleep exactly that long, then retry the same job.
            let wait = decision
                .retry_after()
                .expect("a rejection always carries a retry_after");
            println!(
                "[{:>5.2}s] job {job:>2}  … throttled, waiting {}ms",
                start.elapsed().as_secs_f64(),
                wait.as_millis()
            );
            tokio::time::sleep(wait).await;
        }
    }

    println!(
        "\nprocessed 20 jobs in {:.2}s (bounded to ~5/sec after the burst)",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
