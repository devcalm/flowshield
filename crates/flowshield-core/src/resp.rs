//! Family-B capability for RESP stores (Redis / Valkey) — ADR-006, ADR-019.
//!
//! A server-side store cannot run the Rust [`Algorithm::step`] closure atomically
//! (Redis runs Lua, not Rust — ADR-006). Instead an algorithm that wants to work on
//! a RESP backend *also* describes its transition as a **Lua program** via
//! [`RespScripted`]. A generic RESP backend (`flowshield-resp`) runs that program
//! with a single atomic round trip; the Rust `step` remains the reference oracle the
//! program is cross-checked against (ADR-019).
//!
//! This module is pure data (a script string plus integer parameters) and pulls in
//! no dependencies, so it lives in core alongside the algorithms that implement it.
//! No Redis client is involved here.
//!
//! # Time and precision
//!
//! Lua numbers are IEEE-754 doubles, so integers are exact only below 2⁵³. Epoch
//! **nanoseconds** (~1.7 × 10¹⁸) overflow that; epoch **microseconds** (~1.7 × 10¹⁵)
//! do not. RESP programs therefore work in **microseconds**, a documented
//! per-backend precision difference from the memory backend's nanosecond exactness
//! (ADR-005). All times and durations in the [`RespProgram`] ABI are microseconds.
//!
//! # Script ABI
//!
//! A program is invoked as:
//!
//! ```text
//! EVAL <lua> 1 <key>  <now_micros> <ttl_micros> <cost>  <params...>
//! ```
//!
//! - `now_micros` — the decision time, or **`-1` to make the script read
//!   server-authoritative time** (`redis.call('TIME')`), which is the production
//!   path (ADR-017: source time from the store to remove inter-node skew). A
//!   non-negative value is used verbatim, which is how tests drive the script
//!   deterministically.
//! - `params` — [`RespProgram::params`], appended in order after `cost`.
//!
//! The script MUST return a three-element array `{allowed, remaining, retry_micros}`:
//! `allowed` is `1`/`0`; `remaining` is the tokens left (or `-1` if unknown);
//! `retry_micros` is the wait on rejection (ignored when allowed).

use crate::algorithm::Algorithm;

/// A server-side transition program for a RESP backend, in the ABI documented on the
/// [module](self).
#[derive(Debug, Clone)]
pub struct RespProgram {
    /// Lua source implementing the transition. Must honour the [module](self) ABI.
    pub lua: &'static str,
    /// Static, per-limiter integer parameters (already in microseconds where they
    /// denote time), appended to the script's `ARGV` after `now`, `ttl`, and `cost`,
    /// in the order the script reads them.
    pub params: Vec<i64>,
}

/// An [`Algorithm`] that can additionally run its transition **server-side** on a
/// RESP store (Redis / Valkey), by supplying a Lua [`RespProgram`] (ADR-006, ADR-019).
///
/// The Lua program and the Rust [`Algorithm::step`] implement the same transition
/// and must agree — `step` is the oracle the conformance cross-check compares the
/// program against, at the microsecond resolution of the RESP encoding.
pub trait RespScripted: Algorithm {
    /// The server-side program for this configured algorithm instance.
    fn resp_program(&self) -> RespProgram;
}
