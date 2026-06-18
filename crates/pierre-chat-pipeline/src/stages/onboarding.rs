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

/// The resolved onboarding context for a turn: the topic being probed.
pub struct OnboardingTurn {
    /// What this turn is capturing (North Star or a specific pillar).
    pub probed: CoverageTarget,
}

/// Resolve onboarding state for the current turn.
///
/// Returns `None` (run the turn as normal coaching) when the conversation is
/// not onboarding, when onboarding is already complete (in which case the
/// active marker is cleared), or when the dossier cannot be composed.
pub async fn resolve(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    tenant_id: TenantId,
) -> Option<OnboardingTurn> {
    OnboardingState::from_column(conv.onboarding_state.as_deref())?;
    let user_id = Uuid::parse_str(&conv.user_id).ok()?;

    let dossier = ctx
        .repos
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await
        .ok()?;
    let coverage = CoverageMap::from_dossier(&dossier);

    if coverage.is_complete() {
        // All pillars + North Star covered — leave onboarding mode. The last
        // pillar's answer was captured on the turn that probed it; this turn
        // resumes normal coaching.
        if let Err(e) = ctx
            .repos
            .chat
            .set_conversation_onboarding_state(&conv.id, None, tenant_id)
            .await
        {
            tracing::warn!(error = %e, "failed to clear completed onboarding_state");
        }
        return None;
    }

    coverage
        .next_target()
        .map(|probed| OnboardingTurn { probed })
}

/// The system-prompt directive steering the coach to probe the current topic
/// conversationally (appended after the OKF context bundle).
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
        "\n\n# Onboarding mode\nYou are helping this user build their fitness profile one topic at a time — keep it warm and conversational, never a questionnaire. This turn, focus on {topic}. Explore {hint}. Acknowledge what they share, then ask ONE natural follow-up about this topic. Do not jump to other topics or deliver a full coaching plan yet."
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
