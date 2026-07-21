# flowshield-tower

Tower middleware for [FlowShield](https://github.com/devcalm/flowshield) — per-key HTTP
rate limiting as a `Layer`/`Service`, with a standards-correct `429 Too Many Requests`
+ `Retry-After` on rejection (ADR-009).

This is the foundational HTTP integration; [`flowshield-axum`](https://crates.io/crates/flowshield-axum)
builds on it. Any `tower`/`hyper` stack can use it directly.

```rust,ignore
use flowshield_tower::FlowShieldLayer;

let layer = FlowShieldLayer::per_peer(limiter); // rate-limit by client peer IP
// ServiceBuilder::new().layer(layer).service(your_service)
```

**Keying:** supply any `KeyExtractor`, or use a built-in — `PeerAddrKeyExtractor`
(default, peer IP), `HeaderKeyExtractor` (e.g. API key), or `ForwardedForKeyExtractor`
(**only behind a trusted proxy** — `X-Forwarded-For` is client-spoofable).

## License

Licensed under either of Apache-2.0 or MIT at your option.
