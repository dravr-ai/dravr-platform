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
use tracing::info;

use crate::tool_execution::to_embacle_responses;

/// Format pierre-llm function responses as `<tool_result>` text blocks.
///
/// Thin wrapper around [`embacle::tool_simulation::format_tool_results_as_text`] that
/// handles type conversion.
#[must_use]
pub fn format_tool_results_as_text(responses: &[FunctionResponse]) -> String {
    let embacle_responses = to_embacle_responses(responses);
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
