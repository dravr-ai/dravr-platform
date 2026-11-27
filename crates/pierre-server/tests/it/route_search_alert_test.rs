// ABOUTME: The unavailable route search route_search_monitoring.tf alerts on is the one line the Overpass walk logs, word for word
// ABOUTME: Runs a walk against a refused instance and pins the athlete-facing error, the tool's text and the structured warning
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A log-based alert reads the words of a log line. Reworded on one side only,
//! the alert never fires and nothing says so, so the test reads the filter out
//! of the Terraform and makes the walk log the line.

use crate::common;
use dravr_arpenteur::{OverpassConfig, RouteDiscoveryService};
use pierre_core::constants::project;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::http_client::api_inner_client;
use pierre_core::models::SportType;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_test_support::log_capture;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use regex::Regex;
use serde_json::json;
use tokio::net::TcpListener;
use tracing::Level;
use uuid::Uuid;

/// The `event` value the metric in `route_search_monitoring.tf` counts.
fn alerted_event() -> String {
    // Read on purpose: a parity check between the log line and the Cloud
    // Monitoring filter that matches it, two artefacts nothing at runtime joins.
    let terraform = include_str!("../../../../infra/environments/dev/route_search_monitoring.tf");
    let filter = Regex::new(
        r#"(?s)resource "google_logging_metric" "route_search_unavailable" \{.*?jsonPayload\.event="([^"]+)""#,
    )
    .unwrap();
    filter
        .captures(terraform)
        .expect("route_search_monitoring.tf has no event filter on route_search_unavailable")[1]
        .to_owned()
}

/// A loopback origin nothing listens on, this test's alone: a failed walk
/// holds its answer for the instances it asked, so a walk sharing the
/// suite's refused origin could be answered from another test's hold and
/// log no counted line.
async fn closed_loopback_origin() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    origin
}

#[tokio::test]
async fn an_unavailable_route_search_logs_the_one_line_the_alert_counts() {
    let wanted = alerted_event();
    // A refused origin: the one instance fails at once and nothing leaves the
    // machine. A distinct point keeps the process-wide route cache out of it.
    let origin = closed_loopback_origin().await;
    let service = RouteDiscoveryService::new(OverpassConfig {
        origin: Some(origin),
        user_agent: project::user_agent(),
        client: api_inner_client().clone(),
    });

    let (log, guard) = log_capture::capture_events();
    let err = service
        .discover_routes_for_sport(&SportType::Run, 12.345, -67.891, None, None)
        .await
        .map_err(AppError::from)
        .expect_err("a refused instance answers no route");
    drop(guard);

    assert_eq!(
        err.code,
        ErrorCode::ExternalServiceUnavailable,
        "bug: a walk nobody answered is the unavailable error"
    );
    assert!(
        // number-substring-ok: the instance's address must not sit anywhere in the text
        !err.message.contains("127.0.0.1")
            && !err.message.contains("connection")
            && !err.message.contains("overloaded"),
        "bug: the athlete-facing error names the instance or its failure: {}",
        err.message
    );

    let counted: Vec<_> = log
        .drain()
        .into_iter()
        .filter(|event| event.field("event") == Some(wanted.as_str()))
        .collect();
    assert_eq!(
        counted.len(),
        1,
        "bug: one unavailable search must log exactly one line with event {wanted:?}"
    );
    let line = &counted[0];
    assert_eq!(
        line.level,
        Level::WARN,
        "bug: the counted line is a warning"
    );
    assert_eq!(
        line.field("cause"),
        Some("failed"),
        "bug: a refused connection is not logged as an overload"
    );
    let outcomes = line.field("outcomes").unwrap_or_default();
    assert!(
        outcomes.contains("127.0.0.1") && outcomes.contains("connection failed"),
        "bug: the per-instance detail goes to the log: {outcomes:?}"
    );
}

/// What the model relaying `discover_routes` reads when no instance answers
/// and no loop of the athlete's answers either: the walk's own words with the
/// tool's guidance after them, and still no host, status code or timing.
#[tokio::test]
async fn the_tool_text_an_athlete_reads_names_no_host_status_or_timing() {
    let resources = common::create_test_server_resources().await.unwrap();
    // No tenant: no loop of the athlete's is read, so the walk's failure,
    // on the suite's refused Overpass origin, is the tool's answer.
    let refused = UniversalToolExecutor::new(resources)
        .with_scopes(OAuthScope::self_grant())
        .execute_tool(UniversalRequest {
            tool_name: "discover_routes".to_owned(),
            parameters: json!({
                "latitude": 12.346,
                "longitude": -67.892,
                "sport_type": "run"
            }),
            user_id: Uuid::new_v4().to_string(),
            protocol: "test".to_owned(),
            tenant_id: None,
        })
        .await
        .expect_err("a refused instance and no loop answer no route");
    let text = refused.to_string();

    assert!(
        text.contains("OpenStreetMap route search")
            && text.contains("none of the athlete's own loops")
            && text.contains("get_weather_forecast")
            && text.contains("do not suggest routes from memory"),
        "bug: the tool keeps the walk's words and its own guidance: {text}"
    );
    // number-substring-ok: an address, a status or a duration must not sit anywhere in the text
    for technical in [
        "127.0.0.1",
        "HTTP",
        "Overpass",
        "connection",
        "within",
        "not tried",
    ] {
        assert!(
            !text.contains(technical),
            "bug: the athlete-facing text names {technical:?}: {text}"
        );
    }
    let timing = Regex::new(r"\d+\s?(ms|s)\b").unwrap();
    assert!(
        !timing.is_match(&text),
        "bug: the athlete-facing text carries a timing: {text}"
    );
}
