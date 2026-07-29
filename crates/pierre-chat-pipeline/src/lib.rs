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
//! # Wiring
//!
//! The pipeline takes a [`ChatPipelineContext`] — a concrete struct collecting
//! every Arc handle the stages need. The composition root in `pierre-server`
//! builds the context once from its `ServerContext` and passes it into [`run`].

#![warn(missing_docs)]

pub mod channel_profile;
pub mod hooks;
pub mod stages;
pub mod turn;

pub use channel_profile::{Channel, ChannelProfile, MaxIterations, ModelPolicy};
pub use hooks::{
    AgUiRun, ChatStreamEvent, ChatStreamSink, PipelineHooks, QuotaGate, QuotaWarning,
    ResponsePostProcess, UsageRecorder,
};
// Re-exported so that flows which build `ToolLoopParams` directly (the
// insight route, the messaging ingress) can attach the same per-call
// recorder the chat pipeline uses.
pub use self::UsageRepoCallRecorder as TurnCallRecorder;
pub use turn::{
    CreateConversationResult, DispatchResult, TurnContext, TurnInput, UserMessageResult,
};

use std::mem;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use pierre_agui::AgUiEvent;
use pierre_config::environment::LlmProviderType;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    EvidenceRegistry, MessagingStringsRegistry, PromptRegistry, ToolDescriptionRegistry,
};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{
    AddMessageParams, CoachRuntimeContext, ConversationTurnId, OnboardingState, TenantId,
};
use pierre_database::database::repositories::LlmUsageRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::repositories::ChatRepository;
use pierre_database::RepositoryRegistry;
use pierre_llm::health::{LlmHealthState, LlmHealthStatus};
use pierre_llm::pricing::GLOBAL_PRICING_REGISTRY;
use pierre_llm::{ChatMessage, ChatProvider, LlmProvider, McpServerConfig};
use pierre_runtime_context::{AdminConfigLookup, DataContext};
use pierre_services::advice_capture::{
    spawn_capture_advice, AdviceCaptureStrategy, CapturedTurn, HeuristicGatedLlmExtraction,
};
use pierre_services::memory_extraction::{
    spawn_extract_for_turn, SpawnedExtractionRequest, WITHHELD_REPLY_TRANSCRIPT_MARKER,
};
use pierre_services::prompt_leak;
use pierre_sse::SseManager;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution as chat_tool_loop;
use tracing::field::Empty;
use tracing::{debug, info, warn};

use stages::followups::{ensure_coach_session_attached, finalize_session_state};
use stages::persistence::{
    get_conversation_history, persist_assistant_response, persist_user_message,
};
#[cfg(feature = "tools-verification")]
use stages::verification::persist_pending_verdicts;

/// Mints the MCP servers an ACP-managed provider (Copilot Headless) exposes to
/// the model for native tool calling on a turn.
///
/// Implemented in `pierre-server` (where auth + signing keys live) so the chat
/// pipeline stays free of auth dependencies. The returned config points the
/// agent at Dravr's own `/mcp` endpoint with a freshly-minted, short-TTL,
/// `/mcp`-audience Bearer token scoped to `(user, tenant)`. Returns empty when
/// the bridge is disabled or the token cannot be minted.
#[async_trait::async_trait]
pub trait McpBridgeProvider: Send + Sync {
    /// Build the per-turn MCP server list for `(user_id, tenant_id)`.
    async fn mcp_servers_for(&self, user_id: &str, tenant_id: TenantId) -> Vec<McpServerConfig>;
}

