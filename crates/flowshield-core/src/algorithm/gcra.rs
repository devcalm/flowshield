//! GCRA — the Generic Cell Rate Algorithm (ADR-008).
//!
//! GCRA enforces a **minimum inter-arrival interval** between admitted requests,
//! producing even pacing rather than clustered bursts, while still tolerating a
//! bounded burst. It is the precision counterpart to [`TokenBucket`]: prefer it
//! when protecting a shared downstream whose fairness depends on *when* requests
//! land, not merely how many land per second.
//!
//! [`TokenBucket`]: crate::algorithm::token_bucket::TokenBucket
//!
//! # Model
//!
//! The algorithm tracks a single per-key value, the **theoretical arrival time**
//! (`tat`): the instant at which the *next* request would land if the caller were
//! perfectly paced at the sustained rate. Two parameters shape behaviour:
//!
//! - **Emission interval** `T = 1 / rate` — the spacing between requests at the
//!   steady-state rate.
//! - **Burst tolerance** `τ = (burst − 1) · T` — how far ahead of the schedule a
//!   caller may run. A cold key may therefore admit exactly `burst` requests
//!   instantaneously before pacing kicks in.
//!
//! A request of cost `n` arriving at time `now` conforms when
//! `now ≥ tat + (n − 1)·T − τ`. When it conforms, the schedule advances to
//! `max(tat, now) + n·T`; when it does not, the state is left untouched and the
//! caller is told how long to wait. Because an idle key has `tat ≤ now`, the
//! `max(tat, now)` clamp means idleness is never "banked" beyond the burst
//! tolerance — a key cannot accrue unbounded credit by staying quiet.
//!
//! # Documented properties (ADR-008)
//!
//! - **Burst behaviour:** bounded — up to `burst` requests may arrive together on
//!   a cold key; thereafter admission is paced at one per `T`.
//! - **Smoothing:** high — enforced inter-arrival spacing yields even pacing, so
//!   no caller can monopolise a shared resource by bursting.
//! - **Memory / state per key:** minimal — a single `Duration` (`tat`).
//! - **Distributed suitability:** strong — the transition is an atomic update of
//!   one value, a natural fit for the check-and-consume contract (ADR-005/006).
//!
//! All time math is on the injected monotonic clock (ADR-017); no wall-clock is
//! read here, and [`step`](Gcra::step) is pure.

use crate::algorithm::{Algorithm, nanos_to_duration};
use crate::decision::{Allowed, Decision, Rejected};
use crate::error::{Error, Result};
use crate::resp::{RespProgram, RespScripted};
use std::time::Duration;

/// A GCRA rate limiter configured by a sustained rate and a burst allowance.
///
/// ```
/// use flowshield_core::algorithm::gcra::Gcra;
///
/// // 10 requests/second sustained, tolerating a burst of up to 5 at once.
/// let gcra = Gcra::new(10.0, 5)?;
/// # Ok::<(), flowshield_core::error::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct Gcra {
    /// `T` — the spacing between requests at the sustained rate (`1 / rate`).
    emission_interval: Duration,
    /// `τ` — burst tolerance, `(burst − 1) · T`. Zero when `burst == 1` (no burst).
    tau: Duration,
}

impl Gcra {
    /// Create a GCRA limiter admitting `rate_per_sec` requests per second in the
    /// steady state, with an instantaneous burst allowance of `burst` requests.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if `rate_per_sec` is not finite and positive, or
    /// if `burst` is zero. Invalid configuration is a typed error, never a panic
    /// (ADR-011, FR-072).
    pub fn new(rate_per_sec: f64, burst: u64) -> Result<Self> {
        if !(rate_per_sec.is_finite() && rate_per_sec > 0.0) {
            return Err(Error::Config(
                "rate_per_sec must be finite and positive".into(),
            ));
        }
        if burst == 0 {
            return Err(Error::Config("burst must be greater than zero".into()));
        }

        // The rate → interval conversion is the *only* floating-point step, done
        // once at construction. Everything downstream is exact integer nanos.
        let emission_interval = Duration::from_secs_f64(1.0 / rate_per_sec);
        // A rate so large that the interval rounds to zero would make the limiter
        // admit unboundedly (and diverge from the RESP backend, which clamps to 1µs).
        // Reject it as a config error, symmetrically with Token Bucket's zero-interval
        // check (ADR-011, FR-072).
        if emission_interval.is_zero() {
            return Err(Error::Config(
                "rate_per_sec is too large: the emission interval rounds to zero".into(),
            ));
        }
        // A burst of `b` means `b − 1` requests may run ahead of the schedule.
        let tau = nanos_to_duration(
            emission_interval
                .as_nanos()
                .saturating_mul((burst - 1) as u128),
        );

        Ok(Self {
            emission_interval,
            tau,
        })
    }

