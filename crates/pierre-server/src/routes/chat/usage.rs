// ABOUTME: LLM usage recording, token estimation, and post-processing for chat/insight turns
// ABOUTME: Shared by send_message and send_insight_message — no HTTP handlers live here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{ConversationTurnId, TURN_SUMMARY_CALL_TYPE};
use pierre_core::tokens::estimate_chat_tokens;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::llm::{ChatMessage, ChatProvider};
use crate::mcp::resources::ServerResources;
use crate::models::TenantId;
use crate::services::chat_pipeline;
use crate::services::usage_counter::UsageCounterService;

use super::super::chat_tool_loop;

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

/// Resolve prompt/completion token counts from a pipeline dispatch.
///
/// Prefers real provider-reported counts from `TokenUsage`. When the
/// provider does not report usage (CLI-based providers such as Copilot
/// headless), falls back to character-based estimation on the user's
/// input for the prompt side and on the assistant reply for the
/// completion side.
pub fn tokens_from_dispatch(
    dispatch: &chat_pipeline::DispatchResult,
    user_content: &str,
) -> (Option<u32>, Option<u32>) {
    dispatch.usage.as_ref().map_or_else(
        || {
            let (prompt_est, completion_est) =
                estimate_chat_tokens(user_content, &dispatch.content);
            (Some(prompt_est), Some(completion_est))
        },
        |usage| (Some(usage.prompt_tokens), Some(usage.completion_tokens)),
    )
}

/// Extract real token counts from a tool-loop result, or estimate from
/// the assembled prompt + reply text when the provider returned no
/// usage. Used by the insight-generation flow which runs its own tool
/// loop outside the unified pipeline.
pub fn extract_or_estimate_tokens(
    result: &chat_tool_loop::ToolLoopResult,
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

/// Parameters for the terminal turn-summary `llm_usage` row.
///
/// Per-LLM-call token counts are written separately by the tool loop's
/// [`crate::services::tool_execution::LlmCallRecorder`]; this struct
/// only carries the turn-level aggregates so the summary row can be
/// inserted in one call.
pub struct RecordLlmUsageParams<'a> {
    /// Tenant this usage belongs to
    pub tenant_id: TenantId,
    /// User who initiated the request
    pub user_id: &'a str,
    /// Conversation this message belongs to
    pub conversation_id: &'a str,
    /// Conversation-turn identifier threaded from the inbound request
    pub turn_id: ConversationTurnId,
    /// LLM provider used for this completion
    pub provider: &'a ChatProvider,
    /// Model identifier used for this completion
    pub model: &'a str,
    /// Number of tool calls executed across the turn
    pub tool_calls_count: u32,
    /// MCP tool names invoked during the turn (duplicates preserved in call order)
    pub tools_called: &'a [String],
    /// Total wall-clock time for the LLM interaction in milliseconds
    pub execution_time_ms: u64,
}

/// Record the terminal turn-summary row after a chat/insight turn.
///
/// Token columns are zeroed so the per-call rows and the summary row
/// don't double-count in the aggregate analytics endpoint, which
/// filters by [`TURN_SUMMARY_CALL_TYPE`].
pub async fn record_llm_usage(resources: &Arc<ServerResources>, params: &RecordLlmUsageParams<'_>) {
    let tenant_id_str = params.tenant_id.to_string();

    #[allow(clippy::cast_possible_wrap)]
    let exec_time = params.execution_time_ms as i64;

    let tools_called_json = serde_json::to_string(params.tools_called).unwrap_or_else(|e| {
        warn!("Failed to serialize tools_called, storing empty array: {e}");
        "[]".to_owned()
    });

    if let Err(e) = resources
        .repos
        .llm_usage
        .insert_llm_usage(&InsertLlmUsage {
            tenant_id: &tenant_id_str,
            user_id: params.user_id,
            conversation_id: Some(params.conversation_id),
            turn_id: params.turn_id,
            provider: params.provider.name(),
            model: params.model,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            call_type: TURN_SUMMARY_CALL_TYPE,
            tool_calls_count: i64::from(params.tool_calls_count),
            tools_called: &tools_called_json,
            execution_time_ms: Some(exec_time),
        })
        .await
    {
        warn!("Failed to record LLM usage summary: {e}");
    }
}

/// Increment usage counters after a successful LLM call.
///
/// Tracks daily/weekly messages, tokens, and tool calls. Failures are
/// logged but do not block the chat response to avoid degrading user
/// experience.
pub async fn increment_usage_counters(
    resources: &Arc<ServerResources>,
    tenant_id: &str,
    user_id: &str,
    total_tokens: i64,
    tool_calls_count: u32,
) {
    let Some(ref admin_config) = resources.admin_config else {
        return;
    };

    let usage_svc = UsageCounterService::new(resources.repos.usage_counters.as_ref(), admin_config);

    // Build list of (counter_type, amount) pairs to increment
    let mut counters: Vec<(&str, i64)> = vec![("daily_messages", 1), ("weekly_messages", 1)];
    if total_tokens > 0 {
        counters.push(("daily_tokens", total_tokens));
        counters.push(("weekly_tokens", total_tokens));
    }
    if tool_calls_count > 0 {
        let tool_calls = i64::from(tool_calls_count);
        counters.push(("daily_tool_calls", tool_calls));
        counters.push(("weekly_tool_calls", tool_calls));
    }

    for (counter_type, amount) in counters {
        if let Err(e) = usage_svc
            .increment(tenant_id, user_id, counter_type, amount)
            .await
        {
            warn!("Failed to increment {counter_type} counter: {e}");
        }
    }
}