/// Shared state for every chat pipeline stage.
///
/// Collects the Arc handles the pipeline needs from the composition root's
/// `ServerContext`. The struct is `Clone` (every field is an `Arc` or
/// trivially cloneable) so callers can pass it by value or behind an `Arc`.
///
/// The context-struct pattern mirrors the precedent set by
/// [`pierre_routes_auth::AuthRoutesContext`] — fields pulled from disparate
/// optional crates where growing a wide trait would force
/// `pierre-runtime-context` to depend on the union of all of them.
#[derive(Clone)]
pub struct ChatPipelineContext {
    /// Repository registry — primary data access surface.
    pub repos: Arc<RepositoryRegistry>,
    /// Data context bundle (database + repos + cache + provider registry +
    /// activity intelligence).
    pub data: DataContext,
    /// Tool registry used by prompt assembly (Available Tools section) and
    /// tool dispatch.
    pub tool_registry: Arc<ToolRegistry>,
    /// Narrow `ToolRuntime` façade used by group fitness snapshot fetch and
    /// the `UniversalExecutor` constructed in tool dispatch.
    pub tool_runtime: Arc<dyn ToolRuntime>,
    /// Server config (base URL etc.).
    pub config: Arc<ServerConfig>,
    /// Admin JWT signing secret — used to mint hosted-login link tokens
    /// during provider re-auth recovery.
    pub admin_jwt_secret: Arc<str>,
    /// Optional admin config lookup — used to resolve the tool-loop
    /// iteration budget when no per-coach override is set.
    pub admin_config: Option<Arc<dyn AdminConfigLookup>>,
    /// Pre-built `ChatProvider` singleton (shared subprocess / token cache).
    pub chat_provider: Option<Arc<ChatProvider>>,
    /// Lower-level LLM provider, used as a fallback when no dedicated
    /// `ChatProvider` is configured.
    pub llm_provider: Option<Arc<dyn LlmProvider>>,
    /// Hot-reloadable system prompts (Pierre, coach personas, tool discipline).
    pub prompt_registry: Arc<PromptRegistry>,
    /// Hot-reloadable tool description overlays.
    pub tool_description_registry: Arc<ToolDescriptionRegistry>,
    /// Hot-reloadable claim verification corpus.
    pub evidence_registry: Arc<EvidenceRegistry>,
    /// Hot-reloadable messaging-strings registry (canned replies, banners).
    pub messaging_strings_registry: Arc<MessagingStringsRegistry>,
    /// Hot-reloadable cageux intelligence config.
    pub cageux_config_registry: Arc<CageuxConfigRegistry>,
    /// Hot-reloadable harness config (compaction + Tier 6 guardrails).
    pub harness_config_registry: Arc<HarnessConfigRegistry>,
    /// Hot-reloadable per-persona output-format conformance contracts.
    pub persona_contract_registry: Arc<PersonaContractRegistry>,
    /// SSE manager — publishes refresh-status events to web/mobile clients.
    pub sse_manager: Arc<SseManager>,
    /// Optional health-data sync orchestrator (enforme).
    #[cfg(feature = "health-sync")]
    pub sync_orchestrator: Option<Arc<pierre_enforme::SyncOrchestrator>>,
    /// Group coaching service — used by group context injection.
    #[cfg(feature = "tools-groups")]
    pub group_service: Arc<pierre_groups::GroupService>,
    /// Shared LLM startup-probe state — read by [`run`] to fail fast with
    /// 503 when the primary provider is sustained-broken.
    pub llm_health: Arc<LlmHealthState>,
    /// Main Pierre fitness assistant system prompt (resolved through the
    /// prompt registry with compiled-in fallback).
    pub pierre_system_prompt: String,
    /// Tool-discipline prompt for non-messaging channels.
    pub tool_discipline_prompt: String,
    /// Tool-discipline prompt for messaging channels.
    pub tool_discipline_messaging_prompt: String,
    /// Output-contract directive appended to the system prompt for coaches
    /// that declare an `output_schema` (JSON-only plan, prose refusal, no
    /// process narration).
    pub structured_output_prompt: String,
    /// JSON Schema text for the structured-workout plan contract. Compiled
    /// once and used to validate builder-coach replies before they are
    /// persisted as `structured_content`.
    pub structured_output_schema: String,
    /// Memory extraction system prompt (used by Tier 2 background extraction).
    pub memory_extraction_prompt: String,
    /// Optional MCP bridge — mints the per-turn MCP servers an ACP provider
    /// (Copilot Headless) exposes for native Dravr tool calling. `None`
    /// disables the bridge (text-based tool calling is used instead).
    pub mcp_bridge: Option<Arc<dyn McpBridgeProvider>>,
}

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
/// Thin wrapper around
/// [`stages::prompt_assembly::assemble_prompt_and_messages`] that
/// emits `STEP_STARTED`/`STEP_FINISHED` around the call without
/// inflating the cognitive complexity of [`run_turn`].
/// Parameters for [`assemble_prompt_stage`]. Bundled into a struct to stay
/// under the workspace `clippy::too_many_arguments` budget.
struct AssemblePromptArgs<'a> {
    /// Pipeline hooks for AG-UI step emissions.
    hooks: &'a PipelineHooks<'a>,
    /// Shared pipeline context.
    ctx: &'a ChatPipelineContext,
    /// Turn input (user message + identifiers).
    input: &'a TurnInput,
    /// Per-channel profile.
    profile: &'a ChannelProfile,
    /// Resolved conversation record.
    conv: &'a ConversationRecord,
    /// Optional coach runtime context.
    coach_ctx: Option<&'a CoachRuntimeContext>,
    /// Persisted conversation history.
    history: &'a [MessageRecord],
    /// Onboarding turn context when the conversation is mid pillar walk.
    onboarding: Option<&'a stages::onboarding::OnboardingTurn>,
}

async fn assemble_prompt_stage(
    args: AssemblePromptArgs<'_>,
) -> AppResult<stages::prompt_assembly::AssembledPrompt> {
    emit_step_started(args.hooks, "prompt_assembly").await;
    let result = stages::prompt_assembly::assemble_prompt_and_messages(
        args.ctx,
        args.input,
        args.profile,
        args.conv,
        args.coach_ctx,
        args.history,
        args.onboarding,
    )
    .await;
    emit_step_finished(args.hooks, "prompt_assembly").await;
    result
}

/// Resolve the coach runtime context for a conversation, if it has a coach.
async fn resolve_coach_ctx(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    tenant_id: TenantId,
) -> AppResult<Option<CoachRuntimeContext>> {
    match conv.coach_id.as_deref() {
        Some(coach_id) => {
            ctx.repos
                .coaches
                .get_coach_runtime_context(coach_id, tenant_id)
                .await
        }
        None => Ok(None),
    }
}

/// Fire the Tier 2 background fact extraction for a completed turn, stamping the
/// onboarding pillar/source/force-kind when the conversation is mid pillar walk.
fn spawn_turn_extraction(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    assistant_reply: &str,
    assistant_message_id: &str,
    onboarding: Option<&stages::onboarding::OnboardingTurn>,
) {
    let (pillar, source, force_kind) = stages::onboarding::extraction_params_or_default(onboarding);
    spawn_extract_for_turn(
        Arc::clone(&ctx.repos.memory),
        ctx.chat_provider.as_ref().map(Arc::clone),
        ctx.memory_extraction_prompt.clone(),
        SpawnedExtractionRequest {
            tenant_id: input.conversation_tenant_id,
            user_id: input.user_id.clone(),
            coach_id: conv.coach_id.clone(),
            user_message: input.content.clone(),
            assistant_reply: assistant_reply.to_owned(),
            source_msg_id: Some(assistant_message_id.to_owned()),
            pillar,
            source,
            force_kind,
        },
    );
}

