// ABOUTME: Unified chat pipeline — single orchestrator for web and messaging turn dispatch
// ABOUTME: Stages compose into ChatPipeline::run; channel profiles gate per-channel behavior
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unified chat pipeline.
//!
//! Both the web/mobile chat endpoint (`POST /api/chat/conversations/{id}/messages`)
//! and the messaging ingress (Telegram, `WhatsApp`, Discord, Slack) run every
//! user turn through [`run`]. The pipeline is channel-agnostic — per-channel
//! knobs are expressed via [`ChannelProfile`] and the hook traits in [`hooks`].
//!
//! # Layering
//!
//! - Channel adapters (the Axum `send_message` handler, the messaging ingress
//!   `dispatch_and_respond` wrapper) own transport and quota UX: they extract
//!   auth, decide how to reply (HTTP body vs. webhook outbound delivery),
//!   serialize quota violations per channel, and record usage through the
//!   [`hooks::UsageRecorder`] trait.
//! - [`run`] owns everything between "I have a persisted user message" and
//!   "the assistant reply is persisted and post-processed". It is pure enough
//!   to be tested without HTTP or webhook plumbing.
//! - Pure stage functions live under [`stages`] and are individually unit-testable.
//!
//! # Why unified
//!
//! Prior to this module, web chat and messaging each missed features the other
//! had: messaging lacked `DataRequirements` activity prefetch (causing coaches
//! to hallucinate "last activity" details on Telegram); web lacked Tier 2
//! memory recall, Tier 4 coach followups, Tier 5.5 claim verification, and
//! Tier 6 text guardrails. Callers now opt into a single pipeline and receive
//! every stage consistently.

pub mod channel_profile;
pub mod hooks;
pub mod stages;
pub mod turn;

pub use channel_profile::{Channel, ChannelProfile, MaxIterations, ModelPolicy};
pub use hooks::{
    AgUiRun, PipelineHooks, QuotaGate, QuotaWarning, ResponsePostProcess, UsageRecorder,
};
// Re-exported so that flows which build `ToolLoopParams` directly (the
// insight route, the messaging ingress) can attach the same per-call
// recorder the chat pipeline uses.
pub use self::UsageRepoCallRecorder as TurnCallRecorder;
pub use turn::{
    CreateConversationResult, DispatchResult, TurnContext, TurnInput, UserMessageResult,
};

use std::sync::Arc;
use std::time::Instant;

use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{AddMessageParams, CoachRuntimeContext, ConversationTurnId};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::repositories::LlmUsageRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_llm::prompts::TOOL_DISCIPLINE_PROMPT;
use tracing::{debug, info, warn};

use crate::agui::AgUiEvent;

/// Emit an AG-UI `STEP_STARTED` event if a sink is wired.
///
/// Extracted from the stage calls so `run_turn` stays under the
/// workspace cognitive-complexity budget — the `if let` branch
/// lives in the helper rather than in every emission site.
async fn emit_step_started(hooks: &PipelineHooks<'_>, step: &str) {
    if let Some(agui) = &hooks.agui {
        agui.sink
            .emit(&AgUiEvent::step_started(agui.run_id.clone(), step))
            .await;
    }
}

/// Emit an AG-UI `STEP_FINISHED` event if a sink is wired.
async fn emit_step_finished(hooks: &PipelineHooks<'_>, step: &str) {
    if let Some(agui) = &hooks.agui {
        agui.sink
            .emit(&AgUiEvent::step_finished(agui.run_id.clone(), step))
            .await;
    }
}

/// Run the prompt-assembly stage with AG-UI step emissions.
///
/// Thin wrapper around [`assemble_prompt_and_messages`] that emits
/// `STEP_STARTED`/`STEP_FINISHED` around the call without inflating
/// the cognitive complexity of [`run_turn`].
async fn assemble_prompt_stage(
    hooks: &PipelineHooks<'_>,
    resources: &Arc<ServerResources>,
    input: &TurnInput,
    profile: &ChannelProfile,
    conv: &ConversationRecord,
    coach_ctx: Option<&CoachRuntimeContext>,
    history: &[MessageRecord],
) -> AppResult<(prompt_leak::PromptGuard, Vec<String>, Vec<ChatMessage>)> {
    emit_step_started(hooks, "prompt_assembly").await;
    let result =
        assemble_prompt_and_messages(resources, input, profile, conv, coach_ctx, history).await;
    emit_step_finished(hooks, "prompt_assembly").await;
    result
}

