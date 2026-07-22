//! Allocations-per-decision measurement (ADR-014: allocations on the hot path).
//!
//! A counting global allocator wraps the system allocator and tallies every `alloc`
//! call and byte requested. We warm up (so lazily-initialized state and the first
//! insert of each key are excluded), snapshot the counters, run a fixed number of
//! decisions, and report the per-decision delta. Single-threaded by construction so the
//! counts are exact and deterministic (ADR-014 reproducibility).
//!
//! Output is machine-readable JSON on stdout — one object with a `cases` array — so a
//! run can be archived and diffed. Run with:
//!
//! ```text
//! cargo run -p flowshield-benchmarks --release --bin allocations
//! ```

use flowshield::{Gcra, Key, LimiterBuilder, MemoryStore, TokenBucket};
use flowshield_benchmarks::{block_on_ready, key_pool};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};

/// System allocator that counts allocation calls and bytes. Deallocation is forwarded
/// untouched; we care about allocation *pressure* on the decision path, not net heap.
struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const DECISIONS: u64 = 1_000_000;
const WARMUP: u64 = 100_000;

struct Case {
    name: &'static str,
    decisions: u64,
    allocations: u64,
    bytes: u64,
}

/// Run `f` once per decision for `count` iterations after a warmup, returning the
/// allocation delta measured strictly around the timed section.
fn measure(name: &'static str, warmup: u64, count: u64, mut f: impl FnMut(u64)) -> Case {
    for i in 0..warmup {
        f(i);
    }
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let b0 = BYTES.load(Ordering::Relaxed);
    for i in 0..count {
        f(i);
    }
    let a1 = ALLOCS.load(Ordering::Relaxed);
    let b1 = BYTES.load(Ordering::Relaxed);
    Case {
        name,
        decisions: count,
        allocations: a1 - a0,
        bytes: b1 - b0,
    }
}

fn main() {
    let mut cases = Vec::new();

    // Token Bucket, single hot key.
    {
        let limiter = LimiterBuilder::new(
            TokenBucket::per_second(i64::MAX as u64, 1_000_000).unwrap(),
            MemoryStore::with_capacity(1024).unwrap(),
        )
        .build()
        .unwrap();
        let key = Key::from("hot");
        cases.push(measure("token_bucket/hot_key", WARMUP, DECISIONS, |_| {
            black_box(block_on_ready(limiter.check(black_box(&key))).unwrap());
        }));
    }

    // GCRA, single hot key.
    {
        let limiter = LimiterBuilder::new(
            Gcra::new(1_000_000.0, 1_000_000).unwrap(),
            MemoryStore::with_capacity(1024).unwrap(),
        )
        .build()
        .unwrap();
        let key = Key::from("hot");
        cases.push(measure("gcra/hot_key", WARMUP, DECISIONS, |_| {
            black_box(block_on_ready(limiter.check(black_box(&key))).unwrap());
        }));
    }

    // Token Bucket, high-cardinality round-robin (100k resident keys). Warmup inserts
    // every key first, so we measure the steady-state re-hit path, not first insert.
    {
        let keys = key_pool(100_000);
        let limiter = LimiterBuilder::new(
            TokenBucket::per_second(i64::MAX as u64, 1_000_000).unwrap(),
            MemoryStore::with_capacity(200_000).unwrap(),
        )
        .build()
        .unwrap();
        cases.push(measure(
            "token_bucket/high_cardinality",
            keys.len() as u64, // warmup = one pass to make every key resident
            DECISIONS,
            |i| {
                let key = &keys[(i as usize) % keys.len()];
                black_box(block_on_ready(limiter.check(black_box(key))).unwrap());
            },
        ));
    }

    print_json(&cases);
}

/// Emit the results as a single JSON object (no external serialization dependency).
fn print_json(cases: &[Case]) {
    println!("{{");
    println!("  \"scenario\": \"allocations\",");
    println!("  \"cases\": [");
    for (idx, c) in cases.iter().enumerate() {
        let comma = if idx + 1 < cases.len() { "," } else { "" };
        let per_alloc = c.allocations as f64 / c.decisions as f64;
        let per_bytes = c.bytes as f64 / c.decisions as f64;
        println!("    {{");
        println!("      \"name\": \"{}\",", c.name);
        println!("      \"decisions\": {},", c.decisions);
        println!("      \"allocations\": {},", c.allocations);
        println!("      \"bytes\": {},", c.bytes);
        println!("      \"allocs_per_decision\": {per_alloc:.4},");
        println!("      \"bytes_per_decision\": {per_bytes:.2}");
        println!("    }}{comma}");
    }
    println!("  ]");
    println!("}}");
}
