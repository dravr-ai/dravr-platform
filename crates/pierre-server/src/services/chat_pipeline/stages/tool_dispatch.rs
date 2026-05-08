// ABOUTME: Pre-dispatch prep + multi-turn tool execution stage (stages 9-14)
// ABOUTME: Owns MCP executor construction, startup context prefetch, provider + compaction, tool loop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::models::CoachRuntimeContext;
use pierre_database::database::MessageRecord;
use tracing::{info, warn};

use crate::errors::{AppError, AppResult};
use crate::llm::ChatMessage;
use crate::mcp::resources::ServerContext;
use crate::protocols::universal::UniversalExecutor;
use crate::services::chat_provider_factory::create_chat_provider_from_resources;
use crate::services::provider_error_filter::detect_leaked_provider_error;
use crate::services::tool_execution::{self as chat_tool_loop, build_mcp_tools, ToolLoopParams};

use super::super::channel_profile::ChannelProfile;
use super::super::turn::TurnInput;
use super::super::{call_type_for_profile, UsageRepoCallRecorder};
use super::compaction::apply_tier1_compaction;
use super::prefetch::inject_startup_context;

/// Pre-dispatch prep plus the multi-turn tool execution loop.
///
/// Owns pipeline stages 9 through 14: MCP executor construction, coach
/// `DataRequirements` activity prefetch, LLM provider resolution, Tier
/// 1 context-window compaction, per-channel max-iteration budget
/// resolution, and the tool loop itself.
///
/// # Errors
///
/// Returns [`AppError`] from LLM provider creation or from the tool
/// loop (e.g. provider rate limits, tool handler failures).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(
    skip_all,
    fields(
        turn_id = %input.turn_id,
        channel = profile.channel.as_str(),
        conversation_id = %input.conversation_id,
        model = %active_model,
        max_iterations,
        message_count = llm_messages.len(),
    )
)]
pub(in crate::services::chat_pipeline) async fn dispatch_llm_with_tools(
    resources: &Arc<ServerContext>,
    input: &TurnInput,
    profile: &ChannelProfile,
    active_model: &str,
    coach_ctx: Option<&CoachRuntimeContext>,
    history: &[MessageRecord],
    llm_messages: &mut Vec<ChatMessage>,
    max_iterations: usize,
    stream_sink: Option<super::super::ChatStreamSink>,
) -> AppResult<(chat_tool_loop::ToolLoopResult, String)> {
    // Stage 9: MCP executor for tool calls.
    let executor = Arc::new(UniversalExecutor::new(Arc::clone(resources)));

    // Stage 10: Deterministic activity prefetch driven by coach DataRequirements.
    // Before this stage ran on every channel, Telegram/WhatsApp conversations had
    // to rely on the LLM calling get_activities itself — which it skipped,
    // producing hallucinated "last activity" replies.
    inject_startup_context(
        &executor,
        llm_messages,
        history,
        coach_ctx,
        &input.user_id,
        input.tool_tenant_id,
    )
    .await;

    // Stage 11: LLM provider resolution.
    let provider = create_chat_provider_from_resources(resources).await?;
    let provider_name = provider.name().to_owned();

    // Stage 12: Tier 1 compaction when the assembled message list nears the window.
    apply_tier1_compaction(
        resources,
        &provider,
        input.conversation_tenant_id,
        &input.conversation_id,
        history,
        llm_messages,
    )
    .await;

    info!(
        provider = %provider_name,
        message_count = llm_messages.len(),
        max_iterations,
        "Chat pipeline dispatch starting tool loop"
    );

    // Stage 14: Multi-turn tool execution loop.
    //
    // Build a per-turn call recorder so that every LLM call inside the
    // tool loop writes its own `llm_usage` row keyed on this turn's id.
    // The chat route's terminal `record_llm_usage` call afterwards
    // writes a zero-token summary row that owns `tools_called` and the
    // end-to-end execution time.
    let call_recorder: Option<Arc<dyn chat_tool_loop::LlmCallRecorder>> =
        Some(Arc::new(UsageRepoCallRecorder::new(
            Arc::clone(&resources.repos.llm_usage),
            input.conversation_tenant_id.to_string(),
            input.user_id.clone(),
            Some(input.conversation_id.clone()),
            input.turn_id,
            call_type_for_profile(profile),
        )));
    let tools = build_mcp_tools();
    let tool_params = ToolLoopParams {
        provider: &provider,
        executor,
        tools: &tools,
        model: active_model,
        user_id: &input.user_id,
        tenant_id: input.tool_tenant_id,
        max_iterations,
        call_recorder,
        stream_sink,
        temperature: coach_ctx.and_then(|c| c.temperature),
    };
    let result = chat_tool_loop::run_tool_loop(&tool_params, llm_messages).await?;

    // Copilot CLI sometimes surfaces auth/entitlement failures as streamed
    // assistant content instead of JSON-RPC errors, which bypasses embacle's
    // error path and leaks the raw operator-facing message to end users.
    // Convert a known-signature match into an external-service error so the
    // channel adapter's failure-reply path fires with a user-facing message.
    if let Some(signature) = detect_leaked_provider_error(&result.content) {
        warn!(
            conversation_id = %input.conversation_id,
            signature = %signature,
            content_len = result.content.len(),
            "Provider CLI error text detected in assistant reply; converting to dispatch failure"
        );
        return Err(AppError::external_service(
            "LLM runner",
            format!("Provider CLI error leaked into assistant content: {signature}"),
        ));
    }

    info!(
        conversation_id = %input.conversation_id,
        turn_id = %input.turn_id,
        channel = profile.channel.as_str(),
        content_len = result.content.len(),
        tool_calls = result.tool_calls_count,
        "Chat pipeline dispatch completed"
    );

    Ok((result, provider_name))
}
