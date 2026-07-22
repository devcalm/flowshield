//! End-to-end HTTP-middleware overhead (ADR-014; Phase 6 depends on Phase 5 for this).
//!
//! Answers the operator's real question: *how much latency does the rate-limit layer add
//! to an HTTP request?* It drives the Tower [`Service`] **in-process** — no TCP socket, no
//! hyper, no server — against the in-memory backend, so the number is FlowShield's own
//! marginal cost, not a measurement of the network stack. (`flowshield-axum` builds on
//! this exact Tower layer, so this covers the Axum path too; Actix is a separate,
//! standalone integration.)
//!
//! Three cases isolate the pieces:
//! - **`inner_only`** — the bare downstream service, the floor to subtract.
//! - **`wrapped_allow`** — the same service behind `FlowShieldLayer` on the admit path:
//!   key extraction + `check` + the boxed service future + the `X-RateLimit-Remaining`
//!   header write.
//! - **`wrapped_reject`** — a saturated limiter, exercising the `429`/`Retry-After`
//!   construction path instead of pass-through.
//!
//! The memory backend's decision future is ready immediately and the layer's future
//! completes in a single poll when its awaits are ready, so — as elsewhere in this suite —
//! it is driven with a no-op waker rather than a runtime, keeping executor noise out.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use flowshield::tower::FlowShieldLayer;
use flowshield::{LimiterBuilder, MemoryStore, TokenBucket};
use flowshield_benchmarks::{CARDINALITIES, block_on_ready};
use http::{Request, Response, StatusCode};
use std::convert::Infallible;
use std::future::{Ready, ready};
use std::hint::black_box;
use std::net::SocketAddr;
use std::task::{Context, Poll};
use tower_layer::Layer;
use tower_service::Service;

/// A trivial always-ready downstream service returning `200 OK` with an empty body. The
/// `Response<()>` body is legal here because the layer only requires `ResBody: Default`,
/// not a real HTTP body — keeping the floor measurement free of body-construction cost.
#[derive(Clone)]
struct Ok200;

impl Service<Request<()>> for Ok200 {
    type Response = Response<()>;
    type Error = Infallible;
    type Future = Ready<Result<Response<()>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Request<()>) -> Self::Future {
        ready(Ok(Response::new(())))
    }
}

/// One request carrying a peer `SocketAddr` in its extensions (what `PeerAddrKeyExtractor`
/// reads). `n` selects the IP, so a sweep of `n` produces a controlled key cardinality.
fn request_for(n: u32) -> Request<()> {
    let octets = n.to_be_bytes();
    let addr = SocketAddr::from(([10, octets[1], octets[2], octets[3]], 0));
    let mut req = Request::new(());
    req.extensions_mut().insert(addr);
    req
}

/// Drive one always-ready `Service` call to completion (no-op waker; see module docs).
fn call<S>(svc: &mut S, req: Request<()>) -> Response<()>
where
    S: Service<Request<()>, Response = Response<()>>,
    S::Error: std::fmt::Debug,
{
    block_on_ready(svc.call(req)).expect("service call failed")
}

fn bench_middleware(c: &mut Criterion) {
    // --- Floor: the downstream service with no limiter in front. ---
    {
        let mut group = c.benchmark_group("http_inner_only");
        group.throughput(Throughput::Elements(1));
        let mut svc = Ok200;
        group.bench_function("baseline", |b| {
            let mut i = 0u32;
            b.iter(|| {
                i = i.wrapping_add(1);
                black_box(call(&mut svc, black_box(request_for(i))))
            })
        });
        group.finish();
    }

    // --- Admit path: wrapped service across a peer-IP cardinality sweep. ---
    {
        let mut group = c.benchmark_group("http_wrapped_allow");
        for &n in CARDINALITIES {
            let store = MemoryStore::with_capacity((n * 2).max(1024)).unwrap();
            // Ample capacity ⇒ always admits ⇒ measures the pass-through path.
            let limiter =
                LimiterBuilder::new(TokenBucket::per_second(u64::MAX, 1_000_000).unwrap(), store)
                    .build()
                    .unwrap();
            let mut svc = FlowShieldLayer::per_peer(limiter).layer(Ok200);

            group.throughput(Throughput::Elements(1));
            group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
                let mut i = 0u32;
                b.iter(|| {
                    // Cycle the low bits so we spread across `n` distinct peer IPs.
                    i = i.wrapping_add(1);
                    let sel = if n <= 1 { 0 } else { i % n as u32 };
                    let resp = call(&mut svc, black_box(request_for(sel)));
                    debug_assert_eq!(resp.status(), StatusCode::OK);
                    black_box(resp)
                })
            });
        }
        group.finish();
    }

    // --- Reject path: a saturated single-key limiter builds a 429 every call. ---
    {
        let mut group = c.benchmark_group("http_wrapped_reject");
        let store = MemoryStore::with_capacity(1024).unwrap();
        let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), store)
            .build()
            .unwrap();
        let mut svc = FlowShieldLayer::per_peer(limiter).layer(Ok200);
        // Same peer every call; drain its one token so we stay on the reject branch.
        let _ = call(&mut svc, request_for(7));

        group.throughput(Throughput::Elements(1));
        group.bench_function("token_bucket", |b| {
            b.iter(|| {
                let resp = call(&mut svc, black_box(request_for(7)));
                debug_assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
                black_box(resp)
            })
        });
        group.finish();
    }
}

criterion_group!(benches, bench_middleware);
criterion_main!(benches);
