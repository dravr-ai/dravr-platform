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

pub mod agent_welcome;
/// The shared state every stage reads, built once by the composition root.
mod context;
pub mod envelope;
/// Where an athlete's turn meets the first-run done rules (carnet#1099).
mod first_run_hooks;
/// What a turn still owes once its reply is persisted.
mod follow_through;
pub mod hooks;
pub mod language;
pub mod mcp_bridge;
pub mod next_steps;
pub mod quota_policy;
pub mod recorders;
pub(crate) mod recovery;
pub mod stages;
pub mod suggestions;
pub mod surface_profile;
pub(crate) mod team_offer;
mod tool_budget;
pub mod turn;
pub mod turn_service;
pub mod turn_stop;
pub mod usage_counters;

pub use context::ChatPipelineContext;
pub use mcp_bridge::{McpBridgeProvider, ToolSessionTurn};

pub use envelope::{
    build_envelope, ActionKind, AssistantTurn, NoticeKind, QuotaLevel, QuotaState,
    QuotaWarningState, ReconnectPrompt, ReplyBlock, ReplyBlockKind, SceneImage, TurnAction,
    TurnEnvelope, TurnState, TurnTelemetry, VerdictChip,
};
pub use hooks::{
    AgUiRun, PipelineHooks, ProgressKind, ResponsePostProcess, ScenePublishRequest, ScenePublisher,
    TurnEvent, TurnEventSink, TurnFailure, TurnProgress, STAGE_STATUS_FINISHED,
    STAGE_STATUS_STARTED,
};
pub use quota_policy::{check_pre_chat_quotas_scoped, settle_turn_notice, PreChatScope};
pub use stages::command_persistence::{CommandPersistence, PersistedCommandReply};
pub use surface_profile::{
    BlockSupport, MessagingTransportCaps, ProgressiveSupport, ProseFormat, RenderCapabilities,
    SurfaceId, SurfaceProfile, SurfaceRequest, TurnBudget,
};
pub use turn_service::{
    dispatch_slash, execute, CommandTurn, ServedTurn, SlashRequest, TurnRequest,
};
pub use usage_counters::{increment_usage_counters_scoped, UsageIncrementScope};
// Defined in `pierre-core` so the command dispatcher, which this crate
// depends on, reads the same type; re-exported because [`TurnRequest`] and
// [`SlashRequest`] carry it.
pub use pierre_core::models::{InputSource, TurnOrigin};
// Defined in `pierre-commands`, where the handler records it; re-exported
// because [`CommandTurn`] carries it.
pub use pierre_commands::RotatedTo;
pub use turn::{TurnInput, UserMessageResult};

#[cfg(feature = "tools-verification")]
use follow_through::persist_verdicts_for_turn;
use follow_through::{finish_turn_follow_through, FinishTurnInputs};
use next_steps::NextSteps;
use pierre_core::uuid_utils::parse_uuid;
use recovery::{run_recovery_and_post_process, RecoveryAndPostProcessInputs};

use chrono::{DateTime, Utc};
use pierre_agui::AgUiEvent;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{
    AddMessageParams, AgentRuntimeContext, CoachingPersona, GuidedFlow, MemberFitnessSnapshot,
    UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON, WITHHELD_REPLY_FINISH_REASON,
};
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_llm::health::LlmHealthStatus;
use pierre_llm::ChatMessage;
use pierre_providers::ai_scope;
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_services::provider_notice;
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::tool_execution as chat_tool_loop;
use tracing::{debug, info, warn};

use stages::deterministic_reply::PLATFORM_REPLY_TRANSCRIPT_MARKER;
use stages::followups::{ensure_agent_session_attached, finalize_session_state};
use stages::persistence::{
    append_platform_prompt, get_conversation_history, persist_assistant_response,
    resolve_turn_prompt,
};
use stages::provider_stop;
use tool_budget::resolve_max_iterations;

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
        if sink
            .send(TurnEvent::Progress(TurnProgress::stage(step, status)))
            .is_err()
        {
            debug!(
                step,
                "turn stream receiver dropped; stage event not delivered"
            );
        }
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

