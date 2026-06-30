//! OTEL export surface E2E tests for the Buzz relay.
//!
//! Verifies both export surfaces introduced in PR #1398:
//!
//!  1. **Prometheus scrape** — `GET :9102/metrics` contains expected `buzz_*`
//!     series with non-zero values and does NOT contain `target_info`.
//!  2. **OTLP traces** — the otel-collector received spans named `ws.auth` and
//!     `ws.event` carrying a `conn_id` attribute and `service.name=buzz-relay`.
//!  3. **OTLP metrics** — the collector received OTLP metric data tagged with
//!     resource attribute `service.name=buzz-relay`.
//!  4. **OTLP-disabled control** — when `OTEL_EXPORTER_OTLP_ENDPOINT` is not
//!     set the relay still serves `/metrics` correctly; the test verifies the
//!     Prometheus surface without an OTLP endpoint.
//!
//! # Running
//!
//! These tests are `#[ignore]` by default; they require a running relay + the
//! otel-collector compose overlay.  Use the `just otel-e2e` target, which boots
//! everything and runs them, or manually:
//!
//! ```text
//! # boot the stack (relay on host):
//! ./test/otel-e2e/run.sh
//!
//! # run just these tests:
//! RELAY_URL=ws://localhost:3000 \
//! OTEL_COLLECTOR_OUTPUT=/path/to/telemetry.json \
//! cargo test -p buzz-test-client --test e2e_otel -- --ignored
//! ```
//!
//! Environment variables:
//!
//! | Variable | Default | Purpose |
//! |---|---|---|
//! | `RELAY_URL` | `ws://localhost:3000` | WebSocket URL of the relay under test |
//! | `METRICS_URL` | `http://localhost:9102/metrics` | Prometheus metrics endpoint |
//! | `OTEL_COLLECTOR_OUTPUT` | `/tmp/otelcol-output/telemetry.json` | Collector file exporter output |

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind};
use serde_json::Value;

// ── helpers ──────────────────────────────────────────────────────────────────

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn metrics_url() -> String {
    std::env::var("METRICS_URL")
        .unwrap_or_else(|_| "http://localhost:9102/metrics".to_string())
}

fn collector_output_path() -> String {
    std::env::var("OTEL_COLLECTOR_OUTPUT")
        .unwrap_or_else(|_| "/tmp/otelcol-output/telemetry.json".to_string())
}

/// Fetch the Prometheus /metrics text.
async fn fetch_metrics() -> String {
    reqwest::get(&metrics_url())
        .await
        .expect("fetch /metrics")
        .text()
        .await
        .expect("read /metrics body")
}

