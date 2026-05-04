// ABOUTME: LLM usage recording, token estimation, and post-processing for chat/insight turns
// ABOUTME: Shared by send_message and send_insight_message — no HTTP handlers live here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::tokens::estimate_chat_tokens;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::llm::ChatMessage;
use crate::mcp::resources::ServerContext;
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

/// Per-turn dimensions that drive the scoped (per-conversation,
/// per-coach) counter increments. Mirrors
/// [`crate::routes::chat::quotas::PreChatScope`] on the read side so
/// pre-check and post-increment stay in lockstep.
#[derive(Debug, Default, Clone)]
pub struct UsageIncrementScope<'a> {
    /// `conversations.id` — drives the lifetime per-conversation
    /// message counter that the pre-chat check enforces against
    /// `max_messages_per_conversation`.
    pub conversation_id: Option<&'a str>,
    /// `coaches.id` — drives the daily per-coach message counter
    /// that the pre-chat check enforces against
    /// `max_messages_per_coach_per_day`.
    pub coach_id: Option<&'a str>,
}

/// Increment usage counters after a successful LLM call.
///
/// Tracks daily/weekly messages, tokens, and tool calls. Failures are
/// logged but do not block the chat response to avoid degrading user
/// experience.
pub async fn increment_usage_counters(
    resources: &Arc<ServerContext>,
    tenant_id: &str,
    user_id: &str,
    total_tokens: i64,
    tool_calls_count: u32,
) {
    increment_usage_counters_scoped(
        resources,
        tenant_id,
        user_id,
        total_tokens,
        tool_calls_count,
        &UsageIncrementScope::default(),
    )
    .await;
}

/// Scoped variant of [`increment_usage_counters`] that additionally
/// bumps the per-conversation and per-coach counters when their ids
/// are present in [`UsageIncrementScope`]. The same dimension keys
/// the pre-chat check reads (`conversation_messages:<conv>`,
/// `daily_coach_messages:<coach>`) are written here.
pub async fn increment_usage_counters_scoped(
    resources: &Arc<ServerContext>,
    tenant_id: &str,
    user_id: &str,
    total_tokens: i64,
    tool_calls_count: u32,
    scope: &UsageIncrementScope<'_>,
) {
    let Some(ref admin_config) = resources.admin_config else {
        return;
    };

    let usage_svc = UsageCounterService::new(resources.repos.usage_counters.as_ref(), admin_config);

    increment_base_counters(
        &usage_svc,
        tenant_id,
        user_id,
        total_tokens,
        tool_calls_count,
    )
    .await;
    increment_scoped_counters(&usage_svc, tenant_id, user_id, scope).await;
}

async fn increment_base_counters(
    usage_svc: &UsageCounterService<'_>,
    tenant_id: &str,
    user_id: &str,
    total_tokens: i64,
    tool_calls_count: u32,
) {
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

async fn increment_scoped_counters(
    usage_svc: &UsageCounterService<'_>,
    tenant_id: &str,
    user_id: &str,
    scope: &UsageIncrementScope<'_>,
) {
    if let Some(conv_id) = scope.conversation_id {
        if let Err(e) = usage_svc
            .increment_with_dimension(tenant_id, user_id, "conversation_messages", conv_id, 1)
            .await
        {
            warn!("Failed to increment conversation_messages:{conv_id} counter: {e}");
        }
    }

    if let Some(coach_id) = scope.coach_id {
        if let Err(e) = usage_svc
            .increment_with_dimension(tenant_id, user_id, "daily_coach_messages", coach_id, 1)
            .await
        {
            warn!("Failed to increment daily_coach_messages:{coach_id} counter: {e}");
        }
    }
}