/// Run [`stages::prompt_assembly::assemble_prompt_and_messages`] between
/// AG-UI `STEP_STARTED`/`STEP_FINISHED`, outside [`run_turn`]'s complexity.
async fn assemble_prompt_stage(
    hooks: &PipelineHooks<'_>,
    turn: stages::prompt_assembly::PromptTurn<'_>,
) -> AppResult<stages::prompt_assembly::AssembledPrompt> {
    emit_step_started(hooks, "prompt_assembly").await;
    let result = stages::prompt_assembly::assemble_prompt_and_messages(turn).await;
    emit_step_finished(hooks, "prompt_assembly").await;
    result
}

/// The tool that persists a training plan.
///
/// Named here because the memory extractor's agent-prescription filter is
/// justified entirely by this tool having run: it drops schedule facts on the
/// grounds that plans live in the plan store instead. When it did not run, the
/// drop deletes the only copy (registre#203).
const SAVE_TRAINING_PLAN_TOOL: &str = "save_training_plan";

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
    /// Optional agent runtime context.
    agent_ctx: Option<&'a AgentRuntimeContext>,
    /// Persisted conversation history.
    history: &'a [MessageRecord],
    /// Per-message history-row ids parallel to `llm_messages` (`None` for the
    /// system prompt), threaded to Tier 1 compaction for id-anchored blocks.
    source_ids: &'a [Option<String>],
    /// The turn clock, inserted before the athlete's message after compaction.
    turn_clock: &'a str,
    /// Which guided conversational flow owns this turn, when one does (see
    /// `DispatchLlmInputs::guided_flow`).
    guided_flow: Option<GuidedFlow>,
    /// Group roster (empty outside a group conversation) for peer grounding.
    peer_roster: &'a [MemberFitnessSnapshot],
    /// The agent the turn answers as, bound on the tool executor.
    turn_agent_id: Option<&'a str>,
    /// Whose data the turn's tools run against (see `DispatchLlmInputs::seat`).
    seat: TurnSeat,
}

