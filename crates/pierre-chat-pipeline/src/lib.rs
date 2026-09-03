// ABOUTME: Unified chat pipeline — single orchestrator for web and messaging turn dispatch
// ABOUTME: Stages compose into ChatPipeline::run; surface profiles gate what each stage renders
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unified chat pipeline.
//!
//! Both the web/mobile chat endpoint (`POST /api/chat/conversations/{id}/messages`)
//! and the messaging ingress (Telegram, `WhatsApp`, Discord, Slack) run every
//! user turn through [`turn_service::execute`], which runs [`run`]. The
//! pipeline is surface-agnostic — what a surface can render is expressed via
//! [`SurfaceProfile`] and the hook traits in [`hooks`].
//!
//! # Layering
//!
//! - Channel adapters (the Axum `send_message` handler, the messaging ingress
//!   `dispatch_and_respond` wrapper) own transport only: they extract auth,
//!   name the surface's capabilities, and decide how to reply (HTTP body vs.
//!   webhook outbound delivery). Ordering locks, panic boundaries and
//!   empty-reply guards are theirs too, because they are properties of a
//!   channel.
//! - [`turn_service::execute`] owns the turn: the usage caps, the slash
//!   dispatch, the turn's locale, the tenant's own model key, and the counters
//!   the next turn's check reads. One ladder, so a capability cannot land on
//!   one surface and miss another.
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

pub mod envelope;
pub mod hooks;
pub mod mcp_bridge;
pub mod quota_policy;
pub mod recorders;
pub(crate) mod recovery;
pub mod stages;
pub mod surface_profile;
mod tool_budget;
pub mod turn;
pub mod turn_service;
pub mod usage_counters;

pub use mcp_bridge::McpBridgeProvider;

pub use envelope::{
    build_envelope, ActionKind, AssistantTurn, NoticeKind, QuotaLevel, QuotaState,
    QuotaWarningState, ReconnectPrompt, ReplyBlock, ReplyBlockKind, SceneImage, TurnAction,
    TurnEnvelope, TurnState, TurnTelemetry, VerdictChip,
};
pub use hooks::{
    AgUiRun, PipelineHooks, ProgressKind, ResponsePostProcess, ScenePublishRequest, ScenePublisher,
    TurnEvent, TurnEventSink, TurnProgress, STAGE_STATUS_FINISHED, STAGE_STATUS_STARTED,
};
pub use quota_policy::{check_pre_chat_quotas_scoped, PreChatScope};
pub use stages::command_persistence::{CommandPersistence, PersistedCommandReply};
pub use surface_profile::{
    BlockSupport, MessagingTransportCaps, ModelPolicy, ProgressiveSupport, ProseFormat,
    ProviderStreaming, RenderCapabilities, SurfaceId, SurfaceProfile, SurfaceRequest, TurnBudget,
};
pub use turn_service::{
    detect_turn_locale, dispatch_slash, execute, CommandTurn, ServedTurn, SlashRequest, TurnRequest,
};
pub use usage_counters::{
    increment_usage_counters_scoped, tokens_from_envelope, UsageIncrementScope,
};
// Re-exported so that flows which build `ToolLoopParams` directly (the
// messaging ingress) can attach the same per-call recorder the chat
// pipeline uses.
pub use recorders::UsageRepoCallRecorder as TurnCallRecorder;
pub use turn::{CreateConversationResult, TurnInput, TurnOrigin, UserMessageResult};

use recovery::{run_recovery_and_post_process, RecoveryAndPostProcessInputs};
use std::sync::Arc;

