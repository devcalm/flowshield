//! Shared harness for the e2e suite: Redis gating, per-run key isolation, and helpers that
//! stand up real Axum/Actix servers on ephemeral ports.
//!
//! Each `tests/*.rs` binary includes this with `mod common;` and calls `common::…`. Because
//! every binary compiles the whole module but uses only part of it, `dead_code` is allowed
//! here (an unused helper in one binary is not a defect).
#![allow(dead_code)]

use flowshield::DynLimiter;
use std::net::SocketAddr;
use std::time::SystemTime;

// --- Redis gating (see also flowshield-redis/tests) ---------------------------------

/// The Redis URL under test, overridable for CI/compose. Defaults to the local server.
pub fn redis_url() -> String {
    std::env::var("FLOWSHIELD_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

/// Whether an unreachable server is a hard failure rather than a skip.
///
/// **Deliberately keyed on `FLOWSHIELD_REQUIRE_REDIS` only — not `CI`.** This crate is
/// compiled and run by the plain `test` CI job, where GitHub sets `CI=true` but no Redis
/// service exists; a `CI`-inclusive predicate would turn every gated test into a panic
/// there. The dedicated flag (set by the `integration` job) still forces a hard failure
/// where a server really is expected.
pub fn redis_required() -> bool {
    std::env::var("FLOWSHIELD_REQUIRE_REDIS").is_ok()
}

/// PING probe: can we actually reach a RESP server at `url`?
pub async fn reachable(url: &str) -> bool {
    let Ok(client) = redis::Client::open(url) else {
        return false;
    };
    let Ok(mut conn) = client.get_multiplexed_async_connection().await else {
        return false;
    };
    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .is_ok()
}

/// Gate a Redis-backed test. Returns `true` when the test should proceed; returns `false`
/// (after an explanatory note) when it should early-`return` because no server is present
/// and none is required. Panics when a server is required but unreachable.
pub async fn redis_ready(url: &str) -> bool {
    if reachable(url).await {
        return true;
    }
    if redis_required() {
        panic!("Redis required (FLOWSHIELD_REQUIRE_REDIS) but unreachable at {url}");
    }
    eprintln!("skipping: no Redis at {url} (run `docker compose up -d`)");
    false
}

/// A per-run-unique API-key value → a unique Redis key (keys flow through
/// `HeaderKeyExtractor`), so concurrent/sequential runs never collide on shared server
/// state. This is the HTTP analogue of `flowshield-redis`'s `unique_key`; no FLUSHDB.
pub fn unique_api_key(tag: &str) -> String {
    let n = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("fs-e2e:{tag}:{n}")
}

// --- HTTP client helper -------------------------------------------------------------

/// One GET to `http://{addr}/` carrying the rate-limit key as `x-api-key`.
pub async fn hit(client: &reqwest::Client, addr: SocketAddr, key: &str) -> reqwest::Response {
    client
        .get(format!("http://{addr}/"))
        .header("x-api-key", key)
        .send()
        .await
        .expect("request failed")
}

/// Parse a response's `x-ratelimit-remaining` header, if present.
pub fn remaining(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get("x-ratelimit-remaining")?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

// --- Axum server on an ephemeral port -----------------------------------------------

/// A running Axum server; abort the handle (or drop the guard) to stop it.
pub struct AxumServer {
    pub addr: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for AxumServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Bind `127.0.0.1:0`, read back the assigned port, and serve on a background task. The
/// limiter is keyed on the `x-api-key` header so several servers can share one Redis key
/// (and so tests can isolate themselves with a unique key value).
pub async fn spawn_axum(limiter: impl DynLimiter + 'static) -> AxumServer {
    use axum::{Router, routing::get};
    use flowshield::axum::{FlowShieldLayer, HeaderKeyExtractor};
    use http::HeaderName;

    let layer = FlowShieldLayer::new(
        limiter,
        HeaderKeyExtractor::new(HeaderName::from_static("x-api-key")),
    );
    let app = Router::new()
        .route("/", get(|| async { "ok" }))
        .layer(layer);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app.into_make_service()).await;
    });
    AxumServer { addr, handle }
}

// --- Actix server on an ephemeral port ----------------------------------------------

/// A running Actix server on its own dedicated thread (Actix workers need an Actix
/// `System`, which a bare `tokio::spawn` does not provide). Call [`ActixServer::stop`] to
/// shut it down gracefully and reap the thread.
pub struct ActixServer {
    pub addr: SocketAddr,
    handle: actix_web::dev::ServerHandle,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ActixServer {
    /// Gracefully stop the server and join its thread.
    pub async fn stop(mut self) {
        self.handle.stop(false).await;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Start an Actix server (keyed on `x-api-key`) on an ephemeral port with a single worker.
/// Blocks only until the port is bound and reported back over a channel.
pub fn spawn_actix(limiter: impl DynLimiter + 'static) -> ActixServer {
    spawn_actix_with_workers(limiter, 1)
}

/// Like [`spawn_actix`], but with `workers` worker threads. Several workers share one
/// underlying limiter/store (the middleware holds an `Arc<dyn DynLimiter>` and each
/// `mw.clone()` shares it), so this genuinely exercises concurrent contention on the store
/// — used by the stress suite to hammer the shard lock in parallel.
pub fn spawn_actix_with_workers(limiter: impl DynLimiter + 'static, workers: usize) -> ActixServer {
    use actix_web::{App, HttpResponse, HttpServer, web};
    use flowshield::actix::{FlowShield, HeaderKeyExtractor};

    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let sys = actix_web::rt::System::new();
        sys.block_on(async move {
            let mw = FlowShield::new(
                limiter,
                HeaderKeyExtractor::new(actix_web::http::header::HeaderName::from_static(
                    "x-api-key",
                )),
            );
            let server = HttpServer::new(move || {
                App::new().wrap(mw.clone()).route(
                    "/",
                    web::get().to(|| async { HttpResponse::Ok().body("ok") }),
                )
            })
            .workers(workers)
            .disable_signals()
            .bind(("127.0.0.1", 0))
            .expect("bind ephemeral port");

            let addr = server.addrs()[0];
            let server = server.run();
            tx.send((addr, server.handle())).expect("report addr");
            let _ = server.await;
        });
    });

    let (addr, handle) = rx.recv().expect("actix server failed to start");
    ActixServer {
        addr,
        handle,
        thread: Some(thread),
    }
}
