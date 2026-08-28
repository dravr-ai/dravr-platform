// ABOUTME: Reading a tool's responses — rendering them for the model, and mining one for data
// ABOUTME: Standalone of the tool loops, which is why they live apart from them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a caller does with function responses once they exist.
//!
//! Neither of these belongs to a tool loop: one renders responses for a model
//! that reads results as text, the other mines a specific tool's payload for
//! the activity list the chat pipeline prepends. They sat in the loop module
//! because that is where they were first needed.

use pierre_core::llm::tool_simulation;
use pierre_llm::FunctionResponse;
use serde_json::{Map, Value};
use tracing::info;

use crate::tool_execution::to_embacle_responses;

/// The `get_activities` envelope fields that survive projection verbatim.
///
/// `activity_list` is the prose the coach actually cites. The rest are small
/// scalars that change what the model may legitimately say: `count` and
/// `coverage` keep it from anchoring on the truncated slice, `has_more` /
/// `offset` / `limit` are what the tool's own schema promises for a follow-up
/// request ("Response includes `has_more` and pagination info"), and `provider`
/// names whose data it is on a merged multi-provider window.
const ACTIVITIES_ENVELOPE_KEPT: [&str; 9] = [
    "activity_list",
    "provider",
    "count",
    "mode",
    "format",
    "coverage",
    "offset",
    "limit",
    "has_more",
];

/// The per-activity fields that survive projection.
///
/// Enough to *address* an activity, not to describe one. Five registered tools
/// — `analyze_activity`, `get_activity_intelligence`, `calculate_metrics`,
/// `compare_activities`, `analyze_weather_impact` — take a required
/// `activity_id` and resolve it through `provider.get_activity(...)`, with no
/// name-or-index fallback. Dropping ids to save tokens would leave the model
/// able to describe an activity and unable to act on it, which is a worse
/// answer than a longer prompt.
///
/// `name`, `sport_type` and `start_date` come along because that is how an
/// athlete refers to a session ("my long run Saturday") and the model has to
/// map that phrase onto an id it can pass.
const ACTIVITY_ADDRESSING_FIELDS: [&str; 4] = ["id", "name", "sport_type", "start_date"];

/// Reduce a `get_activities` payload to what a model needs, leaving every other
/// tool untouched.
///
/// `get_activities` answers the same window two or three times over: the
/// rendered `activity_list` prose, the structured `activities` array (or
/// `activities_toon`), a `retrieval_context` sidecar and a `token_estimate`.
/// Serializing the whole envelope into the prompt put every copy in front of
/// the model on every grounded turn.
///
/// This is a **field projection, not the prose reducer**. The prefetch path can
/// keep prose alone ([`prompt` assembly's `injectable_activity_text`]) because
/// nothing chains off it — it is context, injected before the model runs. Here
/// the model *is* mid-loop and may call a tool with an `activity_id` next, so
/// the addressing fields have to survive. Applying the prose reducer at this
/// seam would have made the coach able to discuss a ride and unable to analyse
/// it.
///
/// Returns `None` when the payload is not a recognisable `get_activities`
/// envelope, so an unexpected shape reaches the model intact: the reducer must
/// never be the reason a coach ends up with no data.
#[must_use]
pub fn project_activities_payload(tool_name: &str, response: &Value) -> Option<Value> {
    if tool_name != "get_activities" {
        return None;
    }
    let obj = response.as_object()?;
    // The prose block is the load-bearing half. Without it this is some other
    // shape — an error envelope, a future rewrite — and projecting it would be
    // guessing.
    if !obj.get("activity_list").is_some_and(Value::is_string) {
        return None;
    }

    let mut projected = Map::new();
    for key in ACTIVITIES_ENVELOPE_KEPT {
        if let Some(value) = obj.get(key) {
            projected.insert(key.to_owned(), value.clone());
        }
    }

    if let Some(activities) = obj.get("activities").and_then(Value::as_array) {
        let addressed: Vec<Value> = activities
            .iter()
            .map(|activity| {
                activity.as_object().map_or_else(
                    || activity.clone(),
                    |row| {
                        let mut kept = Map::new();
                        for field in ACTIVITY_ADDRESSING_FIELDS {
                            if let Some(value) = row.get(field) {
                                kept.insert(field.to_owned(), value.clone());
                            }
                        }
                        Value::Object(kept)
                    },
                )
            })
            .collect();
        projected.insert("activities".to_owned(), Value::Array(addressed));
    }

    Some(Value::Object(projected))
}

/// Format pierre-llm function responses as `<tool_result>` text blocks.
///
/// Wraps [`embacle::tool_simulation::format_tool_results_as_text`] with the type
/// conversion, and projects each payload through [`project_activities_payload`]
/// first.
///
/// The projection belongs here rather than at each call site because embacle
/// pretty-prints these blocks: a `get_activities` envelope costs *more* through
/// this path than through the API loop's compact serialization, for identical
/// data. Both the text tool loop and the capability-recovery re-ask read it.
#[must_use]
pub fn format_tool_results_as_text(responses: &[FunctionResponse]) -> String {
    let projected: Vec<FunctionResponse> = responses
        .iter()
        .map(|resp| {
            project_activities_payload(&resp.name, &resp.response).map_or_else(
                || resp.clone(),
                |payload| FunctionResponse {
                    name: resp.name.clone(),
                    response: payload,
                },
            )
        })
        .collect();
    let embacle_responses = to_embacle_responses(&projected);
    tool_simulation::format_tool_results_as_text(&embacle_responses)
}

/// Extract activity list from function responses (for `get_activities` results).
pub fn extract_activity_list(responses: &[FunctionResponse]) -> Option<String> {
    for resp in responses {
        if resp.name == "get_activities" {
            if let Some(activity_list) = resp
                .response
                .get("activity_list")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                let list_len = activity_list.len();
                info!("Extracted activity list ({list_len} chars) to prepend to response");
                return Some(activity_list.to_owned());
            }
        }
    }
    None
}
