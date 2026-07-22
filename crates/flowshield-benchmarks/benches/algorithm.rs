//! Pure-algorithm microbenchmark (ADR-014: allocations / hot-path cost).
//!
//! This isolates the algorithm transition — `Algorithm::step` — from every storage,
//! locking, and async concern. It is the floor of the decision cost: whatever the
//! limiter adds sits on top of this. Because `step` is pure and its state is `Copy`
//! (ADR-008), the timed section allocates nothing; a regression here means the
//! algorithm math itself got more expensive.

use criterion::{Criterion, criterion_group, criterion_main};
use flowshield::{Algorithm, Gcra, TokenBucket};
use std::hint::black_box;
use std::time::Duration;

fn bench_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("algorithm_step");

    // Token Bucket (the default, FR-012). Ample capacity so most steps admit and we
    // measure the common accept path rather than the reject branch.
    let tb = TokenBucket::per_second(1_000_000, 1_000_000).unwrap();
    let tb_state = tb.initial(Duration::ZERO);
    group.bench_function("token_bucket", |b| {
        // Advance time by a nanosecond each iteration so the refill arithmetic actually
        // runs (a fixed `now` would let the compiler hoist part of it).
        let mut t = Duration::from_nanos(1);
        b.iter(|| {
            let (_state, decision) = tb.step(black_box(tb_state), black_box(t), 1);
            t += Duration::from_nanos(1);
            black_box(decision)
        })
    });

    // GCRA (the alternative, ADR-008).
    let gcra = Gcra::new(1_000_000.0, 1_000_000).unwrap();
    let gcra_state = gcra.initial(Duration::ZERO);
    group.bench_function("gcra", |b| {
        let mut t = Duration::from_nanos(1);
        b.iter(|| {
            let (_state, decision) = gcra.step(black_box(gcra_state), black_box(t), 1);
            t += Duration::from_nanos(1);
            black_box(decision)
        })
    });

    group.finish();
}

criterion_group!(benches, bench_step);
criterion_main!(benches);
