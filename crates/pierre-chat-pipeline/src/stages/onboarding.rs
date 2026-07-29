// ABOUTME: Guided-interview turn resolution — runs a conversation in pillars or calibration mode
// ABOUTME: Computes which topic to probe, the LLM directive, and the fact-stamping for the turn
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guided-interview turn resolution.
//!
//! When a conversation carries an active `onboarding_state`, the turn runs in
//! guided mode: a next-topic policy decides what to probe, prompt assembly
//! injects a directive steering the coach to explore it conversationally, and
//! the extraction worker stamps the captured facts with that topic's pillar and
//! kind + `source=onboarding`.
//!
//! Two flows share this machinery and the one delivered-probe ledger, but not
//! their next-topic policy:
//!
//! - **Pillars** derives the next topic from live Dossier coverage, so it is
//!   self-healing: coverage is re-computed every turn and never stored, and a
//!   topic whose answer produced no fact comes back around with an attempt
//!   still in hand.
//! - **Calibration** walks a fixed list. Coverage cannot serve it — most of its
//!   topics land as the same kind in the same pillar, so the Dossier cannot say
//!   which ones are still outstanding — which is why that flow has no re-ask
//!   budget and leans on the completion check instead.

use chrono::Utc;
use pierre_core::models::{
    CalibrationConditions, CalibrationTopic, ConversationRecord, CoverageMap, CoverageTarget,
    Dossier, GuidedFlow, LoadSnapshot, OnboardingState, Pillar, TenantId, TopicSlug,
};
use pierre_memory::{FactKind, FactSource};
use uuid::Uuid;

use super::completion;
use crate::ChatPipelineContext;

/// A session over this many minutes marks the athlete as a long-session
/// athlete, which earns them the fueling topic. Three hours is where
/// carbohydrate intake stops being optional.
const LONG_SESSION_MIN: u32 = 180;

/// What a guided turn is capturing, in whichever flow owns the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidedTarget {
    /// A pillars-walk topic: the North Star or a specific pillar.
    Coverage(CoverageTarget),
    /// A difficulty-calibration interview topic.
    Calibration(CalibrationTopic),
}

impl GuidedTarget {
    /// The stable slug recorded in the delivered-probe ledger.
    #[must_use]
    pub fn slug(self) -> TopicSlug {
        match self {
            Self::Coverage(target) => target.slug(),
            Self::Calibration(topic) => topic.slug(),
        }
    }
}

/// The resolved guided context for a turn: the topic being probed, plus the
/// flow state it was chosen from (so a delivered probe can be recorded onto it
/// at the end of the turn).
pub struct OnboardingTurn {
    /// What this turn is capturing.
    pub target: GuidedTarget,
    /// The flow state as loaded at the start of the turn.
    pub state: OnboardingState,
}

/// Which conditional calibration topics this athlete qualifies for.
///
/// Derived per turn from the persisted snapshot and the live dossier rather
/// than stored: both inputs are already in hand, and re-deriving keeps a goal
/// the athlete set mid-interview from being ignored.
pub(super) fn calibration_conditions(
    dossier: &Dossier,
    snapshot: Option<&LoadSnapshot>,
) -> CalibrationConditions {
    CalibrationConditions {
        long_sessions: snapshot.is_some_and(|s| s.longest_session_min >= LONG_SESSION_MIN),
        dated_goal: dossier
            .pillars
            .values()
            .flatten()
            .any(|f| f.kind == FactKind::Goal.as_str() && !f.stale),
    }
}

/// How a turn relates to a guided flow.
pub enum GuidedResolution {
    /// A topic to probe this turn; the coach asks it.
    Probe(Box<OnboardingTurn>),
    /// The calibration interview just finished. The turn answers with this
    /// platform-rendered text instead of dispatching to the LLM — the wrap-up
    /// reports what was actually captured, which only holds if the platform
    /// writes it.
    CalibrationComplete(String),
    /// Not in a guided flow, or the pillars walk just ended — normal coaching.
    Inactive,
}