/// Spawn background advice capture for the turn (P3 of playbook memory).
///
/// Mirrors [`spawn_turn_extraction`]: best-effort, needs the shared
/// `ChatProvider` singleton, and never blocks the reply. The v1 strategy is
/// [`HeuristicGatedLlmExtraction`]; swapping it (see `DRAVR-BACKLOG.md`) is a
/// one-line change here once a config selector exists.
fn spawn_turn_advice_capture(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    assistant_reply: &str,
    assistant_message_id: &str,
) {
    let strategy: Arc<dyn AdviceCaptureStrategy> = Arc::new(HeuristicGatedLlmExtraction);
    spawn_capture_advice(
        Arc::clone(&ctx.repos.playbooks),
        ctx.chat_provider.as_ref().map(Arc::clone),
        strategy,
        CapturedTurn {
            // Scope the playbook to the TOOL tenant (where the user's activity /
            // health data lives), so the outcome evaluator can read that data and
            // retrieval finds the playbook — these can differ from the
            // conversation tenant for group channels.
            tenant_id: input.tool_tenant_id.to_string(),
            user_id: input.user_id.clone(),
            coach_slug: conv.coach_id.clone(),
            user_message: input.content.clone(),
            assistant_reply: assistant_reply.to_owned(),
            source_msg_id: Some(assistant_message_id.to_owned()),
        },
    );
}

/// Fire the Tier 2 extraction (stage 21) and playbook advice capture (stage 21b)
/// for a completed turn.
///
/// When the reply was withheld and replaced with a canned string, the withheld
/// original must never reach the fact store or playbooks: a leaked narration
/// minted as a fact re-enters every future prompt bundle (reinforcement loop
/// observed 2026-07-10). The athlete's *own* message is not tainted by that,
/// though — so extraction still runs over the user turn with
/// [`WITHHELD_REPLY_TRANSCRIPT_MARKER`] standing in for the reply, and only
/// assistant-side learning (playbook advice capture, which exists to learn from
/// what the coach said) is skipped.
///
/// Dropping the user turn as well is what stalled the guided pillar walk: the
/// answer was never extracted, the topic never flipped to covered, and the next
/// turn re-asked the same question. Every recorded withhold to date is on the
/// coach this flow runs against.
///
/// Owning the `leak_replaced` branch here keeps `run_turn` itself branch-free
/// over this concern.
fn spawn_turn_background_learning(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    assistant_reply: &str,
    assistant_message_id: &str,
    onboarding: Option<&stages::onboarding::OnboardingTurn>,
    leak_replaced: bool,
) {
    if leak_replaced {
        spawn_turn_extraction(
            ctx,
            input,
            conv,
            WITHHELD_REPLY_TRANSCRIPT_MARKER,
            assistant_message_id,
            onboarding,
        );
        return;
    }
    spawn_turn_extraction(
        ctx,
        input,
        conv,
        assistant_reply,
        assistant_message_id,
        onboarding,
    );
    spawn_turn_advice_capture(ctx, input, conv, assistant_reply, assistant_message_id);
}

/// Record that this turn's guided-flow probe reached the athlete, so the next
/// turn advances to the following topic instead of re-asking while fact
/// extraction is still in flight.
///
/// A withheld reply is not recorded: the athlete saw the withhold marker, not
/// the question. Nothing to do when no guided flow owns the turn.
async fn record_guided_flow_probe(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    onboarding: Option<&stages::onboarding::OnboardingTurn>,
    leak_replaced: bool,
    tenant_id: TenantId,
) {
    let Some(turn) = onboarding else {
        return;
    };
    if leak_replaced {
        info!("onboarding probe withheld; not recording it as delivered");
        return;
    }
    stages::onboarding::record_delivered_probe(ctx, conv, turn, tenant_id).await;
}

/// Persist this turn's claim verdicts, now that the assistant message they
/// reference is durable. No-op when the turn produced none.
#[cfg(feature = "tools-verification")]
async fn persist_verdicts_for_turn(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    assistant_message_id: &str,
    pending_verdicts: &[(pierre_evals::ExtractedClaim, pierre_evals::VerdictOutcome)],
) {
    if pending_verdicts.is_empty() {
        return;
    }
    persist_pending_verdicts(
        &ctx.data,
        input.conversation_tenant_id,
        &input.user_id,
        &input.conversation_id,
        conv.coach_id.as_deref(),
        assistant_message_id,
        pending_verdicts,
    )
    .await;
}

/// Inputs for [`finish_turn_follow_through`].
struct FinishTurnInputs<'a> {
    ctx: &'a ChatPipelineContext,
    input: &'a TurnInput,
    conv: &'a ConversationRecord,
    assistant_reply: &'a str,
    assistant_message_id: &'a str,
    onboarding: Option<&'a stages::onboarding::OnboardingTurn>,
    leak_replaced: bool,
}

