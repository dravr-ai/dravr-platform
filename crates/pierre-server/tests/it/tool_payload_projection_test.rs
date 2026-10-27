// ABOUTME: The plan-building tools' results are cut before the model reads them, and stay under a byte budget
// ABOUTME: One worst-case fixture per tool: size, field allowlist, chained ids, omitted marker, determinism

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_llm::FunctionResponse;
use pierre_tool_runtime::tool_results::{
    format_tool_results_as_text, project_tool_payload, render_tool_payload_for_prompt,
};
use serde_json::{json, Value};

/// What a projected `get_group_member_activities` result may weigh, bytes.
const PEER_ACTIVITIES_MAX_BYTES: usize = 6 * 1024;
/// What a projected `get_training_plan` result may weigh: a fortnight with a
/// structured, fuelled long session on three days of each week.
const TRAINING_PLAN_MAX_BYTES: usize = 12 * 1024;
/// What a projected `list_workout_templates` summary may weigh, bytes.
const WORKOUT_TEMPLATES_MAX_BYTES: usize = 12 * 1024;
/// Rows a peer read keeps, and a template summary keeps: the seam's published cut.
const PEER_ACTIVITY_ROWS: usize = 20;
const TEMPLATE_ROWS: usize = 30;

fn bytes(value: &Value) -> usize {
    serde_json::to_string(value).unwrap().len()
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

// ---------------------------------------------------------------------------
// get_group_member_activities
// ---------------------------------------------------------------------------

/// The tool's own clamp is 100 rows; every field populated is its worst case.
fn peer_activities(rows: usize) -> Value {
    let activities: Vec<Value> = (0..rows)
        .map(|i| {
            json!({
                "id": format!("strava-1234567890{i}"),
                "name": format!("Tuesday threshold repeats with the group, week {i}"),
                "sport": "ride",
                "start_date": "2026-10-20T11:04:00Z",
                "distance_km": 62.35,
                "duration_seconds": 7_320,
                "elevation_gain_m": 810.5,
                "average_heart_rate": 148,
                "max_heart_rate": 181,
                "average_power": 231,
                "max_power": 842,
                "calories": 1_480,
                "provider": "strava",
            })
        })
        .collect();
    json!({"member": "Alex", "group": "Tuesday Crew", "count": rows, "activities": activities})
}

#[test]
fn peer_activities_clamp_case_fits_its_budget() {
    let response = peer_activities(100);
    let full = bytes(&response);
    let projected = project_tool_payload("get_group_member_activities", &response).unwrap();
    let cut = bytes(&projected);

    assert!(
        cut <= PEER_ACTIVITIES_MAX_BYTES,
        "projected peer activities weigh {cut} B, budget {PEER_ACTIVITIES_MAX_BYTES} B (was {full} B)"
    );
    assert!(
        cut * 2 < full,
        "{full} B -> {cut} B is not the cut it exists to make"
    );
}

#[test]
fn peer_activities_keep_what_a_comparison_and_the_verifier_read() {
    let projected =
        project_tool_payload("get_group_member_activities", &peer_activities(3)).unwrap();
    let row = &projected["activities"][0];

    assert_eq!(
        keys(row),
        [
            "average_heart_rate",
            "average_power",
            "distance_km",
            "duration_seconds",
            "elevation_gain_m",
            "id",
            "max_heart_rate",
            "max_power",
            "name",
            "sport",
            "start_date",
        ],
        "a field added to the peer row must be a decision, not a drift"
    );
    assert_eq!(projected["member"], "Alex");
    assert_eq!(projected["group"], "Tuesday Crew");
    assert_eq!(projected["count"], 3);
    assert!(
        projected.get("omitted").is_none(),
        "nothing was cut, nothing to mark"
    );
}

#[test]
fn peer_activities_drop_null_fields_instead_of_sending_them() {
    let response = json!({
        "member": "Alex", "group": "Crew", "count": 1,
        "activities": [{
            "id": "a1", "name": "Easy spin", "sport": "ride",
            "start_date": "2026-10-20T11:04:00Z", "distance_km": null,
            "duration_seconds": 1_800, "elevation_gain_m": null,
            "average_heart_rate": null, "max_heart_rate": null,
            "average_power": null, "max_power": null, "calories": null,
            "provider": "strava",
        }],
    });
    let projected = project_tool_payload("get_group_member_activities", &response).unwrap();

    assert_eq!(
        keys(&projected["activities"][0]),
        ["duration_seconds", "id", "name", "sport", "start_date"]
    );
}

#[test]
fn peer_activities_say_when_rows_were_left_out() {
    let projected =
        project_tool_payload("get_group_member_activities", &peer_activities(100)).unwrap();

    assert_eq!(
        projected["activities"].as_array().unwrap().len(),
        PEER_ACTIVITY_ROWS
    );
    assert_eq!(projected["omitted"], 100 - PEER_ACTIVITY_ROWS);
    let note = projected["note"].as_str().unwrap();
    assert!(
        note.contains("not the full list") && note.contains("not evidence"),
        "a capped list must not read as the member's whole history: {note}"
    );
    assert_eq!(projected["count"], 100, "count stays the tool's own figure");
}

// ---------------------------------------------------------------------------
// get_training_plan
// ---------------------------------------------------------------------------

/// A structured, fuelled long session (`long`) or a short unstructured one.
fn plan_day(date: &str, long: bool) -> Value {
    if !long {
        return json!({
            "date": date, "sport": "run", "workout": "Easy aerobic run, conversational",
            "duration_min": 40, "intensity": "Z2",
        });
    }
    json!({
        "date": date,
        "sport": "ride",
        "workout": "Sweet spot over-unders with a long cool-down on the flat",
        "duration_min": 120,
        "intensity": "3x15min @ 88-93% FTP",
        "steps": (0..6).map(|n| json!({
            "kind": "interval", "repeat": 3, "duration_seconds": 900,
            "target": {"type": "ftp_percent", "low": 88, "high": 93},
            "label": format!("sweet spot block {n}"),
        })).collect::<Vec<_>>(),
        "fueling": {"carbs_g_per_hour": 70, "fluid_ml_per_hour": 600, "sodium_mg_per_hour": 700},
        "template_slug": "sweet-spot-over-unders",
        "template_params": {"blocks": 3, "block_minutes": 15},
    })
}

fn plan_week(n: u32) -> Value {
    json!({
        "id": format!("week-{n}"),
        "tenant_id": "11111111-1111-1111-1111-111111111111",
        "user_id": "22222222-2222-2222-2222-222222222222",
        "plan_id": "plan-1",
        "week_start": format!("2026-11-{:02}", 2 + n * 7),
        "focus": "Volume back up, one moderate day, protect the long ride",
        "phase_index": 1,
        "days": (0..7)
            .map(|d| plan_day(&format!("2026-11-{:02}", 2 + n * 7 + d), d % 3 == 1))
            .collect::<Vec<_>>(),
        "status": "active",
        "supersedes_id": null,
        "adjustment_reason": "",
        "created_at": "2026-10-26T12:00:00Z",
        "updated_at": "2026-10-26T12:00:00Z",
    })
}

/// A fortnight of full days, the most `get_training_plan` returns by default.
fn training_plan() -> Value {
    json!({
        "plan": {
            "id": "plan-1",
            "tenant_id": "11111111-1111-1111-1111-111111111111",
            "user_id": "22222222-2222-2222-2222-222222222222",
            "name": "Fall build to the gran fondo",
            "created_at": "2026-10-01T12:00:00Z",
            "updated_at": "2026-10-26T12:00:00Z",
        },
        "athlete": null,
        "weeks": [plan_week(0), plan_week(1)],
        "weeks_omitted": 6,
        "authors": [],
        "goal_stale": false,
        "calendar": {"scope": "what Dravr pushed", "entries": []},
    })
}

#[test]
fn training_plan_fortnight_fits_its_budget() {
    let response = training_plan();
    let full = bytes(&response);
    let projected = project_tool_payload("get_training_plan", &response).unwrap();
    let cut = bytes(&projected);

    assert!(
        cut <= TRAINING_PLAN_MAX_BYTES,
        "projected training plan weighs {cut} B, budget {TRAINING_PLAN_MAX_BYTES} B (was {full} B)"
    );
    assert!(cut < full);
}

#[test]
fn training_plan_drops_the_repeated_owner_keys_and_keeps_what_edits_chain_on() {
    let projected = project_tool_payload("get_training_plan", &training_plan()).unwrap();

    for noise in ["tenant_id", "user_id", "created_at", "updated_at"] {
        assert!(
            projected["plan"].get(noise).is_none(),
            "plan still carries {noise}"
        );
    }
    let week = &projected["weeks"][0];
    for noise in [
        "tenant_id",
        "user_id",
        "plan_id",
        "created_at",
        "updated_at",
        "supersedes_id",
        "adjustment_reason",
    ] {
        assert!(week.get(noise).is_none(), "week still carries {noise}");
    }
    // What a save_training_plan edit and a calendar push key on.
    assert_eq!(projected["plan"]["id"], "plan-1");
    assert_eq!(week["id"], "week-0");
    assert_eq!(week["week_start"], "2026-11-02");
    assert_eq!(week["days"].as_array().unwrap().len(), 7);
    assert_eq!(week["days"][1]["template_slug"], "sweet-spot-over-unders");
    assert_eq!(
        week["days"][1]["steps"].as_array().unwrap().len(),
        6,
        "a day's steps are the prescription, not bookkeeping"
    );
    assert_eq!(
        projected["weeks_omitted"], 6,
        "the tool's own marker survives"
    );
}

#[test]
fn training_plan_keeps_an_adjustment_reason_that_says_something() {
    let mut response = training_plan();
    response["weeks"][0]["adjustment_reason"] = json!("legs heavy, moved tempo to Wednesday");
    response["weeks"][0]["supersedes_id"] = json!("week-0-v1");
    let projected = project_tool_payload("get_training_plan", &response).unwrap();

    assert_eq!(
        projected["weeks"][0]["adjustment_reason"],
        "legs heavy, moved tempo to Wednesday"
    );
    assert_eq!(projected["weeks"][0]["supersedes_id"], "week-0-v1");
}

#[test]
fn training_plan_with_no_plan_still_reads_as_no_plan() {
    let response = json!({
        "plan": null, "message": "no active training plan — build one",
        "calendar": {"scope": "what Dravr pushed", "entries": []},
    });
    let projected = project_tool_payload("get_training_plan", &response).unwrap();

    assert!(projected["plan"].is_null());
    assert_eq!(projected["message"], "no active training plan — build one");
}

// ---------------------------------------------------------------------------
// list_workout_templates
// ---------------------------------------------------------------------------

fn template_summary(i: usize) -> Value {
    json!({
        "slug": format!("threshold-{i}"),
        "name": format!("Threshold repeats variant {i}"),
        "purpose": "threshold",
        "sport": "ride",
        "sport_variants": ["gravel", "mtb"],
        "duration_minutes": 75,
        "intensity_distribution": {"z1": 0.2, "z2": 0.4, "z3": 0.1, "z4": 0.3, "z5": 0.0},
        "evidence_tier": "tier_1",
        "caveat": "Not in the first week back from illness.",
        "params": {
            "reps": {"min": 3, "max": 6, "default": 4},
            "rep_minutes": {"min": 8, "max": 20, "default": 12},
            "recovery_minutes": {"min": 3, "max": 6, "default": 4},
            "target_percent_ftp": {"min": 95, "max": 105, "default": 100},
        },
        "fit": {"phases": ["build"], "readiness_min": "moderate", "max_per_week": 2,
                "min_spacing_hours": 48},
        "progression": {"order": ["reps", "rep_minutes"], "max_weekly_step": 1},
        "is_compiled_in": true,
    })
}

fn workout_templates(rows: usize) -> Value {
    json!({
        "count": rows,
        "filters": {"purpose": null, "phase": null, "sport": null, "detail": "summary"},
        "templates": (0..rows).map(template_summary).collect::<Vec<_>>(),
    })
}

#[test]
fn workout_templates_catalogue_fits_its_budget() {
    let response = workout_templates(80);
    let full = bytes(&response);
    let projected = project_tool_payload("list_workout_templates", &response).unwrap();
    let cut = bytes(&projected);

    assert!(
        cut <= WORKOUT_TEMPLATES_MAX_BYTES,
        "projected template list weighs {cut} B, budget {WORKOUT_TEMPLATES_MAX_BYTES} B (was {full} B)"
    );
    assert!(
        cut * 2 < full,
        "{full} B -> {cut} B is not the cut it exists to make"
    );
}

#[test]
fn workout_templates_keep_the_picking_fields_and_drop_the_raw_params() {
    let projected = project_tool_payload("list_workout_templates", &workout_templates(3)).unwrap();
    let row = &projected["templates"][0];

    // What `detail=full` and `prescribe_workout` return once a template is picked.
    for dropped in ["params", "progression", "sport_variants", "is_compiled_in"] {
        assert!(
            row.get(dropped).is_none(),
            "summary row still carries {dropped}"
        );
    }
    // What a plan day cites (`template_slug`) and what picking one weighs.
    for kept in [
        "slug",
        "name",
        "purpose",
        "sport",
        "duration_minutes",
        "intensity_distribution",
        "evidence_tier",
        "caveat",
        "fit",
    ] {
        assert!(row.get(kept).is_some(), "summary row lost {kept}");
    }
    assert_eq!(row["slug"], "threshold-0");
    assert_eq!(projected["count"], 3);
    assert!(projected.get("omitted").is_none());
}

#[test]
fn workout_templates_say_when_rows_were_left_out() {
    let projected = project_tool_payload("list_workout_templates", &workout_templates(80)).unwrap();

    assert_eq!(
        projected["templates"].as_array().unwrap().len(),
        TEMPLATE_ROWS
    );
    assert_eq!(projected["omitted"], 80 - TEMPLATE_ROWS);
    assert!(projected["note"].as_str().unwrap().contains("`purpose`"));
}

#[test]
fn workout_templates_full_rows_travel_with_their_params() {
    let response = json!({
        "count": 1,
        "filters": {"purpose": null, "phase": null, "sport": null, "detail": "full"},
        "templates": [{
            "slug": "threshold-0", "name": "Threshold",
            "steps": [{"kind": "interval"}],
            "params": {"reps": {"min": 3, "max": 6, "default": 4}},
        }],
    });
    let projected = project_tool_payload("list_workout_templates", &response).unwrap();

    assert!(
        projected["templates"][0].get("params").is_some(),
        "a full row is the model's explicit ask; it is not cut"
    );
}

// ---------------------------------------------------------------------------
// The seams
// ---------------------------------------------------------------------------

#[test]
fn an_unrecognised_shape_travels_whole() {
    for tool in [
        "get_group_member_activities",
        "get_training_plan",
        "list_workout_templates",
    ] {
        let error_shape = json!({"error": "provider did not respond", "reason": "fetch_failed"});
        assert!(
            project_tool_payload(tool, &error_shape).is_none(),
            "{tool}: an error envelope must reach the model intact"
        );
        assert!(project_tool_payload(tool, &json!("text")).is_none());
    }
    assert!(project_tool_payload("get_athlete", &peer_activities(3)).is_none());
}

#[test]
fn projection_is_byte_identical_across_runs() {
    // Replayed history is a cache prefix: the same stored payload has to render
    // to the same bytes every turn, or the prefix never hits.
    for (tool, response) in [
        ("get_group_member_activities", peer_activities(100)),
        ("get_training_plan", training_plan()),
        ("list_workout_templates", workout_templates(80)),
    ] {
        let first = render_tool_payload_for_prompt(tool, &response);
        let second = render_tool_payload_for_prompt(tool, &response);
        assert_eq!(
            first, second,
            "{tool} rendered differently on a second pass"
        );
    }
}

#[test]
fn the_text_loop_and_the_api_loop_render_the_same_projection() {
    let response = workout_templates(80);
    let api = render_tool_payload_for_prompt("list_workout_templates", &response);
    let text = format_tool_results_as_text(&[FunctionResponse {
        name: "list_workout_templates".to_owned(),
        response,
    }]);

    assert!(api.len() <= WORKOUT_TEMPLATES_MAX_BYTES);
    assert!(
        !text.contains("\"params\""),
        "the text loop must project through the same seam as the API loop"
    );
}
