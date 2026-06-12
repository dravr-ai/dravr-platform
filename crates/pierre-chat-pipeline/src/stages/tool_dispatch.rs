// ABOUTME: Pre-dispatch prep + multi-turn tool execution stage (stages 9-14)
// ABOUTME: Owns MCP executor construction, startup context prefetch, provider + compaction, tool loop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::CoachRuntimeContext;
use pierre_database::database::MessageRecord;
use pierre_llm::ChatMessage;
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_services::provider_error_filter::detect_leaked_provider_error;
use pierre_tool_runtime::protocol::UniversalExecutor;
use pierre_tool_runtime::tool_execution::{
    self as chat_tool_loop, build_mcp_tools, build_mcp_tools_filtered, ToolLoopParams,
};
use tracing::{info, warn};

use crate::channel_profile::ChannelProfile;
use crate::turn::TurnInput;
use crate::{
    call_type_for_profile, ChatPipelineContext, ChatRepoToolMessageRecorder, UsageRepoCallRecorder,
};

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
/// Read-only inputs threaded into [`dispatch_llm_with_tools`]. The remaining
/// arguments (`llm_messages: &mut Vec<...>`, `max_iterations: usize`,
/// `stream_sink: Option<...>`) stay positional because they carry distinct
/// lifetimes / ownership requirements (mutable borrow + by-value usize +
/// `Send + 'static` channel handle respectively).
pub(crate) struct DispatchLlmInputs<'a> {
    /// Shared pipeline context.
    pub ctx: &'a ChatPipelineContext,
    /// Inbound turn payload (carries `turn_id`, `conversation_id`, user content).
    pub input: &'a TurnInput,
    /// Channel-shape profile (web vs messaging vs A2A vs ...).
    pub profile: &'a ChannelProfile,
    /// Effective LLM model id selected for this turn.
    pub active_model: &'a str,
    /// Active coach runtime context, when one is bound.
    pub coach_ctx: Option<&'a CoachRuntimeContext>,
    /// Prior conversation messages (oldest first) used for context.
    pub history: &'a [MessageRecord],
}

#[tracing::instrument(
    skip_all,
    fields(
        turn_id = %inputs.input.turn_id,
        channel = inputs.profile.channel.as_str(),
        conversation_id = %inputs.input.conversation_id,
        model = %inputs.active_model,
        max_iterations,
        message_count = llm_messages.len(),
    )
)]
pub(crate) async fn dispatch_llm_with_tools(
    inputs: DispatchLlmInputs<'_>,
    llm_messages: &mut Vec<ChatMessage>,
    max_iterations: usize,
    stream_sink: Option<crate::ChatStreamSink>,
) -> AppResult<(chat_tool_loop::ToolLoopResult, String)> {
    let DispatchLlmInputs {
        ctx,
        input,
        profile,
        active_model,
        coach_ctx,
        history,
    } = inputs;
    // Stage 9: MCP executor for tool calls.
    let executor = Arc::new(UniversalExecutor::new(Arc::clone(&ctx.tool_runtime)));

    // Stage 10: Deterministic activity prefetch driven by coach DataRequirements.
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
    let provider_arc =
        chat_provider_from_resources_arc(ctx.chat_provider.as_ref(), ctx.llm_provider.as_ref())?;
    let provider: &pierre_llm::ChatProvider = provider_arc.as_ref();
    let provider_name = provider.name().to_owned();

    // Stage 12: Tier 1 compaction when the assembled message list nears the window.
    apply_tier1_compaction(
        &ctx.harness_config_registry,
        ctx.repos.memory.as_ref(),
        provider,
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
            Arc::clone(&ctx.repos.llm_usage),
            input.conversation_tenant_id.to_string(),
            input.user_id.clone(),
            Some(input.conversation_id.clone()),
            input.turn_id,
            call_type_for_profile(profile),
        )));
    // Persist each tool round into chat_messages so follow-up turns can
    // see the same grounded evidence the model just consumed.
    let tool_message_recorder: Option<Arc<dyn chat_tool_loop::ToolMessageRecorder>> =
        Some(Arc::new(ChatRepoToolMessageRecorder::new(
            Arc::clone(&ctx.repos.chat),
            input.conversation_id.clone(),
            input.user_id.clone(),
            input.conversation_tenant_id,
        )));
    // For ACP-managed providers (Copilot Headless with SDK tool calling),
    // expose Dravr tools natively via the MCP bridge so the model calls them
    // over ACP instead of fragile text-based `<tool_call>` simulation. The
    // same capability flag that routes the turn to the headless loop gates the
    // bridge, so a single provider config controls both.
    let mcp_servers = if provider.capabilities().supports_sdk_tool_calling() {
        match ctx.mcp_bridge.as_ref() {
            Some(bridge) => {
                bridge
                    .mcp_servers_for(&input.user_id, input.tool_tenant_id)
                    .await
            }
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    // Stage 13: optional per-turn tool narrowing (default-off). When enabled,
    // the selector drops tool categories irrelevant to this message + coach so
    // the model sees a focused list; it falls back to the full set when the
    // signal is too weak to narrow safely.
    let (tools, tool_selection_reason) = match ctx.tool_intent_prefilter.as_ref() {
        Some(prefilter) => {
            let outcome = prefilter
                .select(
                    &ctx.tool_registry,
                    &input.content,
                    coach_ctx.map(|c| c.category.as_str()),
                )
                .await;
            let reason = outcome.reason;
            let keep: HashSet<String> = outcome.keep.into_iter().collect();
            (
                build_mcp_tools_filtered(&ctx.tool_registry, &keep),
                Some(reason),
            )
        }
        None => (build_mcp_tools(&ctx.tool_registry), None),
    };
    let tool_params = ToolLoopParams {
        provider,
        executor,
        tools: &tools,
        model: active_model,
        user_id: &input.user_id,
        tenant_id: input.tool_tenant_id,
        max_iterations,
        call_recorder,
        tool_message_recorder,
        stream_sink,
        temperature: coach_ctx.and_then(|c| c.temperature),
        mcp_servers,
    };
    let loop_start = Instant::now();
    let result = chat_tool_loop::run_tool_loop(&tool_params, llm_messages).await?;
    let dispatch_ms = u64::try_from(loop_start.elapsed().as_millis()).unwrap_or(u64::MAX);

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
        // Per-turn tool-usage observability: the exact tools the coach ran and
        // how long the agentic loop took. One structured line per turn answers
        // "which tools fired and was it slow" without grepping per-call logs.
        tools_called = ?result.tools_called,
        dispatch_ms,
        tokens = result.usage.as_ref().map_or(0, |u| u.total_tokens),
        // Tool-prefilter telemetry: `None` = disabled (full set sent);
        // otherwise the selector's verdict for this turn (deterministic /
        // fallback / llm). Lets us A/B token savings before defaulting on.
        tool_selection_reason = ?tool_selection_reason,
        "Chat pipeline dispatch completed"
    );

    Ok((result, provider_name))
}