/// Resolve guided-flow state for the current turn.
///
/// Returns [`GuidedResolution::Inactive`] (run the turn as normal coaching)
/// when the conversation is not in a guided flow, when the pillars walk has
/// nothing left to probe — in which case the active marker is cleared — or when
/// the dossier cannot be composed.
pub async fn resolve(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    tenant_id: TenantId,
    locale: Option<&str>,
) -> GuidedResolution {
    let Some(state) = OnboardingState::from_column(conv.onboarding_state.as_deref()) else {
        return GuidedResolution::Inactive;
    };
    let Ok(user_id) = Uuid::parse_str(&conv.user_id) else {
        return GuidedResolution::Inactive;
    };

    let Ok(dossier) = ctx.repos.dossier.compose_dossier(tenant_id, user_id).await else {
        return GuidedResolution::Inactive;
    };

    let target = match state.flow {
        GuidedFlow::Pillars => CoverageMap::from_dossier(&dossier)
            .next_target(&state.probed)
            .map(GuidedTarget::Coverage),
        GuidedFlow::Calibration => CalibrationTopic::next_target(
            &state.probed,
            calibration_conditions(&dossier, state.snapshot.as_ref()),
        )
        .map(GuidedTarget::Calibration),
    };

    if let Some(target) = target {
        return GuidedResolution::Probe(Box::new(OnboardingTurn { target, state }));
    }

    // Nothing left to ask — leave guided mode. For the pillars walk that means
    // every topic is covered or burned its probe budget; for calibration it
    // means every topic on this athlete's list was delivered.
    //
    // The completion summary is rendered BEFORE the marker is retired, so a
    // failed write does not also cost the athlete their wrap-up.
    let summary = match state.flow {
        GuidedFlow::Calibration => {
            Some(completion::render(ctx, conv, &state, tenant_id, &dossier, locale).await)
        }
        GuidedFlow::Pillars => None,
    };

    // The row is retired rather than deleted: it becomes an inactive,
    // timestamped marker that lets exactly one later turn know the interview
    // just ended, so [`release_directive`] can revoke the interview's
    // no-writing rule before the model acts on the transcript it shaped.
    // A state that will not serialize falls back to deleting the row, which is
    // the pre-existing behaviour — the flow still ends, only the release
    // directive is lost.
    let retired = state
        .completed(Utc::now().to_rfc3339())
        .to_column()
        .map_err(|e| {
            tracing::warn!(error = %e, "failed to serialize completed onboarding_state; clearing it instead");
        })
        .ok();
    if let Err(e) = ctx
        .repos
        .chat
        .set_conversation_onboarding_state(&conv.id, retired.as_deref(), tenant_id)
        .await
    {
        tracing::warn!(error = %e, "failed to retire completed onboarding_state");
    }

    summary.map_or(
        GuidedResolution::Inactive,
        GuidedResolution::CalibrationComplete,
    )
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
    let state = turn.state.clone().with_delivered_probe(turn.target.slug());
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

/// The system-prompt directive revoking the interview's rules, for the first
/// turn after a guided flow ends.
///
/// [`directive`] is the most forcefully worded block in the whole prompt: it
/// claims to override every other instruction, and it closes with "do not
/// build, propose, or save a training plan on this turn". On 2026-07-28 an
/// athlete finished a calibration interview that had carried that block on
/// eight consecutive turns; 48 seconds later, with the block gone and
/// `save_training_plan` back in the catalogue, the coach told him it could not
/// save his plan "cette fois-ci" — and the logs show it never called the tool.
/// Removing a prohibition does not retract it from the transcript that
/// prohibition already shaped, so it is retracted explicitly.
///
/// Placed with the same recency logic as [`directive`], and worded to revoke
/// rather than merely permit, because "you may now save" competes with eight
/// turns of "do not save" while "the earlier instruction no longer applies"
/// resolves them.
#[must_use]
pub fn release_directive() -> String {
    "\n\n# Interview complete (this overrides the interview rules you were following)\n\
     The guided interview is over. The instruction not to build, propose or save a training \
     plan applied only while it was running and no longer applies — disregard it, along with \
     any statement you made under it about being unable to save.\n\
     Every tool listed under Available Tools is callable again this turn, including \
     save_training_plan.\n\
     Never tell the athlete that saving failed unless you called the tool on this turn and it \
     returned an error. If you intend to save, call the tool and report what it actually \
     returned."
        .to_owned()
}

/// Retire the just-completed marker so [`release_directive`] fires once.
///
/// Called after the turn that carried the directive. Leaving it in place would
/// have every later turn told the interview "just" ended;
/// [`OnboardingState::just_completed`] bounds that to
/// [`COMPLETION_RELEASE_WINDOW_MINUTES`] anyway, so a failed clear costs a
/// repeat of the directive rather than a permanent one.
pub async fn clear_completed_marker(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    tenant_id: TenantId,
) {
    if let Err(e) = ctx
        .repos
        .chat
        .set_conversation_onboarding_state(&conv.id, None, tenant_id)
        .await
    {
        tracing::warn!(error = %e, "failed to clear the completed-interview marker");
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
    let (mode, purpose, topic, hint) = match turn.target {
        GuidedTarget::Coverage(CoverageTarget::NorthStar) => (
            "Onboarding mode",
            "You are helping this athlete build their fitness profile one topic at a time",
            "their North Star — the core life motivations behind why they train".to_owned(),
            "what success looks like in their life: one to three deep motivations (being present for family, a first race, managing work stress)".to_owned(),
        ),
        GuidedTarget::Coverage(CoverageTarget::Pillar(p)) => (
            "Onboarding mode",
            "You are helping this athlete build their fitness profile one topic at a time",
            format!("the {} pillar", p.display_label()),
            p.probe_hint().to_owned(),
        ),
        GuidedTarget::Calibration(t) => (
            "Calibration mode",
            "You are calibrating how hard this athlete's training should be, one question at a time",
            calibration_topic_label(t).to_owned(),
            t.probe_hint().to_owned(),
        ),
    };
    let baseline = calibration_baseline_line(turn);
    format!(
        "\n\n# {mode} (overrides every other instruction in this prompt)\n\
         {purpose} — keep it warm and conversational, never a questionnaire.\n\
         {baseline}\
         This turn, focus on {topic}. Explore {hint}.\n\
         Acknowledge what they just shared, then ask ONE natural follow-up about this topic.\n\
         If their previous message already covered {topic}, acknowledge it and go deeper on the \
         same topic instead of asking the same question again.\n\
         If the athlete asked a question rather than answering, answer it briefly and re-ask \
         before moving on — the interview advances by turn, so an unanswered topic is a lost one.\n\
         This directive supersedes any startup instruction, first-turn protocol, or \
         output-format contract stated earlier in this prompt, including ones marked mandatory or \
         non-negotiable. Do not jump to other topics. Do not build, propose, or save a training \
         plan on this turn, and do not list assumptions in place of one."
    )
}

/// What the coach is asking about, for a calibration topic.
const fn calibration_topic_label(topic: CalibrationTopic) -> &'static str {
    match topic {
        CalibrationTopic::ProgressionIntent => "how they want their training to get harder",
        CalibrationTopic::BaselineConfirm => "whether their recent training is a fair baseline",
        CalibrationTopic::Availability => "the time they actually have to train",
        CalibrationTopic::Injury => "any injury or niggle that flares under load",
        CalibrationTopic::RpeHeadroom => "how much was left in the tank on their hard sessions",
        CalibrationTopic::RecoverySpeed => "how quickly they recover from a hard day",
        CalibrationTopic::Fueling => "what they actually eat on long sessions",
        CalibrationTopic::EventDemand => "what their goal event demands",
    }
}

/// The inferred-baseline line injected on the topic that asks the athlete to
/// confirm it.
///
/// Only that topic gets it: quoting the figures on every turn would have the
/// coach reciting the athlete's training history back at them repeatedly, and
/// the numbers are only load-bearing for the confirmation question. Empty when
/// there is no snapshot — no provider connected — so the coach asks cold
/// instead of inventing figures.
fn calibration_baseline_line(turn: &OnboardingTurn) -> String {
    if turn.target != GuidedTarget::Calibration(CalibrationTopic::BaselineConfirm) {
        return String::new();
    }
    turn.state.snapshot.as_ref().map_or_else(
        || {
            "You have no connected training data for this athlete, so ask for their recent \
             typical week rather than quoting figures.\n"
                .to_owned()
        },
        |s| {
            format!(
                "Their connected training data over the last {} weeks averages {:.1} hours per \
                 week across {:.1} sessions, with a longest session of {} minutes. State these \
                 figures and ask whether they are a fair baseline. Do not invent other numbers.\n",
                s.weeks, s.weekly_hours, s.sessions_per_week, s.longest_session_min
            )
        },
    )
}

/// Fact-stamping parameters for the extraction worker.
///
/// Returns which pillar to tag, the provenance, and an optional forced kind
/// (North Star answers are stored as `FactKind::NorthStar` regardless of how
/// the extractor labels them; every calibration topic forces the kind its
/// answer means, so an injury answer can never be filed as a preference).
#[must_use]
pub fn extraction_params(turn: &OnboardingTurn) -> (Option<Pillar>, FactSource, Option<FactKind>) {
    match turn.target {
        GuidedTarget::Coverage(CoverageTarget::NorthStar) => {
            (None, FactSource::Onboarding, Some(FactKind::NorthStar))
        }
        GuidedTarget::Coverage(CoverageTarget::Pillar(p)) => {
            (Some(p), FactSource::Onboarding, None)
        }
        GuidedTarget::Calibration(t) => (
            Some(t.pillar()),
            FactSource::Onboarding,
            Some(FactKind::parse_lenient(t.fact_kind())),
        ),
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