/// Run the dispatch stage with AG-UI step emissions.
async fn dispatch_stage(
    args: DispatchStageArgs<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> AppResult<stages::tool_dispatch::DispatchedTurn> {
    emit_step_started(args.hooks, "dispatch").await;
    let max_iterations =
        resolve_max_iterations(args.profile.budget, args.ctx, args.agent_ctx).await;
    let result = stages::tool_dispatch::dispatch_llm_with_tools(
        stages::tool_dispatch::DispatchLlmInputs {
            ctx: args.ctx,
            input: args.input,
            profile: args.profile,
            active_model: args.active_model,
            agent_ctx: args.agent_ctx,
            history: args.history,
            source_ids: args.source_ids,
            turn_clock: args.turn_clock,
            guided_flow: args.guided_flow,
            peer_roster: args.peer_roster,
            turn_agent_id: args.turn_agent_id,
            seat: args.seat,
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

/// Record the turn span's deferred `agent_id`/`group_id` fields (declared
/// `Empty` on the `chat.turn` span [`turn_service::execute`] opens) once the
/// conversation record resolves them, so every log line for the rest of the
/// turn names the agent answering it — the mentioned one on a `@handle` turn —
/// and the group. A no-op when unset.
///
/// Written through [`pierre_logging::record_in_scope`], so the fields reach
/// the `chat.turn` span however many spans sit between it and the caller.
pub(crate) fn record_turn_span_context(agent_id: Option<&str>, group_id: Option<&str>) {
    if let Some(agent_id) = agent_id {
        pierre_logging::record_in_scope("agent_id", &agent_id);
    }
    if let Some(group_id) = group_id {
        pierre_logging::record_in_scope("group_id", &group_id);
    }
}

/// Run a single turn through the unified chat pipeline.
///
/// Runs under the `chat.turn` span [`turn_service::execute`] opens, so the
/// usage caps, slash commands and usage recording around the pipeline share
/// one span with every stage, LLM call and tool call inside it.
///
/// # Errors
///
/// Returns `AppError` variants produced by any stage.
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

    let (persona, asker_created_at) = stages::prompt_assembly::resolve_user_persona_and_created_at(
        ctx.repos.users.as_ref(),
        &input.user_id,
    )
    .await;
    let origin = input.analytics_origin();
    let first_run = first_run_hooks::TurnFacts::of(&input);
    info!(
        target: "notify",
        event = "chat.question_asked",
        persona = persona.as_str(),
        origin = origin,
        "user asked a question"
    );

    refuse_when_llm_unhealthy(ctx).await?;

    // A turn exists to feed a model, so every read its stages make is a read
    // for one: each provider's AI policy governs what the prompt builders see
    // (carnet#723). Tools scope themselves the same way in the executor; a
    // chart drawn for the athlete lifts it (`ai_scope::for_display`). The
    // athlete's AI consents govern them too (carnet#726): a provider whose
    // data they have not consented to hand to AI reaches no prompt.
    let consent_withheld = match parse_uuid(&input.user_id) {
        Ok(user_id) => {
            provider_notice::ai_consent_withheld(
                &ctx.repos,
                input.tool_tenant_id.as_uuid(),
                user_id,
            )
            .await
        }
        // An unparseable user owns no consent: every AI-consent provider is
        // withheld, as an undeclared read would withhold it.
        Err(_) => ai_scope::ai_consent_withheld().as_ref().clone(),
    };
    let outcome = ai_scope::for_model(ai_scope::with_ai_consent(
        consent_withheld,
        Box::pin(run_turn(ctx, input, profile, hooks)),
    ))
    .await;

    if outcome.is_ok() {
        // A real served turn is the strongest proof the LLM provider is
        // live — stamp it so the periodic health probe can skip its billed
        // synthetic `copilot --acp` round-trip while real traffic flows.
        ctx.llm_health.note_success();
    }
    // A withheld platform reply reached nobody, so no answer was delivered.
    if let Some(envelope) = outcome.as_ref().ok().filter(|envelope| envelope.posted) {
        let served = ai_scope::current_provenance().is_some_and(|p| p.served_provider_data());
        emit_answer_delivered(
            persona,
            origin,
            asker_created_at,
            &envelope.telemetry,
            served,
        );
        first_run
            .after_turn(&ctx.repos, envelope, profile, served)
            .await;
    }

    emit_run_outcome(hooks, &outcome).await;
    outcome
}

/// Fail the turn fast with a 503 while the LLM health probe reads
/// `Unhealthy`, rather than sending it to a provider known to be down.
///
/// # Errors
///
/// Returns [`ErrorCode::ResourceUnavailable`] naming the provider the probe
/// found down.
async fn refuse_when_llm_unhealthy(ctx: &ChatPipelineContext) -> AppResult<()> {
    let snapshot = ctx.llm_health.snapshot().await;
    if !matches!(snapshot.status, LlmHealthStatus::Unhealthy) {
        return Ok(());
    }
    let provider = snapshot.provider.unwrap_or_else(|| "llm".to_owned());
    let last_error = snapshot.error.unwrap_or_else(|| "unknown".to_owned());
    warn!(
        provider = %provider,
        last_error = %last_error,
        "LLM probe is Unhealthy — failing chat turn fast with 503"
    );
    Err(AppError::new(
        ErrorCode::ResourceUnavailable,
        format!(
            "Our AI provider ({provider}) is temporarily unavailable. We've been notified — please try again in a minute or two."
        ),
    ))
}

/// Close the AG-UI run [`run`] opened, when the surface streams one: finished
/// on a served turn, errored otherwise.
async fn emit_run_outcome(hooks: &PipelineHooks<'_>, outcome: &AppResult<TurnEnvelope>) {
    let Some(agui) = &hooks.agui else {
        return;
    };
    match outcome {
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

/// Record that a turn's reply was persisted (carnet#828).
///
/// What the activation funnel reads: whether the athlete's own provider data
/// was in front of the model, and how old the account was, which places the
/// answer in the athlete's first session without a signup event.
///
/// Grounded is any of: activities the model fetched, activities prefetched
/// for it, or a tool call during the turn that served the athlete's provider
/// items to the model — sleep, recovery, load, goals — which the dispatch
/// chokepoint marks on the turn's provenance (`provider_data_served`). The
/// turn's own prompt context does not count. Skipped when the asker
/// could not be read: an answer with no account age cannot be placed in any
/// session.
fn emit_answer_delivered(
    persona: CoachingPersona,
    origin: &str,
    asker_created_at: Option<DateTime<Utc>>,
    telemetry: &TurnTelemetry,
    provider_data_served: bool,
) {
    let Some(created_at) = asker_created_at else {
        debug!("asker unreadable; chat.answer_delivered not emitted");
        return;
    };
    info!(
        target: "notify",
        event = "chat.answer_delivered",
        persona = persona.as_str(),
        origin = origin,
        grounded = first_run_hooks::grounded(telemetry, provider_data_served),
        account_age_hours = Utc::now().signed_duration_since(created_at).num_hours(),
        "answer delivered"
    );
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
const fn persisted_finish_reason<'a>(
    leak_replaced: bool,
    capability_claim_unverified: bool,
    stop_stamp: Option<&'static str>,
    model_finish_reason: Option<&'a str>,
) -> Option<&'a str> {
    if leak_replaced {
        Some(WITHHELD_REPLY_FINISH_REASON)
    } else if capability_claim_unverified {
        // Ranked below the withhold: a withheld row holds platform text the
        // model never wrote, which is the stronger reason to keep it out of a
        // prompt. Both stamps drop the row at replay.
        Some(UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON)
    } else if let Some(stamp) = stop_stamp {
        // Ranked below both drops: a row either of them keeps out of every
        // prompt needs no caveat cut from it at replay.
        Some(stamp)
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
    let database = ctx.repos.chat.as_ref();

    // Stage 2: Resolve the prompt this turn answers. Persisted as the
    // athlete's message when they sent it, and deliberately not when the
    // platform composed it — see `resolve_turn_prompt`.
    let (groups, channel) = (ctx.repos.groups.as_ref(), profile.surface.wire_channel());
    let msg_result = resolve_turn_prompt(database, groups, &input, channel).await?;
    let user_message = msg_result.message;
    turn_stop::question_stored(hooks, &user_message);
    let conv = msg_result.conversation;

    // Stage 3: Resolve the model this turn runs on.
    let active_model = resolve_active_model(
        ctx.config.llm_chain.head_model(),
        &input.conversation_id,
        &conv.model,
    );

    // Stage 4: Ensure a long-lived agent session exists and is attached.
    let conv = ensure_agent_session_attached(&ctx.data, conv, input.conversation_tenant_id).await;

    // Fill the turn span's deferred context now that the conversation record is
    // resolved, so every downstream log line carries the agent and group-chat
    // identifiers alongside user/tenant/conversation/turn.
    record_turn_span_context(input.turn_agent_id(&conv), conv.group_id.as_deref());

    // Stage 4.1: who the turn is about, named once. Every stage below that
    // reads, fetches or files the sender's own data follows it.
    let standing = stages::group_subject::turn_standing(&ctx.repos, &conv, &input).await;
    let seat = standing.seat;

    // Stage 4.5: Guided mode. If this conversation is mid pillars walk, mid
    // calibration interview or mid season walk, resolve the topic being probed
    // (and clear the flag once complete). Drives the prompt directive (below)
    // and the fact-stamping at Stage 21.
    // A finished walk answers deterministically and skips the LLM entirely
    // (see `resolve_guided_or_answer`).
    // A platform-composed prompt is not the athlete answering the probe, so it
    // never drives the guided flow. Left in, a finished walk would
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
            standing,
        })
        .await?
        {
            GuidedOutcome::Answered(result) => return Ok(*result),
            GuidedOutcome::Continue { probe } => probe,
        }
    } else {
        None
    };

    // Stage 4.6: scope rails. An athlete's message reporting acute red-flag
    // symptoms, or asking for investment advice or a retail price, is answered
    // with platform copy before any model sees it (carnet#819). After the
    // guided stage so a walk's answer is still stamped with its topic; a
    // platform-composed prompt is not the athlete asking, so it never trips.
    if input.origin.persists_user_row() {
        if let Some(answered) =
            stages::guardrails::answer_off_scope(stages::guardrails::OffScopeTurn {
                ctx,
                input: &input,
                profile,
                active_model: &active_model,
                user_message: &user_message,
                conv: &conv,
                standing,
                guided_answer: onboarding_turn
                    .as_ref()
                    .and_then(|turn| stages::onboarding::answered_target(&turn.state)),
            })
            .await?
        {
            return Ok(answered);
        }
    }

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

    // What of the history the turn may read: an external turn in a
    // first-party-only thread, or a coach's athlete thread the gates refuse,
    // reads only its own new message (`history_scope`).
    let subject_turn =
        stages::history_scope::scope_history(ctx, &input, &conv, &user_message.id, &mut history)
            .await?;

    append_platform_prompt(&mut history, &user_message, input.origin);

    stages::agent_mention::apply_prompt_text(&mut history, &user_message.id, &input);

    // Stage 6: Resolve the agent runtime context this turn answers as.
    let agent_ctx = stages::prompt_assembly::resolve_turn_agent_ctx(ctx, &input, &conv).await?;

    // Stages 7a–7h + 8: assemble the hardened system prompt and flatten the
    // conversation history into a ready-to-dispatch LLM message list.
    let (
        prompt_guard,
        pending_followup_ids,
        mut llm_messages,
        source_ids,
        group_roster,
        intro,
        turn_clock,
    ) = assemble_prompt_stage(
        hooks,
        stages::prompt_assembly::PromptTurn {
            ctx,
            input: &input,
            profile,
            conv: &conv,
            standing,
            agent_ctx: agent_ctx.as_ref(),
            history: &history,
            onboarding: onboarding_turn.as_ref(),
            subject_turn,
        },
    )
    .await?;

    // Stages 9–14: pre-dispatch preparation followed by the multi-turn tool loop.
    let stages::tool_dispatch::DispatchedTurn {
        mut result,
        provider_name,
        activities_prefetched,
        tool_rounds,
        peer_blocks,
        streams_partial_text,
    } = dispatch_stage(
        DispatchStageArgs {
            hooks,
            ctx,
            input: &input,
            profile,
            active_model: &active_model,
            agent_ctx: agent_ctx.as_ref(),
            history: &history,
            source_ids: &source_ids,
            turn_clock: &turn_clock,
            guided_flow: onboarding_turn.as_ref().map(|turn| turn.state.flow),
            peer_roster: &group_roster,
            turn_agent_id: input.turn_agent_id(&conv),
            seat,
        },
        &mut llm_messages,
    )
    .await?;

    // Stages 14a–18: the bounded identity re-ask, the provider re-auth recovery short-circuit,
    // then either skip post-processing (recovery content is already canonical) or run the
    // standard guardrails/verification/hook chain on LLM-produced text.
    let (post_processed, reconnect) = run_recovery_and_post_process(
        RecoveryAndPostProcessInputs {
            ctx,
            input: &input,
            profile,
            conv: &conv,
            agent_ctx: agent_ctx.as_ref(),
            prompt_guard: &prompt_guard,
            llm_messages: &llm_messages,
            active_model: &active_model,
            peer_roster: &group_roster,
            peer_blocks: &peer_blocks,
            guided_walk: onboarding_turn.as_ref().map(|turn| &turn.state),
            seat,
            streamed_to: hooks.stream_sink.as_ref().filter(|_| streams_partial_text),
        },
        &mut result,
        hooks,
    )
    .await;
    result.content = post_processed.content;
    let leak_replaced = post_processed.leak_replaced;
    let plan_was_saved = result
        .tools_called
        .iter()
        .any(|t| t == SAVE_TRAINING_PLAN_TOOL);
    let identity_leak = post_processed.identity_leak;
    let verdict_chips = post_processed.verdict_chips;

    // Stage 19: Persist assistant response.
    let token_count = result.usage.as_ref().map(|u| u.completion_tokens);
    let prompt_tokens = result.usage.as_ref().map(|u| u.prompt_tokens);
    // Strip once; send and persist the same bytes. Cleaning only the durable copy
    // left the wire carrying scaffolding the record never showed (registre#40) — a
    // real answer is unchanged, scaffolding-only empties into a localized error.
    result.content = chat_tool_loop::strip_simulation_artifacts(&result.content);
    let (persisted_assistant_content, stop_stamp) = provider_stop::caveat_delivered_reply(
        ctx,
        &input,
        &result,
        post_processed.provider_stop,
        &profile.locale,
    )
    .map_or_else(
        || (result.content.clone(), None),
        |(caveated, stamp)| (caveated, Some(stamp)),
    );
    // A reply derived in part from Garmin device-sourced data says so, in the
    // platform's words on every surface (carnet#521).
    let persisted_assistant_content = stages::source_attribution::attribute_reply(
        ctx,
        persisted_assistant_content,
        &stages::source_attribution::owed_attributions(),
        stages::source_attribution::model_answered(&result),
        &profile.locale,
    );
    // A reply the platform composed on its own initiative is posted only when
    // it grounded in the athlete's data and did not refuse (carnet#831).
    if let Some(withheld) = stages::platform_grounding::withheld_platform_reply(
        ctx,
        profile,
        stages::platform_grounding::PlatformReplyDraft {
            input: &input,
            user_message: &user_message,
            conv: &conv,
            content: &persisted_assistant_content,
            result: &result,
            active_model: &active_model,
            provider_name: &provider_name,
            activities_prefetched,
            identity_leak,
            answered_by: agent_ctx.as_ref().map(|agent| agent.title.clone()),
        },
    ) {
        return Ok(withheld);
    }
    // The steps this reply hands over ride its stored row and its live
    // actions alike. A reply withheld for a leak is an apology, not the
    // answer the steps would have followed.
    let next_steps = if leak_replaced {
        NextSteps::default()
    } else {
        next_steps::for_reply(ctx, &input, profile, &conv, plan_was_saved).await
    };
    let content_blocks = next_steps
        .stored_blocks(post_processed.content_blocks.as_deref())
        .unwrap_or_else(|e| {
            warn!(error = %e, "next steps could not be stored; persisting the reply without them");
            post_processed.content_blocks.clone()
        });
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
            stop_stamp,
            result.finish_reason.as_deref(),
        ),
        prompt_tokens,
        model: Some(&active_model),
        content_blocks: content_blocks.as_deref(),
        // Stamped with what the turn served so far (carnet#769).
        transport_policy: ai_scope::derived_policy(),
        channel_type: Some(profile.surface.wire_channel()),
    };
    if let Some(stopped) = turn_stop::stopped_before_reply(ctx, hooks, profile, &input).await? {
        return Ok(stopped);
    }
    // A platform turn's tool rounds land now that its reply is admitted,
    // ahead of the reply drawn from them.
    tool_rounds.persist_held().await;
    let (assistant_message, updated_conversation) = persist_assistant_response(
        database,
        ctx.repos.groups.as_ref(),
        &assistant_params,
        input.conversation_tenant_id,
        input.origin,
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
        &input,
    )
    .await;

    // Stages 21/21b/21c/21d: end-of-turn follow-through — Tier 2 memory
    // extraction, playbook advice capture, the guided-flow probe record and the
    // agent's introduction. All four key on whether the reply actually reached
    // the athlete, so the branch lives in one place (see
    // `finish_turn_follow_through`).
    finish_turn_follow_through(FinishTurnInputs {
        ctx,
        input: &input,
        conv: &conv,
        assistant_reply: &result.content,
        assistant_message_id: &assistant_message.id,
        onboarding: onboarding_turn.as_ref(),
        introduction: intro.as_ref(),
        leak_replaced,
        plan_was_saved,
        standing,
    })
    .await;

    // Stage 22: publish the reply's chart specs for a surface that fetches
    // pixels. After persistence on purpose — the specs are addressed by
    // message id, so there is nothing to sign before the row exists.
    let scene_images = publish_scenes(hooks, profile, &input, &assistant_message);

    // A surface without an activity panel has the list folded into its prose, and a raw
    // 186-row history folded into a chat bubble is unreadable — so the list is shaped for the
    // fold here. Both this and the block-or-fold decision below read `activity_list_card`: one
    // capability, two consequences.
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

    // Settled here, with a reply persisted for the notice to ride under — see
    // `settle_turn_notice` for why it cannot be decided any earlier.
    let quota = settle_turn_notice(ctx, &input).await;
    next_steps.report_shown(
        &input.user_id,
        input.conversation_tenant_id,
        profile.surface,
    );

    Ok(build_envelope(
        profile,
        TurnState {
            turn_id: input.turn_id,
            user_message,
            assistant_message,
            conversation: updated_conversation,
            introduction_end: intro.and_then(|i| i.end_in(&persisted_assistant_content)),
            content: persisted_assistant_content,
            finish_reason: result.finish_reason,
            activity_list,
            telemetry: TurnTelemetry {
                model: active_model,
                provider_name,
                tools_called: result.tools_called,
                tool_calls_count: result.tool_calls_count,
                activity_list_captured,
                activities_prefetched,
                usage: result.usage,
                identity_leak,
                provider_warnings: result.provider_warnings,
            },
            quota,
            reconnect,
            verdict_chips,
            scene_images,
            actions: next_steps.into_actions(),
            actions_title: None,
            answered_by: agent_ctx.as_ref().map(|agent| agent.title.clone()),
        },
    ))
}