/// Everything the turn still owes once its reply is persisted: Tier 2 memory
/// extraction, playbook advice capture, and the guided-flow probe record.
///
/// Grouped because all three answer the same question — did this reply actually
/// reach the athlete? — and a caller that got that branch half-right is exactly
/// how a withheld turn used to orphan both the athlete's answer and the walk's
/// progress.
async fn finish_turn_follow_through(inputs: FinishTurnInputs<'_>) {
    let FinishTurnInputs {
        ctx,
        input,
        conv,
        assistant_reply,
        assistant_message_id,
        onboarding,
        leak_replaced,
    } = inputs;
    spawn_turn_background_learning(
        ctx,
        input,
        conv,
        assistant_reply,
        assistant_message_id,
        onboarding,
        leak_replaced,
    );
    record_guided_flow_probe(
        ctx,
        conv,
        onboarding,
        leak_replaced,
        input.conversation_tenant_id,
    )
    .await;
    retire_completed_interview_marker(ctx, conv, onboarding, input.conversation_tenant_id).await;
}

/// Clear the just-completed-interview marker once its release directive has
/// been delivered, so the next turn is an ordinary one.
///
/// Nothing to do while a flow still owns the turn — that state is the live
/// interview, not a finished one — and nothing to do when the conversation
/// carries no marker at all, which is every normal turn.
async fn retire_completed_interview_marker(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    onboarding: Option<&stages::onboarding::OnboardingTurn>,
    tenant_id: TenantId,
) {
    if onboarding.is_some() {
        return;
    }
    if !OnboardingState::just_completed(conv.onboarding_state.as_deref(), Utc::now()) {
        return;
    }
    stages::onboarding::clear_completed_marker(ctx, conv, tenant_id).await;
}

/// Parameters for [`dispatch_stage`].
///
/// Bundled into a struct to stay under the workspace
/// `clippy::too_many_arguments` budget.
struct DispatchStageArgs<'a> {
    /// Pipeline hooks, including the optional AG-UI sink.
    hooks: &'a PipelineHooks<'a>,
    /// Shared pipeline context.
    ctx: &'a ChatPipelineContext,
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
    /// Per-message history-row ids parallel to `llm_messages` (`None` for the
    /// system prompt), threaded to Tier 1 compaction for id-anchored blocks.
    source_ids: &'a [Option<String>],
    /// Whether a guided conversational flow owns this turn (see
    /// `DispatchLlmInputs::guided_flow_active`).
    guided_flow_active: bool,
}

/// Run the dispatch stage with AG-UI step emissions.
async fn dispatch_stage(
    args: DispatchStageArgs<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> AppResult<(chat_tool_loop::ToolLoopResult, String)> {
    emit_step_started(args.hooks, "dispatch").await;
    let max_iterations =
        resolve_max_iterations(args.profile.max_iterations, args.ctx, args.coach_ctx).await;
    let result = stages::tool_dispatch::dispatch_llm_with_tools(
        stages::tool_dispatch::DispatchLlmInputs {
            ctx: args.ctx,
            input: args.input,
            profile: args.profile,
            active_model: args.active_model,
            coach_ctx: args.coach_ctx,
            history: args.history,
            source_ids: args.source_ids,
            guided_flow_active: args.guided_flow_active,
        },
        llm_messages,
        max_iterations,
        args.hooks.stream_sink.clone(),
    )
    .await;
    emit_step_finished(args.hooks, "dispatch").await;
    result
}

/// Bundled inputs for [`run_recovery_and_post_process`].
struct RecoveryAndPostProcessInputs<'a> {
    ctx: &'a ChatPipelineContext,
    input: &'a TurnInput,
    profile: &'a ChannelProfile,
    conv: &'a ConversationRecord,
    coach_ctx: Option<&'a CoachRuntimeContext>,
    prompt_guard: &'a prompt_leak::PromptGuard,
}

/// Wrap stages 14b–18: run the auth-recovery short-circuit, then either
/// pass the result through post-processing (LLM-produced text) or build a
/// minimal `PostProcessedReply` straight from the deterministic re-auth
/// content.
async fn run_recovery_and_post_process(
    inputs: RecoveryAndPostProcessInputs<'_>,
    result: &mut chat_tool_loop::ToolLoopResult,
    hooks: &PipelineHooks<'_>,
) -> stages::post_process::PostProcessedReply {
    let RecoveryAndPostProcessInputs {
        ctx,
        input,
        profile,
        conv,
        coach_ctx,
        prompt_guard,
    } = inputs;
    // Guardian-denied short-circuit takes precedence over re-auth: a tool
    // blocked by the runtime Guardian (enforce mode) is rendered as a
    // deterministic "blocked for safety" reply, bypassing both LLM
    // post-processing and the re-auth mint below.
    let guardian_denied = stages::guardian_denied::apply_guardian_denied(
        &ctx.messaging_strings_registry,
        input,
        result,
    );
    if guardian_denied {
        return stages::post_process::PostProcessedReply {
            content: mem::take(&mut result.content),
            #[cfg(feature = "tools-verification")]
            pending_verdicts: Vec::new(),
            structured_content: None,
            leak_replaced: false,
            identity_leak: None,
        };
    }

    let recovery_dispatched = AtomicBool::new(false);
    let recovery_active = stages::auth_recovery::apply_auth_recovery(
        stages::auth_recovery::AuthRecoveryDeps {
            admin_jwt_secret: &ctx.admin_jwt_secret,
            base_url: &ctx.config.base_url,
            messaging_strings_registry: &ctx.messaging_strings_registry,
            tool_runtime: &ctx.tool_runtime,
            short_links: &ctx.repos.short_links,
        },
        input,
        profile,
        result,
        &recovery_dispatched,
    )
    .await;

    if recovery_active {
        return stages::post_process::PostProcessedReply {
            content: mem::take(&mut result.content),
            #[cfg(feature = "tools-verification")]
            pending_verdicts: Vec::new(),
            structured_content: None,
            leak_replaced: false,
            identity_leak: None,
        };
    }

    stages::post_process::post_process_assistant_reply(
        stages::post_process::PostProcessInputs {
            ctx,
            input,
            conv,
            coach_ctx,
            prompt_guard,
            is_messaging: profile.channel.is_messaging(),
        },
        mem::take(&mut result.content),
        hooks,
    )
    .await
}

