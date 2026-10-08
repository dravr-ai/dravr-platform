// ABOUTME: End-of-turn follow-through — memory extraction, advice capture, guided-flow probe record, introduction
// ABOUTME: Everything a turn still owes once its reply is persisted, keyed on whether the reply reached the athlete
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a turn still owes once its assistant reply is durable.
//!
//! [`finish_turn_follow_through`] runs Tier 2 memory extraction, playbook
//! advice capture, the guided-flow probe record and the agent's introduction.
//! All four answer one question — did this reply actually reach the athlete?
//! — so the withheld-reply branch lives here, once, instead of at each call
//! site in the turn orchestrator.

use std::sync::Arc;

use chrono::Utc;
use pierre_core::models::{OnboardingState, TenantId};
use pierre_database::database::ConversationRecord;
use pierre_llm::stage::LlmStage;
use pierre_providers::ai_scope;
use pierre_services::advice_capture::{
    spawn_capture_advice, AdviceCaptureStrategy, CapturedTurn, HeuristicGatedLlmExtraction,
};
use pierre_services::memory_extraction::WITHHELD_REPLY_TRANSCRIPT_MARKER;
use tracing::info;

use crate::recorders::turn_call_recorder;
#[cfg(feature = "tools-verification")]
use crate::stages::verification::persist_pending_verdicts;
use crate::{stages, ChatPipelineContext, TurnInput};

/// Spawn background advice capture for the turn (P3 of playbook memory).
///
/// Mirrors [`crate::stages::turn_extraction::spawn_turn_extraction`]: best-effort, needs the shared
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
    let strategy: Arc<dyn AdviceCaptureStrategy> = Arc::new(HeuristicGatedLlmExtraction::new(
        Arc::clone(&ctx.prompt_registry),
    ));
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
            agent_slug: input.turn_agent_id(conv).map(ToOwned::to_owned),
            user_message: input.content.clone(),
            assistant_reply: assistant_reply.to_owned(),
            source_msg_id: Some(assistant_message_id.to_owned()),
            transport_policy: ai_scope::derived_policy(),
        },
        Some(turn_call_recorder(
            ctx,
            input,
            LlmStage::AdviceCapture.call_type(),
        )),
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
/// what the agent said) is skipped.
///
/// Dropping the user turn as well is what stalled the guided pillar walk: the
/// answer was never extracted, the topic never flipped to covered, and the next
/// turn re-asked the same question. Every recorded withhold to date is on the
/// agent this flow runs against.
///
/// Owning the `leak_replaced` branch here keeps `run_turn` itself branch-free
/// over this concern.
async fn spawn_turn_background_learning(
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
        stages::turn_extraction::spawn_turn_extraction(
            ctx,
            input,
            conv,
            WITHHELD_REPLY_TRANSCRIPT_MARKER,
            assistant_message_id,
            answered,
            plan_was_saved,
        )
        .await;
        return;
    }
    stages::turn_extraction::spawn_turn_extraction(
        ctx,
        input,
        conv,
        assistant_reply,
        assistant_message_id,
        answered,
        plan_was_saved,
    )
    .await;
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
pub async fn persist_verdicts_for_turn(
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
        input.turn_agent_id(conv),
        assistant_message_id,
        pending_verdicts,
    )
    .await;
}

/// Inputs for [`finish_turn_follow_through`].
#[derive(Clone, Copy)]
pub struct FinishTurnInputs<'a> {
    pub ctx: &'a ChatPipelineContext,
    pub input: &'a TurnInput,
    pub conv: &'a ConversationRecord,
    pub assistant_reply: &'a str,
    pub assistant_message_id: &'a str,
    pub onboarding: Option<&'a stages::onboarding::OnboardingTurn>,
    /// The introduction the reply was asked to open with, if any.
    pub introduction: Option<&'a stages::introduction::PendingIntroduction>,
    pub leak_replaced: bool,
    /// Whether `save_training_plan` ran on this turn. Decides whether the
    /// memory extractor may drop an agent-prescription schedule fact on the
    /// grounds that the plan is stored elsewhere (registre#203).
    pub plan_was_saved: bool,
}

/// Everything the turn still owes once its reply is persisted: Tier 2 memory
/// extraction, playbook advice capture, the guided-flow probe record and the
/// agent's introduction.
///
/// Grouped because all four answer the same question — did this reply actually
/// reach the athlete? — and a caller that got that branch half-right is exactly
/// how a withheld turn used to orphan both the athlete's answer and the walk's
/// progress.
pub async fn finish_turn_follow_through(inputs: FinishTurnInputs<'_>) {
    let FinishTurnInputs {
        ctx,
        input,
        conv,
        onboarding,
        introduction,
        leak_replaced,
        ..
    } = inputs;
    if let Some(pending) = introduction {
        let reply = inputs.assistant_reply;
        stages::introduction::record_if_named(ctx, pending, reply, input.conversation_tenant_id)
            .await;
    }
    // The message being extracted answers the probe the PREVIOUS turn delivered
    // — `onboarding.target` is the question this turn asks, one topic further
    // on. Stamping with it filed every guided answer under the next topic's
    // pillar and forced kind.
    let answered = onboarding.and_then(|turn| stages::onboarding::answered_target(&turn.state));
    spawn_turn_background_learning(&inputs, answered).await;
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