use chrono::Utc;
use pierre_agui::AgUiEvent;
use pierre_commands::CommandHandlerRegistry;
use pierre_config::environment::{LlmProviderType, ServerConfig};
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    EvidenceRegistry, MessagingStringsRegistry, PromptRegistry, ToolDescriptionRegistry,
};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{
    AddMessageParams, CoachRuntimeContext, MemberFitnessSnapshot, OnboardingState, TenantId,
    UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON, WITHHELD_REPLY_FINISH_REASON,
};
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::RepositoryRegistry;
use pierre_llm::health::{LlmHealthState, LlmHealthStatus};
use pierre_llm::{ChatMessage, ChatProvider, LlmProvider};
use pierre_messaging::commands::CommandRegistry;
use pierre_runtime_context::{AdminConfigLookup, CommandCtx, DataContext};
use pierre_services::advice_capture::{
    spawn_capture_advice, AdviceCaptureStrategy, CapturedTurn, HeuristicGatedLlmExtraction,
};
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_services::memory_dedup::DedupConfig;
use pierre_services::memory_extraction::{
    spawn_extract_for_turn, SpawnedExtractionRequest, WITHHELD_REPLY_TRANSCRIPT_MARKER,
};
use pierre_services::tenant_chat_provider::TenantChatProviderCache;
use pierre_sse::SseManager;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution as chat_tool_loop;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use tracing::field::Empty;
use tracing::{info, warn};

use stages::deterministic_reply::PLATFORM_REPLY_TRANSCRIPT_MARKER;
use stages::followups::{ensure_coach_session_attached, finalize_session_state};
use stages::persistence::{
    append_platform_prompt, get_conversation_history, persist_assistant_response,
    resolve_turn_prompt,
};
#[cfg(feature = "tools-verification")]
use stages::verification::persist_pending_verdicts;
use tool_budget::resolve_max_iterations;

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
    /// Narrow runtime context the slash-command handlers run against.
    ///
    /// The same composition root behind [`Self::tool_runtime`], behind a
    /// second trait so `pierre-runtime-context` need not know about tool
    /// dispatch. Held here because
    /// [`turn_service::dispatch_slash`] is the single dispatch authority for
    /// every chat surface, and a surface must not be able to reach a command
    /// handler by any other route.
    pub command_ctx: Arc<dyn CommandCtx>,
    /// Slash-command catalog, built from `commands/*.md` at startup. `None` in
    /// a host that skips the catalog, which resolves every `/`-prefixed text
    /// to a coaching turn.
    pub command_registry: Option<Arc<CommandRegistry>>,
    /// Handler-name to handler mapping, populated alongside
    /// [`Self::command_registry`].
    pub command_handler_registry: Option<Arc<CommandHandlerRegistry>>,
    /// Short-TTL cache of per-`(tenant, athlete)` chat providers, so a turn
    /// resolves a stored BYO LLM key without a database read on the common
    /// no-key path.
    pub tenant_chat_providers: TenantChatProviderCache,
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
    /// Contract telling a granted coach how to embed an inline chart or table,
    /// and which of those rules the platform enforces. Appended only when the
    /// coach carries a non-empty `visuals:` grant and the channel can render a
    /// block.
    pub visual_blocks_prompt: String,
    /// JSON Schema texts keyed by schema id, compiled once on first use.
    ///
    /// Keyed rather than singular because a reply can carry more than one kind
    /// of structured payload: a whole-reply workout plan (`structured-workout`)
    /// or inline visual blocks (`dravr-viz`), which are a different content
    /// model and can appear several times in one reply.
    ///
    /// Injected like every other contremaitre datum the pipeline needs — the
    /// prompts above arrive the same way — rather than fetched here, so the
    /// pipeline keeps one entry point for configuration.
    pub structured_output_schemas: stages::structured_output::SchemaTexts,
    /// Memory extraction system prompt (used by Tier 2 background extraction).
    pub memory_extraction_prompt: String,
    /// Optional MCP bridge — mints the per-turn MCP servers an ACP provider
    /// (Copilot Headless) exposes for native Dravr tool calling. `None`
    /// disables the bridge (text-based tool calling is used instead).
    pub mcp_bridge: Option<Arc<dyn McpBridgeProvider>>,
}

