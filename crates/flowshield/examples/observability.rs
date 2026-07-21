//! Observability: Prometheus metrics + `tracing` spans (Phase 3, ADR-012).
//!
//! FlowShield **emits** telemetry but never configures where it goes — collection
//! and export are the host application's job (ADR-012). This example plays that host
//! role: it installs a Prometheus recorder and a `tracing` subscriber, drives a
//! little traffic through a limiter, and then prints what each pillar captured.
//!
//! Run it:
//!
//! ```text
//! cargo run --example observability --features metrics,tracing
//! ```
//!
//! What you will see:
//!
//! - **tracing**: one `flowshield.check` span per decision, each carrying
//!   `algorithm`, `backend`, and a **redacted** key digest (`k#…`) — never the raw
//!   key (FR-073). A `warn` event would appear here if the backend degraded.
//! - **metrics**: a Prometheus text exposition with `flowshield_decisions_total`
//!   (split by `outcome`), `flowshield_backend_duration_seconds`, and
//!   `flowshield_degradations_total`.
//!
//! # Wiring OpenTelemetry
//!
//! Because the spans above are ordinary `tracing` spans, exporting them to an OTel
//! collector needs **no FlowShield code** — you add a `tracing-opentelemetry` layer
//! to the subscriber below, exactly as you would for any other library's spans:
//!
//! ```text
//! let otel = tracing_opentelemetry::layer().with_tracer(my_tracer);
//! tracing_subscriber::registry().with(fmt_layer).with(otel).init();
//! ```
//!
//! That is the whole of FlowShield's OTel story: emit standard spans, let the host
//! route them.

use flowshield::{Key, LimiterBuilder, MemoryStore, TokenBucket};
use metrics_exporter_prometheus::PrometheusBuilder;
use tracing_subscriber::fmt::format::FmtSpan;

#[tokio::main]
async fn main() -> flowshield::Result<()> {
    // --- Host-owned collection setup (NOT part of FlowShield) ----------------

    // 1) A Prometheus recorder. `install_recorder` returns a handle we can render
    //    to the same text format a Prometheus server would scrape.
    let prometheus = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    // 2) A tracing subscriber that prints each span as it is created, so the
    //    per-decision `flowshield.check` spans (and their fields) are visible.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::NEW)
        .with_target(false)
        .init();

    // --- Ordinary FlowShield usage -------------------------------------------

    // Capacity 3, refilling 1/sec: the 4th back-to-back request is rejected, giving
    // us both an "allowed" and a "rejected" outcome to observe.
    let limiter =
        LimiterBuilder::new(TokenBucket::per_second(3, 1)?, MemoryStore::new()).build()?;
    let key = Key::from("192.168.0.42:secret-api-token"); // sensitive on purpose

    println!("-- driving traffic (watch the redacted key in the spans) --\n");
    for i in 1..=4 {
        let decision = limiter.check(&key).await?;
        println!(
            "  request {i}: {}",
            if decision.is_allowed() {
                "allowed"
            } else {
                "rejected"
            }
        );
    }

    // --- Show what the metrics pillar captured -------------------------------

    println!("\n-- Prometheus exposition --\n");
    print!("{}", prometheus.render());

    Ok(())
}
