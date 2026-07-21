# flowshield-axum

[Axum](https://github.com/tokio-rs/axum) integration for
[FlowShield](https://github.com/devcalm/flowshield) — per-key rate limiting middleware
with a standards-correct `429 Too Many Requests` + `Retry-After` (ADR-009).

Adds Axum ergonomics on top of [`flowshield-tower`](https://crates.io/crates/flowshield-tower)
rather than reimplementing it: the Tower layer works directly, plus a
`ConnectInfoKeyExtractor` that reads the client address from Axum's `ConnectInfo`.

```rust,ignore
use axum::{routing::get, Router};
use flowshield_axum::per_client_ip;

let app = Router::new()
    .route("/", get(handler))
    .layer(per_client_ip(limiter)); // rate-limit by client IP

axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
```

For API-key / tenant / header keying, use `FlowShieldLayer` with any `KeyExtractor`
(the built-ins are re-exported from `flowshield-tower`).

## License

Licensed under either of Apache-2.0 or MIT at your option.
