use crate::algorithm::Algorithm;
use crate::decision::{Allowed, Decision, Rejected};
use crate::error::{Error, Result};
use crate::key::Key;
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
}

pub struct Limiter<A: Algorithm, S, C = MonotonicClock> {
    algorithm: A,
    store: S,
    clock: C,
    ttl: Duration,
    degrade: DegradationPolicy,
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
        let now = self.clock.now();
        let algo = &self.algorithm;
        match self
            .store
            .update(key, now, self.ttl, move |st| {
                let s = st.unwrap_or_else(|| algo.initial(now));
                algo.step(s, now, cost)
            })
            .await
        {
            Ok(decision) => Ok(decision),
            Err(e) => {
                // TODO(Phase 3): emit a degradation metric/log here (ADR-012).
                // Config errors should propagate; only backend/timeout degrade:
                match e {
                    Error::Config(_) => Err(e),
                    _ => Ok(self.degrade.fallback()),
                }
            }
        }
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
        Ok(Limiter {
            algorithm: self.algorithm,
            store: self.store,
            clock: self.clock,
            ttl: self.ttl,
            degrade: self.degrade,
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
}
