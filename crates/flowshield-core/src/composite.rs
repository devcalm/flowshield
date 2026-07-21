//! Composite / layered limits by explicit composition (ADR-004 #3, FR-013).
//!
//! Some requests must satisfy several limits at once — a per-second *and* a per-day
//! quota for the same client, or a global *and* a per-tenant *and* a per-user cap.
//! FlowShield does not express this with a rule DSL; you compose independent
//! [`Limiter`]s and admit a request only when **every** one admits it.
//!
//! Doing that correctly has one sharp edge: the combined `Retry-After`. When two
//! limits reject, the caller must wait for the *longer* of the two — the shorter
//! wait would let the request retry into a still-closed limit. This module exists so
//! that computation lives in exactly one place instead of being re-derived (and
//! occasionally mis-derived) in every application.
//!
//! Two pieces, from thin to thinner:
//!
//! - [`combine`] — a pure function over already-collected, labelled decisions. It
//!   applies the rule "allowed iff all allowed; otherwise reject with the maximum
//!   retry-after, naming the binding limit." Use it directly when your layers use
//!   *different keys* (global/tenant/user), which you gather yourself.
//! - [`LimiterSet`] — a small convenience that owns a set of limiters, checks them
//!   all against one key, and returns the [`combine`]d result. Ideal for layered
//!   quotas on the *same* key (e.g. `10/s` and `1000/day` for one client).
//!
//! Both are optional. A single [`Limiter`] is complete on its own; nothing here is
//! on the single-limiter hot path (FR-013 AC-3).
//!
//! # Consumption semantics
//!
//! A [`LimiterSet`] **evaluates every member** — it does not stop at the first
//! rejection, because finding the maximum retry-after requires every member's
//! answer. A member that admits therefore *consumes* from its limiter even when
//! another member rejects the request overall. v1 has no reserve-then-commit
//! (that awaits the future "peek"/reservation capability, FR-014); if the extra
//! consumption matters, order cheap or likely-to-reject limits first and prefer
//! [`combine`] with hand-gathered decisions when you need finer control.

use crate::decision::Decision;
use crate::error::Result;
use crate::key::Key;
use crate::limiter::Limiter;
use crate::store::StorageBackend;
use crate::{algorithm::Algorithm, time::TimeSource};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// The outcome of evaluating a composite of limits.
///
/// [`decision`](Self::decision) is [`Allowed`] only when every member allowed; on
/// rejection it carries the maximum retry-after across the members, and
/// [`binding`](Self::binding) names the limit that produced it.
///
/// [`Allowed`]: crate::decision::Allowed
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Combined {
    /// The combined decision: allowed iff all members allowed, else the rejection
    /// with the largest retry-after.
    pub decision: Decision,
    /// On rejection, the label of the **binding** limit (the one with the maximum
    /// retry-after — the wait the caller must actually observe). `None` when the
    /// request was allowed.
    pub binding: Option<&'static str>,
}

impl Combined {
    /// Whether the composite admitted the request (all members allowed).
    pub fn is_allowed(&self) -> bool {
        self.decision.is_allowed()
    }

    /// The relative wait before retrying, if rejected (the maximum across members).
    pub fn retry_after(&self) -> Option<Duration> {
        self.decision.retry_after()
    }

    /// The label of the binding limit, if rejected.
    pub fn binding(&self) -> Option<&'static str> {
        self.binding
    }

    /// Consume `self`, yielding the plain [`Decision`] (e.g. to feed a middleware
    /// that only needs allow/reject + retry-after).
    pub fn into_decision(self) -> Decision {
        self.decision
    }
}

