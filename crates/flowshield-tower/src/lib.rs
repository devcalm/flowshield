#![forbid(unsafe_code)]
//! # FlowShield — Tower middleware
//!
//! `flowshield-tower` is FlowShield's foundational HTTP integration (ADR-009): a Tower
//! [`Layer`]/[`Service`] that rate-limits requests by a key derived from each request,
//! emitting a standards-correct `429 Too Many Requests` with a `Retry-After` header on
//! rejection and passing allowed requests through untouched. Axum builds on this crate;
//! any `tower`/`hyper` stack can use it directly.
//!
//! ```no_run
//! use flowshield_tower::FlowShieldLayer;
//! use flowshield_core::algorithm::token_bucket::TokenBucket;
//! use flowshield_core::limiter::LimiterBuilder;
//! use flowshield_memory::MemoryStore;
//!
//! # fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let limiter = LimiterBuilder::new(TokenBucket::per_second(100, 20)?, MemoryStore::new()).build()?;
//! // Rate-limit by client peer address (the safe default — see `PeerAddrKeyExtractor`).
//! let layer = FlowShieldLayer::per_peer(limiter);
//! // `ServiceBuilder::new().layer(layer)` in front of your Tower service.
//! # Ok(())
//! # }
//! ```
//!
//! ## How the limiter is held
//!
//! The layer stores an [`Arc<dyn DynLimiter>`](flowshield_core::composite::DynLimiter),
//! so one concrete middleware type serves every algorithm/backend combination without
//! threading their generics through your service stack. The per-request box is
//! negligible next to a real (often networked) limiter check.
//!
//! ## Keying — and a security note
//!
//! The rate-limit key comes from an application-supplied [`KeyExtractor`]. The
//! provided defaults:
//!
//! - [`PeerAddrKeyExtractor`] (the default) — keys on the **direct peer IP**, read from
//!   a `SocketAddr` request extension the server must inject. This is the safe default.
//! - [`HeaderKeyExtractor`] — keys on a named header (e.g. an API key).
//! - [`ForwardedForKeyExtractor`] — keys on `X-Forwarded-For`. **Only use this behind a
//!   trusted proxy that overwrites the header**, or clients trivially spoof their key
//!   and evade the limit.

use flowshield_core::composite::DynLimiter;
use flowshield_core::key::{Key, KeyExtractor};
use http::{HeaderName, HeaderValue, Request, Response, StatusCode, header};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tower_layer::Layer;
use tower_service::Service;

/// The `X-RateLimit-Remaining` response header: how many further requests the caller
/// may make right now. Emitted on allowed responses when the algorithm reports it, and
/// set to `0` on a `429`.
const X_RATELIMIT_REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");

/// Keys a request on the **direct peer IP address**, read from a [`SocketAddr`] stored
/// in the request's extensions (which the server/connector must inject). This is the
/// safe default: unlike `X-Forwarded-For`, the peer address cannot be spoofed by the
/// client.
///
/// If no `SocketAddr` extension is present (a misconfigured server), all such requests
/// fall into one shared `"unknown-peer"` bucket — they are limited *together* rather
/// than silently unlimited.
#[derive(Debug, Clone, Default)]
pub struct PeerAddrKeyExtractor;

impl<B> KeyExtractor<Request<B>> for PeerAddrKeyExtractor {
    fn extract_key(&self, req: &Request<B>) -> Key {
        req.extensions()
            .get::<SocketAddr>()
            .map(|addr| Key::from(addr.ip().to_string()))
            .unwrap_or_else(|| Key::from("unknown-peer"))
    }
}

/// Keys a request on the value of a named header — e.g. an API key or tenant id.
///
/// Requests missing the header (or whose value is not valid UTF-8) share a single
/// fallback bucket, so a missing credential is limited rather than unlimited.
#[derive(Debug, Clone)]
pub struct HeaderKeyExtractor {
    header: HeaderName,
    fallback: Key,
}

impl HeaderKeyExtractor {
    /// Extract the key from `header` (e.g. `x-api-key`).
    pub fn new(header: HeaderName) -> Self {
        Self {
            header,
            fallback: Key::from("unknown-header"),
        }
    }
}

impl<B> KeyExtractor<Request<B>> for HeaderKeyExtractor {
    fn extract_key(&self, req: &Request<B>) -> Key {
        req.headers()
            .get(&self.header)
            .and_then(|v| v.to_str().ok())
            .map(Key::from)
            .unwrap_or_else(|| self.fallback.clone())
    }
}

/// Keys a request on the client IP from the `X-Forwarded-For` header (the first,
/// left-most entry).
///
/// # Security
///
/// `X-Forwarded-For` is set by clients and proxies and is **trivially spoofable**. Use
/// this extractor **only** when your service sits behind a trusted reverse proxy that
/// *overwrites* (not appends to) the header with the real client address. Exposed
/// directly, a client can pick any key and evade its limit. When in doubt, prefer
/// [`PeerAddrKeyExtractor`].
#[derive(Debug, Clone, Default)]
pub struct ForwardedForKeyExtractor;

impl<B> KeyExtractor<Request<B>> for ForwardedForKeyExtractor {
    fn extract_key(&self, req: &Request<B>) -> Key {
        // `X-Forwarded-For` is a comma-separated list of IPs; the left-most is the
        // originating client. Read *only* that header — the RFC 7239 `Forwarded`
        // header is a different grammar (`for=1.2.3.4;proto=https;by=…`) and must not
        // be parsed as a bare IP list, or the key becomes `for=1.2.3.4;proto=…` noise
        // and the same client scatters across buckets.
        req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(Key::from)
            .unwrap_or_else(|| Key::from("unknown-forwarded"))
    }
}

