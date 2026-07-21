use crate::algorithm::Algorithm;
use crate::decision::{Allowed, Decision, Rejected};
use crate::error::{Error, Result};
use crate::key::Key;
use crate::observe::{self, Labels};
use crate::store::StorageBackend;
use crate::time::{MonotonicClock, TimeSource};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub enum DegradationPolicy {
    #[default]
    FailClosed, // safe default (ADR-007)
    FailOpen, // explicit opt-in
              // FailToLocal { .. }    // reserved; #[non_exhaustive] lets you add it without breaking
}

impl DegradationPolicy {
    fn fallback(self) -> Decision {
        match self {
            DegradationPolicy::FailOpen  => Decision::Allowed(Allowed { remaining: None }),
            _ /* FailClosed & future */  => Decision::Rejected(Rejected {
                retry_after: Duration::from_secs(1),  // conservative
            }),
        }
    }

    /// A static, low-cardinality name for the `policy` telemetry label (ADR-012).
    /// Only consulted by feature-gated emission, hence unused with observability off.
    #[cfg_attr(not(any(feature = "metrics", feature = "tracing")), allow(dead_code))]
    pub(crate) fn telemetry_name(self) -> &'static str {
        match self {
            DegradationPolicy::FailOpen => "fail_open",
            _ /* FailClosed & future */ => "fail_closed",
        }
    }
}

pub struct Limiter<A: Algorithm, S, C = MonotonicClock> {
    algorithm: A,
    store: S,
    clock: C,
    ttl: Duration,
    degrade: DegradationPolicy,
    /// Static telemetry labels (algorithm name + backend kind), resolved once at
    /// build time (ADR-012). Copied onto each emitted signal; carries no per-key or
    /// otherwise high-cardinality data.
    labels: Labels,
}

impl<A, S, C> Limiter<A, S, C>
where
    A: Algorithm,
    S: StorageBackend<A::State>,
    C: TimeSource,
{
    pub async fn check(&self, key: &Key) -> Result<Decision> {
        self.check_n(key, 1).await
    }

    pub async fn check_n(&self, key: &Key, cost: u64) -> Result<Decision> {
        // Wrap the whole evaluation in a decision span so that the backend await and
        // every event emitted below are attributed to it — the natural unit an
        // OpenTelemetry exporter correlates with surrounding request spans (ADR-012,
        // FR-052). `.instrument` (rather than a held guard) is the async-correct way
        // to span across an await. When `tracing` is off the wrap disappears and the
        // inner future is awaited directly, at no cost.
        let fut = self.evaluate(key, cost);
        #[cfg(feature = "tracing")]
        let fut = {
            use tracing::Instrument as _;
            fut.instrument(self.decision_span(key))
        };
        fut.await
    }

    /// The actual decision, uninstrumented at the boundary but emitting metrics and
    /// events from within (so they land inside the [`check_n`](Self::check_n) span).
    async fn evaluate(&self, key: &Key, cost: u64) -> Result<Decision> {
        let now = self.clock.now();
        let algo = &self.algorithm;

        // Time the backend round-trip (compiled out unless `metrics` is on — the
        // `Timer` is a zero-sized no-op otherwise, so no clock is read).
        let timer = observe::Timer::start();
        let result = self
            .store
            .update(key, now, self.ttl, move |st| {
                let s = st.unwrap_or_else(|| algo.initial(now));
                algo.step(s, now, cost)
            })
            .await;
        observe::record_backend_latency(self.labels, timer);

        match result {
            Ok(decision) => {
                observe::record_decision(self.labels, &decision);
                Ok(decision)
            }
            Err(e) => match e {
                // A configuration error is a programming/setup bug: surface it, never
                // silently degrade (ADR-011). Only backend/timeout failures fall back.
                Error::Config(_) => Err(e),
                _ => {
                    let fallback = self.degrade.fallback();
                    observe::record_degradation(self.labels, &e, self.degrade);
                    Ok(fallback)
                }
            },
        }
    }

    /// Build the per-decision `tracing` span. The key appears only in **redacted**
    /// form (ADR-012, FR-073) — never the raw value, which may be an IP or token.
    #[cfg(feature = "tracing")]
    fn decision_span(&self, key: &Key) -> tracing::Span {
        tracing::trace_span!(
            "flowshield.check",
            algorithm = self.labels.algorithm,
            backend = self.labels.backend,
            key = %observe::redact(key),
        )
    }
}