/// Combine labelled per-limit decisions into one [`Combined`] result.
///
/// The rule (FR-013 AC-2):
///
/// - **All allowed →** allowed. The reported `remaining` is the *minimum* across
///   members (the tightest headroom), or `None` if any member could not report it.
/// - **Any rejected →** rejected, with the **maximum** retry-after across the
///   rejecting members, and `binding` set to that member's label. On ties the first
///   member with the maximum wins, so ordering is stable.
/// - **Empty →** allowed with `remaining: None` (vacuously, nothing forbids it).
///
/// This is pure and synchronous; gather the decisions however you like (different
/// keys, concurrent checks, cached results) and pass them in.
///
/// ```
/// use flowshield_core::composite::combine;
/// use flowshield_core::decision::Decision;
/// use std::time::Duration;
///
/// let combined = combine([
///     ("per-second", Decision::allowed(Some(4))),
///     ("per-day", Decision::rejected(Duration::from_secs(3600))),
/// ]);
/// assert!(!combined.is_allowed());
/// assert_eq!(combined.binding(), Some("per-day"));
/// assert_eq!(combined.retry_after(), Some(Duration::from_secs(3600)));
/// ```
pub fn combine<I>(outcomes: I) -> Combined
where
    I: IntoIterator<Item = (&'static str, Decision)>,
{
    let mut worst_retry: Option<Duration> = None;
    let mut binding: Option<&'static str> = None;
    let mut min_remaining: Option<u64> = None;
    let mut saw_unknown_remaining = false;

    for (label, decision) in outcomes {
        match decision {
            Decision::Allowed(allowed) => match allowed.remaining {
                Some(r) => min_remaining = Some(min_remaining.map_or(r, |m| m.min(r))),
                None => saw_unknown_remaining = true,
            },
            Decision::Rejected(rejected) => {
                // Strictly greater → the first member holding the maximum keeps the
                // "binding" title, making the result independent of equal ties.
                if worst_retry.is_none_or(|w| rejected.retry_after > w) {
                    worst_retry = Some(rejected.retry_after);
                    binding = Some(label);
                }
            }
        }
    }

    match worst_retry {
        Some(retry_after) => Combined {
            decision: Decision::rejected(retry_after),
            binding,
        },
        None => {
            let remaining = if saw_unknown_remaining {
                None
            } else {
                min_remaining
            };
            Combined {
                decision: Decision::allowed(remaining),
                binding: None,
            }
        }
    }
}

/// A type-erased, checkable limit — the member type of a [`LimiterSet`].
///
/// Layered limits usually mix algorithms, stores, and clocks (a global token bucket,
/// a per-tenant GCRA, …), so their concrete [`Limiter`] types differ and cannot sit
/// in one homogeneous collection. This object-safe trait erases those type
/// parameters behind a boxed future. Every [`Limiter`] implements it via a blanket
/// impl, so you rarely name it; implement it yourself only to place a custom limit
/// source into a set.
pub trait DynLimiter: Send + Sync {
    /// Check `key` against this limit, boxing the resulting future so the trait
    /// stays object-safe.
    fn check_dyn<'a>(
        &'a self,
        key: &'a Key,
    ) -> Pin<Box<dyn Future<Output = Result<Decision>> + Send + 'a>>;
}

impl<A, S, C> DynLimiter for Limiter<A, S, C>
where
    A: Algorithm,
    S: StorageBackend<A::State>,
    C: TimeSource,
{
    fn check_dyn<'a>(
        &'a self,
        key: &'a Key,
    ) -> Pin<Box<dyn Future<Output = Result<Decision>> + Send + 'a>> {
        Box::pin(self.check(key))
    }
}

/// An optional, thin set of limits evaluated together against a single key.
///
/// Admits a request only when every member admits it; on rejection reports the
/// binding limit and the maximum retry-after (via [`combine`]). Each limiter is
/// added with a `&'static str` label used to identify it in the result.
///
/// A runnable end-to-end example (with a concrete backend) lives in the `flowshield`
/// facade docs; core exposes no public storage backend, so the sketch below is
/// illustrative:
///
/// ```ignore
/// let limits = LimiterSet::new()
///     .with("per-second", per_second_limiter)
///     .with("per-day", per_day_limiter);
///
/// let outcome = limits.check(&key).await?;
/// if !outcome.is_allowed() {
///     // outcome.binding()      -> Some("per-second") | Some("per-day")
///     // outcome.retry_after()  -> the longer of the two waits
/// }
/// ```
pub struct LimiterSet {
    members: Vec<(&'static str, Box<dyn DynLimiter>)>,
}

impl LimiterSet {
    /// An empty set. An empty set admits everything (see [`combine`]); add members
    /// with [`with`](Self::with).
    pub fn new() -> Self {
        Self {
            members: Vec::new(),
        }
    }

    /// Add a labelled limit. Takes any [`DynLimiter`] — every [`Limiter`] qualifies.
    /// Consumes and returns `self` for chaining.
    pub fn with(mut self, label: &'static str, limiter: impl DynLimiter + 'static) -> Self {
        self.members.push((label, Box::new(limiter)));
        self
    }

    /// Number of member limits.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the set has no members (and so admits everything).
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Check `key` against every member and return the combined result. Evaluates
    /// **all** members (see the module-level note on consumption semantics), then
    /// applies [`combine`].
    ///
    /// # Errors
    ///
    /// Propagates the first member error. Note a [`Limiter`] applies its own
    /// degradation policy internally, so a *backend* failure surfaces as a decision,
    /// not an error here; in practice this only carries a configuration error.
    pub async fn check(&self, key: &Key) -> Result<Combined> {
        let mut outcomes = Vec::with_capacity(self.members.len());
        for (label, limiter) in &self.members {
            outcomes.push((*label, limiter.check_dyn(key).await?));
        }
        Ok(combine(outcomes))
    }
}

impl Default for LimiterSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithm::token_bucket::TokenBucket;
    use crate::decision::Decision;
    use crate::limiter::LimiterBuilder;
    use crate::memory::MemoryStore;
    use crate::time::TestClock;
    use std::sync::Arc;

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

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    // --- pure combiner ---

