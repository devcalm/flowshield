# flowshield-actix

[Actix Web](https://actix.rs) integration for
[FlowShield](https://github.com/devcalm/flowshield) — per-key rate limiting middleware
with a standards-correct `429 Too Many Requests` + `Retry-After` (ADR-009).

Actix Web doesn't use Tower's model, so this is a **standalone** integration built on
Actix's own `Transform`/`Service` traits — it does not depend on `flowshield-tower`.

```rust,ignore
use flowshield_actix::FlowShield;

let mw = FlowShield::per_peer(limiter); // rate-limit by client peer IP
App::new().wrap(mw.clone()).route("/", web::get().to(handler));
```

**Keying:** `PeerAddrKeyExtractor` (default, peer IP — safe), `HeaderKeyExtractor`
(e.g. API key), or `RealIpKeyExtractor` (**trusted-proxy-only** — trusts
`X-Forwarded-For`).

## License

Licensed under either of Apache-2.0 or MIT at your option.
