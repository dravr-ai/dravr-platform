// ABOUTME: Unit tests for the admin tool-usage aggregation over llm_usage rows
// ABOUTME: Pins per-tool invocation, turn and latency counts, ordering and the empty window

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::usage::LlmUsageRecord;
use pierre_core::models::ConversationTurnId;
use pierre_routes_admin::handlers::llm_consumption::LlmConsumptionRoutes;
use uuid::Uuid;

fn rec(turn: u128, tools: &[&str], latency: Option<i64>) -> LlmUsageRecord {
    LlmUsageRecord {
        id: "id".to_owned(),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        conversation_id: None,
        turn_id: ConversationTurnId::from_uuid(Uuid::from_u128(turn)),
        provider: "google".to_owned(),
        model: "gemini".to_owned(),
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        cached_tokens: 0,
        cached_write_tokens: 0,
        reasoning_tokens: 0,
        call_type: "chat".to_owned(),
        tool_calls_count: i64::try_from(tools.len()).unwrap_or(0),
        tools_called: serde_json::to_string(tools).unwrap_or_else(|_| "[]".to_owned()),
        execution_time_ms: latency,
        cost_usd: 0.0,
        call_sequence: Some(1),
        created_at: "2026-06-06T00:00:00Z".to_owned(),
    }
}

#[test]
fn aggregates_per_tool_counts_turns_and_latency() {
    // turn 1: two calls (discover_routes+weather @100, discover_routes @200)
    // turn 2: one call (discover_routes @300)
    let rows = vec![
        rec(1, &["discover_routes", "get_weather_forecast"], Some(100)),
        rec(1, &["discover_routes"], Some(200)),
        rec(2, &["discover_routes"], Some(300)),
    ];
    let resp = LlmConsumptionRoutes::build_tool_usage_response(&rows, 30);

    assert_eq!(resp.days, 30);
    assert_eq!(resp.summary.turns_with_tools, 2);
    assert_eq!(resp.summary.unique_tools, 2);
    assert_eq!(resp.summary.total_invocations, 4);

    let discover = resp
        .breakdown
        .iter()
        .find(|i| i.tool_name == "discover_routes");
    let weather = resp
        .breakdown
        .iter()
        .find(|i| i.tool_name == "get_weather_forecast");

    // discover_routes: 3 invocations across 2 turns, avg (100+200+300)/3.
    assert!(
        discover.is_some_and(|d| d.invocation_count == 3
            && d.turn_count == 2
            && d.avg_latency_ms == Some(200)),
        "discover_routes breakdown wrong: {discover:?}"
    );
    // get_weather_forecast: 1 invocation, 1 turn, avg 100.
    assert!(
        weather.is_some_and(|w| w.invocation_count == 1
            && w.turn_count == 1
            && w.avg_latency_ms == Some(100)),
        "get_weather_forecast breakdown wrong: {weather:?}"
    );

    // Most-used tool sorts first.
    assert_eq!(
        resp.breakdown.first().map(|i| i.tool_name.as_str()),
        Some("discover_routes")
    );
}

#[test]
fn empty_rows_yield_empty_breakdown() {
    let resp = LlmConsumptionRoutes::build_tool_usage_response(&[], 7);
    assert_eq!(resp.summary.total_invocations, 0);
    assert_eq!(resp.summary.unique_tools, 0);
    assert_eq!(resp.summary.turns_with_tools, 0);
    assert!(resp.breakdown.is_empty());
    assert_eq!(resp.days, 7);
}