    #[test]
    fn empty_admits() {
        let c = combine(std::iter::empty());
        assert!(c.is_allowed());
        assert_eq!(c.binding(), None);
    }

    #[test]
    fn all_allowed_reports_min_remaining() {
        let c = combine([
            ("a", Decision::allowed(Some(9))),
            ("b", Decision::allowed(Some(3))),
            ("c", Decision::allowed(Some(7))),
        ]);
        assert!(c.is_allowed());
        assert_eq!(
            c.decision,
            Decision::allowed(Some(3)),
            "tightest headroom wins"
        );
    }

    #[test]
    fn unknown_remaining_makes_combined_remaining_unknown() {
        let c = combine([
            ("a", Decision::allowed(Some(5))),
            ("b", Decision::allowed(None)),
        ]);
        assert_eq!(c.decision, Decision::allowed(None));
    }

    #[test]
    fn rejection_uses_maximum_retry_and_names_binding() {
        let c = combine([
            ("per-second", Decision::rejected(secs(1))),
            ("per-day", Decision::rejected(secs(3600))),
        ]);
        assert!(!c.is_allowed());
        assert_eq!(
            c.retry_after(),
            Some(secs(3600)),
            "must wait for the longer limit"
        );
        assert_eq!(c.binding(), Some("per-day"));
    }

    #[test]
    fn one_rejection_among_allows_still_rejects() {
        let c = combine([
            ("a", Decision::allowed(Some(4))),
            ("b", Decision::rejected(secs(2))),
            ("c", Decision::allowed(Some(1))),
        ]);
        assert!(!c.is_allowed());
        assert_eq!(c.retry_after(), Some(secs(2)));
        assert_eq!(c.binding(), Some("b"));
    }

    #[test]
    fn ties_keep_the_first_binding() {
        let c = combine([
            ("first", Decision::rejected(secs(5))),
            ("second", Decision::rejected(secs(5))),
        ]);
        assert_eq!(c.binding(), Some("first"), "stable on ties");
    }

    // --- LimiterSet end-to-end ---

    #[test]
    fn set_admits_only_when_all_admit() {
        // Same key, two layers: a tight 2-burst and a looser 5-burst. Frozen time so
        // neither refills — the tight one binds first.
        let clock = Arc::new(TestClock::default());
        let tight = LimiterBuilder::new(TokenBucket::per_second(2, 1).unwrap(), MemoryStore::new())
            .clock(clock.clone())
            .build()
            .unwrap();
        let loose = LimiterBuilder::new(TokenBucket::per_second(5, 1).unwrap(), MemoryStore::new())
            .clock(clock.clone())
            .build()
            .unwrap();
        let set = LimiterSet::new().with("tight", tight).with("loose", loose);
        let key = Key::from("client");

        // First two: both layers admit.
        assert!(block_on(set.check(&key)).unwrap().is_allowed());
        assert!(block_on(set.check(&key)).unwrap().is_allowed());

        // Third: the tight layer is drained → composite rejects and names it.
        let c = block_on(set.check(&key)).unwrap();
        assert!(!c.is_allowed());
        assert_eq!(c.binding(), Some("tight"));
        assert_eq!(
            c.retry_after(),
            Some(secs(1)),
            "the tight layer's 1/sec refill"
        );
    }

    #[test]
    fn set_reports_the_longer_wait_when_both_reject() {
        // Two drained layers with different refill intervals → the slower one binds.
        let clock = Arc::new(TestClock::default());
        // 1 token, refilling 1/sec vs 1 token refilling 1 per 4s.
        let fast = LimiterBuilder::new(TokenBucket::per_second(1, 1).unwrap(), MemoryStore::new())
            .clock(clock.clone())
            .build()
            .unwrap();
        let slow = LimiterBuilder::new(TokenBucket::new(1, secs(4)).unwrap(), MemoryStore::new())
            .clock(clock.clone())
            .build()
            .unwrap();
        let set = LimiterSet::new().with("fast", fast).with("slow", slow);
        let key = Key::from("client");

        assert!(block_on(set.check(&key)).unwrap().is_allowed()); // drain both
        let c = block_on(set.check(&key)).unwrap();
        assert!(!c.is_allowed());
        assert_eq!(c.binding(), Some("slow"));
        assert_eq!(c.retry_after(), Some(secs(4)));
    }

    #[test]
    fn empty_set_admits() {
        let set = LimiterSet::new();
        assert!(set.is_empty());
        assert!(block_on(set.check(&Key::from("k"))).unwrap().is_allowed());
    }
}
