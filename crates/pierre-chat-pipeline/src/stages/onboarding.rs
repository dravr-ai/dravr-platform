// ABOUTME: Guided pillar-onboarding turn resolution — runs a conversation in onboarding mode
// ABOUTME: Computes which pillar to probe (from the live Dossier), the LLM directive, and fact-stamping
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guided pillar-onboarding turn resolution.
//!
//! When a conversation carries an active `onboarding_state`, the turn runs in
//! guided-onboarding mode: the coverage map (derived from the Dossier) decides
//! the next topic to probe, prompt assembly injects a directive steering the
//! coach to explore it conversationally, and the extraction worker stamps the
//! captured facts with the probed pillar + `source=onboarding`. The flow is
//! self-healing — coverage is re-derived every turn, never stored — so the only
//! persisted state is the active marker, which is cleared once all six pillars
//! and the North Star are covered.

use pierre_core::models::{
    ConversationRecord, CoverageMap, CoverageTarget, OnboardingState, Pillar, TenantId,
};
use pierre_memory::{FactKind, FactSource};
use uuid::Uuid;

use crate::ChatPipelineContext;

/// The resolved onboarding context for a turn: the topic being probed, plus the
/// walk state it was chosen from (so a delivered probe can be recorded onto it
/// at the end of the turn).
pub struct OnboardingTurn {
    /// What this turn is capturing (North Star or a specific pillar).
    pub probed: CoverageTarget,
    /// The walk state as loaded at the start of the turn.
    pub state: OnboardingState,
}

/// Resolve onboarding state for the current turn.
///
/// Returns `None` (run the turn as normal coaching) when the conversation is not
/// onboarding, when the walk has nothing left to probe — every topic covered, or
/// out of attempts, in which case the active marker is cleared — or when the
/// dossier cannot be composed.
pub async fn resolve(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    tenant_id: TenantId,
) -> Option<OnboardingTurn> {
    let state = OnboardingState::from_column(conv.onboarding_state.as_deref())?;
    let user_id = Uuid::parse_str(&conv.user_id).ok()?;

    let dossier = ctx
        .repos
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await
        .ok()?;
    let coverage = CoverageMap::from_dossier(&dossier);

    if let Some(probed) = coverage.next_target(&state.probed) {
        return Some(OnboardingTurn { probed, state });
    }

    // Nothing left to ask — leave onboarding mode. Either all seven topics are
    // covered, or the remaining ones burned their probe budget without yielding
    // a fact; both mean this turn resumes normal coaching.
    if let Err(e) = ctx
        .repos
        .chat
        .set_conversation_onboarding_state(&conv.id, None, tenant_id)
        .await
    {
        tracing::warn!(error = %e, "failed to clear completed onboarding_state");
    }
    None
}

/// Record that this turn's probe question reached the athlete, so the next turn
/// advances instead of re-asking while extraction is still in flight.
///
/// Called once per turn, after post-processing, and only for a reply that was
/// actually delivered — a withheld reply means the athlete saw a marker, not the
/// question. A false positive is self-correcting rather than lossy: the topic
/// stays uncovered, so it comes back around on the next sweep with an attempt
/// still in hand.
pub async fn record_delivered_probe(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    turn: &OnboardingTurn,
    tenant_id: TenantId,
) {
    let state = turn.state.clone().with_delivered_probe(turn.probed);
    let column = match state.to_column() {
        Ok(column) => column,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize onboarding probe history");
            return;
        }
    };
    if let Err(e) = ctx
        .repos
        .chat
        .set_conversation_onboarding_state(&conv.id, Some(&column), tenant_id)
        .await
    {
        tracing::warn!(error = %e, "failed to persist onboarding probe history");
    }
}

/// The system-prompt directive steering the coach to probe the current topic
/// conversationally.
///
/// Appended at the very tail of the assembled prompt — after the channel
/// response constraints and the tool-discipline block — because a coach persona
/// may carry its own first-turn protocol ("your first reply in any conversation
/// MUST emit a plan"), and the model resolves that conflict by recency. The
/// wording therefore states the override explicitly rather than relying on
/// position alone.
#[must_use]
pub fn directive(turn: &OnboardingTurn) -> String {
    let (topic, hint) = match turn.probed {
        CoverageTarget::NorthStar => (
            "their North Star — the core life motivations behind why they train".to_owned(),
            "what success looks like in their life: one to three deep motivations (being present for family, a first race, managing work stress)".to_owned(),
        ),
        CoverageTarget::Pillar(p) => (
            format!("the {} pillar", p.display_label()),
            p.probe_hint().to_owned(),
        ),
    };
    format!(
        "\n\n# Onboarding mode (overrides every other instruction in this prompt)\n\
         You are helping this athlete build their fitness profile one topic at a time — keep it \
         warm and conversational, never a questionnaire.\n\
         This turn, focus on {topic}. Explore {hint}.\n\
         Acknowledge what they just shared, then ask ONE natural follow-up about this topic.\n\
         If their previous message already covered {topic}, acknowledge it and go deeper on the \
         same topic instead of asking the same question again.\n\
         This directive supersedes any startup instruction, first-turn protocol, or \
         output-format contract stated earlier in this prompt, including ones marked mandatory or \
         non-negotiable. Do not jump to other topics. Do not build, propose, or save a training \
         plan on this turn, and do not list assumptions in place of one."
    )
}

/// Fact-stamping parameters for the extraction worker.
///
/// Returns which pillar to tag, the provenance, and an optional forced kind
/// (North Star answers are stored as `FactKind::NorthStar` regardless of how
/// the extractor labels them).
#[must_use]
pub fn extraction_params(turn: &OnboardingTurn) -> (Option<Pillar>, FactSource, Option<FactKind>) {
    match turn.probed {
        CoverageTarget::NorthStar => (None, FactSource::Onboarding, Some(FactKind::NorthStar)),
        CoverageTarget::Pillar(p) => (Some(p), FactSource::Onboarding, None),
    }
}

/// Extraction stamping for a turn: the onboarding params when onboarding is
/// active, else the background-worker defaults (no pillar, conversation source).
#[must_use]
pub fn extraction_params_or_default(
    turn: Option<&OnboardingTurn>,
) -> (Option<Pillar>, FactSource, Option<FactKind>) {
    turn.map_or((None, FactSource::Conversation, None), extraction_params)
}