pub struct LimiterBuilder<A, S, C> {
    algorithm: A,
    store: S,
    clock: C,
    ttl: Duration,
    degrade: DegradationPolicy,
}

impl<A: Algorithm, S: StorageBackend<A::State>> LimiterBuilder<A, S, MonotonicClock> {
    pub fn new(algorithm: A, store: S) -> Self {
        Self {
            algorithm,
            store,
            clock: MonotonicClock::default(),
            ttl: Duration::from_secs(60),
            degrade: DegradationPolicy::FailClosed,
        }
    }
}
impl<A: Algorithm, S: StorageBackend<A::State>, C: TimeSource> LimiterBuilder<A, S, C> {
    pub fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }
    pub fn degradation(mut self, p: DegradationPolicy) -> Self {
        self.degrade = p;
        self
    }
    pub fn clock<C2: TimeSource>(self, clock: C2) -> LimiterBuilder<A, S, C2> {
        LimiterBuilder {
            algorithm: self.algorithm,
            store: self.store,
            clock,
            ttl: self.ttl,
            degrade: self.degrade,
        }
    }

    pub fn build(self) -> Result<Limiter<A, S, C>> {
        if self.ttl.is_zero() {
            return Err(Error::Config("ttl must be greater than zero".into()));
        }
        // Resolve the static telemetry labels once, here, rather than on every
        // decision (ADR-012). Both are `&'static str` from provided trait methods.
        let labels = Labels {
            algorithm: self.algorithm.name(),
            backend: self.store.kind(),
        };
        Ok(Limiter {
            algorithm: self.algorithm,
            store: self.store,
            clock: self.clock,
            ttl: self.ttl,
            degrade: self.degrade,
            labels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithm::token_bucket::TokenBucket;
    use crate::memory::MemoryStore;
    use crate::time::TestClock;
    use std::future::Future;
    use std::sync::Arc;

    // Our futures are immediately ready; a busy-poll with the no-op waker is a
    // sufficient executor and avoids an async-runtime dependency for tests.
    fn block_on<F: Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll};
        let mut cx = Context::from_waker(std::task::Waker::noop());
        let mut fut = std::pin::pin!(fut);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn admits_up_to_capacity_then_refills() {
        let clock = Arc::new(TestClock::default());
        let limiter =
            LimiterBuilder::new(TokenBucket::per_second(2, 1).unwrap(), MemoryStore::new())
                .clock(clock.clone())
                .build()
                .unwrap();
        let key = Key::from("user-1");

        assert!(block_on(limiter.check(&key)).unwrap().is_allowed()); // 2 -> 1
        assert!(block_on(limiter.check(&key)).unwrap().is_allowed()); // 1 -> 0
        assert!(!block_on(limiter.check(&key)).unwrap().is_allowed()); // reject

        clock.advance(Duration::from_secs(1)); // refill one token
        assert!(block_on(limiter.check(&key)).unwrap().is_allowed());
    }

    #[test]
    fn build_rejects_zero_ttl() {
        let result =
            LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), MemoryStore::new())
                .ttl(Duration::ZERO)
                .build();
        assert!(matches!(result, Err(Error::Config(_))));
    }

    // A backend that always fails, to exercise the degradation path independently of
    // any real store. Generic over the state type so it slots under any algorithm.
    struct FailingStore;
    impl<S: Send + 'static> StorageBackend<S> for FailingStore {
        fn kind(&self) -> &'static str {
            "failing"
        }
        fn update(
            &self,
            _key: &Key,
            _now: Duration,
            _ttl: Duration,
            _step: impl FnOnce(Option<S>) -> (S, Decision) + Send,
        ) -> impl Future<Output = Result<Decision>> + Send {
            std::future::ready(Err(Error::Timeout {
                elapsed: Duration::from_millis(5),
            }))
        }
    }

    #[test]
    fn backend_failure_degrades_fail_closed_by_default() {
        let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), FailingStore)
            .build()
            .unwrap();
        // Default policy rejects when the backend is unreachable (ADR-007).
        let d = block_on(limiter.check(&Key::from("k"))).unwrap();
        assert!(
            !d.is_allowed(),
            "fail-closed must reject on backend failure"
        );
    }

    #[test]
    fn backend_failure_can_fail_open_when_opted_in() {
        let limiter = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), FailingStore)
            .degradation(DegradationPolicy::FailOpen)
            .build()
            .unwrap();
        let d = block_on(limiter.check(&Key::from("k"))).unwrap();
        assert!(d.is_allowed(), "fail-open must admit on backend failure");
    }

    // --- Observability (Phase 3, ADR-012) ------------------------------------

    #[cfg(feature = "metrics")]
    #[test]
    fn metrics_count_decisions_and_degradations() {
        use metrics_util::debugging::{DebugValue, DebuggingRecorder};

        // Find the summed counter value for `name` filtered to a given label pair.
        fn counter_with(
            snapshot: &[(
                metrics_util::CompositeKey,
                Option<metrics::Unit>,
                Option<metrics::SharedString>,
                DebugValue,
            )],
            name: &str,
            want_label: (&str, &str),
        ) -> u64 {
            snapshot
                .iter()
                .filter(|(ck, _, _, _)| ck.key().name() == name)
                .filter(|(ck, _, _, _)| {
                    ck.key()
                        .labels()
                        .any(|l| l.key() == want_label.0 && l.value() == want_label.1)
                })
                .filter_map(|(_, _, _, v)| match v {
                    DebugValue::Counter(n) => Some(*n),
                    _ => None,
                })
                .sum()
        }

        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            let clock = Arc::new(TestClock::default());
            let limiter =
                LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), MemoryStore::new())
                    .clock(clock)
                    .build()
                    .unwrap();
            let key = Key::from("client");

            block_on(limiter.check(&key)).unwrap(); // allowed
            block_on(limiter.check(&key)).unwrap(); // rejected (bucket drained)

            // A failing backend, same recorder, to land a degradation counter.
            let degrading =
                LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), FailingStore)
                    .build()
                    .unwrap();
            block_on(degrading.check(&Key::from("x"))).unwrap();
        });

        let snap = snapshotter.snapshot().into_vec();

        assert_eq!(
            counter_with(&snap, "flowshield_decisions_total", ("outcome", "allowed")),
            1,
            "one allowed decision expected"
        );
        assert_eq!(
            counter_with(&snap, "flowshield_decisions_total", ("outcome", "rejected")),
            1,
            "one rejected decision expected"
        );
        assert_eq!(
            counter_with(
                &snap,
                "flowshield_degradations_total",
                ("policy", "fail_closed")
            ),
            1,
            "one fail-closed degradation expected"
        );

        // The algorithm label must be present and correct on decisions.
        assert_eq!(
            counter_with(
                &snap,
                "flowshield_decisions_total",
                ("algorithm", "token_bucket")
            ),
            2,
            "both decisions carry the algorithm label"
        );

        // A backend-latency histogram must have been recorded.
        let has_latency = snap.iter().any(|(ck, _, _, v)| {
            ck.key().name() == "flowshield_backend_duration_seconds"
                && matches!(v, DebugValue::Histogram(samples) if !samples.is_empty())
        });
        assert!(has_latency, "backend latency histogram must record samples");
    }

    #[cfg(feature = "tracing")]
    #[test]
    fn tracing_span_redacts_the_key_and_carries_labels() {
        use std::io::Write;
        use std::sync::Mutex;
        use tracing_subscriber::fmt::format::FmtSpan;

        // A MakeWriter that appends everything into a shared buffer we can assert on.
        #[derive(Clone)]
        struct BufWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufWriter {
            type Writer = BufWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let raw_key = "192.168.0.42:secret-api-token";
        let buf = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_span_events(FmtSpan::NEW) // log span fields at creation
            .with_ansi(false)
            .with_writer(BufWriter(buf.clone()))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let limiter =
                LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), MemoryStore::new())
                    .build()
                    .unwrap();
            block_on(limiter.check(&Key::from(raw_key))).unwrap();
        });

        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();

        assert!(
            !out.contains("secret-api-token"),
            "raw key material must never appear in telemetry; got:\n{out}"
        );
        assert!(
            out.contains("k#"),
            "the redacted key digest should be present; got:\n{out}"
        );
        assert!(
            out.contains("token_bucket") && out.contains("memory"),
            "the span must carry algorithm and backend labels; got:\n{out}"
        );
    }
}