/// Compiled-in default tool-loop iteration budget when neither the coach
/// runtime context nor the admin config override specify a value.
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Hard ceiling for the admin-config-provided tool-loop budget.
const MAX_ADMIN_CONFIG_TOOL_ITERATIONS: usize = 50;

/// Pick the `call_type` string used for per-LLM-call `llm_usage` rows
/// produced by the chat pipeline.
pub(crate) fn call_type_for_profile(profile: &ChannelProfile) -> &'static str {
    match profile.channel {
        Channel::WebChat => "chat",
        Channel::Telegram
        | Channel::WhatsApp
        | Channel::Discord
        | Channel::Slack
        | Channel::Messenger => "messaging",
    }
}

/// Per-call sink that persists one `llm_usage` row per LLM invocation.
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
        let base_call_type = self.call_type;
        let call_sequence = record.call_sequence;
        let tenant_id_for_cost = self.tenant_id.clone();
        tokio::spawn(async move {
            let total_tokens = record.prompt_tokens + record.completion_tokens;
            let cost_usd = GLOBAL_PRICING_REGISTRY.calculate_cost(
                Some(tenant_id_for_cost.as_str()),
                &record.provider,
                &record.model,
                record.prompt_tokens,
                record.cached_tokens,
                record.completion_tokens,
            );
            info!(
                target: "notify",
                event = "embacle.cost_usd",
                user_id = %user_id,
                tenant_id = %tenant_id,
                model = %record.model,
                cost_usd = cost_usd,
                "llm call cost"
            );
            let call_type_owned = if record.token_counts_estimated {
                format!("{base_call_type}_estimated")
            } else {
                base_call_type.to_owned()
            };
            let tool_calls_count = i64::try_from(record.tools_called.len()).unwrap_or(i64::MAX);
            let tools_called_json =
                serde_json::to_string(&record.tools_called).unwrap_or_else(|_| "[]".to_owned());
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
                cached_tokens: record.cached_tokens,
                call_type: &call_type_owned,
                tool_calls_count,
                tools_called: &tools_called_json,
                execution_time_ms: Some(record.latency_ms),
                cost_usd,
                call_sequence,
            };
            if let Err(e) = llm_usage.insert_llm_usage(&params).await {
                warn!("Failed to record per-LLM-call usage: {e}");
            }
        });
    }
}

/// Persists each tool dispatch round as `chat_messages` rows.
pub struct ChatRepoToolMessageRecorder {
    chat: Arc<dyn ChatRepository>,
    conversation_id: String,
    user_id: String,
    tenant_id: TenantId,
}

impl ChatRepoToolMessageRecorder {
    /// Build a recorder scoped to a single conversation.
    #[must_use]
    pub fn new(
        chat: Arc<dyn ChatRepository>,
        conversation_id: String,
        user_id: String,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            chat,
            conversation_id,
            user_id,
            tenant_id,
        }
    }
}

impl chat_tool_loop::ToolMessageRecorder for ChatRepoToolMessageRecorder {
    fn record(&self, record: chat_tool_loop::ToolRoundRecord) {
        let chat = Arc::clone(&self.chat);
        let conversation_id = self.conversation_id.clone();
        let user_id = self.user_id.clone();
        let tenant_id = self.tenant_id;
        // Strip tool-call/tool-result scaffolding before persisting. The raw
        // `<tool_call>`/`<tool_result>` blocks are per-turn LLM plumbing, not
        // durable conversation content; persisting them verbatim lets a thread
        // accrete scaffolding the model later parrots (the read path strips them
        // on replay, so a stored block is dead weight anyway). A real preamble
        // ("Pulling your activities…") survives the strip and is still kept;
        // pure scaffolding reduces to empty and is skipped.
        let assistant_text = chat_tool_loop::strip_simulation_artifacts(&record.assistant_text);
        let tool_result_text = chat_tool_loop::strip_simulation_artifacts(&record.tool_result_text);
        tokio::spawn(async move {
            if !assistant_text.is_empty() {
                let params = AddMessageParams {
                    tenant_id,
                    conversation_id: &conversation_id,
                    user_id: &user_id,
                    role: "tool_call",
                    content: &assistant_text,
                    token_count: None,
                    finish_reason: None,
                    prompt_tokens: None,
                    model: None,
                    structured_content: None,
                };
                if let Err(e) = chat.add_message(&params).await {
                    warn!("Failed to persist tool_call message: {e}");
                    return;
                }
            }
            if !tool_result_text.is_empty() {
                let params = AddMessageParams {
                    tenant_id,
                    conversation_id: &conversation_id,
                    user_id: &user_id,
                    role: "tool_result",
                    content: &tool_result_text,
                    token_count: None,
                    finish_reason: None,
                    prompt_tokens: None,
                    model: None,
                    structured_content: None,
                };
                if let Err(e) = chat.add_message(&params).await {
                    warn!("Failed to persist tool_result message: {e}");
                }
            }
        });
    }
}

