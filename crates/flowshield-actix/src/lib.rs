#![forbid(unsafe_code)]
//! # FlowShield — Actix Web integration
//!
//! `flowshield-actix` is FlowShield's integration for [Actix Web](https://actix.rs)
//! (ADR-009): a middleware that rate-limits requests by a key derived from each
//! request, returning a standards-correct `429 Too Many Requests` with a `Retry-After`
//! header on rejection and passing allowed requests through untouched.
//!
//! Actix Web does not use Tower's `Service`/`Layer` model, so this is a **standalone**
//! integration (it does not depend on `flowshield-tower`), built on Actix's own
//! [`Transform`]/[`Service`] traits.
//!
//! ```no_run
//! use actix_web::{web, App, HttpServer, HttpResponse};
//! use flowshield_actix::FlowShield;
//! use flowshield_core::{algorithm::token_bucket::TokenBucket, limiter::LimiterBuilder};
//! use flowshield_memory::MemoryStore;
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     let limiter = LimiterBuilder::new(
//!         TokenBucket::per_second(100, 20).unwrap(),
//!         MemoryStore::new(),
//!     )
//!     .build()
//!     .unwrap();
//!     let mw = FlowShield::per_peer(limiter);
//!
//!     HttpServer::new(move || {
//!         App::new()
//!             .wrap(mw.clone()) // rate-limit by client peer IP
//!             .route("/", web::get().to(|| async { HttpResponse::Ok().finish() }))
//!     })
//!     .bind(("127.0.0.1", 8080))?
//!     .run()
//!     .await
//! }
//! ```
//!
//! ## Keying — and a security note
//!
//! Keying uses an application-supplied [`KeyExtractor`]. Provided defaults:
//!
//! - [`PeerAddrKeyExtractor`] (default) — keys on the **direct peer IP**
//!   ([`ServiceRequest::peer_addr`]). Safe: it cannot be spoofed by the client.
//! - [`HeaderKeyExtractor`] — keys on a named header (e.g. an API key).
//! - [`RealIpKeyExtractor`] — keys on Actix's `realip_remote_addr`, which trusts
//!   `X-Forwarded-For`/`Forwarded`. **Only use behind a trusted proxy** — otherwise a
//!   client can spoof its key and evade the limit.

use actix_web::body::{EitherBody, MessageBody};
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform, forward_ready};
use actix_web::http::header;
use actix_web::{Error, HttpResponse};
use flowshield_core::composite::DynLimiter;
use flowshield_core::key::{Key, KeyExtractor};
use std::future::{Ready, ready};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Keys a request on the **direct peer IP** ([`ServiceRequest::peer_addr`]). This is the
/// safe default — the address comes from the accepted socket, not a client header, so it
/// cannot be spoofed. Requests without a peer address share one `"unknown-peer"` bucket.
#[derive(Debug, Clone, Default)]
pub struct PeerAddrKeyExtractor;

impl KeyExtractor<ServiceRequest> for PeerAddrKeyExtractor {
    fn extract_key(&self, req: &ServiceRequest) -> Key {
        req.peer_addr()
            .map(|addr| Key::from(addr.ip().to_string()))
            .unwrap_or_else(|| Key::from("unknown-peer"))
    }
}

/// Keys a request on the value of a named header — e.g. an API key or tenant id.
/// Requests missing the header (or with a non-UTF-8 value) share one fallback bucket.
#[derive(Debug, Clone)]
pub struct HeaderKeyExtractor {
    header: header::HeaderName,
    fallback: Key,
}

impl HeaderKeyExtractor {
    /// Extract the key from `header` (e.g. `x-api-key`).
    pub fn new(header: header::HeaderName) -> Self {
        Self {
            header,
            fallback: Key::from("unknown-header"),
        }
    }
}

impl KeyExtractor<ServiceRequest> for HeaderKeyExtractor {
    fn extract_key(&self, req: &ServiceRequest) -> Key {
        req.headers()
            .get(&self.header)
            .and_then(|v| v.to_str().ok())
            .map(Key::from)
            .unwrap_or_else(|| self.fallback.clone())
    }
}

/// Keys a request on Actix's `realip_remote_addr`, which reads the client IP from
/// `X-Forwarded-For` / `Forwarded` when present, else the peer address.
///
/// # Security
///
/// Those headers are **client-spoofable**. Use this extractor **only** behind a trusted
/// reverse proxy that overwrites them with the real client address; exposed directly, a
/// client can pick any key and evade its limit. When in doubt, prefer
/// [`PeerAddrKeyExtractor`].
#[derive(Debug, Clone, Default)]
pub struct RealIpKeyExtractor;