/// Parameters for [`dispatch_stage`].
///
/// Bundled into a struct to stay under the workspace
/// `clippy::too_many_arguments` budget.
struct DispatchStageArgs<'a> {
    /// Pipeline hooks, including the optional AG-UI sink.
    hooks: &'a PipelineHooks<'a>,
    /// Shared server resources.
    resources: &'a Arc<ServerResources>,
    /// Turn input (user message + identifiers).
    input: &'a TurnInput,
    /// Per-channel profile.
    profile: &'a ChannelProfile,
    /// Resolved active LLM model identifier.
    active_model: &'a str,
    /// Optional coach runtime context.
    coach_ctx: Option<&'a CoachRuntimeContext>,
    /// Persisted conversation history.
    history: &'a [MessageRecord],
}

/// Run the dispatch stage with AG-UI step emissions.
///
/// Thin wrapper around [`dispatch_llm_with_tools`] that emits
/// `STEP_STARTED`/`STEP_FINISHED` around the call without inflating
/// the cognitive complexity of [`run_turn`].
async fn dispatch_stage(
    args: DispatchStageArgs<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> AppResult<(chat_tool_loop::ToolLoopResult, String)> {
    emit_step_started(args.hooks, "dispatch").await;
    let result = dispatch_llm_with_tools(
        args.resources,
        args.input,
        args.profile,
        args.active_model,
        args.coach_ctx,
        args.history,
        llm_messages,
    )
    .await;
    emit_step_finished(args.hooks, "dispatch").await;
    result
}

use crate::config::LlmProviderType;
use crate::errors::{AppError, AppResult};
use crate::llm::ChatMessage;
use crate::mcp::resources::ServerResources;
use crate::protocols::universal::UniversalExecutor;
use crate::services::chat_provider_factory::create_chat_provider;
use crate::services::memory_extraction::{spawn_extract_for_turn, SpawnedExtractionRequest};
use crate::services::prompt_leak;
use crate::services::provider_error_filter::detect_leaked_provider_error;
use crate::services::tool_execution::{self as chat_tool_loop, build_mcp_tools, ToolLoopParams};

use stages::compaction::apply_tier1_compaction;
use stages::followups::{
    ensure_coach_session_attached, finalize_session_state, inject_pending_followups,
};
use stages::guardrails::apply_text_guardrails;
use stages::memory::inject_memory_recall;
use stages::persistence::{
    get_conversation_history, persist_assistant_response, persist_user_message,
};
use stages::prefetch::inject_startup_context;
#[cfg(feature = "tools-groups")]
use stages::prompt_builder::resolve_group_context;
use stages::prompt_builder::{build_llm_messages, build_provider_context};
use stages::refresh::inject_refresh_context;
#[cfg(feature = "tools-verification")]
use stages::verification::{
    apply_claim_verification, persist_pending_verdicts, ClaimVerificationOutcome,
    ClaimVerificationParams,
};

/// Compiled-in default tool-loop iteration budget when neither the coach
/// runtime context nor the admin config override specify a value.
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Hard ceiling for the admin-config-provided tool-loop budget. Prevents a
/// misconfigured tenant from wedging a turn in a near-infinite tool loop.
const MAX_ADMIN_CONFIG_TOOL_ITERATIONS: usize = 50;

/// Pick the `call_type` string used for per-LLM-call `llm_usage` rows
/// produced by the chat pipeline. Web-chat turns are tagged `"chat"`;
/// messaging-channel turns are tagged `"messaging"` so the analytics
/// aggregates stay consistent with the terminal summary row the route
/// layer writes.
fn call_type_for_profile(profile: &ChannelProfile) -> &'static str {
    match profile.channel {
        Channel::WebChat => "chat",
        Channel::Telegram
        | Channel::WhatsApp
        | Channel::Discord
        | Channel::Slack
        | Channel::Messenger => "messaging",
    }
}