/// Record the turn span's deferred `coach_id`/`group_id` fields (declared
/// `Empty` on the `run` span) once the conversation record resolves them, so
/// every log line emitted for the rest of the turn carries the coach and
/// group-chat context. A no-op for fields the conversation leaves unset.
fn record_turn_span_context(conv: &ConversationRecord) {
    let span = tracing::Span::current();
    if let Some(coach_id) = conv.coach_id.as_deref() {
        span.record("coach_id", coach_id);
    }
    if let Some(group_id) = conv.group_id.as_deref() {
        span.record("group_id", group_id);
    }
}

/// Run a single turn through the unified chat pipeline.
///
/// # Errors
///
/// Returns `AppError` variants produced by any stage.
#[tracing::instrument(
    skip_all,
    fields(
        turn_id = %input.turn_id,
        channel = profile.channel.as_str(),
        conversation_id = %input.conversation_id,
        tenant_id = %input.conversation_tenant_id,
        user_id = %input.user_id,
        // Resolved mid-turn once the conversation record loads; recorded below.
        coach_id = Empty,
        group_id = Empty,
    )
)]
pub async fn run(
    ctx: &ChatPipelineContext,
    input: TurnInput,
    profile: &ChannelProfile,
    hooks: &PipelineHooks<'_>,
) -> AppResult<DispatchResult> {
    if let Some(agui) = &hooks.agui {
        agui.sink
            .emit(&AgUiEvent::run_started(
                agui.run_id.clone(),
                agui.thread_id.as_deref(),
            ))
            .await;
    }

    let persona =
        stages::prompt_assembly::resolve_user_persona(ctx.repos.users.as_ref(), &input.user_id)
            .await;
    info!(
        target: "notify",
        event = "chat.question_asked",
        persona = persona.as_str(),
        "user asked a question"
    );

    let snapshot = ctx.llm_health.snapshot().await;
    if matches!(snapshot.status, LlmHealthStatus::Unhealthy) {
        let provider = snapshot.provider.unwrap_or_else(|| "llm".to_owned());
        let last_error = snapshot.error.unwrap_or_else(|| "unknown".to_owned());
        warn!(
            provider = %provider,
            last_error = %last_error,
            "LLM probe is Unhealthy — failing chat turn fast with 503"
        );
        return Err(AppError::new(
            ErrorCode::ResourceUnavailable,
            format!(
                "Our AI provider ({provider}) is temporarily unavailable. We've been notified — please try again in a minute or two."
            ),
        ));
    }

    let outcome = run_turn(ctx, input, profile, hooks).await;

    if outcome.is_ok() {
        // A real served turn is the strongest proof the LLM provider is
        // live — stamp it so the periodic health probe can skip its billed
        // synthetic `copilot --acp` round-trip while real traffic flows.
        ctx.llm_health.note_success();
    }

    if let Some(agui) = &hooks.agui {
        match &outcome {
            Ok(_) => {
                agui.sink
                    .emit(&AgUiEvent::run_finished(agui.run_id.clone()))
                    .await;
            }
            Err(err) => {
                // Never surface raw internal error detail to the client — use
                // the sanitized, per-code message (generic for internal/auth/db
                // errors, specific only for safe classes like validation).
                agui.sink
                    .emit(&AgUiEvent::run_error(
                        agui.run_id.clone(),
                        format!("{:?}", err.code),
                        err.sanitized_message(),
                    ))
                    .await;
            }
        }
    }

    outcome
}

