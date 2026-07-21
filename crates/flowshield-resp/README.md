# flowshield-resp

Shared **RESP (Redis / Valkey)** backend logic for [FlowShield](https://github.com/devcalm/flowshield).

Redis and Valkey speak the same RESP protocol and server-side Lua scripting, so the
atomic rate-limiting logic is written once here and reused by both
[`flowshield-redis`](https://crates.io/crates/flowshield-redis) and
[`flowshield-valkey`](https://crates.io/crates/flowshield-valkey) — the thin,
independently-selectable vendor crates most users depend on (ADR-006).

This crate is a **family-B** storage backend (ADR-019): it runs an algorithm's
`RespScripted` Lua program server-side, atomically, in a single round trip, and
implements the limiter's `Evaluate` bridge directly. It is Tokio-coupled
(`redis` + `deadpool-redis`) and operates at microsecond resolution.

Most users should depend on `flowshield-redis` or `flowshield-valkey` rather than
this crate directly.

## License

Licensed under either of Apache-2.0 or MIT at your option.
