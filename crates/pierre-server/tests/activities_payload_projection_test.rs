// ABOUTME: get_activities answers the same window three times over — only one copy reaches the model
// ABOUTME: The projection has to shrink the payload AND keep every field a chained activity_id call needs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_llm::{ChatMessage, FunctionResponse};
use pierre_tool_runtime::tool_execution::add_function_responses_to_messages;
use pierre_tool_runtime::tool_results::{format_tool_results_as_text, project_activities_payload};
use serde_json::{json, Value};

/// A realistic `get_activities` envelope: the prose block the coach cites, the
/// structured array, the TOON copy, and the two sidecars — the shape
/// `build_activities_success_response` actually emits.
fn activities_envelope(rows: usize) -> Value {
    let activities: Vec<Value> = (0..rows)
        .map(|i| {
            json!({
                "id": format!("strava-{i}"),
                "name": format!("Morning Run {i}"),
                "sport_type": "run",
                "start_date": "2026-08-20T11:04:00Z",
                "start_date_local": "2026-08-20T07:04:00-04:00",
                "distance_meters": 12_345.6,
                "duration_seconds": 3_600,
                "elevation_gain_meters": 210.5,
                "average_heartrate": 148,
                "max_heartrate": 171,
                "average_speed": 3.42,
                "calories": 780,
                "description": "Felt strong through the back half, negative split.",
            })
        })
        .collect();

    json!({
        "activity_list": "2026-08-20 · Morning Run 0 · 12.3 km · 1h00 · 210 m\n",
        "activities": activities,
        "activities_toon": "id,name,sport_type\nstrava-0,Morning Run 0,run\n".repeat(rows),
        "provider": "strava",
        "count": rows,
        "mode": "summary",
        "format": "json",
        "offset": 0,
        "limit": 30,
        "has_more": true,
        "coverage": { "window_total": 552, "showing": rows },
        "token_estimate": { "tokens": 4_200, "chars": 16_800 },
        "retrieval_context": {
            "sufficiency": "adequate",
            "fragment_report": { "merged": 3, "note": "count sessions, not rows" },
        },
    })
}

#[test]
fn the_projection_keeps_every_field_a_chained_call_needs() {
    let projected = project_activities_payload("get_activities", &activities_envelope(3))
        .expect("a real get_activities envelope is recognised");

    // Five registered tools take a required `activity_id` and resolve it via
    // provider.get_activity(...) with no name-or-index fallback. Losing the id
    // would leave the coach able to describe a session and unable to analyse it.
    let rows = projected["activities"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        3,
        "every activity survives — this is not truncation"
    );
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row["id"], json!(format!("strava-{i}")));
        assert_eq!(row["name"], json!(format!("Morning Run {i}")));
        assert_eq!(row["sport_type"], json!("run"));
        assert_eq!(row["start_date"], json!("2026-08-20T11:04:00Z"));
    }

    // The prose is what the coach cites, and the pagination scalars are what
    // the tool's own schema promises for a follow-up request.
    assert!(projected["activity_list"]
        .as_str()
        .unwrap()
        .contains("Morning Run 0"));
    assert_eq!(projected["provider"], json!("strava"));
    assert_eq!(projected["count"], json!(3));
    assert_eq!(projected["has_more"], json!(true));
    assert_eq!(projected["coverage"]["window_total"], json!(552));
}

#[test]
fn the_projection_drops_the_duplicate_copies_and_the_sidecars() {
    let projected = project_activities_payload("get_activities", &activities_envelope(3)).unwrap();

    assert!(
        projected.get("activities_toon").is_none(),
        "the TOON copy is a second rendering of the same window"
    );
    assert!(projected.get("token_estimate").is_none());
    assert!(projected.get("retrieval_context").is_none());

    // Per-activity detail bodies go; the addressing fields stay.
    let row = &projected["activities"][0];
    for dropped in [
        "distance_meters",
        "average_heartrate",
        "description",
        "calories",
    ] {
        assert!(
            row.get(dropped).is_none(),
            "{dropped} should not survive projection"
        );
    }
}

#[test]
fn a_thirty_activity_window_shrinks_by_more_than_half() {
    let full = activities_envelope(30);
    let projected = project_activities_payload("get_activities", &full).unwrap();

    let before = serde_json::to_string(&full).unwrap().len();
    let after = serde_json::to_string(&projected).unwrap().len();

    assert!(
        after * 2 < before,
        "projection should more than halve a 30-activity window; {before} -> {after}"
    );
}

#[test]
fn any_other_tool_passes_through_untouched() {
    let payload = json!({ "total_distance_km": 1234.5, "activity_list": "not an activities call" });
    assert!(
        project_activities_payload("get_stats", &payload).is_none(),
        "only get_activities is projected"
    );
}

#[test]
fn an_unrecognised_shape_passes_through_untouched() {
    // No `activity_list` — an error envelope, or a future rewrite. The reducer
    // must never be the reason a coach ends up with no data.
    let error_shape = json!({ "error": "provider unavailable", "activities": [] });
    assert!(project_activities_payload("get_activities", &error_shape).is_none());

    // `activity_list` present but not a string is equally unrecognised.
    let wrong_type = json!({ "activity_list": ["a", "b"], "activities": [] });
    assert!(project_activities_payload("get_activities", &wrong_type).is_none());
}

#[test]
fn the_api_loop_injects_the_projected_payload_not_the_whole_envelope() {
    let mut messages: Vec<ChatMessage> = Vec::new();
    let responses = vec![FunctionResponse {
        name: "get_activities".to_owned(),
        response: activities_envelope(30),
    }];

    let added = add_function_responses_to_messages(&mut messages, &responses);

    let injected = &messages.last().unwrap().content;
    assert!(injected.contains("strava-0"), "ids reach the model");
    assert!(injected.contains("Morning Run 0"));
    assert!(
        !injected.contains("retrieval_context"),
        "the sidecar must not reach the prompt"
    );
    assert!(
        !injected.contains("activities_toon"),
        "the duplicate rendering must not reach the prompt"
    );
    assert!(
        !injected.contains("Felt strong through the back half"),
        "per-activity detail bodies must not reach the prompt"
    );

    // The prepended list is unaffected — it is read off the ORIGINAL response,
    // so projecting the injected copy cannot cost the reply its activity list.
    assert!(added.activity_list.is_some());
}

#[test]
fn the_text_loop_and_the_recovery_reask_share_the_same_projection() {
    let responses = vec![FunctionResponse {
        name: "get_activities".to_owned(),
        response: activities_envelope(30),
    }];

    // embacle pretty-prints these blocks, so this seam was strictly more
    // expensive than the API loop's compact serialization for identical data.
    let text = format_tool_results_as_text(&responses);
    assert!(text.contains("<tool_result name=\"get_activities\">"));
    assert!(
        text.contains("strava-0"),
        "ids survive the text rendering too"
    );
    assert!(!text.contains("retrieval_context"));
    assert!(!text.contains("activities_toon"));
    assert!(!text.contains("Felt strong through the back half"));
}
