#![forbid(unsafe_code)]
//! # FlowShield — Axum integration
//!
//! `flowshield-axum` adds Axum-idiomatic ergonomics on top of the
//! [`flowshield-tower`](flowshield_tower) middleware (ADR-009): the same `Layer` works
//! directly in an Axum app, plus a [`ConnectInfoKeyExtractor`] that reads the client
//! address from Axum's [`ConnectInfo`](axum::extract::ConnectInfo).
//!
//! ```no_run
//! use axum::{routing::get, Router};
//! use flowshield_axum::per_client_ip;
//! use flowshield_core::{algorithm::token_bucket::TokenBucket, limiter::LimiterBuilder};
//! use flowshield_memory::MemoryStore;
//! use std::net::SocketAddr;
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let limiter = LimiterBuilder::new(TokenBucket::per_second(100, 20)?, MemoryStore::new()).build()?;
//!
//! let app = Router::new()
//!     .route("/", get(|| async { "hello" }))
//!     .layer(per_client_ip(limiter));
//!
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! // `ConnectInfo` requires the connect-info make-service so the client address is
//! // available to the extractor:
//! axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! For custom keying (API key, tenant, header), use [`flowshield_tower::FlowShieldLayer`]
//! directly with any [`KeyExtractor`](flowshield_core::key::KeyExtractor) — the ones
//! from `flowshield-tower` are re-exported here for convenience.

use axum::extract::ConnectInfo;
use flowshield_core::composite::DynLimiter;
use flowshield_core::key::{Key, KeyExtractor};
use http::Request;
use std::net::SocketAddr;

// Re-export the Tower building blocks so Axum users need only depend on this crate.
pub use flowshield_tower::{
    FlowShieldLayer, FlowShieldService, ForwardedForKeyExtractor, HeaderKeyExtractor,
    PeerAddrKeyExtractor,
};

/// Keys a request on the client IP from Axum's
/// [`ConnectInfo<SocketAddr>`](axum::extract::ConnectInfo), which Axum populates when
/// the app is served via
/// [`into_make_service_with_connect_info`](axum::Router::into_make_service_with_connect_info).
///
/// This is the Axum counterpart to
/// [`PeerAddrKeyExtractor`](flowshield_tower::PeerAddrKeyExtractor) (which reads a bare
/// `SocketAddr` extension). Like it, it is spoof-resistant — the address comes from the
/// accepted connection, not a client-controlled header. Requests without connect-info
/// fall into one shared `"unknown-peer"` bucket.
#[derive(Debug, Clone, Default)]
pub struct ConnectInfoKeyExtractor;

impl<B> KeyExtractor<Request<B>> for ConnectInfoKeyExtractor {
    fn extract_key(&self, req: &Request<B>) -> Key {
        req.extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| Key::from(info.0.ip().to_string()))
            .unwrap_or_else(|| Key::from("unknown-peer"))
    }
}

/// Build a [`FlowShieldLayer`] that rate-limits by client IP using Axum's
/// [`ConnectInfo`](axum::extract::ConnectInfo) (see [`ConnectInfoKeyExtractor`]).
///
/// Serve the router with `into_make_service_with_connect_info::<SocketAddr>()` so the
/// client address is available.
pub fn per_client_ip(
    limiter: impl DynLimiter + 'static,
) -> FlowShieldLayer<ConnectInfoKeyExtractor> {
    FlowShieldLayer::new(limiter, ConnectInfoKeyExtractor)
}