/// Per-call sink that persists one `llm_usage` row per LLM
/// invocation.
///
/// Each row is attributed to the turn that created this recorder.
/// The write is dispatched on a detached tokio task so the
/// synchronous `record` method never blocks the tool loop.
///
/// The chat pipeline, the insight flow, and the messaging ingress all
/// construct one of these per turn so their tool loops emit per-call
/// rows that the `/internal/conversation-turn/{id}` endpoint surfaces
/// as the `llm_calls` array.
pub struct UsageRepoCallRecorder {
    llm_usage: Arc<dyn LlmUsageRepository>,
    tenant_id: String,
    user_id: String,
    conversation_id: Option<String>,
    turn_id: ConversationTurnId,
    call_type: &'static str,
}

impl UsageRepoCallRecorder {
    /// Build a recorder scoped to a single turn.
    pub fn new(
        llm_usage: Arc<dyn LlmUsageRepository>,
        tenant_id: String,
        user_id: String,
        conversation_id: Option<String>,
        turn_id: ConversationTurnId,
        call_type: &'static str,
    ) -> Self {
        Self {
            llm_usage,
            tenant_id,
            user_id,
            conversation_id,
            turn_id,
            call_type,
        }
    }
}

impl chat_tool_loop::LlmCallRecorder for UsageRepoCallRecorder {
    fn record(&self, record: chat_tool_loop::LlmCallRecord) {
        let llm_usage = Arc::clone(&self.llm_usage);
        let tenant_id = self.tenant_id.clone();
        let user_id = self.user_id.clone();
        let conversation_id = self.conversation_id.clone();
        let turn_id = self.turn_id;
        let call_type = self.call_type;
        tokio::spawn(async move {
            let total_tokens = record.prompt_tokens + record.completion_tokens;
            let params = InsertLlmUsage {
                tenant_id: &tenant_id,
                user_id: &user_id,
                conversation_id: conversation_id.as_deref(),
                turn_id,
                provider: &record.provider,
                model: &record.model,
                prompt_tokens: record.prompt_tokens,
                completion_tokens: record.completion_tokens,
                total_tokens,
                call_type,
                tool_calls_count: 0,
                tools_called: "[]",
                execution_time_ms: Some(record.latency_ms),
            };
            if let Err(e) = llm_usage.insert_llm_usage(&params).await {
                warn!("Failed to record per-LLM-call usage: {e}");
            }
        });
    }
}

/// Run a single turn through the unified chat pipeline.
///
/// Coordinates memory recall, prompt assembly, the tool loop, and the
/// response post-processing stages for both the web chat route and
/// messaging ingress. Per-channel behaviour is gated by the supplied
/// [`ChannelProfile`]; optional hooks
/// ([`QuotaGate`], [`ResponsePostProcess`]) plug in at the edges.
///
/// Per-LLM-call usage is written from inside the tool loop via the
/// [`UsageRepoCallRecorder`] this function constructs; the caller
/// writes the turn-summary row after `run` returns.
///
/// # Errors
///
/// Returns `AppError` variants produced by any stage: database errors
/// on persistence, LLM provider errors during the tool loop, quota
/// exhaustion from a [`QuotaGate`], etc.
pub async fn run(
    resources: &Arc<ServerResources>,
    input: TurnInput,
    profile: &ChannelProfile,
    hooks: &PipelineHooks<'_>,
) -> AppResult<DispatchResult> {
    // AG-UI: signal run start. Emission is best-effort; when no AG-UI
    // sink is wired the calls short-circuit inside the helper.
    if let Some(agui) = &hooks.agui {
        agui.sink
            .emit(&AgUiEvent::run_started(
                agui.run_id.clone(),
                agui.thread_id.as_deref(),
            ))
            .await;
    }

    let outcome = run_turn(resources, input, profile, hooks).await;

    if let Some(agui) = &hooks.agui {
        match &outcome {
            Ok(_) => {
                agui.sink
                    .emit(&AgUiEvent::run_finished(agui.run_id.clone()))
                    .await;
            }
            Err(err) => {
                agui.sink
                    .emit(&AgUiEvent::run_error(
                        agui.run_id.clone(),
                        format!("{:?}", err.code),
                        err.to_string(),
                    ))
                    .await;
            }
        }
    }

    outcome
}