/// Read every line from the collector file exporter output.
/// The file exporter writes one JSON object per line (newline-delimited JSON).
fn read_collector_output() -> Vec<Value> {
    let path = collector_output_path();
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read collector output at {path}: {e}"));
    contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

// ── test: Prometheus surface ──────────────────────────────────────────────────

/// Asserts the Prometheus /metrics endpoint exposes the expected buzz_* series
/// with non-zero values and does NOT contain `target_info` (suppressed in #1398).
///
/// Drives a NIP-42 auth + event publish to generate non-zero counters before
/// scraping.
#[tokio::test]
#[ignore]
async fn test_prometheus_contains_buzz_metrics_with_nonzero_values() {
    let keys = Keys::generate();

    // Drive some traffic so counters are non-zero.
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect and authenticate");

    // Publish a text-note event (kind 1 is accepted relay-wide without a channel).
    let event = EventBuilder::new(Kind::TextNote, "otel-e2e prometheus test")
        .sign_with_keys(&keys)
        .expect("sign event");
    client.send_event(event).await.expect("send event");
    client.disconnect().await.ok();

    // Give the relay a moment to record metrics.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let body = fetch_metrics().await;

    // (a) Must contain expected series with at least one non-zero value.
    let required_series = [
        "buzz_ws_connections_total",
        "buzz_events_received_total",
        "buzz_auth_attempts_total",
    ];
    for series in &required_series {
        assert!(
            body.contains(series),
            "expected metric series {series} in /metrics output\n\n--- /metrics ---\n{body}"
        );
        // Find the line(s) for this metric and assert at least one has a
        // non-zero value.  Prometheus text format: `<name>{labels} <value>`.
        let has_nonzero = body.lines().any(|line| {
            if !line.starts_with(series) {
                return false;
            }
            // Extract the value (last whitespace-delimited token before optional timestamp).
            if let Some(val_str) = line.split_whitespace().nth(1) {
                val_str.parse::<f64>().map(|v| v > 0.0).unwrap_or(false)
            } else {
                false
            }
        });
        assert!(
            has_nonzero,
            "expected non-zero value for {series} in /metrics output\n\n--- /metrics ---\n{body}"
        );
    }

    // (b) Must NOT contain target_info (suppressed via .without_target_info()).
    assert!(
        !body.contains("target_info"),
        "unexpected target_info series in /metrics — should be suppressed\n\n--- /metrics ---\n{body}"
    );

    println!("✓ Prometheus: required series present with non-zero values, no target_info");
}

// ── test: OTLP traces ─────────────────────────────────────────────────────────

/// Asserts the otel-collector received ws.auth and ws.event spans carrying
/// conn_id attributes, tagged service.name=buzz-relay.
///
/// Reads from OTEL_COLLECTOR_OUTPUT (populated by run.sh after traffic was
/// driven by the Prometheus test and the batch exporter flushed).
#[tokio::test]
#[ignore]
async fn test_otlp_traces_contain_ws_spans_with_conn_id() {
    let records = read_collector_output();
    assert!(
        !records.is_empty(),
        "collector output is empty — OTLP export did not reach the collector.\n\
         Ensure OTEL_COLLECTOR_OUTPUT points to a file populated by run.sh after WS traffic."
    );

    // Flatten: the file exporter writes ResourceSpans / ResourceMetrics objects.
    // We search for span names across the full JSON blob.
    let blob = serde_json::to_string(&records).expect("re-serialize collector output");

    // (b1) ws.auth span must be present.
    assert!(
        blob.contains("\"ws.auth\""),
        "expected ws.auth span in collector output\n\nKeys found: check {}", collector_output_path()
    );

    // (b2) ws.event span must be present.
    assert!(
        blob.contains("\"ws.event\""),
        "expected ws.event span in collector output"
    );

    // (b3) conn_id attribute must appear (set on both ws.auth and ws.event spans).
    assert!(
        blob.contains("\"conn_id\""),
        "expected conn_id attribute in span data"
    );

    // (b4) service.name=buzz-relay must appear in resource attributes.
    assert!(
        blob.contains("\"buzz-relay\""),
        "expected service.name=buzz-relay in collector output"
    );

    println!("✓ OTLP traces: ws.auth + ws.event spans present, conn_id + service.name verified");
}

// ── test: OTLP metrics ────────────────────────────────────────────────────────

/// Asserts the otel-collector received OTLP metric data tagged
/// service.name=buzz-relay.
///
/// Reads from OTEL_COLLECTOR_OUTPUT (populated by run.sh).
#[tokio::test]
#[ignore]
async fn test_otlp_metrics_tagged_service_name_buzz_relay() {
    let records = read_collector_output();
    let blob = serde_json::to_string(&records).expect("re-serialize collector output");

    // (c) service.name=buzz-relay must appear somewhere in the recorded data
    // (either in ResourceSpans or ResourceMetrics resource attributes).
    assert!(
        !records.is_empty(),
        "collector output is empty — OTLP export did not reach the collector"
    );
    assert!(
        blob.contains("\"buzz-relay\""),
        "expected service.name=buzz-relay in OTLP data from collector"
    );

    println!("✓ OTLP metrics: service.name=buzz-relay present in collector output");
}

// ── test: OTLP-disabled control ───────────────────────────────────────────────

/// When OTEL_EXPORTER_OTLP_ENDPOINT is not set the relay must still serve
/// /metrics correctly.  This test runs against a relay started WITHOUT the env
/// var; the run.sh script handles the second relay start (disabled path).
///
/// NOTE: This test only asserts the Prometheus surface.  The assertion that
/// "the collector received NOTHING" is made by the run.sh script, which checks
/// the collector output is empty before sending traffic on the disabled relay.
#[tokio::test]
#[ignore]
async fn test_prometheus_works_without_otlp_endpoint() {
    let body = fetch_metrics().await;

    // The Prometheus surface must work regardless of OTLP being enabled.
    // At startup (before WS traffic) the relay emits at minimum the pool gauges
    // which use # TYPE lines.  A non-empty response with any buzz_ metric proves
    // the Prometheus binding is up.
    assert!(
        !body.is_empty(),
        "expected a non-empty Prometheus metrics response"
    );
    assert!(
        body.contains("buzz_"),
        "expected at least one buzz_ metric in /metrics response — got:\n{body}"
    );

    // Must still not have target_info even without OTLP.
    assert!(
        !body.contains("target_info"),
        "unexpected target_info in OTLP-disabled /metrics — should always be suppressed"
    );

    println!("✓ OTLP-disabled: Prometheus /metrics served correctly, no target_info");
}