/// A Tower [`Layer`] that wraps a service with FlowShield rate limiting.
///
/// Construct it with [`FlowShieldLayer::per_peer`] (peer-IP keying) or
/// [`FlowShieldLayer::new`] with a custom [`KeyExtractor`].
pub struct FlowShieldLayer<K> {
    limiter: Arc<dyn DynLimiter>,
    extractor: Arc<K>,
}

impl<K> FlowShieldLayer<K> {
    /// Build a layer from any limiter and a key-extraction strategy.
    pub fn new(limiter: impl DynLimiter + 'static, extractor: K) -> Self {
        Self {
            limiter: Arc::new(limiter),
            extractor: Arc::new(extractor),
        }
    }
}

impl FlowShieldLayer<PeerAddrKeyExtractor> {
    /// Build a layer that rate-limits by client peer IP ([`PeerAddrKeyExtractor`]).
    pub fn per_peer(limiter: impl DynLimiter + 'static) -> Self {
        Self::new(limiter, PeerAddrKeyExtractor)
    }
}

// Manual `Clone`: the derive would demand `K: Clone`, but both fields are `Arc`, so the
// layer clones regardless of `K` (Tower clones layers/services freely).
impl<K> Clone for FlowShieldLayer<K> {
    fn clone(&self) -> Self {
        Self {
            limiter: self.limiter.clone(),
            extractor: self.extractor.clone(),
        }
    }
}

impl<S, K> Layer<S> for FlowShieldLayer<K> {
    type Service = FlowShieldService<S, K>;

    fn layer(&self, inner: S) -> Self::Service {
        FlowShieldService {
            inner,
            limiter: self.limiter.clone(),
            extractor: self.extractor.clone(),
        }
    }
}

/// The [`Service`] produced by [`FlowShieldLayer`]. Checks the limiter before delegating
/// to the inner service, returning `429` on rejection.
pub struct FlowShieldService<S, K> {
    inner: S,
    limiter: Arc<dyn DynLimiter>,
    extractor: Arc<K>,
}

impl<S: Clone, K> Clone for FlowShieldService<S, K> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            limiter: self.limiter.clone(),
            extractor: self.extractor.clone(),
        }
    }
}

impl<S, K, ReqBody, ResBody> Service<Request<ReqBody>> for FlowShieldService<S, K>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    K: KeyExtractor<Request<ReqBody>> + 'static,
    ReqBody: Send + 'static,
    ResBody: Default + Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // Readiness gotcha (Tower): `poll_ready` was called on `self.inner`, but the
        // clone we move into the future would not carry that readiness. Swap so the
        // *ready* inner goes into the future and a fresh clone stays behind.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let limiter = self.limiter.clone();
        let extractor = self.extractor.clone();

        Box::pin(async move {
            let key = extractor.extract_key(&req);
            match limiter.check_dyn(&key).await {
                Ok(decision) if decision.is_allowed() => {
                    // Pass through, annotating the downstream response with the caller's
                    // remaining headroom when the algorithm can report it (FR-011).
                    let remaining = decision.remaining();
                    let mut response = inner.call(req).await?;
                    if let Some(remaining) = remaining {
                        response
                            .headers_mut()
                            .insert(X_RATELIMIT_REMAINING, HeaderValue::from(remaining));
                    }
                    Ok(response)
                }
                Ok(decision) => {
                    // Normal rejection → 429 with the reported wait (ADR-009, FR-011).
                    let retry = decision.retry_after().unwrap_or(Duration::from_secs(1));
                    Ok(too_many_requests(retry))
                }
                // A runtime `Err` is only a config error (backend failures already
                // degrade to a decision, ADR-007); reject conservatively rather than
                // leak a limiter internal into the HTTP layer.
                Err(_) => Ok(too_many_requests(Duration::from_secs(1))),
            }
        })
    }
}

/// Build a `429 Too Many Requests` response carrying a `Retry-After` header.
fn too_many_requests<B: Default>(retry_after: Duration) -> Response<B> {
    let mut response = Response::new(B::default());
    *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
    let headers = response.headers_mut();
    headers.insert(
        header::RETRY_AFTER,
        HeaderValue::from(retry_after_secs(retry_after)),
    );
    // A rejected request has, by definition, no headroom left this instant.
    headers.insert(X_RATELIMIT_REMAINING, HeaderValue::from(0u64));
    response
}

/// `Retry-After` uses whole seconds; round **up** so a client never retries early.
fn retry_after_secs(d: Duration) -> u64 {
    // `saturating_add` so a near-`u64::MAX`-seconds wait cannot overflow to `0` (which
    // would emit `Retry-After: 0` and invite an immediate retry) or panic in debug.
    d.as_secs().saturating_add(u64::from(d.subsec_nanos() > 0))
}

#[cfg(test)]
mod retry_after_tests {
    use super::retry_after_secs;
    use std::time::Duration;

    #[test]
    fn rounds_sub_second_up_and_saturates_without_zero() {
        assert_eq!(retry_after_secs(Duration::from_millis(1)), 1); // sub-second rounds UP
        assert_eq!(retry_after_secs(Duration::from_secs(3)), 3); // whole seconds unchanged
        // Regression: a near-u64::MAX wait must not overflow to 0 (immediate-retry) or panic.
        let huge = Duration::new(u64::MAX, 999_999_999);
        assert_eq!(retry_after_secs(huge), u64::MAX);
    }
}