/// Announce that `step` was entered, on whichever progress rails are wired.
///
/// Both rails carry the same fact to different audiences: the turn's own
/// event stream is what an in-app client renders in its progress strip, and
/// the AG-UI registry is what the messaging status bridge reads in process to
/// edit a Telegram/Slack/Discord placeholder. A surface wires one, the other,
/// or neither.
///
/// Extracted from the stage calls so `run_turn` stays under the workspace
/// cognitive-complexity budget — the branches live in the helper rather than
/// at every emission site.
async fn emit_step_started(hooks: &PipelineHooks<'_>, step: &str) {
    emit_step(hooks, step, STAGE_STATUS_STARTED).await;
}

/// Announce that `step` was left, on whichever progress rails are wired.
async fn emit_step_finished(hooks: &PipelineHooks<'_>, step: &str) {
    emit_step(hooks, step, STAGE_STATUS_FINISHED).await;
}

/// Shared body of [`emit_step_started`] and [`emit_step_finished`].
async fn emit_step(hooks: &PipelineHooks<'_>, step: &str, status: &str) {
    if let Some(sink) = &hooks.stream_sink {
        // A closed channel means the client hung up mid-turn; the turn still
        // runs to completion so its reply is persisted.
        let _ = sink.send(TurnEvent::Progress(TurnProgress::stage(step, status)));
    }
    if let Some(agui) = &hooks.agui {
        let event = if status == STAGE_STATUS_STARTED {
            AgUiEvent::step_started(agui.run_id.clone(), step)
        } else {
            AgUiEvent::step_finished(agui.run_id.clone(), step)
        };
        agui.sink.emit(&event).await;
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
    /// What the turn's surface can render, plus its resolved locale.
    profile: &'a SurfaceProfile,
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

/// Fire the Tier 2 background fact extraction for a completed turn, stamping the
/// onboarding pillar/source/force-kind when the conversation is mid pillar walk.
///
/// `answered` is the guided topic the athlete's inbound message replies to (see
/// [`stages::onboarding::answered_target`]), which is what extraction reads —
/// never the topic this turn goes on to ask.
/// The tool that persists a training plan.
///
/// Named here because the memory extractor's coach-prescription filter is
/// justified entirely by this tool having run: it drops schedule facts on the
/// grounds that plans live in the plan store instead. When it did not run, the
/// drop deletes the only copy (registre#203).
const SAVE_TRAINING_PLAN_TOOL: &str = "save_training_plan";

fn spawn_turn_extraction(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    assistant_reply: &str,
    assistant_message_id: &str,
    answered: Option<stages::onboarding::GuidedTarget>,
    plan_was_saved: bool,
) {
    let (pillar, source, force_kind) = stages::onboarding::extraction_params_or_default(answered);
    // A guided answer is stamped under the TOOL tenant — the athlete's own,
    // which a room turn resolves while its conversation row stays under the
    // channel tenant. The subject gate in `stages::onboarding::resolve` means
    // `answered` is only ever `Some` on the walking member's own turn, and the
    // dossier the walk composes reads that same tenant, so the answer lands
    // where the walk (and the athlete's own DM) will find it. Ordinary turns
    // keep the conversation-tenant stamp; the precedent for a divergent stamp
    // is `spawn_turn_advice_capture` below.
    let extraction_tenant = if answered.is_some() {
        input.tool_tenant_id
    } else {
        input.conversation_tenant_id
    };
    // The de-dup tunables are read per turn, so a threshold change through
    // contremaitre reaches the next extraction without a deploy.
    let memory_config = ctx.harness_config_registry.current_memory();
    spawn_extract_for_turn(
        Arc::clone(&ctx.repos.memory),
        ctx.chat_provider.as_ref().map(Arc::clone),
        DedupConfig {
            candidate_limit: memory_config.dedup_candidate_limit as usize,
        },
        ctx.memory_extraction_prompt.clone(),
        SpawnedExtractionRequest {
            tenant_id: extraction_tenant,
            user_id: input.user_id.clone(),
            coach_id: input.turn_coach_id(conv).map(ToOwned::to_owned),
            user_message: input.content.clone(),
            assistant_reply: assistant_reply.to_owned(),
            source_msg_id: Some(assistant_message_id.to_owned()),
            pillar,
            source,
            force_kind,
            plan_was_saved,
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
            coach_slug: input.turn_coach_id(conv).map(ToOwned::to_owned),
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
    inputs: &FinishTurnInputs<'_>,
    answered: Option<stages::onboarding::GuidedTarget>,
) {
    let FinishTurnInputs {
        ctx,
        input,
        conv,
        assistant_reply,
        assistant_message_id,
        leak_replaced,
        plan_was_saved,
        ..
    } = *inputs;
    if leak_replaced {
        spawn_turn_extraction(
            ctx,
            input,
            conv,
            WITHHELD_REPLY_TRANSCRIPT_MARKER,
            assistant_message_id,
            answered,
            plan_was_saved,
        );
        return;
    }
    spawn_turn_extraction(
        ctx,
        input,
        conv,
        assistant_reply,
        assistant_message_id,
        answered,
        plan_was_saved,
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
        input.turn_coach_id(conv),
        assistant_message_id,
        pending_verdicts,
    )
    .await;
}

/// Inputs for [`finish_turn_follow_through`].
#[derive(Clone, Copy)]
struct FinishTurnInputs<'a> {
    ctx: &'a ChatPipelineContext,
    input: &'a TurnInput,
    conv: &'a ConversationRecord,
    assistant_reply: &'a str,
    assistant_message_id: &'a str,
    onboarding: Option<&'a stages::onboarding::OnboardingTurn>,
    leak_replaced: bool,
    /// Whether `save_training_plan` ran on this turn. Decides whether the
    /// memory extractor may drop a coach-prescription schedule fact on the
    /// grounds that the plan is stored elsewhere (registre#203).
    plan_was_saved: bool,
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
        onboarding,
        leak_replaced,
        ..
    } = inputs;
    // The message being extracted answers the probe the PREVIOUS turn delivered
    // — `onboarding.target` is the question this turn asks, one topic further
    // on. Stamping with it filed every guided answer under the next topic's
    // pillar and forced kind.
    let answered = onboarding.and_then(|turn| stages::onboarding::answered_target(&turn.state));
    spawn_turn_background_learning(&inputs, answered);
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
    /// What the turn's surface can render, plus its resolved locale.
    profile: &'a SurfaceProfile,
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
    /// Group roster (empty outside a group conversation) for peer grounding.
    peer_roster: &'a [MemberFitnessSnapshot],
}

/// Run the dispatch stage with AG-UI step emissions.
async fn dispatch_stage(
    args: DispatchStageArgs<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> AppResult<(ToolLoopResult, String)> {
    emit_step_started(args.hooks, "dispatch").await;
    let max_iterations =
        resolve_max_iterations(args.profile.budget, args.ctx, args.coach_ctx).await;
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
            peer_roster: args.peer_roster,
        },
        llm_messages,
        max_iterations,
        args.hooks.stream_sink.clone(),
    )
    .await;
    emit_step_finished(args.hooks, "dispatch").await;
    result
}
/// Pick the `call_type` string used for per-LLM-call `llm_usage` rows
/// produced by the chat pipeline.
pub(crate) fn call_type_for_profile(profile: &SurfaceProfile) -> &'static str {
    profile.surface.call_type()
}

