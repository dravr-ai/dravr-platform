// ABOUTME: Reading a tool's responses — rendering them for the model, and mining one for data
// ABOUTME: Standalone of the tool loops, which is why they live apart from them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a caller does with function responses once they exist.
//!
//! None of this belongs to a tool loop. One half renders responses for a model
//! that reads results as text; the other mines a specific tool's payload for
//! what the chat pipeline needs out of it — the activity list it prepends, and
//! the backend key of a provider whose window was served without it. The
//! `get_activities` projection contract lives here, so every render of that
//! envelope goes through one allowlist; the served-without-a-provider sidecar
//! is read by [`crate::reconnect`], which the headless loop shares from a
//! surface that holds no responses at all.

use std::borrow::Cow;

use pierre_core::llm::tool_simulation;
use pierre_llm::FunctionResponse;
use serde_json::{Map, Value};
use tracing::info;

use crate::guardian::StepOutput;
use crate::reconnect::offer_in_payload;

/// Convert pierre-llm function responses to embacle `tool_simulation` responses.
///
/// Lives here rather than in `tool_execution` because this module is its only
/// caller: the text loop reaches embacle's formatter through
/// [`format_tool_results_as_text`], so the conversion travels with the
/// formatting rather than with the loop.
fn to_embacle_responses(resps: &[FunctionResponse]) -> Vec<tool_simulation::FunctionResponse> {
    resps
        .iter()
        .map(|r| tool_simulation::FunctionResponse {
            name: r.name.clone(),
            response: r.response.clone(),
        })
        .collect()
}

/// Serialize one tool response for injection into the prompt.
///
/// Projects a `get_activities` envelope through [`project_activities_payload`]
/// first; every other tool, and any shape the projection does not recognise,
/// serializes unchanged.
///
/// The API tool loop injects the result as a `[Tool Result for X]: {json}` user
/// message AND hands it to the recorder that persists the round — which the next
/// turn replays as history. So one unreduced payload is re-paid for the rest of
/// the conversation, which is why the projection belongs at the serialization
/// step rather than at the injection site.
#[must_use]
pub fn render_tool_payload_for_prompt(tool_name: &str, response: &Value) -> String {
    let projected = project_activities_payload(tool_name, response);
    let payload = projected.as_ref().unwrap_or(response);
    serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_owned())
}

/// The `get_activities` envelope fields that survive projection verbatim.
///
/// `activity_list` is the prose the agent actually cites. The rest are small
/// scalars that change what the model may legitimately say: `count` and
/// `coverage` keep it from anchoring on the truncated slice, `has_more` /
/// `offset` / `limit` are what the tool's own schema promises for a follow-up
/// request ("Response includes `has_more` and pagination info"), and `provider`
/// names whose data it is on a merged multi-provider window.
///
/// `reconnect_required` and `provider_unavailable` are the two sidecars that
/// survive, and they are here for the same reason `coverage` is: they change
/// what the model may legitimately say. A window served without a connection
/// that is dead, or that could not answer just now, is a PARTIAL window, and an
/// agent that never learns so answers it as if it were the whole history. The
/// projection is the only thing between those sidecars and the tool-loop
/// prompt — both [`render_tool_payload_for_prompt`] and
/// [`format_tool_results_as_text`] project through it, so a key absent from
/// this list reaches no model at all. The prefetch, which keeps the prose
/// alone, carries them through [`served_without_notes`].
const ACTIVITIES_ENVELOPE_KEPT: [&str; 11] = [
    "activity_list",
    "provider",
    "count",
    "mode",
    "format",
    "coverage",
    "offset",
    "limit",
    "has_more",
    SERVED_WITHOUT_SIDECARS[0],
    SERVED_WITHOUT_SIDECARS[1],
];

/// The sidecars a `get_activities` window carries when it was served without
/// one of the athlete's connections, each with a `note` addressed to the
/// model: `reconnect_required` for a dead connection, `provider_unavailable`
/// for one that could not answer just now.
const SERVED_WITHOUT_SIDECARS: [&str; 2] = ["reconnect_required", "provider_unavailable"];

/// The notes of the served-without-a-provider sidecars a `get_activities`
/// payload carries: a dead connection's (`reconnect_required`), then an
/// unreachable one's (`provider_unavailable`).
///
/// A render that keeps the prose `activity_list` alone appends these, or the
/// partial window reads as the athlete's whole history and the answer never
/// says which sessions are missing.
#[must_use]
pub fn served_without_notes(payload: &Value) -> Vec<&str> {
    SERVED_WITHOUT_SIDECARS
        .iter()
        .filter_map(|key| payload.get(key)?.get("note")?.as_str())
        .collect()
}

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
/// seam would have made the agent able to discuss a ride and unable to analyse
/// it.
///
/// Returns `None` when the payload is not a recognisable `get_activities`
/// envelope, so an unexpected shape reaches the model intact: the reducer must
/// never be the reason an agent ends up with no data.
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

/// First served-without-a-provider signal across a round of tool responses.
///
/// The sidecar itself is read by [`offer_in_payload`], which the headless loop's
/// per-turn store shares — the loops that hold their own tool payloads reach it
/// through here, and the one that does not reaches it at the dispatch chokepoint.
///
/// `pub` so the integration suite can pin the bridge itself — a tool payload in,
/// the slug the chat pipeline mints from out — rather than restating the shape
/// of the sidecar and proving nothing about the reader.
#[must_use]
pub fn reconnect_offer_in_responses(responses: &[FunctionResponse]) -> Option<String> {
    responses
        .iter()
        .find_map(|resp| offer_in_payload(&resp.name, &resp.response))
}

/// The same signal read off a verified plan's executed steps, so a planned turn
/// carries the reconnect offer the `ReAct` loops carry.
pub(crate) fn reconnect_offer_in_steps(outputs: &[StepOutput]) -> Option<String> {
    outputs
        .iter()
        .find_map(|output| offer_in_payload(&output.tool_name, &output.result))
}

// ============================================================================
// Content Sanitization
// ============================================================================

/// Strip synthetic function call syntax from LLM content.
///
/// Some models (like Llama via Groq) output function calls both as proper
/// `tool_calls` AND as text content using syntax like
/// `<function(name)>{...}</function>`. This helper removes that synthetic
/// syntax to avoid displaying raw tool-call markup to users.
#[must_use]
pub fn strip_synthetic_function_calls(content: &str) -> Cow<'_, str> {
    use regex::Regex;
    use std::sync::OnceLock;

    fn function_pattern() -> Option<&'static Regex> {
        static PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
        PATTERN
            .get_or_init(|| Regex::new(r"<function[/\(][^>]+>[\s\S]*?</function>").ok())
            .as_ref()
    }

    let Some(pattern) = function_pattern() else {
        return Cow::Borrowed(content);
    };

    let cleaned = pattern.replace_all(content, "");
    let trimmed = cleaned.trim();

    if trimmed.is_empty() {
        Cow::Borrowed("")
    } else if trimmed.len() == content.len() {
        Cow::Borrowed(content)
    } else {
        Cow::Owned(trimmed.to_owned())
    }
}