    /// How many further cost-1 requests would be admitted *right now*, given a
    /// schedule whose theoretical arrival time is `tat` (already clamped to be
    /// no earlier than `now`).
    ///
    /// Reported in units of single requests, mirroring [`TokenBucket`]'s
    /// `remaining`. Derived from the schedule rather than stored.
    ///
    /// [`TokenBucket`]: crate::algorithm::token_bucket::TokenBucket
    fn remaining(&self, tat: Duration, now: Duration) -> u64 {
        // How far the schedule already runs into the future.
        let ahead = tat.saturating_sub(now);
        if ahead > self.tau {
            // No slack left; the next request must wait.
            return 0;
        }
        let interval_nanos = self.emission_interval.as_nanos();
        if interval_nanos == 0 {
            // Sub-nanosecond interval (an enormous rate): effectively unlimited.
            return u64::MAX;
        }
        let slack_nanos = (self.tau - ahead).as_nanos();
        // Exact integer division — `+ 1` because zero remaining slack still
        // admits one more request. No floating point, no epsilon.
        (slack_nanos / interval_nanos) as u64 + 1
    }
}

impl Algorithm for Gcra {
    type State = GcraState;

    fn name(&self) -> &'static str {
        "gcra"
    }

    fn initial(&self, now: Duration) -> Self::State {
        // A fresh key is fully idle: its schedule sits at `now`, so the full
        // burst is immediately available.
        GcraState { tat: now }
    }

    fn step(&self, state: Self::State, now: Duration, cost: u64) -> (Self::State, Decision) {
        let tat = state.tat;

        // A zero-cost probe never advances the schedule; report headroom only.
        if cost == 0 {
            let remaining = self.remaining(tat.max(now), now);
            return (
                state,
                Decision::Allowed(Allowed {
                    remaining: Some(remaining),
                }),
            );
        }

        // All schedule math below is exact integer nanoseconds.
        let interval_nanos = self.emission_interval.as_nanos();

        // Earliest instant at which a request of this cost conforms:
        //   tat + (cost − 1)·T − τ
        // Using the *stored* `tat` (not clamped to `now`): an idle key has a
        // `tat` in the past, which pushes `earliest` before `now` and admits.
        let extra = nanos_to_duration(interval_nanos.saturating_mul((cost - 1) as u128));
        let earliest = (tat + extra).saturating_sub(self.tau);

        if now < earliest {
            // Non-conforming: leave the schedule untouched and report the wait.
            // Relative duration only — never an absolute timestamp (ADR-017 #4).
            let retry_after = earliest.saturating_sub(now);
            return (state, Decision::Rejected(Rejected { retry_after }));
        }

        // Conforming: advance the schedule by the full cost. The `max` clamp
        // prevents an idle key from banking credit beyond the burst tolerance.
        let step = nanos_to_duration(interval_nanos.saturating_mul(cost as u128));
        let new_tat = tat.max(now).saturating_add(step);
        let remaining = self.remaining(new_tat, now);

        (
            GcraState { tat: new_tat },
            Decision::Allowed(Allowed {
                remaining: Some(remaining),
            }),
        )
    }
}

