//! Token Bucket — the default algorithm (ADR-008), with exact integer accounting.
//!
//! The bucket holds up to `capacity` tokens and accrues **one token per
//! `refill_interval`**. Each admitted request of cost `n` removes `n` tokens; a
//! request that cannot be paid in full is rejected together with the time until
//! it can. A fresh or long-idle key starts full, permitting a burst of up to
//! `capacity` before the sustained one-per-interval rate governs.
//!
//! # Precision
//!
//! State is three integers — whole `tokens`, a sub-token `carry` in nanoseconds,
//! and the last-update instant — so refill is exact integer arithmetic with no
//! floating-point drift. The fractional progress toward the next token is
//! preserved across calls in `carry`, which makes decisions bit-reproducible
//! across runs and backends (ADR-005/017). The only rounding is a one-time,
//! sub-nanosecond quantisation of the refill interval at construction.
//!
//! # Documented properties (ADR-008)
//!
//! - **Burst behaviour:** allows bursts up to `capacity`, then the average
//!   one-per-`refill_interval` rate.
//! - **Smoothing:** moderate — bursty while tokens remain, smooth once drained.
//! - **Memory / state per key:** low — two integers plus a timestamp.
//! - **Distributed suitability:** strong — an atomic update of small state.

use crate::algorithm::{Algorithm, nanos_to_duration};
use crate::decision::{Allowed, Decision, Rejected};
use crate::error::{Error, Result};
use crate::resp::{RespProgram, RespScripted};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: u64,
    /// Nanoseconds required to accrue one token. Always `1..=u64::MAX`.
    interval_nanos: u128,
}

impl TokenBucket {
    /// Create a bucket of `capacity` tokens that accrues one token per
    /// `refill_interval`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if `capacity` is zero, if `refill_interval` is
    /// zero, or if `refill_interval` exceeds ~584 years (its nanosecond count
    /// must fit in `u64`). Invalid configuration is a typed error, never a panic
    /// (ADR-011, FR-072).
    pub fn new(capacity: u64, refill_interval: Duration) -> Result<Self> {
        if capacity == 0 {
            return Err(Error::Config("capacity must be greater than zero".into()));
        }
        // The RESP backends marshal `capacity` as a Redis integer (`i64`); a value above
        // `i64::MAX` would wrap negative server-side and reject every request while the
        // in-memory backend admits — a silent cross-backend divergence. Reject it here so
        // both backends agree and the misconfiguration fails fast (FR-072).
        if capacity > i64::MAX as u64 {
            return Err(Error::Config("capacity must not exceed i64::MAX".into()));
        }
        let interval_nanos = refill_interval.as_nanos();
        if interval_nanos == 0 {
            return Err(Error::Config(
                "refill_interval must be greater than zero".into(),
            ));
        }
        if interval_nanos > u64::MAX as u128 {
            return Err(Error::Config(
                "refill_interval must be under ~584 years".into(),
            ));
        }
        Ok(Self {
            capacity,
            interval_nanos,
        })
    }

    /// Convenience constructor: `capacity` tokens, refilling `tokens_per_sec`
    /// tokens per second. The interval is quantised to whole nanoseconds once,
    /// here — a sub-nanosecond, non-accumulating rounding.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if `capacity` or `tokens_per_sec` is zero, or if
    /// `tokens_per_sec` exceeds one billion (which would round the interval down
    /// to zero).
    ///
    /// ```
    /// use flowshield_core::algorithm::token_bucket::TokenBucket;
    ///
    /// // 100 requests/second sustained, tolerating a burst of up to 20.
    /// let tb = TokenBucket::per_second(20, 100)?;
    /// # Ok::<(), flowshield_core::error::Error>(())
    /// ```
    pub fn per_second(capacity: u64, tokens_per_sec: u64) -> Result<Self> {
        if tokens_per_sec == 0 {
            return Err(Error::Config(
                "tokens_per_sec must be greater than zero".into(),
            ));
        }
        let interval_nanos = 1_000_000_000u128 / tokens_per_sec as u128;
        if interval_nanos == 0 {
            return Err(Error::Config(
                "tokens_per_sec must not exceed 1_000_000_000".into(),
            ));
        }
        // Delegate to `new` for the capacity check and construction; the interval
        // is well under the u64 bound, so `new` will not reject it.
        Self::new(capacity, nanos_to_duration(interval_nanos))
    }
}

impl Algorithm for TokenBucket {
    type State = TokenBucketState;