/// Record the turn span's deferred `coach_id`/`group_id` fields (declared
/// `Empty` on the `run` span) once the conversation record resolves them, so
/// every log line for the rest of the turn names the coach answering it — the
/// mentioned one on a `@handle` turn — and the group. A no-op when unset.
fn record_turn_span_context(coach_id: Option<&str>, group_id: Option<&str>) {
    let span = tracing::Span::current();
    if let Some(coach_id) = coach_id {
        span.record("coach_id", coach_id);
    }
    if let Some(group_id) = group_id {
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
        channel = profile.surface.as_str(),
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
    profile: &SurfaceProfile,
    hooks: &PipelineHooks<'_>,
) -> AppResult<TurnEnvelope> {
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

/// The `finish_reason` to persist for this turn's assistant row.
///
/// A withheld turn stores the localized apology the athlete saw, not the model's
/// output, so it is stamped with [`WITHHELD_REPLY_FINISH_REASON`]. That lets
/// `push_history_row` drop it from later prompts by marker instead of by
/// pattern-matching prose that is authored remotely in five locales.
///
/// Private and extracted purely to keep `run_turn` inside the cognitive-
/// complexity budget; it adds no public API surface.
const fn persisted_finish_reason(
    leak_replaced: bool,
    capability_claim_unverified: bool,
    model_finish_reason: Option<&str>,
) -> Option<&str> {
    if leak_replaced {
        Some(WITHHELD_REPLY_FINISH_REASON)
    } else if capability_claim_unverified {
        // Ranked below the withhold: a withheld row holds platform text the
        // model never wrote, which is the stronger reason to keep it out of a
        // prompt. Both stamps drop the row at replay.
        Some(UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON)
    } else {
        model_finish_reason
    }
}

/// Run the body of a single turn.
async fn run_turn(
    ctx: &ChatPipelineContext,
    input: TurnInput,
    profile: &SurfaceProfile,
    hooks: &PipelineHooks<'_>,
) -> AppResult<TurnEnvelope> {
    // The standing [`turn_service::execute`]'s pre-turn check measured, carried
    // on the input. The envelope surfaces it as a notice block; a hard breach
    // never reaches here, having refused the turn already.
    let quota = input.quota.clone();

    let database = ctx.repos.chat.as_ref();

    // Stage 2: Resolve the prompt this turn answers. Persisted as the
    // athlete's message when they sent it, and deliberately not when the
    // platform composed it — see `resolve_turn_prompt`.
    let msg_result = resolve_turn_prompt(database, ctx.repos.groups.as_ref(), &input).await?;
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
    record_turn_span_context(input.turn_coach_id(&conv), conv.group_id.as_deref());

    // Stage 4.5: Guided mode. If this conversation is mid pillars walk or mid
    // calibration interview, resolve the topic being probed (and clear the flag
    // once complete). Drives the prompt directive (below) and the fact-stamping
    // at Stage 21.
    // A finished calibration interview answers deterministically and skips the
    // LLM entirely (see `resolve_guided_or_answer`).
    // A platform-composed prompt is not the athlete answering the probe, so it
    // never drives the guided flow. Left in, a finished calibration would
    // answer this turn with its wrap-up (`GuidedOutcome::Answered` returns
    // outright) and the athlete would get an interview summary where their
    // history was supposed to arrive — the same impersonation as the `user`
    // row above, one table over.
    let onboarding_turn = if input.origin.persists_user_row() {
        match resolve_guided_or_answer(GuidedStageInputs {
            ctx,
            input: &input,
            profile,
            active_model: &active_model,
            user_message: &user_message,
            conv: &conv,
        })
        .await?
        {
            GuidedOutcome::Answered(result) => return Ok(*result),
            GuidedOutcome::Continue(turn) => turn,
        }
    } else {
        None
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
    let mut history = get_conversation_history(
        database,
        &input.conversation_id,
        &input.user_id,
        input.conversation_tenant_id,
        history_load_limit,
    )
    .await?;

    append_platform_prompt(&mut history, &user_message, input.origin);

    stages::coach_mention::apply_prompt_text(&mut history, &user_message.id, &input);

    // Stage 6: Resolve the coach runtime context this turn answers as.
    let coach_ctx = stages::prompt_assembly::resolve_turn_coach_ctx(ctx, &input, &conv).await?;

    // Stages 7a–7h + 8: assemble the hardened system prompt and flatten the
    // conversation history into a ready-to-dispatch LLM message list.
    let (prompt_guard, pending_followup_ids, mut llm_messages, source_ids, group_roster) =
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
            peer_roster: &group_roster,
        },
        &mut llm_messages,
    )
    .await?;

    // Stages 14a–18: the bounded identity re-ask, the provider re-auth recovery
    // short-circuit, then either skip post-processing (recovery content is
    // already canonical) or run the standard guardrails/verification/hook chain
    // on LLM-produced text.
    let (post_processed, reconnect) = run_recovery_and_post_process(
        RecoveryAndPostProcessInputs {
            ctx,
            input: &input,
            profile,
            conv: &conv,
            coach_ctx: coach_ctx.as_ref(),
            prompt_guard: &prompt_guard,
            llm_messages: &llm_messages,
            active_model: &active_model,
            peer_roster: &group_roster,
        },
        &mut result,
        hooks,
    )
    .await;
    result.content = post_processed.content;
    let content_blocks = post_processed.content_blocks;
    let leak_replaced = post_processed.leak_replaced;
    let identity_leak = post_processed.identity_leak;
    let verdict_chips = post_processed.verdict_chips;

    // Stage 19: Persist assistant response.
    let token_count = result.usage.as_ref().map(|u| u.completion_tokens);
    let prompt_tokens = result.usage.as_ref().map(|u| u.prompt_tokens);
    // Strip once; send and persist the same bytes. Cleaning only the durable copy
    // left the wire carrying scaffolding the record never showed (registre#40) — a
    // real answer is unchanged, scaffolding-only empties into a localized error.
    result.content = chat_tool_loop::strip_simulation_artifacts(&result.content);
    let persisted_assistant_content = result.content.clone();
    let assistant_params = AddMessageParams {
        tenant_id: input.conversation_tenant_id,
        conversation_id: &input.conversation_id,
        user_id: &input.user_id,
        role: "assistant",
        content: &persisted_assistant_content,
        token_count,
        finish_reason: persisted_finish_reason(
            leak_replaced,
            result.capability_claim_unverified,
            result.finish_reason.as_deref(),
        ),
        prompt_tokens,
        model: Some(&active_model),
        content_blocks: content_blocks.as_deref(),
    };
    let (assistant_message, updated_conversation) = persist_assistant_response(
        database,
        ctx.repos.groups.as_ref(),
        &assistant_params,
        input.conversation_tenant_id,
    )
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
        plan_was_saved: result
            .tools_called
            .iter()
            .any(|t| t == SAVE_TRAINING_PLAN_TOOL),
    })
    .await;

    // Stage 22: publish the reply's chart specs for a surface that fetches
    // pixels. After persistence on purpose — the specs are addressed by
    // message id, so there is nothing to sign before the row exists.
    let scene_images = publish_scenes(hooks, profile, &input, &assistant_message);

    // A surface without an activity panel has the list folded into its prose,
    // and a raw 186-row history folded into a chat bubble is unreadable — so
    // the list is shaped for the fold here. Both this and the block-or-fold
    // decision below read `activity_list_card`: one capability, two
    // consequences.
    // Captured before the fold consumes the list: this is the honest "the model
    // asked for activities and got them" signal, which `tools_called` cannot be
    // because the platform injects the tool name when it prefetches.
    let activity_list_captured = result.activity_list.is_some();
    let activity_list = if profile.render.blocks.activity_list_card {
        result.activity_list
    } else {
        stages::activity_fold::shape_for_fold(
            result.activity_list.as_deref(),
            &ctx.messaging_strings_registry,
            &profile.locale,
        )
    };

    Ok(build_envelope(
        profile,
        TurnState {
            turn_id: input.turn_id,
            user_message,
            assistant_message,
            conversation: updated_conversation,
            content: result.content,
            finish_reason: result.finish_reason,
            activity_list,
            telemetry: TurnTelemetry {
                model: active_model,
                provider_name,
                tools_called: result.tools_called,
                tool_calls_count: result.tool_calls_count,
                activity_list_captured,
                usage: result.usage,
                identity_leak,
            },
            quota,
            reconnect,
            verdict_chips,
            scene_images,
            actions: Vec::new(),
            actions_title: None,
        },
    ))
}