/// Ask the wired [`ScenePublisher`] to mint an image per stored chart spec.
///
/// Returns empty when the surface draws specs itself, when no publisher is
/// wired, or when the reply carried no blocks — all three mean the reply keeps
/// the sentences the agent wrote around the chart, which is the contract the
/// visual-blocks prompt sets.
fn publish_scenes(
    hooks: &PipelineHooks<'_>,
    profile: &SurfaceProfile,
    input: &TurnInput,
    assistant_message: &MessageRecord,
) -> Vec<SceneImage> {
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
    standing: stages::group_subject::TurnStanding,
}

/// What Stage 4.5 concluded: either the turn is already answered, or it carries
/// on with an optional guided probe attached.
enum GuidedOutcome {
    /// The platform answered the turn itself; the caller returns this verbatim.
    Answered(Box<TurnEnvelope>),
    /// Run the turn normally.
    Continue {
        /// The topic to probe, when one is set.
        probe: Option<stages::onboarding::OnboardingTurn>,
    },
}

/// Stage 4.5: resolve the guided flow, answering the turn outright when a
/// walk — calibration, season or pillars — has just finished.
///
/// The wrap-up reports how many answers actually landed and names a safety
/// topic that produced none — claims only the platform can make truthfully,
/// since an agent asked to summarize its own interview has no view of what the
/// extractor wrote and every incentive to declare success.
async fn resolve_guided_or_answer(inputs: GuidedStageInputs<'_>) -> AppResult<GuidedOutcome> {
    let GuidedStageInputs {
        ctx,
        input,
        profile,
        active_model,
        user_message,
        conv,
        standing,
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
            Ok(GuidedOutcome::Continue { probe: Some(*turn) })
        }
        stages::onboarding::GuidedResolution::Inactive => {
            Ok(GuidedOutcome::Continue { probe: None })
        }
        stages::onboarding::GuidedResolution::WalkComplete { reply, answered } => {
            let result = stages::deterministic_reply::deliver(
                stages::deterministic_reply::DeterministicReplyInputs {
                    ctx,
                    input,
                    profile,
                    active_model: active_model.to_owned(),
                    user_message: user_message.clone(),
                    conv,
                },
                reply,
            )
            .await?;
            // This turn carries the athlete's answer to the interview's LAST
            // question, and the reply skipping the LLM does not make that
            // answer any less theirs. Returning here without extracting dropped
            // it on every walk — calibration's last core topic is recovery
            // speed, which is safety-critical and the sole writer of its kind,
            // so its absence is exactly what the wrap-up would have named.
            stages::turn_extraction::spawn_turn_extraction(
                stages::turn_extraction::ExtractedTurn {
                    ctx,
                    input,
                    conv,
                    standing,
                    assistant_reply: PLATFORM_REPLY_TRANSCRIPT_MARKER,
                    assistant_message_id: &result.assistant.message.id,
                    answered,
                    // A walk's closing turn skips the LLM entirely, so no tool
                    // ran and no plan was stored.
                    plan_was_saved: false,
                },
            )
            .await;
            Ok(GuidedOutcome::Answered(Box::new(result)))
        }
    }
}