/// Run the body of a single turn.
///
/// Separated from [`run`] so the outer wrapper can emit AG-UI
/// `RUN_STARTED` / `RUN_FINISHED` / `RUN_ERROR` around the full turn
/// without branching inside the linear stage sequence.
async fn run_turn(
    resources: &Arc<ServerResources>,
    input: TurnInput,
    profile: &ChannelProfile,
    hooks: &PipelineHooks<'_>,
) -> AppResult<DispatchResult> {
    let turn_started = Instant::now();
    let ctx = TurnContext::from_input(&input);

    // Stage 1: Pre-dispatch quota gate (hook).
    if let Some(gate) = hooks.quota_gate {
        if let Some(warning) = gate.pre_check(&ctx).await? {
            debug!(
                channel = profile.channel.as_str(),
                warning_code = %warning.code,
                "quota gate emitted soft warning"
            );
        }
    }

    let database = resources.repos.chat.as_ref();

    // Stage 2: Persist user message (uses conversation tenant for DB lookup).
    let msg_result = persist_user_message(
        database,
        &input.conversation_id,
        &input.user_id,
        input.conversation_tenant_id,
        &input.content,
    )
    .await?;
    let user_message = msg_result.message;
    let conv = msg_result.conversation;

    // Stage 3: Resolve active model per channel policy.
    let active_model =
        resolve_active_model(profile.model_policy, &input.conversation_id, &conv.model);

    // Stage 4: Ensure a long-lived coach session exists and is attached.
    let conv = ensure_coach_session_attached(resources, conv, input.conversation_tenant_id).await;

    // Stage 5: Load conversation history for LLM context.
    let history =
        get_conversation_history(database, &input.conversation_id, &input.user_id).await?;

    // Stage 6: Resolve coach runtime context.
    let coach_ctx = match conv.coach_id.as_deref() {
        Some(coach_id) => {
            resources
                .repos
                .coaches
                .get_coach_runtime_context(coach_id, input.conversation_tenant_id)
                .await?
        }
        None => None,
    };

    // Stages 7a–7h + 8: assemble the hardened system prompt and flatten the
    // conversation history into a ready-to-dispatch LLM message list.
    let (prompt_guard, pending_followup_ids, mut llm_messages) = assemble_prompt_stage(
        hooks,
        resources,
        &input,
        profile,
        &conv,
        coach_ctx.as_ref(),
        &history,
    )
    .await?;

    // Stages 9–14: pre-dispatch preparation (executor, prefetch, provider,
    // compaction, iteration budget) followed by the multi-turn tool loop.
    let (mut result, provider_name) = dispatch_stage(
        DispatchStageArgs {
            hooks,
            resources,
            input: &input,
            profile,
            active_model: &active_model,
            coach_ctx: coach_ctx.as_ref(),
            history: &history,
        },
        &mut llm_messages,
    )
    .await?;

    // Stages 15–18: scan for canary leaks, apply text guardrails, run claim
    // verification, and run the channel-specific post-processor hook.
    let post_processed = post_process_assistant_reply(
        resources,
        &input,
        &conv,
        coach_ctx.as_ref(),
        &prompt_guard,
        result.content,
        hooks,
    )
    .await;
    result.content = post_processed.content;

    // Stage 19: Persist assistant response (uses conversation tenant for DB lookup).
    let token_count = result.usage.as_ref().map(|u| u.completion_tokens);
    let prompt_tokens = result.usage.as_ref().map(|u| u.prompt_tokens);
    let assistant_params = AddMessageParams {
        conversation_id: &input.conversation_id,
        user_id: &input.user_id,
        role: "assistant",
        content: &result.content,
        token_count,
        finish_reason: result.finish_reason.as_deref(),
        prompt_tokens,
        model: Some(&active_model),
    };
    let (assistant_message, updated_conversation) =
        persist_assistant_response(database, &assistant_params, input.conversation_tenant_id)
            .await?;

    // Stage 19.5: Persist Tier 5.5 verdicts now that the assistant message is
    // durable. Linking via `message_id` keeps the audit trail intact — if the
    // message write above failed, verdicts were never written, avoiding orphan
    // rows that previously occurred when verification persisted first.
    #[cfg(feature = "tools-verification")]
    if !post_processed.pending_verdicts.is_empty() {
        persist_pending_verdicts(
            resources,
            input.conversation_tenant_id,
            &input.user_id,
            &input.conversation_id,
            conv.coach_id.as_deref(),
            &assistant_message.id,
            &post_processed.pending_verdicts,
        )
        .await;
    }

    // Stage 20: Tier 4 session finalize — touch session timestamp, mark
    // followups delivered.
    finalize_session_state(
        resources,
        conv.session_id.as_deref(),
        &pending_followup_ids,
        input.conversation_tenant_id,
    )
    .await;

    // Stage 21: Tier 2 background memory extraction. Detached via tokio::spawn
    // so extraction failures never affect the stored reply.
    spawn_extract_for_turn(
        Arc::clone(resources),
        SpawnedExtractionRequest {
            tenant_id: input.conversation_tenant_id,
            user_id: input.user_id.clone(),
            coach_id: conv.coach_id.clone(),
            user_message: input.content.clone(),
            assistant_reply: result.content.clone(),
            source_msg_id: Some(assistant_message.id.clone()),
        },
    );

    let dispatch_result = DispatchResult {
        content: result.content,
        usage: result.usage,
        tool_calls_count: result.tool_calls_count,
        tools_called: result.tools_called,
        turn_id: input.turn_id,
        finish_reason: result.finish_reason,
        activity_list: result.activity_list,
        model: active_model,
        provider_name,
        user_message,
        assistant_message,
        conversation: updated_conversation,
    };

    // Stage 22: UsageRecorder hook — post-turn cost + quota tracking.
    #[allow(clippy::cast_possible_truncation)]
    let elapsed_ms = turn_started.elapsed().as_millis() as u64;
    if let Some(recorder) = hooks.usage_recorder {
        recorder.record(&ctx, &dispatch_result, elapsed_ms).await;
    }

    Ok(dispatch_result)
}

