// ABOUTME: Counts over an external transport: a FirstPartyOnly relay's workouts filtered out item by item, with a note
// ABOUTME: Dravr's own surfaces count every source; MCP counts the rest and says how many were unavailable there

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `query_activities` and `get_stats` across transports (carnet#1485).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The relay (`transport_gate_e2e_test`) is a coach platform whose terms keep
// its data inside Dravr's own surfaces. Its sessions are stored beside an
// upload of the athlete's own: a count over MCP serves the upload and notes
// the relay's sessions as unavailable there, never a whole refusal (OQ5).
#![cfg(all(
    feature = "tools-data",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::activity_query_test::{call_tool, noon, ride, store, Athlete, TOOL};
use crate::transport_gate_e2e_test::{athlete_with_relay, server_with_relay, sessions, RELAY};

use std::sync::Arc;

use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::transport::Transport;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};

/// The key of the note a caller over an external transport reads.
const UNAVAILABLE_HERE: &str = "_unavailable_over_this_interface";

/// An athlete with the relay connected, its two sessions stored, and one
/// upload of their own.
async fn relay_and_upload() -> (Arc<ServerContext>, Athlete) {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let athlete = (rider.user_id, rider.tenant);
    store(&resources, athlete, RELAY, &sessions()).await;
    store(
        &resources,
        athlete,
        oauth_providers::UPLOAD,
        &[ride(oauth_providers::UPLOAD, noon(2025, 3, 1))],
    )
    .await;
    (resources, athlete)
}

fn unavailable(payload: &Value) -> &Value {
    &payload[UNAVAILABLE_HERE]["items_unavailable"]
}

#[tokio::test]
async fn dravrs_own_app_counts_the_relays_workouts() {
    let (resources, athlete) = relay_and_upload().await;

    let payload = call_tool(&resources, Transport::MobileApp, athlete, TOOL, json!({})).await;

    assert_eq!(
        payload["totals"]["count"], 3,
        "bug: the app's own surface does not count the relay's sessions: {payload}"
    );
    assert!(
        payload.get(UNAVAILABLE_HERE).is_none(),
        "bug: the app's own surface is told something is unavailable: {payload}"
    );
}

#[tokio::test]
async fn mcp_counts_the_rest_and_notes_what_it_may_not_see() {
    let (resources, athlete) = relay_and_upload().await;

    for (tool, count) in [
        (TOOL, "/totals/count"),
        ("get_stats", "/stats/total_activities"),
    ] {
        let payload = call_tool(&resources, Transport::McpHttp, athlete, tool, json!({})).await;
        assert_eq!(
            payload.pointer(count),
            Some(&json!(1)),
            "bug: {tool} over MCP counts other than the athlete's own upload: {payload}"
        );
        assert_eq!(
            unavailable(&payload),
            2,
            "bug: {tool} over MCP does not say two workouts were unavailable there: {payload}"
        );
        let text = payload.to_string();
        assert!(
            !text.contains("Garmin Hills") && !text.contains("Garmin Tempo"),
            "bug: {tool} over MCP names a relay session: {text}"
        );
    }
}
