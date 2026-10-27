// ABOUTME: What the model reads of a plan-building tool's result — the get_activities seam, extended
// ABOUTME: Each projection keeps what a chained call or a cited fact needs and drops the rest

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Prompt-side projections for the tools whose results are replayed.
//!
//! A tool round is persisted and replayed into every later call and turn of the
//! conversation (ADR-016), so a result the model read once is paid for again on
//! each of them — and, on a cold cache, written again. `get_activities` was cut
//! at the serialization step in carnet#128; this module gives the plan-building
//! tools the same treatment (carnet#1197).
//!
//! Every projection here is a **field allowlist or a noise drop, never a
//! guess**: it returns `None` for a shape it does not recognise, and the caller
//! then sends the payload whole — the reducer must never be the reason an agent
//! ends up with no data. A projection that drops rows says so in the payload
//! (`omitted` and a `note`), because a list the model believes is complete is
//! worse than a long one: it reads "absent" as "did not happen".
//!
//! The byte budget per tool is pinned by the test per tool in
//! `crates/pierre-server/tests/it/tool_payload_projection_test.rs`.

use serde_json::{json, Map, Value};

/// Most peer activities a `get_group_member_activities` result keeps. The
/// tool's own default `limit` is 30 and its clamp 100; past this the oldest
/// rows go, newest first, with a count of what went.
const PEER_ACTIVITY_ROWS: usize = 20;

/// Most template rows a `list_workout_templates` summary keeps. The filters
/// (`purpose`, `phase`, `sport`) are how the model narrows to the rest.
const TEMPLATE_ROWS: usize = 30;

/// The summary-row fields that only matter once a template is picked, and that
/// `detail=full` / `prescribe_workout` return on demand.
const TEMPLATE_SUMMARY_PICKED_FIELDS: [&str; 4] =
    ["params", "progression", "sport_variants", "is_compiled_in"];

/// The per-activity fields a peer read keeps.
///
/// What a training partner compares — when, what, how far, how long, how hard —
/// and the id the verifier and any follow-up key on. `calories` is the
/// provider's estimate and `provider` is bookkeeping; neither is something an
/// athlete asks about, and null fields are dropped rather than sent as `null`.
const PEER_ACTIVITY_KEPT: [&str; 11] = [
    "id",
    "name",
    "sport",
    "start_date",
    "distance_km",
    "duration_seconds",
    "elevation_gain_m",
    "average_heart_rate",
    "max_heart_rate",
    "average_power",
    "max_power",
];

/// Project a tool's result for the prompt, or `None` when the tool has no
/// projection or the payload is not the shape it expects.
///
/// `get_activities` is handled by [`crate::tool_results::project_activities_payload`],
/// which the dispatch there tries first.
#[must_use]
pub(crate) fn project_plan_tool_payload(tool_name: &str, response: &Value) -> Option<Value> {
    match tool_name {
        "get_group_member_activities" => project_peer_activities(response),
        "get_training_plan" => project_training_plan(response),
        "list_workout_templates" => project_workout_templates(response),
        _ => None,
    }
}

/// Attach the "rows left out" marker to a projected object.
fn mark_omitted(projected: &mut Map<String, Value>, omitted: usize, how_to_narrow: &str) {
    if omitted > 0 {
        projected.insert("omitted".to_owned(), json!(omitted));
        projected.insert(
            "note".to_owned(),
            json!(format!(
                "{omitted} more not shown — this is not the full list. {how_to_narrow}"
            )),
        );
    }
}

fn project_peer_activities(response: &Value) -> Option<Value> {
    let obj = response.as_object()?;
    let rows = obj.get("activities")?.as_array()?;

    let mut projected = Map::new();
    for key in ["member", "group", "count"] {
        if let Some(value) = obj.get(key) {
            projected.insert(key.to_owned(), value.clone());
        }
    }
    let kept: Vec<Value> = rows
        .iter()
        .take(PEER_ACTIVITY_ROWS)
        .map(|row| {
            row.as_object().map_or_else(
                || row.clone(),
                |fields| {
                    let mut out = Map::new();
                    for key in PEER_ACTIVITY_KEPT {
                        keep_present(fields, &mut out, key);
                    }
                    Value::Object(out)
                },
            )
        })
        .collect();
    let omitted = rows.len().saturating_sub(kept.len());
    projected.insert("activities".to_owned(), Value::Array(kept));
    mark_omitted(
        &mut projected,
        omitted,
        "Ask again with a smaller `limit` or a `sport` filter; absence from this list is not evidence the member did not train.",
    );
    Some(Value::Object(projected))
}