/// Resolve the active LLM model for a turn per the channel's
/// [`ModelPolicy`].
///
/// Messaging channels use [`ModelPolicy::OverrideWithEnv`] so a production
/// config bump (e.g. `PIERRE_LLM_MODEL` sonnet → opus) takes effect for
/// long-lived conversations. Web chat uses [`ModelPolicy::UseStored`] to
/// honor an explicit per-conversation model choice.
fn resolve_active_model(policy: ModelPolicy, conversation_id: &str, stored_model: &str) -> String {
    match policy {
        ModelPolicy::UseStored => stored_model.to_owned(),
        ModelPolicy::OverrideWithEnv => {
            let active =
                LlmProviderType::model_from_env().unwrap_or_else(|| stored_model.to_owned());
            if active != stored_model {
                info!(
                    conversation_id = %conversation_id,
                    stored_model = %stored_model,
                    active_model = %active,
                    "Overriding stored conversation model with current env default"
                );
            }
            active
        }
    }
}

/// Resolve the tool-loop iteration budget for the turn.
///
/// [`MaxIterations::Fixed`] takes precedence (messaging channels pin this
/// to keep webhook latency predictable). [`MaxIterations::CoachOrAdminDefault`]
/// resolves in order: coach runtime context → admin config override
/// (clamped to [`MAX_ADMIN_CONFIG_TOOL_ITERATIONS`]) → compiled-in default.
async fn resolve_max_iterations(
    policy: MaxIterations,
    resources: &Arc<ServerResources>,
    coach_ctx: Option<&CoachRuntimeContext>,
) -> usize {
    if let MaxIterations::Fixed(n) = policy {
        return n;
    }

    if let Some(iterations) = coach_ctx.and_then(|c| c.max_tool_iterations) {
        #[allow(clippy::cast_sign_loss)]
        let value = iterations.max(1) as usize;
        debug!(
            max_tool_iterations = value,
            "Using coach-level tool iteration limit"
        );
        return value;
    }

    if let Some(ref admin_config) = resources.admin_config {
        if let Ok(Some(val)) = admin_config
            .get_value("tool_execution.max_iterations", None)
            .await
        {
            if let Some(config_val) = val.as_i64() {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let value = (config_val.max(1) as usize).min(MAX_ADMIN_CONFIG_TOOL_ITERATIONS);
                debug!(
                    max_tool_iterations = value,
                    "Using admin config tool iteration limit"
                );
                return value;
            }
        }
    }

    DEFAULT_MAX_TOOL_ITERATIONS
}