    fn name(&self) -> &'static str {
        "token_bucket"
    }

    fn min_ttl(&self) -> Duration {
        // Time for a fully-drained bucket to refill to capacity: `capacity · interval`.
        // Below this, an idle drained key that expires would reset to full and bypass the
        // limit. `saturating_mul` (capacity ≤ i64::MAX, interval_nanos ≤ u64::MAX) and
        // `nanos_to_duration` (saturates seconds) keep this panic-free.
        nanos_to_duration((self.capacity as u128).saturating_mul(self.interval_nanos))
    }

    fn initial(&self, now: Duration) -> Self::State {
        // A fresh key starts full, permitting an immediate burst to capacity.
        TokenBucketState {
            tokens: self.capacity,
            carry_nanos: 0,
            last_update: now,
        }
    }

    fn step(&self, state: Self::State, now: Duration, cost: u64) -> (Self::State, Decision) {
        // --- Refill: turn elapsed time into whole tokens, exactly. ---
        // Monotonic time should never go backwards; clamp defensively so a
        // reordered call can never *remove* tokens.
        let elapsed = now.checked_sub(state.last_update).unwrap_or(Duration::ZERO);
        // Nanoseconds available to spend on new tokens: carried remainder plus
        // the newly elapsed time.
        let accrued = state.carry_nanos as u128 + elapsed.as_nanos();
        let gained = accrued / self.interval_nanos;
        let remainder = (accrued % self.interval_nanos) as u64;

        let uncapped = state.tokens as u128 + gained;
        let (mut tokens, carry_nanos) = if uncapped >= self.capacity as u128 {
            // Refilled to (or beyond) full: the overflow — and any fractional
            // progress toward a further token — spills and is discarded.
            (self.capacity, 0)
        } else {
            (uncapped as u64, remainder)
        };

        // --- Consume: admit only if the whole cost is available. ---
        let decision = if tokens >= cost {
            tokens -= cost;
            Decision::Allowed(Allowed {
                remaining: Some(tokens),
            })
        } else {
            // Time until `cost` tokens exist: finish the token currently in
            // progress (`interval - carry`), then one whole interval per further
            // token still missing. Relative duration only (ADR-017 #4).
            //
            // Note: a `cost` greater than `capacity` can never be satisfied; the
            // returned wait elapses but the request will still be rejected. That
            // mirrors classic token-bucket semantics and is the caller's contract.
            let missing = (cost - tokens) as u128;
            // Saturating so an extreme `cost × interval` can never overflow `u128` and
            // wrap to a bogusly-small wait (the only non-saturating arithmetic left).
            let retry_nanos = missing
                .saturating_mul(self.interval_nanos)
                .saturating_sub(carry_nanos as u128);
            Decision::Rejected(Rejected {
                retry_after: nanos_to_duration(retry_nanos),
            })
        };

        (
            TokenBucketState {
                tokens,
                carry_nanos,
                // Advance the refill anchor monotonically. The limiter samples
                // `now` *outside* the store's atomic section, so under concurrency
                // a reordered stale `now` can arrive after a newer one. Storing it
                // verbatim would rewind the anchor and re-count elapsed time on the
                // next call, minting phantom tokens. `max` pins the anchor forward
                // (mirroring GCRA's `tat.max(now)`); a stale call already refilled
                // nothing here, since `elapsed` above clamped to zero. This is a
                // conformance requirement for every algorithm — see the ADR-017
                // addendum (2026-07-20).
                last_update: state.last_update.max(now),
            },
            decision,
        )
    }
}

/// The Token Bucket transition as Lua, for RESP backends (ADR-006). Mirrors
/// [`TokenBucket::step`] exactly at **microsecond** resolution (see
/// [`crate::resp`] for why micros, and for the script ABI). The refill math, the
/// carried sub-token remainder, the monotonic anchor (`last.max(now)`), and the
/// retry-after formula all match the Rust implementation above.
const TOKEN_BUCKET_LUA: &str = r#"
-- Use effects replication so `redis.call('TIME')` before a write is allowed on
-- Redis 3.2–6 too (a harmless no-op on Redis 7+/Valkey, which already default to it).
redis.replicate_commands()

local key = KEYS[1]
local now = tonumber(ARGV[1])
if now < 0 then
  local t = redis.call('TIME')            -- { seconds, microseconds }
  now = (tonumber(t[1]) * 1000000) + tonumber(t[2])
end
local ttl = tonumber(ARGV[2])
local cost = tonumber(ARGV[3])
local capacity = tonumber(ARGV[4])
local interval = tonumber(ARGV[5])