/// The model a turn runs on: the model the `PIERRE_LLM_CHAIN` head runs, on
/// every surface, so a configuration change reaches existing conversations too,
/// not only the ones created after it (carnet#824). The conversation's
/// stored model is the answer only when nothing is configured.
fn resolve_active_model(
    configured: Option<String>,
    conversation_id: &str,
    stored_model: &str,
) -> String {
    let active = configured.unwrap_or_else(|| stored_model.to_owned());
    if active != stored_model {
        info!(
            conversation_id = %conversation_id,
            stored_model = %stored_model,
            active_model = %active,
            "Overriding stored conversation model with the configured model"
        );
    }
    active
}

#[cfg(test)]
mod tests {
    use super::resolve_active_model;

    #[test]
    fn a_configured_model_replaces_the_one_the_conversation_was_created_with() {
        // dev moved to claude-sonnet-5.5 on 2026-10-01; a conversation created
        // before kept answering on claude-sonnet-5 (carnet#824).
        assert_eq!(
            resolve_active_model(
                Some("claude-sonnet-5.5".to_owned()),
                "conv",
                "claude-sonnet-5"
            ),
            "claude-sonnet-5.5"
        );
    }

    #[test]
    fn the_stored_model_answers_only_when_nothing_is_configured() {
        assert_eq!(
            resolve_active_model(None, "conv", "claude-sonnet-5"),
            "claude-sonnet-5"
        );
    }
}