/// The GCRA transition as Lua, for RESP backends (ADR-006). Mirrors [`Gcra::step`]
/// exactly at **microsecond** resolution (see [`crate::resp`]). State is the single
/// theoretical-arrival-time (`tat`) value; the conformance rule
/// `now ≥ tat + (cost−1)·T − τ`, the `max(tat, now)` idle clamp, and the derived
/// `remaining` all match the Rust implementation above.
const GCRA_LUA: &str = r#"
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
local T = tonumber(ARGV[4])               -- emission interval (micros)
local tau = tonumber(ARGV[5])             -- burst tolerance (micros)

local stored = redis.call('GET', key)
local tat
if stored == false then tat = now else tat = tonumber(stored) end
local pexpire = math.ceil(ttl / 1000)

-- Zero-cost probe: report headroom without advancing the schedule (mirrors Rust).
if cost == 0 then
  local base = tat
  if now > base then base = now end
  local ahead = base - now
  local remaining
  if ahead > tau then remaining = 0 else remaining = math.floor((tau - ahead) / T) + 1 end
  redis.call('SET', key, tat, 'PX', pexpire)   -- unchanged tat, refreshed TTL
  return { 1, remaining, 0 }
end

-- Earliest conforming instant, using the stored (unclamped) tat.
local earliest = tat + ((cost - 1) * T) - tau

if now < earliest then
  -- Non-conforming: the schedule (tat) is left untouched, but the key's TTL MUST be
  -- refreshed — exactly as the memory backend and the Token Bucket script do on every
  -- call. Skipping it would let an actively-rejected key expire mid-limiting and reset
  -- to a fresh (more-permissive) schedule, violating ADR-005 §3. Persist the unchanged
  -- tat (also creating the key if this fresh key rejected, e.g. cost > burst).
  redis.call('SET', key, tat, 'PX', pexpire)
  return { 0, -1, earliest - now }
end

-- Conforming: advance by the full cost; the max clamp caps banked idle credit.
local base = tat
if now > base then base = now end
local new_tat = base + (cost * T)
local ahead = new_tat - now
if ahead < 0 then ahead = 0 end
local remaining
if ahead > tau then remaining = 0 else remaining = math.floor((tau - ahead) / T) + 1 end

redis.call('SET', key, new_tat, 'PX', pexpire)
return { 1, remaining, 0 }
"#;

impl RespScripted for Gcra {
    fn resp_program(&self) -> RespProgram {
        // Quantise to microseconds once (see `crate::resp`). Clamp T to ≥ 1µs: rates
        // above 1e6/s exceed the RESP backend's resolution and are pinned to its
        // ceiling rather than dividing by zero in Lua. τ may legitimately be 0
        // (burst == 1).
        let t_micros = (self.emission_interval.as_micros() as i64).max(1);
        let tau_micros = self.tau.as_micros() as i64;
        RespProgram {
            lua: GCRA_LUA,
            params: vec![t_micros, tau_micros],
        }
    }
}