impl KeyExtractor<ServiceRequest> for RealIpKeyExtractor {
    fn extract_key(&self, req: &ServiceRequest) -> Key {
        req.connection_info()
            .realip_remote_addr()
            .map(Key::from)
            .unwrap_or_else(|| Key::from("unknown-realip"))
    }
}

/// The FlowShield Actix middleware factory (an Actix [`Transform`]).
///
/// Construct it with [`FlowShield::per_peer`] (peer-IP keying) or [`FlowShield::new`]
/// with a custom [`KeyExtractor`], then `App::wrap(mw)`.
pub struct FlowShield<K> {
    limiter: Arc<dyn DynLimiter>,
    extractor: Arc<K>,
}

impl<K> FlowShield<K> {
    /// Build the middleware from any limiter and a key-extraction strategy.
    pub fn new(limiter: impl DynLimiter + 'static, extractor: K) -> Self {
        Self {
            limiter: Arc::new(limiter),
            extractor: Arc::new(extractor),
        }
    }
}

impl FlowShield<PeerAddrKeyExtractor> {
    /// Build middleware that rate-limits by client peer IP ([`PeerAddrKeyExtractor`]).
    pub fn per_peer(limiter: impl DynLimiter + 'static) -> Self {
        Self::new(limiter, PeerAddrKeyExtractor)
    }
}

// Manual `Clone` (both fields are `Arc`, so `K: Clone` is not required). Actix clones
// the middleware factory into each worker's `App`.
impl<K> Clone for FlowShield<K> {
    fn clone(&self) -> Self {
        Self {
            limiter: self.limiter.clone(),
            extractor: self.extractor.clone(),
        }
    }
}

impl<S, K, B> Transform<S, ServiceRequest> for FlowShield<K>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    K: KeyExtractor<ServiceRequest> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = FlowShieldMiddleware<S, K>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(FlowShieldMiddleware {
            service: Rc::new(service),
            limiter: self.limiter.clone(),
            extractor: self.extractor.clone(),
        }))
    }
}

/// The middleware [`Service`] produced by [`FlowShield`]. Checks the limiter before
/// delegating to the inner service, returning `429` on rejection.
pub struct FlowShieldMiddleware<S, K> {
    // `Rc` because Actix services are per-worker and single-threaded (`!Send`).
    service: Rc<S>,
    limiter: Arc<dyn DynLimiter>,
    extractor: Arc<K>,
}

impl<S, K, B> Service<ServiceRequest> for FlowShieldMiddleware<S, K>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    K: KeyExtractor<ServiceRequest> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    // Actix futures are not required to be `Send` (per-worker current-thread runtime).
    type Future = Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>>>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let limiter = self.limiter.clone();
        let extractor = self.extractor.clone();

        Box::pin(async move {
            let key = extractor.extract_key(&req);
            match limiter.check_dyn(&key).await {
                Ok(decision) if decision.is_allowed() => {
                    // Allowed: run the inner handler, unifying its body into the left arm.
                    let res = service.call(req).await?;
                    Ok(res.map_into_left_body())
                }
                Ok(decision) => {
                    // Normal rejection → 429 (ADR-009, FR-011); body is the right arm.
                    let retry = decision.retry_after().unwrap_or(Duration::from_secs(1));
                    Ok(req
                        .into_response(too_many_requests(retry))
                        .map_into_right_body())
                }
                // A runtime `Err` is only a config error (backend failures already
                // degrade to a decision, ADR-007); reject conservatively.
                Err(_) => Ok(req
                    .into_response(too_many_requests(Duration::from_secs(1)))
                    .map_into_right_body()),
            }
        })
    }
}

/// Build a `429 Too Many Requests` response with a `Retry-After` header.
fn too_many_requests(retry_after: Duration) -> HttpResponse {
    HttpResponse::TooManyRequests()
        .insert_header((
            header::RETRY_AFTER,
            retry_after_secs(retry_after).to_string(),
        ))
        .finish()
}

/// `Retry-After` uses whole seconds; round **up** so a client never retries early.
fn retry_after_secs(d: Duration) -> u64 {
    d.as_secs() + u64::from(d.subsec_nanos() > 0)
}