/// Run the body of a single turn.
async fn run_turn(
    ctx: &ChatPipelineContext,
    input: TurnInput,
    profile: &ChannelProfile,
    hooks: &PipelineHooks<'_>,
) -> AppResult<DispatchResult> {
    let turn_started = Instant::now();
    let turn_ctx = TurnContext::from_input(&input);

    // Stage 1: Pre-dispatch quota gate (hook).
    if let Some(gate) = hooks.quota_gate {
        if let Some(warning) = gate.pre_check(&turn_ctx).await? {
            debug!(
                channel = profile.channel.as_str(),
                warning_code = %warning.code,
                "quota gate emitted soft warning"
            );
        }
    }

    let database = ctx.repos.chat.as_ref();

    // Stage 2: Persist user message.
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
    let conv = ensure_coach_session_attached(&ctx.data, conv, input.conversation_tenant_id).await;

    // Fill the turn span's deferred context now that the conversation record is
    // resolved, so every downstream log line carries the coach and group-chat
    // identifiers alongside user/tenant/conversation/turn.
    record_turn_span_context(&conv);

    // Stage 4.5: Guided mode. If this conversation is mid pillars walk or mid
    // calibration interview, resolve the topic being probed (and clear the flag
    // once complete). Drives the prompt directive (below) and the fact-stamping
    // at Stage 21.
    // A finished calibration interview answers deterministically and skips the
    // LLM entirely (see `resolve_guided_or_answer`).
    let onboarding_turn = match resolve_guided_or_answer(GuidedStageInputs {
        ctx,
        input: &input,
        hooks,
        turn_ctx: &turn_ctx,
        turn_started,
        active_model: &active_model,
        user_message: &user_message,
        conv: &conv,
    })
    .await?
    {
        GuidedOutcome::Answered(result) => return Ok(*result),
        GuidedOutcome::Continue(turn) => turn,
    };

    // Stage 5: Load conversation history for LLM context. Bound the load to a
    // generous multiple of the compaction message cap so a long thread loads
    // its recent working set, not its full unbounded history (a 200-turn thread
    // would otherwise build a 200-message vector every turn before compaction
    // trims it to the cap). The multiple leaves headroom above `max_messages`
    // for compaction-block reconstruction; compaction still governs the final
    // in-prompt size.
    let history_load_limit = i64::try_from(
        ctx.harness_config_registry
            .current_compaction()
            .max_messages
            .saturating_mul(4)
            .clamp(80, 500),
    )
    .unwrap_or(160);
    let history = get_conversation_history(
        database,
        &input.conversation_id,
        &input.user_id,
        input.conversation_tenant_id,
        history_load_limit,
    )
    .await?;

    // Stage 6: Resolve coach runtime context.
    let coach_ctx = resolve_coach_ctx(ctx, &conv, input.conversation_tenant_id).await?;

    // Stages 7a–7h + 8: assemble the hardened system prompt and flatten the
    // conversation history into a ready-to-dispatch LLM message list.
    let (prompt_guard, pending_followup_ids, mut llm_messages, source_ids) =
        assemble_prompt_stage(AssemblePromptArgs {
            hooks,
            ctx,
            input: &input,
            profile,
            conv: &conv,
            coach_ctx: coach_ctx.as_ref(),
            history: &history,
            onboarding: onboarding_turn.as_ref(),
        })
        .await?;

    // Stages 9–14: pre-dispatch preparation followed by the multi-turn tool loop.
    let (mut result, provider_name) = dispatch_stage(
        DispatchStageArgs {
            hooks,
            ctx,
            input: &input,
            profile,
            active_model: &active_model,
            coach_ctx: coach_ctx.as_ref(),
            history: &history,
            source_ids: &source_ids,
            guided_flow_active: onboarding_turn.is_some(),
        },
        &mut llm_messages,
    )
    .await?;

    // Stages 14b–18: provider re-auth recovery short-circuit, then either
    // skip post-processing (recovery content is already canonical) or run
    // the standard guardrails/verification/hook chain on LLM-produced text.
    let post_processed = run_recovery_and_post_process(
        RecoveryAndPostProcessInputs {
            ctx,
            input: &input,
            profile,
            conv: &conv,
            coach_ctx: coach_ctx.as_ref(),
            prompt_guard: &prompt_guard,
        },
        &mut result,
        hooks,
    )
    .await;
    result.content = post_processed.content;
    let structured_content = post_processed.structured_content;
    let leak_replaced = post_processed.leak_replaced;
    let identity_leak = post_processed.identity_leak;

    // Stage 19: Persist assistant response.
    let token_count = result.usage.as_ref().map(|u| u.completion_tokens);
    let prompt_tokens = result.usage.as_ref().map(|u| u.prompt_tokens);
    // Persist the stripped reply, never a parroted scaffolding echo: a real
    // answer is unchanged by the strip, a pure `<tool_result>` echo reduces to
    // empty. Belt-and-suspenders with the read-side strip; the outbound reply
    // (`result.content`) is sent unchanged, only the durable copy is cleaned.
    let persisted_assistant_content = chat_tool_loop::strip_simulation_artifacts(&result.content);
    let assistant_params = AddMessageParams {
        tenant_id: input.conversation_tenant_id,
        conversation_id: &input.conversation_id,
        user_id: &input.user_id,
        role: "assistant",
        content: &persisted_assistant_content,
        token_count,
        finish_reason: result.finish_reason.as_deref(),
        prompt_tokens,
        model: Some(&active_model),
        structured_content: structured_content.as_deref(),
    };
    let (assistant_message, updated_conversation) =
        persist_assistant_response(database, &assistant_params, input.conversation_tenant_id)
            .await?;

    // Stage 19.5: Persist claim verdicts now that the assistant message is durable.
    #[cfg(feature = "tools-verification")]
    persist_verdicts_for_turn(
        ctx,
        &input,
        &conv,
        &assistant_message.id,
        &post_processed.pending_verdicts,
    )
    .await;

    // Stage 20: Tier 4 session finalize.
    finalize_session_state(
        &ctx.data,
        conv.session_id.as_deref(),
        &pending_followup_ids,
        input.conversation_tenant_id,
    )
    .await;

    // Stages 21/21b/21c: end-of-turn follow-through — Tier 2 memory extraction,
    // playbook advice capture, and the guided-flow probe record. All three key on
    // whether the reply actually reached the athlete, so the branch lives in one
    // place (see `finish_turn_follow_through`).
    finish_turn_follow_through(FinishTurnInputs {
        ctx,
        input: &input,
        conv: &conv,
        assistant_reply: &result.content,
        assistant_message_id: &assistant_message.id,
        onboarding: onboarding_turn.as_ref(),
        leak_replaced,
    })
    .await;

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
        identity_leak,
    };

    // Stage 22: UsageRecorder hook.
    #[allow(clippy::cast_possible_truncation)]
    let elapsed_ms = turn_started.elapsed().as_millis() as u64;
    if let Some(recorder) = hooks.usage_recorder {
        recorder
            .record(&turn_ctx, &dispatch_result, elapsed_ms)
            .await;
    }

    Ok(dispatch_result)
}

/// Everything the guided stage needs from the turn so far.
struct GuidedStageInputs<'a> {
    ctx: &'a ChatPipelineContext,
    input: &'a TurnInput,
    hooks: &'a PipelineHooks<'a>,
    turn_ctx: &'a TurnContext,
    turn_started: Instant,
    active_model: &'a str,
    user_message: &'a MessageRecord,
    conv: &'a ConversationRecord,
}