local st = redis.call('HMGET', key, 't', 'c', 'u')
local tokens, carry, last
if st[1] == false then
  tokens = capacity; carry = 0; last = now
else
  tokens = tonumber(st[1]); carry = tonumber(st[2]); last = tonumber(st[3])
end

-- Refill: turn elapsed micros into whole tokens, exactly, carrying the remainder.
local elapsed = now - last
if elapsed < 0 then elapsed = 0 end
local accrued = carry + elapsed
local gained = math.floor(accrued / interval)
local remainder = accrued - (gained * interval)
local uncapped = tokens + gained
if uncapped >= capacity then
  tokens = capacity; carry = 0
else
  tokens = uncapped; carry = remainder
end

-- Advance the refill anchor monotonically (mirrors Rust `last_update.max(now)`).
if now > last then last = now end

local allowed, remaining, retry = 0, -1, 0
if tokens >= cost then
  tokens = tokens - cost
  allowed = 1
  remaining = tokens
else
  local missing = cost - tokens
  retry = (missing * interval) - carry
end

redis.call('HSET', key, 't', tokens, 'c', carry, 'u', last)
-- PEXPIRE takes milliseconds; round up so a key never expires before its TTL.
redis.call('PEXPIRE', key, math.ceil(ttl / 1000))

return { allowed, remaining, retry }
"#;

impl RespScripted for TokenBucket {
    fn resp_program(&self) -> RespProgram {
        // Quantise the nanosecond interval to microseconds once (see `crate::resp`).
        // Clamp to ≥ 1µs: rates above 1e6/s exceed the RESP backend's resolution and
        // are pinned to its ceiling rather than dividing by zero in Lua.
        let interval_micros = (self.interval_nanos / 1_000).max(1) as i64;
        RespProgram {
            lua: TOKEN_BUCKET_LUA,
            params: vec![self.capacity as i64, interval_micros],
        }
    }
}