/// Pre-dispatch prep plus the multi-turn tool execution loop.
///
/// Owns pipeline stages 9 through 14: MCP executor construction, coach
/// `DataRequirements` activity prefetch, LLM provider resolution, Tier 1
/// context-window compaction, per-channel max-iteration budget resolution,
/// and the tool loop itself.
///
/// # Errors
///
/// Returns `AppError` from LLM provider creation or from the tool loop
/// (e.g. provider rate limits, tool handler failures).
async fn dispatch_llm_with_tools(
    resources: &Arc<ServerResources>,
    input: &TurnInput,
    profile: &ChannelProfile,
    active_model: &str,
    coach_ctx: Option<&CoachRuntimeContext>,
    history: &[MessageRecord],
    llm_messages: &mut Vec<ChatMessage>,
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
    let provider = create_chat_provider().await?;
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

    // Stage 13: Resolve per-channel tool-loop iteration budget.
    let max_iterations = resolve_max_iterations(profile.max_iterations, resources, coach_ctx).await;

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

/// Assemble the hardened system prompt and flatten history into an LLM-ready message list.
///
/// Owns pipeline stages 7a through 8: coach/default prompt, connected-
/// provider context, group context, freshness hint, memory recall,
/// pending followups, channel-specific response constraints, and canary
/// hardening — followed by `build_llm_messages` flattening.
///
/// Extracted from [`run`] to keep its cognitive complexity under the
/// workspace clippy threshold without fragmenting the linear stage order.
///
/// # Errors
///
/// Returns `AppError` from the group context resolver when the
/// conversation is group-scoped and repository lookups surface errors.
/// All other assembly steps are infallible or log-and-continue.
async fn assemble_prompt_and_messages(
    resources: &Arc<ServerResources>,
    input: &TurnInput,
    profile: &ChannelProfile,
    conv: &ConversationRecord,
    coach_ctx: Option<&CoachRuntimeContext>,
    history: &[MessageRecord],
) -> AppResult<(prompt_leak::PromptGuard, Vec<String>, Vec<ChatMessage>)> {
    // Stage 7a: Start from coach-defined or default Pierre system prompt.
    let base_prompt = coach_ctx.map_or_else(
        || resources.pierre_system_prompt(),
        |c| c.system_prompt.clone(),
    );

    // Stage 7b: Append connected-provider context so the LLM never asks the
    // user to connect providers that are already connected.
    let user_uuid = parse_uuid(&input.user_id).unwrap_or_default();
    let provider_context = build_provider_context(resources, user_uuid).await;
    let base_prompt = if provider_context.is_empty() {
        base_prompt
    } else {
        format!("{base_prompt}{provider_context}")
    };

    // Stage 7c: Inject group coaching context — only when the conversation is
    // explicitly group-scoped. Personal 1:1 chats never inherit group context
    // from the user's membership.
    #[cfg(feature = "tools-groups")]
    let base_prompt = {
        let group_service = resources.group_service();
        let (resolved_group_id, snapshots) =
            resolve_group_context(resources, conv.group_id.as_deref(), input.tool_tenant_id)
                .await?;
        group_service
            .inject_group_context(
                &base_prompt,
                "",
                user_uuid,
                input.tool_tenant_id,
                resolved_group_id.as_deref(),
                &snapshots,
            )
            .await
            .unwrap_or(base_prompt)
    };

    // Stage 7d: Trigger background provider refresh and append freshness hint.
    let base_prompt = if profile.emit_data_freshness_hint {
        inject_refresh_context(resources, &input.user_id, input.tool_tenant_id, base_prompt).await
    } else {
        base_prompt
    };

    // Stage 7e: Inject recalled user memory facts into the prompt.
    let base_prompt = inject_memory_recall(
        resources,
        input.conversation_tenant_id,
        &input.user_id,
        conv.coach_id.as_deref(),
        base_prompt,
    )
    .await;

    // Stage 7f: Render pending coach followups. Surfaced IDs are marked
    // delivered after the turn succeeds.
    let (base_prompt, pending_followup_ids) = inject_pending_followups(
        resources,
        input.conversation_tenant_id,
        &input.user_id,
        conv.coach_id.as_deref(),
        base_prompt,
    )
    .await;

    // Stage 7g: Append the channel-profile response-constraints prompt.
    let raw_system_prompt = match profile.response_constraints_prompt.as_deref() {
        Some(suffix) => format!("{base_prompt}\n\n{suffix}"),
        None => base_prompt,
    };

    // Stage 7g.1: Append mandatory tool-discipline rules at the very end of
    // the system prompt, immediately before the user turn. LLMs
    // (claude-opus-4.7 especially) recency-bias heavily — mid-prompt rules
    // get drowned out by 20 KB of coach persona + provider context.
    // Keeping this block last ensures the tool-call format and
    // "no narration" constraint are the freshest instructions when the
    // model starts generating.
    let raw_system_prompt = format!("{raw_system_prompt}\n\n{TOOL_DISCIPLINE_PROMPT}");

    // Stage 7h: Harden the prompt with a per-turn canary.
    let prompt_guard = prompt_leak::harden_system_prompt(
        input.conversation_tenant_id,
        conv.coach_id.as_deref(),
        &raw_system_prompt,
    );

    // Stage 8: Flatten into the LLM message list.
    let llm_messages = build_llm_messages(Some(&prompt_guard.hardened_prompt), history);

    Ok((prompt_guard, pending_followup_ids, llm_messages))
}

/// Aggregates the outputs of [`post_process_assistant_reply`] so the caller
/// can persist the assistant message first and then link any claim verdicts
/// to the resulting `message_id`.
///
/// The `pending_verdicts` field is compiled out when `tools-verification` is
/// disabled to keep the struct free of `pierre_evals` types.
pub(super) struct PostProcessedReply {
    /// Final assistant content ready to be persisted and returned.
    pub content: String,
    /// Verdicts waiting for their assistant `message_id`. Always empty when
    /// the verification feature is disabled.
    #[cfg(feature = "tools-verification")]
    pub pending_verdicts: Vec<(pierre_evals::ExtractedClaim, pierre_evals::VerdictOutcome)>,
}

/// Run post-LLM content processing over the raw assistant reply.
///
/// Owns pipeline stages 15 through 18: canary scan, text guardrails,
/// claim verification, and the channel-specific [`ResponsePostProcess`]
/// hook. Returns the content string alongside any pending verdicts so the
/// caller can persist them with the assistant `message_id` after the
/// message write succeeds — emitting verdicts before the message exists
/// would leave orphan rows if the message write failed.
async fn post_process_assistant_reply(
    #[cfg_attr(not(feature = "tools-verification"), allow(unused_variables))] resources: &Arc<
        ServerResources,
    >,
    input: &TurnInput,
    conv: &ConversationRecord,
    #[cfg_attr(not(feature = "tools-verification"), allow(unused_variables))] coach_ctx: Option<
        &CoachRuntimeContext,
    >,
    prompt_guard: &prompt_leak::PromptGuard,
    raw_content: String,
    hooks: &PipelineHooks<'_>,
) -> PostProcessedReply {
    // Stage 15: Scan for verbatim system-prompt leaks / canary hits.
    prompt_leak::scan_assistant_reply(
        prompt_guard,
        &raw_content,
        input.conversation_tenant_id,
        conv.coach_id.as_deref(),
    );

    // Stage 16: Tier 6 text guardrails.
    let locale_opt = input.locale.as_deref();
    let mut content = apply_text_guardrails(resources, &raw_content, locale_opt);

    // Stage 17: Tier 5.5 claim verification (gated behind tools-verification).
    // Verdicts are computed now — content may be rewritten under Warn/Block —
    // but persistence is deferred to after the assistant message is stored.
    #[cfg(feature = "tools-verification")]
    let pending_verdicts = {
        let verification_config = coach_ctx
            .map(|c| pierre_evals::VerificationConfig::parse_from_system_prompt(&c.system_prompt))
            .unwrap_or_default();
        let ClaimVerificationOutcome {
            content: verified_content,
            pending_verdicts,
        } = apply_claim_verification(ClaimVerificationParams {
            resources,
            reply: &content,
            config: &verification_config,
            locale: locale_opt,
        })
        .await;
        content = verified_content;
        pending_verdicts
    };

    // Stage 18: ResponsePostProcess hook.
    if let Some(post) = hooks.response_post_process {
        content = post.transform(&content);
    }

    PostProcessedReply {
        content,
        #[cfg(feature = "tools-verification")]
        pending_verdicts,
    }
}