/// What Stage 4.5 concluded: either the turn is already answered, or it carries
/// on with an optional guided probe attached.
enum GuidedOutcome {
    /// The platform answered the turn itself; the caller returns this verbatim.
    Answered(Box<DispatchResult>),
    /// Run the turn normally, probing this topic if one is set.
    Continue(Option<stages::onboarding::OnboardingTurn>),
}

/// Stage 4.5: resolve the guided flow, answering the turn outright when a
/// calibration interview has just finished.
///
/// The wrap-up reports how many answers actually landed and names a safety
/// topic that produced none — claims only the platform can make truthfully,
/// since a coach asked to summarize its own interview has no view of what the
/// extractor wrote and every incentive to declare success.
async fn resolve_guided_or_answer(inputs: GuidedStageInputs<'_>) -> AppResult<GuidedOutcome> {
    let GuidedStageInputs {
        ctx,
        input,
        hooks,
        turn_ctx,
        turn_started,
        active_model,
        user_message,
        conv,
    } = inputs;

    let guided = stages::onboarding::resolve(
        ctx,
        conv,
        input.conversation_tenant_id,
        input.locale.as_deref(),
    )
    .await;

    match guided {
        stages::onboarding::GuidedResolution::Probe(turn) => {
            Ok(GuidedOutcome::Continue(Some(*turn)))
        }
        stages::onboarding::GuidedResolution::Inactive => Ok(GuidedOutcome::Continue(None)),
        stages::onboarding::GuidedResolution::CalibrationComplete(summary) => {
            let result = deliver_deterministic_reply(
                DeterministicReplyInputs {
                    ctx,
                    input,
                    hooks,
                    turn_ctx,
                    turn_started,
                    active_model: active_model.to_owned(),
                    user_message: user_message.clone(),
                    conv,
                },
                summary,
            )
            .await?;
            Ok(GuidedOutcome::Answered(Box::new(result)))
        }
    }
}

/// Everything [`deliver_deterministic_reply`] needs from the turn so far.
struct DeterministicReplyInputs<'a> {
    ctx: &'a ChatPipelineContext,
    input: &'a TurnInput,
    hooks: &'a PipelineHooks<'a>,
    turn_ctx: &'a TurnContext,
    turn_started: Instant,
    active_model: String,
    user_message: MessageRecord,
    conv: &'a ConversationRecord,
}

/// Answer the turn with platform-authored text, skipping the LLM entirely.
///
/// Used when the reply is something only the platform can state truthfully —
/// today, the calibration wrap-up, whose whole value is reporting what the
/// extractor actually wrote. Everything downstream of the dispatch still runs
/// as usual: the reply is persisted, the conversation reloaded, and the usage
/// hook fired, so the turn is indistinguishable from any other to callers.
///
/// Deliberately skipped: post-processing (guardrails, claim verification,
/// identity-leak detection) and memory extraction. All of them exist to police
/// model-generated text, and this text has no model behind it — running the
/// leak detector over a locale string the platform wrote would only ever
/// produce false positives, and extracting facts from it would feed the
/// athlete's own summary back in as new evidence.
async fn deliver_deterministic_reply(
    inputs: DeterministicReplyInputs<'_>,
    content: String,
) -> AppResult<DispatchResult> {
    let DeterministicReplyInputs {
        ctx,
        input,
        hooks,
        turn_ctx,
        turn_started,
        active_model,
        user_message,
        conv,
    } = inputs;

    let assistant_params = AddMessageParams {
        tenant_id: input.conversation_tenant_id,
        conversation_id: &input.conversation_id,
        user_id: &input.user_id,
        role: "assistant",
        content: &content,
        token_count: None,
        finish_reason: Some(DETERMINISTIC_FINISH_REASON),
        prompt_tokens: None,
        model: Some(&active_model),
        structured_content: None,
    };
    let (assistant_message, updated_conversation) = persist_assistant_response(
        ctx.repos.chat.as_ref(),
        &assistant_params,
        input.conversation_tenant_id,
    )
    .await?;

    finalize_session_state(
        &ctx.data,
        conv.session_id.as_deref(),
        &[],
        input.conversation_tenant_id,
    )
    .await;

    let dispatch_result = DispatchResult {
        content,
        usage: None,
        tool_calls_count: 0,
        tools_called: Vec::new(),
        turn_id: input.turn_id,
        finish_reason: Some(DETERMINISTIC_FINISH_REASON.to_owned()),
        activity_list: None,
        model: active_model,
        provider_name: DETERMINISTIC_PROVIDER.to_owned(),
        user_message,
        assistant_message,
        conversation: updated_conversation,
        identity_leak: None,
    };

    #[allow(clippy::cast_possible_truncation)]
    let elapsed_ms = turn_started.elapsed().as_millis() as u64;
    if let Some(recorder) = hooks.usage_recorder {
        recorder
            .record(turn_ctx, &dispatch_result, elapsed_ms)
            .await;
    }

    Ok(dispatch_result)
}

/// Provider label recorded for a turn the platform answered itself. Named
/// rather than blank so a usage row with no token counts is explicable.
const DETERMINISTIC_PROVIDER: &str = "platform";
/// Finish reason recorded for a platform-authored reply.
const DETERMINISTIC_FINISH_REASON: &str = "deterministic";

/// Resolve the active LLM model for a turn per the channel's [`ModelPolicy`].
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
async fn resolve_max_iterations(
    policy: MaxIterations,
    ctx: &ChatPipelineContext,
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

    if let Some(ref admin_config) = ctx.admin_config {
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
