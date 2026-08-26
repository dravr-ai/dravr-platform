// ABOUTME: Insight-path token accounting and JSON post-processing for the chat routes
// ABOUTME: Turn-level counter recording lives in pierre_chat_pipeline::usage_counters
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::tokens::estimate_chat_tokens;
use serde::Deserialize;
use tracing::{debug, warn};

use pierre_llm::ChatMessage;
use pierre_tool_runtime::tool_execution;

/// JSON shape the insight-generation prompt is instructed to emit. Kept
/// local to this module because it is only ever produced by the LLM and
/// consumed by [`parse_insight_json_response`].
#[derive(Debug, Deserialize)]
struct InsightGenerationResponse {
    content: String,
}

/// Parse the JSON payload the insight-generation prompt is supposed to
/// return. Accepts raw JSON or JSON wrapped in markdown code fences;
/// falls back to the raw string with a warning when parsing fails.
fn parse_insight_json_response(raw_content: &str) -> String {
    // Try to parse as JSON
    if let Ok(response) = serde_json::from_str::<InsightGenerationResponse>(raw_content) {
        return response.content;
    }

    // Sometimes LLMs wrap JSON in markdown code blocks, try to extract
    let trimmed = raw_content.trim();
    if let Some(json_start) = trimmed.find('{') {
        if let Some(json_end) = trimmed.rfind('}') {
            let json_str = &trimmed[json_start..=json_end];
            if let Ok(response) = serde_json::from_str::<InsightGenerationResponse>(json_str) {
                return response.content;
            }
        }
    }

    // Fallback: return raw content with warning (avoid logging raw content which may contain user data)
    warn!(
        "Failed to parse insight generation JSON response, using raw content ({} bytes)",
        raw_content.len()
    );
    raw_content.to_owned()
}

/// Post-process LLM content: extract JSON for insight requests, pass
/// plain chat replies through unchanged.
pub fn post_process_content(raw_content: &str, is_insight_request: bool) -> String {
    if is_insight_request {
        parse_insight_json_response(raw_content)
    } else {
        raw_content.to_owned()
    }
}

/// Extract real token counts from a tool-loop result, or estimate from
/// the assembled prompt + reply text when the provider returned no
/// usage. Used by the insight-generation flow which runs its own tool
/// loop outside the unified pipeline.
pub fn extract_or_estimate_tokens(
    result: &tool_execution::ToolLoopResult,
    llm_messages: &[ChatMessage],
) -> (Option<u32>, Option<u32>) {
    result.usage.as_ref().map_or_else(
        || {
            let prompt_text: String = llm_messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let (est_prompt, est_completion) = estimate_chat_tokens(&prompt_text, &result.content);
            debug!(
                est_prompt,
                est_completion, "Using estimated token counts (provider returned no usage)"
            );
            (Some(est_prompt), Some(est_completion))
        },
        |usage| (Some(usage.prompt_tokens), Some(usage.completion_tokens)),
    )
}