/// Ask the wired [`ScenePublisher`] to mint an image per stored chart spec.
///
/// Returns empty when the surface draws specs itself, when no publisher is
/// wired, or when the reply carried no blocks — all three mean the reply keeps
/// the sentences the coach wrote around the chart, which is the contract the
/// visual-blocks prompt sets.
fn publish_scenes(
    hooks: &PipelineHooks<'_>,
    profile: &SurfaceProfile,
    input: &TurnInput,
    assistant_message: &MessageRecord,
) -> Vec<envelope::SceneImage> {
    if !profile.render.blocks.scene_raster {
        return Vec::new();
    }
    let (Some(publisher), Some(specs)) = (
        hooks.scene_publisher,
        assistant_message.content_blocks.as_deref(),
    ) else {
        return Vec::new();
    };
    publisher.publish(&ScenePublishRequest {
        specs,
        conversation_id: &input.conversation_id,
        user_id: &input.user_id,
        tenant_id: input.conversation_tenant_id,
        message_id: &assistant_message.id,
        locale: &profile.locale,
    })
}

/// Everything the guided stage needs from the turn so far.
struct GuidedStageInputs<'a> {
    ctx: &'a ChatPipelineContext,
    input: &'a TurnInput,
    profile: &'a SurfaceProfile,
    active_model: &'a str,
    user_message: &'a MessageRecord,
    conv: &'a ConversationRecord,
}