/// Per-key GCRA state: a single theoretical-arrival-time value (ADR-008).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct GcraState {
    tat: Duration,
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
    fn cold_key_admits_full_burst_then_rejects() {
        // 1 req/sec sustained, burst of 3 → T = 1s, τ = 2s.
        let gcra = Gcra::new(1.0, 3).unwrap();
        let t0 = Duration::ZERO;
        let mut s = gcra.initial(t0);

        // Three requests conform instantly on a cold key.
        for expected_remaining in [2, 1, 0] {
            let (ns, d) = gcra.step(s, t0, 1);
            assert!(d.is_allowed());
            assert_eq!(allowed_remaining(&d), Some(expected_remaining));
            s = ns;
        }

        // The fourth is paced out; it must wait one emission interval.
        let (_, d) = gcra.step(s, t0, 1);
        assert!(!d.is_allowed());
        assert_eq!(d.retry_after(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn admits_one_per_emission_interval_after_burst() {
        let gcra = Gcra::new(1.0, 3).unwrap();
        let t0 = Duration::ZERO;
        let mut s = gcra.initial(t0);

        // Drain the burst.
        for _ in 0..3 {
            let (ns, d) = gcra.step(s, t0, 1);
            assert!(d.is_allowed());
            s = ns;
        }
        // Rejected at t0 ...
        assert!(!gcra.step(s, t0, 1).1.is_allowed());

        // ... but exactly one admit becomes available each second thereafter.
        for sec in 1..=3 {
            let now = Duration::from_secs(sec);
            let (ns, d) = gcra.step(s, now, 1);
            assert!(d.is_allowed(), "should admit at second {sec}");
            s = ns;
            // Still paced: a second request in the same instant is rejected.
            assert!(!gcra.step(s, now, 1).1.is_allowed());
        }
    }

    #[test]
    fn retry_after_shrinks_as_time_passes() {
        let gcra = Gcra::new(1.0, 1).unwrap(); // no burst: strict one-per-second
        let t0 = Duration::ZERO;
        let (s, d) = gcra.step(gcra.initial(t0), t0, 1);
        assert!(d.is_allowed());

        // Immediately after: full interval to wait.
        let reject_now = gcra.step(s, t0, 1).1;
        assert_eq!(reject_now.retry_after(), Some(Duration::from_secs(1)));

        // Halfway through the interval: half remains.
        let reject_mid = gcra.step(s, Duration::from_millis(400), 1).1;
        assert_eq!(reject_mid.retry_after(), Some(Duration::from_millis(600)));
    }

    #[test]
    fn idle_key_does_not_bank_unbounded_credit() {
        // burst 2 → at most 2 admits back-to-back regardless of prior idleness.
        let gcra = Gcra::new(1.0, 2).unwrap();
        let (s, _) = gcra.step(gcra.initial(Duration::ZERO), Duration::ZERO, 1);

        // Idle for a long time, then hammer: only `burst` requests admit at once.
        let now = Duration::from_secs(1_000);
        let mut s = s;
        let mut admitted = 0;
        for _ in 0..10 {
            let (ns, d) = gcra.step(s, now, 1);
            s = ns;
            if d.is_allowed() {
                admitted += 1;
            }
        }
        assert_eq!(
            admitted, 2,
            "idle credit must be capped at the burst tolerance"
        );
    }

    #[test]
    fn cost_n_consumes_n_units() {
        let gcra = Gcra::new(1.0, 5).unwrap(); // T = 1s, τ = 4s
        let t0 = Duration::ZERO;

        // A single cost-3 request draws down three units of the burst.
        let (s, d) = gcra.step(gcra.initial(t0), t0, 3);
        assert!(d.is_allowed());
        assert_eq!(allowed_remaining(&d), Some(2));

        // A cost-3 follow-up now exceeds the remaining 2 units → rejected.
        let (_, d2) = gcra.step(s, t0, 3);
        assert!(!d2.is_allowed());
    }

    #[test]
    fn zero_cost_probe_is_allowed_without_advancing() {
        let gcra = Gcra::new(1.0, 3).unwrap();
        let t0 = Duration::ZERO;
        let s = gcra.initial(t0);

        let (ns, d) = gcra.step(s, t0, 0);
        assert!(d.is_allowed());
        assert_eq!(ns, s, "a zero-cost probe must not move the schedule");
        // Full burst still reported as available.
        assert_eq!(allowed_remaining(&d), Some(3));
    }

    #[test]
    fn rejects_zero_burst() {
        assert!(matches!(Gcra::new(1.0, 0), Err(Error::Config(_))));
    }

    #[test]
    fn rejects_nonpositive_rate() {
        assert!(matches!(Gcra::new(0.0, 1), Err(Error::Config(_))));
        assert!(matches!(Gcra::new(-1.0, 1), Err(Error::Config(_))));
        assert!(matches!(Gcra::new(f64::NAN, 1), Err(Error::Config(_))));
        assert!(matches!(Gcra::new(f64::INFINITY, 1), Err(Error::Config(_))));
    }

    #[test]
    fn rejects_rate_whose_interval_rounds_to_zero() {
        // A rate beyond ~1e9/s makes `1/rate` round to a zero nanosecond interval,
        // which would otherwise silently admit without limit. Symmetric with Token
        // Bucket's zero-interval rejection.
        assert!(matches!(Gcra::new(1e18, 1), Err(Error::Config(_))));
    }
}