/// Copy `key` across when the source has it and it is not null.
fn keep_present(from: &Map<String, Value>, to: &mut Map<String, Value>, key: &str) {
    if let Some(value) = from.get(key).filter(|v| !v.is_null()) {
        to.insert(key.to_owned(), value.clone());
    }
}

/// Keys that repeat the owner on every row of a plan reply: the tenant and
/// athlete the read was already scoped to, and the row's own bookkeeping
/// timestamps. None of them is something the athlete or the agent cites, and
/// no tool takes them as an argument — ids that *are* chained on (`id`,
/// `week_start`, `plan_id` on the outline) stay.
const PLAN_ROW_NOISE: [&str; 4] = ["tenant_id", "user_id", "created_at", "updated_at"];

/// A week additionally repeats its outline's id; the outline is in the same reply.
const PLAN_WEEK_NOISE: [&str; 1] = ["plan_id"];

fn strip_keys(row: &mut Map<String, Value>, keys: &[&str]) {
    for key in keys {
        row.remove(*key);
    }
}

/// Clean one week: the repeated owner keys, and the audit fields when empty.
fn clean_week(week: &mut Value) {
    let Some(row) = week.as_object_mut() else {
        return;
    };
    strip_keys(row, &PLAN_ROW_NOISE);
    strip_keys(row, &PLAN_WEEK_NOISE);
    if row
        .get("adjustment_reason")
        .is_some_and(|v| v.as_str() == Some(""))
    {
        row.remove("adjustment_reason");
    }
    if row.get("supersedes_id").is_some_and(Value::is_null) {
        row.remove("supersedes_id");
    }
}

fn clean_weeks(weeks: &mut Value) {
    if let Some(weeks) = weeks.as_array_mut() {
        weeks.iter_mut().for_each(clean_week);
    }
}

fn project_training_plan(response: &Value) -> Option<Value> {
    let mut projected = response.as_object()?.clone();
    // A reply with neither is not a plan reply (an error envelope).
    if !projected.contains_key("plan") && !projected.contains_key("weeks") {
        return None;
    }
    if let Some(plan) = projected.get_mut("plan").and_then(Value::as_object_mut) {
        strip_keys(plan, &PLAN_ROW_NOISE);
    }
    if let Some(weeks) = projected.get_mut("weeks") {
        clean_weeks(weeks);
    }
    if let Some(draft) = projected.get_mut("draft").and_then(Value::as_object_mut) {
        if let Some(plan) = draft.get_mut("plan").and_then(Value::as_object_mut) {
            strip_keys(plan, &PLAN_ROW_NOISE);
        }
        if let Some(weeks) = draft.get_mut("weeks") {
            clean_weeks(weeks);
        }
    }
    Some(Value::Object(projected))
}

fn project_workout_templates(response: &Value) -> Option<Value> {
    let mut projected = response.as_object()?.clone();
    let rows = projected.get_mut("templates")?.as_array_mut()?;

    let total = rows.len();
    rows.truncate(TEMPLATE_ROWS);
    for row in rows.iter_mut().filter_map(Value::as_object_mut) {
        // A summary row is for picking: slug, name, purpose, sport, length, how
        // hard, and where it fits. `params` (the bank's raw parameter object),
        // `progression` and the variant list are what `detail=full` or
        // `prescribe_workout` return once one is picked. A full row carries
        // `steps` and is the model's explicit request, so it travels whole.
        if !row.contains_key("steps") {
            strip_keys(row, &TEMPLATE_SUMMARY_PICKED_FIELDS);
        }
    }
    let omitted = total.saturating_sub(TEMPLATE_ROWS);
    mark_omitted(
        &mut projected,
        omitted,
        "Narrow with the `purpose`, `phase` or `sport` filter.",
    );
    Some(Value::Object(projected))
}