/// Per-key token-bucket state — all integer, so `PartialEq`/`Eq` are exact and
/// decisions are reproducible.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TokenBucketState {
    /// Whole tokens currently available, in `0..=capacity`.
    tokens: u64,
    /// Nanoseconds of progress toward the next token, in `0..interval_nanos`.
    carry_nanos: u64,
    /// Refill anchor: the timestamp elapsed time is measured from. Advanced
    /// monotonically (never rewound), so a stale `now` reordered under
    /// concurrency cannot cause elapsed time to be counted twice.
    last_update: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed_remaining(d: &Decision) -> Option<u64> {
        match d {
            Decision::Allowed(a) => a.remaining,
            _ => None,
        }
    }

    #[test]
    fn capacity_above_i64_max_is_rejected() {
        // A capacity that would wrap negative when marshalled to Redis (i64) must fail at
        // construction so both backends agree (regression: silent RESP-vs-memory divergence).
        assert!(matches!(
            TokenBucket::new(i64::MAX as u64 + 1, Duration::from_secs(1)),
            Err(Error::Config(_))
        ));
        assert!(matches!(
            TokenBucket::per_second(u64::MAX, 1),
            Err(Error::Config(_))
        ));
        // The boundary value itself is fine.
        assert!(TokenBucket::new(i64::MAX as u64, Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn min_ttl_is_time_to_refill_from_empty() {
        // capacity 10, 2 tokens/sec ⇒ 5s to refill fully ⇒ state must persist ≥ 5s.
        let tb = TokenBucket::per_second(10, 2).unwrap();
        assert_eq!(tb.min_ttl(), Duration::from_secs(5));
    }

    #[test]
    fn admits_full_burst_then_refills_one_per_interval() {
        let tb = TokenBucket::new(2, Duration::from_secs(1)).unwrap(); // cap 2, 1 tok/s
        let t0 = Duration::ZERO;
        let mut s = tb.initial(t0);

        let (ns, d) = tb.step(s, t0, 1); // 2 -> 1
        assert_eq!(allowed_remaining(&d), Some(1));
        s = ns;
        let (ns, d) = tb.step(s, t0, 1); // 1 -> 0
        assert_eq!(allowed_remaining(&d), Some(0));
        s = ns;
        let (ns, d) = tb.step(s, t0, 1); // reject
        assert!(!d.is_allowed());
        assert_eq!(d.retry_after(), Some(Duration::from_secs(1)));
        s = ns;

        // Exactly one interval later, one token is available again.
        let (_, d) = tb.step(s, Duration::from_secs(1), 1);
        assert!(d.is_allowed());
    }

    #[test]
    fn partial_interval_accrues_no_token_but_is_remembered() {
        let tb = TokenBucket::new(1, Duration::from_secs(1)).unwrap();
        let mut s = tb.initial(Duration::ZERO);
        s = tb.step(s, Duration::ZERO, 1).0; // spend the one token

        // 400ms later: no whole token yet, and retry shrinks to the remaining 600ms.
        let (ns, d) = tb.step(s, Duration::from_millis(400), 1);
        assert!(!d.is_allowed());
        assert_eq!(d.retry_after(), Some(Duration::from_millis(600)));
        s = ns;

        // Another 600ms (total 1s): the carried 400ms makes this land exactly.
        let (_, d) = tb.step(s, Duration::from_millis(1000), 1);
        assert!(d.is_allowed());
    }

    #[test]
    fn carry_is_exact_across_many_sub_interval_steps() {
        // The float version drifts here; integer carry must not. One token per
        // 3ns — an awkward interval that never divides an elapsed nanosecond.
        let tb = TokenBucket::new(1, Duration::from_nanos(3)).unwrap();
        let mut s = tb.initial(Duration::ZERO);
        s = tb.step(s, Duration::ZERO, 1).0; // spend initial token

        // Slice the interval 1ns at a time; a token must appear on exactly the
        // 3rd ns, never the 2nd or 4th, no matter how the interval is sliced.
        let (ns, d) = tb.step(s, Duration::from_nanos(1), 1);
        assert!(!d.is_allowed());
        s = ns;
        let (ns, d) = tb.step(s, Duration::from_nanos(2), 1);
        assert!(!d.is_allowed());
        s = ns;
        let (_, d) = tb.step(s, Duration::from_nanos(3), 1);
        assert!(d.is_allowed());
    }

    #[test]
    fn idle_refill_caps_at_capacity() {
        let tb = TokenBucket::new(3, Duration::from_secs(1)).unwrap();
        let mut s = tb.initial(Duration::ZERO);
        for _ in 0..3 {
            s = tb.step(s, Duration::ZERO, 1).0; // drain
        }
        // Idle a long time, then hammer: at most `capacity` admits back-to-back.
        let now = Duration::from_secs(10_000);
        let mut admitted = 0;
        for _ in 0..10 {
            let (ns, d) = tb.step(s, now, 1);
            s = ns;
            if d.is_allowed() {
                admitted += 1;
            }
        }
        assert_eq!(admitted, 3, "idle refill must not exceed capacity");
    }

    #[test]
    fn cost_n_consumes_n_tokens() {
        let tb = TokenBucket::new(5, Duration::from_secs(1)).unwrap();
        let t0 = Duration::ZERO;
        let (s, d) = tb.step(tb.initial(t0), t0, 3);
        assert!(d.is_allowed());
        assert_eq!(allowed_remaining(&d), Some(2));

        // A cost-3 follow-up exceeds the remaining 2 → reject, wait for 1 token.
        let (_, d) = tb.step(s, t0, 3);
        assert!(!d.is_allowed());
        assert_eq!(d.retry_after(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn per_second_matches_interval_form() {
        let a = TokenBucket::per_second(10, 5).unwrap();
        let b = TokenBucket::new(10, Duration::from_millis(200)).unwrap();
        assert_eq!(a.interval_nanos, b.interval_nanos);
        assert_eq!(a.capacity, b.capacity);
    }

    #[test]
    fn backwards_time_never_grants_tokens() {
        let tb = TokenBucket::new(2, Duration::from_secs(1)).unwrap();
        // Establish state at t = 10s, fully drained.
        let mut s = tb.initial(Duration::from_secs(10));
        s = tb.step(s, Duration::from_secs(10), 1).0;
        s = tb.step(s, Duration::from_secs(10), 1).0;
        // A call with an earlier `now` must not mint tokens.
        let (_, d) = tb.step(s, Duration::from_secs(5), 1);
        assert!(!d.is_allowed());
    }

    #[test]
    fn rejects_zero_capacity() {
        assert!(matches!(
            TokenBucket::new(0, Duration::from_secs(1)),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn rejects_zero_interval() {
        assert!(matches!(
            TokenBucket::new(1, Duration::ZERO),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn rejects_excessive_rate() {
        assert!(matches!(
            TokenBucket::per_second(1, 2_000_000_000),
            Err(Error::Config(_))
        ));
    }
}