/// What Stage 4.5 concluded: either the turn is already answered, or it carries
/// on with an optional guided probe attached.
enum GuidedOutcome {
    /// The platform answered the turn itself; the caller returns this verbatim.
    Answered(Box<TurnEnvelope>),
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
        profile,
        active_model,
        user_message,
        conv,
    } = inputs;

    let guided = stages::onboarding::resolve(
        ctx,
        conv,
        &input.user_id,
        input.conversation_tenant_id,
        input.tool_tenant_id,
        &profile.locale,
    )
    .await;

    match guided {
        stages::onboarding::GuidedResolution::Probe(turn) => {
            Ok(GuidedOutcome::Continue(Some(*turn)))
        }
        stages::onboarding::GuidedResolution::Inactive => Ok(GuidedOutcome::Continue(None)),
        stages::onboarding::GuidedResolution::CalibrationComplete { summary, answered } => {
            let result = stages::deterministic_reply::deliver(
                stages::deterministic_reply::DeterministicReplyInputs {
                    ctx,
                    input,
                    profile,
                    active_model: active_model.to_owned(),
                    user_message: user_message.clone(),
                    conv,
                },
                summary,
            )
            .await?;
            // This turn carries the athlete's answer to the interview's LAST
            // question, and the reply skipping the LLM does not make that
            // answer any less theirs. Returning here without extracting dropped
            // it on every calibration run — and the last core topic is recovery
            // speed, which is safety-critical and the sole writer of its kind,
            // so its absence is exactly what the wrap-up would have named.
            spawn_turn_extraction(
                ctx,
                input,
                conv,
                PLATFORM_REPLY_TRANSCRIPT_MARKER,
                &result.assistant.message.id,
                answered,
                // A guided calibration turn skips the LLM entirely, so no tool
                // ran and no plan was stored.
                false,
            );
            Ok(GuidedOutcome::Answered(Box::new(result)))
        }
    }
}

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
