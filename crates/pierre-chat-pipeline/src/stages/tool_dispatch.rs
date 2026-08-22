// ABOUTME: Pre-dispatch prep + multi-turn tool execution stage (stages 9-14)
// ABOUTME: Owns MCP executor construction, startup context prefetch, provider + compaction, tool loop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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
    self as chat_tool_loop, build_mcp_tools, is_withheld_during_guided_flow, ToolLoopParams,
};
use tracing::{info, warn};

use crate::channel_profile::ChannelProfile;
use crate::recorders::{ChatRepoToolMessageRecorder, UsageRepoCallRecorder};
use crate::turn::TurnInput;
use crate::{call_type_for_profile, ChatPipelineContext};
use embacle_tool_host::ToolSession;

use super::compaction::apply_tier1_compaction;
use super::prefetch::{
    inject_startup_context, maybe_refresh_activity_context, ActivityRefreshInputs, PREFETCH_TOOL,
};

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
    /// Per-message history-row ids parallel to `llm_messages` (`None` for the
    /// system prompt), used by Tier 1 compaction to anchor block first/last
    /// message ids to real persisted rows rather than positional guesses.
    pub source_ids: &'a [Option<String>],
    /// Whether a guided conversational flow (today: the `/pillars` walk) owns
    /// this turn. Deliberately a flow-agnostic bool rather than the pillar-typed
    /// turn: everything this stage does with it — suppress activity injection,
    /// withhold the plan-writing tool — is true of any guided interview, and a
    /// second flow should not have to re-thread these call sites.
    pub guided_flow_active: bool,
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
        source_ids,
        guided_flow_active,
    } = inputs;
    // Stage 9: MCP executor for tool calls. Bind the originating conversation id
    // so a tool that spawns detached work (e.g. a historical activity backfill)
    // can route a completion notice back to the channel that triggered it.
    let executor = Arc::new(
        UniversalExecutor::new(Arc::clone(&ctx.tool_runtime))
            .with_conversation_id(input.conversation_id.clone())
            // Guardian turn key = the per-utterance turn_id, so taint/budget
            // accumulate across THIS message's ReAct loop and reset next message
            // (not across the whole conversation). conversation_id stays above,
            // for routing detached work back to the thread.
            .with_turn_token(input.turn_id.0.to_string()),
    );

    // Stage 10: Deterministic activity prefetch driven by coach DataRequirements.
    //
    // The return value is provenance, not status: `true` means a real
    // `get_activities` run put the athlete's activities in front of the model.
    // Stage 14 folds that into `tools_called` so downstream gates can tell
    // platform-fetched data from anything the model made up.
    let mut prefetched_activities = inject_startup_context(
        &executor,
        llm_messages,
        history,
        coach_ctx,
        &input.user_id,
        input.tool_tenant_id,
        guided_flow_active,
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
        source_ids,
        llm_messages,
    )
    .await;

    // Stage 12b: Later-turn activity grounding. On any turn past the first,
    // a plan/analysis/recommendation ask must be answered from the athlete's
    // real activities, not the coach persona alone. Runs AFTER compaction so
    // the freshly injected block is never summarized away and cannot desync
    // the `source_ids`/`llm_messages` vectors compaction consumed above.
    prefetched_activities |= maybe_refresh_activity_context(
        ActivityRefreshInputs {
            executor: &executor,
            history,
            coach_ctx,
            user_id: &input.user_id,
            tenant_id: input.tool_tenant_id,
            guided_flow_active,
        },
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
    // Held for the whole turn: dropping it revokes the agent's credential, so
    // the binding must outlive the tool loop and die with it.
    // Opened for every provider that will NOT receive tools as structured
    // declarations, which is both the ACP path and the CLI runners. Gating on
    // `supports_sdk_tool_calling()` alone left `copilot` — which now executes
    // MCP tools itself — with nothing to execute.
    let tool_session = if provider.capabilities().supports_function_calling() {
        // Structured declarations reach this provider on the request itself.
        None
    } else {
        match ctx.mcp_bridge.as_ref() {
            Some(bridge) => {
                bridge
                    .open_tool_session(&input.user_id, input.tool_tenant_id, &input.conversation_id)
                    .await
            }
            None => None,
        }
    };
    let mcp_servers = tool_session
        .as_ref()
        .map(ToolSession::mcp_servers)
        .unwrap_or_default();

    // Stage 13: the coaching tool surface. Every turn sees the full
    // chat-callable set (`ToolRegistry::chat_callable_schemas`), so a capable
    // coach model routes natively over all coaching tools — no per-turn
    // narrowing that could starve a turn of a tool it needs.
    //
    // The one exception is a guided flow: a profile interview withholds the
    // plan-writing tool for its duration, in lockstep with the Stage 7a.2 prose
    // list, so the model neither sees it advertised nor finds it callable.
    let mut tools = build_mcp_tools(&ctx.tool_registry);
    if guided_flow_active {
        let before = tools.function_declarations.len();
        tools
            .function_declarations
            .retain(|decl| !is_withheld_during_guided_flow(&decl.name));
        info!(
            withheld = before.saturating_sub(tools.function_declarations.len()),
            "guided flow active: withholding write tools from the turn"
        );
    }
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
    let loop_result = chat_tool_loop::run_tool_loop(&tool_params, llm_messages).await;

    // The loopback seam is otherwise invisible: ACP reports tool-call titles,
    // but only the platform's own MCP host knows how many calls it actually
    // served. `None` = the turn ran without a loopback session (native
    // function calling); `Some(0)` on an ACP turn = the subprocess never
    // reached our tools — the discriminating signal the 2026-08-22 silent-ACP
    // triage lacked. Read before the error return so a timed-out turn still
    // reports whether a tool round completed before the stall.
    let loopback_calls_served = tool_session.as_ref().map(ToolSession::calls_served);
    let mut result = match loop_result {
        Ok(result) => result,
        Err(e) => {
            warn!(
                conversation_id = %input.conversation_id,
                turn_id = %input.turn_id,
                channel = profile.channel.as_str(),
                loopback_calls_served = ?loopback_calls_served,
                dispatch_ms = u64::try_from(loop_start.elapsed().as_millis()).unwrap_or(u64::MAX),
                error = %e,
                "Chat pipeline dispatch failed in the tool loop"
            );
            return Err(e);
        }
    };

    // Record the prefetch as what it is: a tool run. Stage 10/12b invoked
    // `get_activities` on the athlete's behalf and put the rows in the prompt,
    // and the platform contract then tells the coach to use them WITHOUT
    // re-fetching — so the model answering from pre-loaded data calls nothing
    // and the loop reports an empty `tools_called`.
    //
    // Downstream that read as "no data was fetched", which is the opposite of
    // what happened. The `dravr-viz` anti-fabrication gate refused every chart
    // built from pre-loaded activities and left the raw fence in the reply, so
    // the athlete got a wall of JSON instead of a graph (observed on Slack,
    // 2026-08-20). Appended rather than inserted so a tool the model really did
    // call still leads the list.
    if prefetched_activities && !result.tools_called.iter().any(|t| t == PREFETCH_TOOL) {
        result.tools_called.push(PREFETCH_TOOL.to_owned());
    }
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
        loopback_calls_served = ?loopback_calls_served,
        dispatch_ms,
        tokens = result.usage.as_ref().map_or(0, |u| u.total_tokens),
        "Chat pipeline dispatch completed"
    );

    Ok((result, provider_name))
}
