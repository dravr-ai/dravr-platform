// ABOUTME: Guided-interview turn resolution — runs a conversation in pillars, calibration or season mode
// ABOUTME: Computes which topic to probe, the LLM directive, and the fact-stamping for the turn
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guided-interview turn resolution.
//!
//! When a conversation carries an active `onboarding_state`, the turn runs in
//! guided mode: a next-topic policy decides what to probe, prompt assembly
//! injects a directive steering the agent to explore it conversationally, and
//! the extraction worker stamps the captured facts with that topic's pillar and
//! kind + `source=onboarding`.
//!
//! Three flows share this machinery and the one delivered-probe ledger, but
//! not their next-topic policy:
//!
//! - **Pillars** derives the next topic from live Dossier coverage, so it is
//!   self-healing: coverage is re-computed every turn and never stored, and a
//!   topic whose answer produced no fact comes back around with an attempt
//!   still in hand.
//! - **Calibration** and **Season** each walk a fixed list. Coverage cannot
//!   serve them — most of their topics land as the same kind in the same
//!   pillar, so the Dossier cannot say which ones are still outstanding —
//!   which is why those flows have no re-ask budget and lean on the
//!   completion check instead.

use chrono::{DateTime, Utc};
use pierre_core::models::{
    CalibrationConditions, CalibrationTopic, ConversationRecord, CoverageMap, CoverageTarget,
    Dossier, GuidedFlow, GuidedWindow, LoadSnapshot, OnboardingState, Pillar, SeasonConditions,
    SeasonTopic, TenantId, TopicSlug, WalkAudience,
};
use pierre_memory::{FactKind, FactSource};
use uuid::Uuid;

use super::completion;
use crate::ChatPipelineContext;

/// How many turns the fortnight rail owns before it retires.
///
/// The drafting turn plus two to negotiate it — "make Thursday easier",
/// "swap the long run to Sunday" — which is the conversation the one-turn
/// shape could not hold. It is a fixed budget rather than a condition
/// because the alternative is a flow that owns the conversation until
/// something tells it to stop, and nothing reliably would: an athlete who
/// simply changes the subject would be answered by the fortnight rail for
/// the rest of the day.
pub const FORTNIGHT_FLOW_TURNS: u8 = 3;

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
    /// A season-walk topic.
    Season(SeasonTopic),
}

impl GuidedTarget {
    /// The stable slug recorded in the delivered-probe ledger.
    #[must_use]
    pub fn slug(self) -> TopicSlug {
        match self {
            Self::Coverage(target) => target.slug(),
            Self::Calibration(topic) => topic.slug(),
            Self::Season(topic) => topic.slug(),
        }
    }
}

/// The resolved guided context for a turn: the topic being probed, plus the
/// flow state it was chosen from (so a delivered probe can be recorded onto it
/// at the end of the turn).
pub struct OnboardingTurn {
    /// What this turn is capturing, or `None` for a flow that captures
    /// nothing.
    ///
    /// The four interview walks always carry a topic: they exist to ask a
    /// question and record the answer. The fortnight rail owns its turns
    /// without asking anything — it briefs the agent and the agent writes —
    /// so it carries `None`, and every consumer of a topic has to say what it
    /// does without one. That is the point of the `Option`: a fortnight slug
    /// in the delivered-probe ledger would make [`answered_target`] fail to
    /// parse it, silently, and stamp the athlete's next fact with the wrong
    /// provenance.
    pub target: Option<GuidedTarget>,
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

/// Which conditional season topics this athlete qualifies for, from the
/// load snapshot taken when the walk started. No snapshot — no connected
/// provider — reads as single-sport, the shorter walk.
pub(super) fn season_conditions(snapshot: Option<&LoadSnapshot>) -> SeasonConditions {
    SeasonConditions {
        multi_sport: snapshot.is_some_and(|s| s.sport_families >= 2),
    }
}

/// How a turn relates to a guided flow.
pub enum GuidedResolution {
    /// A topic to probe this turn; the agent asks it.
    Probe(Box<OnboardingTurn>),
    /// A fixed-list walk — calibration or season — just finished. The turn
    /// answers with platform-rendered text instead of dispatching to the LLM —
    /// the wrap-up reports what was actually captured, which only holds if
    /// the platform writes it.
    WalkComplete {
        /// The wrap-up delivered in place of an LLM reply.
        summary: String,
        /// The topic this turn's inbound message answers — see
        /// [`answered_target`]. Carried out of the resolver because the reply
        /// skips the LLM but the athlete's message still has to be extracted,
        /// and the last core topic ([`CalibrationTopic::RecoverySpeed`]) is
        /// safety-critical.
        answered: Option<GuidedTarget>,
    },
    /// Not in a guided flow, or the pillars walk just ended — normal coaching.
    Inactive,
}

/// Resolve guided-flow state for the current turn.
///
/// Returns [`GuidedResolution::Inactive`] (run the turn as normal coaching)
/// when the conversation is not in a guided flow, when the speaker is not the
/// walk's subject, when the pillars walk has nothing left to probe — in which
/// case the active marker is cleared — or when the dossier cannot be composed.
///
/// `tenant_id` is the tenant that owns the conversation row (where the state
/// marker lives); `facts_tenant` is the tenant that owns the subject's own
/// data — the tool tenant, which a room turn resolves to the athlete's home
/// tenant while the room's row stays under the channel tenant. Coverage is
/// composed and the wrap-up counted under `facts_tenant`, because that is
/// where this walk's extraction lands its answers.
pub async fn resolve(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    speaker_user_id: &str,
    tenant_id: TenantId,
    facts_tenant: TenantId,
    locale: &str,
) -> GuidedResolution {
    let Some(mut state) = OnboardingState::from_column(conv.onboarding_state.as_deref()) else {
        return GuidedResolution::Inactive;
    };
    // The intake is asked and parsed by the platform in messaging ingress, so
    // this pipeline has no topic for it — but "no topic to probe" is how the
    // resolver recognises a *finished* walk, and it would retire the marker on
    // the way past. That would end an intake the athlete has not answered yet,
    // silently, on the very turn that opened it. Step aside entirely instead.
    if state.flow == GuidedFlow::Intake {
        return GuidedResolution::Inactive;
    }
    // The subject gate: a walk bound to a member advances only on that
    // member's own turns. Everyone else in the thread — an agent watching, a
    // participant on a shared in-app thread — gets an ordinary coaching turn:
    // no probe, no directive, no interview stamp on their extraction, and no
    // write-tool withhold, all of which follow from returning Inactive here.
    // Messaging rooms are per-member rows so the gate is defense-in-depth
    // there; on a shared in-app thread it is the enforcement.
    if state
        .subject_user_id
        .as_deref()
        .is_some_and(|subject| subject != speaker_user_id)
    {
        return GuidedResolution::Inactive;
    }
    // The fortnight rail owns the turn without probing: it asks nothing and
    // extracts nothing, so it never reaches the dossier load or the topic
    // machinery below. It is a flow rather than a one-turn directive because
    // the athlete negotiates the weeks after they are drafted, and an
    // ordinary coaching turn would answer that with the brief only as far
    // back as the transcript carries it.
    if state.flow == GuidedFlow::Fortnight {
        if state.turns_owned >= FORTNIGHT_FLOW_TURNS {
            // The rail's turns are spent. It retires here rather than holding
            // the conversation: the athlete keeps adjusting, just as ordinary
            // coaching, and the wrap-up reads the saved weeks back.
            let subject = walk_subject_id(&state, conv).map(|id| id.to_string());
            return finish_fortnight(ctx, conv, tenant_id, facts_tenant, subject, locale).await;
        }
        return GuidedResolution::Probe(Box::new(OnboardingTurn {
            target: None,
            state,
        }));
    }
    let Some(subject_id) = walk_subject_id(&state, conv) else {
        return GuidedResolution::Inactive;
    };
    let Some(dossier) = load_dossier(ctx, conv, facts_tenant, subject_id).await else {
        return GuidedResolution::Inactive;
    };

    // A walk on a group-bound conversation is a room walk even when its state
    // predates the audience field: auto-started group walks and legacy rows
    // parse as Private, and running them private would probe DM-only topics
    // in front of the room. The derived value is written back onto the state
    // so the delivered-probe CAS persists the truth.
    if state.audience == WalkAudience::Private && conv.group_id.is_some() {
        state.audience = WalkAudience::Room;
    }

    if let Some(target) = next_target(&state, &dossier) {
        return GuidedResolution::Probe(Box::new(OnboardingTurn {
            target: Some(target),
            state,
        }));
    }

    let subject = subject_id.to_string();
    leave_guided_mode(LeaveGuidedMode {
        ctx,
        conv,
        state,
        tenant_id,
        facts_tenant,
        subject_user_id: &subject,
        dossier: &dossier,
        locale,
    })
    .await
}

/// The member this walk interviews: the bound subject when the state carries
/// one, else the conversation's owner.
///
/// The gate above already established that a bound subject IS the speaker, so
/// a subject that fails to parse here is row corruption — warned and degraded
/// exactly like a corrupt `conversation.user_id`.
fn walk_subject_id(state: &OnboardingState, conv: &ConversationRecord) -> Option<Uuid> {
    state.subject_user_id.as_deref().map_or_else(
        || conversation_user_id(conv),
        |subject| match Uuid::parse_str(subject) {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    conversation_id = %conv.id,
                    "guided flow: subject_user_id is not a UUID — running the turn as ordinary coaching"
                );
                None
            }
        },
    )
}

/// The conversation's owner, or `None` when the column does not hold a UUID.
///
/// Degrading here drops the athlete mid-interview into an ordinary coaching
/// turn: no probe is asked, the directive is dropped, and their answer is
/// extracted without the onboarding stamp — so the topic comes back around
/// later as if it had never been asked. Silence made that indistinguishable
/// from a walk that simply ended, hence the warn. A non-UUID here means row
/// corruption rather than a transient fault.
fn conversation_user_id(conv: &ConversationRecord) -> Option<Uuid> {
    match Uuid::parse_str(&conv.user_id) {
        Ok(user_id) => Some(user_id),
        Err(e) => {
            tracing::warn!(
                error = %e,
                conversation_id = %conv.id,
                "guided flow: conversation user_id is not a UUID — running the turn as ordinary coaching"
            );
            None
        }
    }
}

/// The athlete's dossier, or `None` when it cannot be composed.
///
/// Degrades the turn to ordinary coaching for the reasons in
/// [`conversation_user_id`], and warns for the same reason: a transient
/// repository fault otherwise looks exactly like a finished walk.
async fn load_dossier(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    tenant_id: TenantId,
    user_id: Uuid,
) -> Option<Dossier> {
    match ctx.repos.dossier.compose_dossier(tenant_id, user_id).await {
        Ok(dossier) => Some(dossier),
        Err(e) => {
            tracing::warn!(
                error = %e,
                conversation_id = %conv.id,
                "guided flow: could not compose the dossier — running the turn as ordinary coaching"
            );
            None
        }
    }
}

/// The next topic to probe, or `None` when the flow has nothing left that its
/// audience may hear.
fn next_target(state: &OnboardingState, dossier: &Dossier) -> Option<GuidedTarget> {
    match state.flow {
        GuidedFlow::Pillars => CoverageMap::from_dossier(dossier)
            .next_target(&state.probed, state.audience)
            .map(GuidedTarget::Coverage),
        GuidedFlow::Calibration => CalibrationTopic::next_target(
            &state.probed,
            calibration_conditions(dossier, state.snapshot.as_ref()),
            state.audience,
        )
        .map(GuidedTarget::Calibration),
        GuidedFlow::Season => SeasonTopic::next_target(
            &state.probed,
            season_conditions(state.snapshot.as_ref()),
            state.audience,
        )
        .map(GuidedTarget::Season),
        // Unreachable: `resolve` returns before this for an intake, precisely so
        // that `None` here is never read as "the walk is finished", and a
        // fortnight marker is never active so no turn resolves against it.
        // Kept exhaustive rather than wildcarded so a fourth platform-driven
        // flow has to make the same decision deliberately.
        GuidedFlow::Intake | GuidedFlow::Fortnight => None,
    }
}

/// Everything ending a walk needs, bundled so the call names its tenants.
struct LeaveGuidedMode<'a> {
    ctx: &'a ChatPipelineContext,
    conv: &'a ConversationRecord,
    state: OnboardingState,
    /// Owns the conversation row the marker is retired on.
    tenant_id: TenantId,
    /// Owns the subject's facts and plans, for the wrap-up's reads.
    facts_tenant: TenantId,
    /// The member this walk interviewed.
    subject_user_id: &'a str,
    dossier: &'a Dossier,
    locale: &'a str,
}

/// End the fortnight rail: read the saved weeks back, then retire the marker.
///
/// Separate from [`leave_guided_mode`] because that one needs a dossier and a
/// walk window, and the fortnight has neither — it interviews nobody, so
/// there are no facts to supersede and no coverage to summarise. What it does
/// have is a claim to check: the agent said it wrote two weeks.
async fn finish_fortnight(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    tenant_id: TenantId,
    facts_tenant: TenantId,
    subject_user_id: Option<String>,
    locale: &str,
) -> GuidedResolution {
    // Rendered before the marker is retired, so a failed write does not also
    // cost the athlete the answer to "did it land?".
    let Some(subject) = subject_user_id else {
        // No subject, no plan to read. Retire quietly rather than assert
        // anything about weeks nobody can look up.
        tracing::warn!(
            conversation_id = %conv.id,
            "fortnight rail ended with no subject; skipping the re-check"
        );
        clear_marker(ctx, conv, tenant_id).await;
        return GuidedResolution::Inactive;
    };
    let summary = completion::render_fortnight(
        ctx,
        facts_tenant,
        &subject,
        conv.agent_id.as_deref(),
        locale,
    )
    .await;
    clear_marker(ctx, conv, tenant_id).await;
    GuidedResolution::WalkComplete {
        summary,
        answered: None,
    }
}

/// Clear the fortnight rail's marker outright, rather than retiring it.
///
/// A walk is *retired* so that one later turn can carry
/// [`release_directive`] and revoke the interview's no-writing rule. The
/// fortnight rail has no such rule to revoke and no wrap-up question to
/// answer — it already said whether the weeks landed — and a retired marker
/// would fall to the release's default arm and open with "the guided
/// interview is over" to an athlete who was never interviewed. Clearing
/// leaves the next turn as ordinary coaching, which is what it is.
async fn clear_marker(ctx: &ChatPipelineContext, conv: &ConversationRecord, tenant_id: TenantId) {
    if let Err(e) = ctx
        .repos
        .chat
        .set_conversation_onboarding_state(&conv.id, None, tenant_id)
        .await
    {
        tracing::warn!(error = %e, "failed to clear the fortnight rail marker");
    }
}

/// End the walk: render any wrap-up, retire the marker, and leave guided mode.
///
/// For the pillars walk "nothing left to ask" means every topic is covered or
/// burned its probe budget; for calibration it means every topic on this
/// athlete's list was delivered.
async fn leave_guided_mode(inputs: LeaveGuidedMode<'_>) -> GuidedResolution {
    let LeaveGuidedMode {
        ctx,
        conv,
        state,
        tenant_id,
        facts_tenant,
        subject_user_id,
        dossier,
        locale,
    } = inputs;
    // The completion summary is rendered BEFORE the marker is retired, so a
    // failed write does not also cost the athlete their wrap-up. The wrap-up
    // counts facts under the subject's own tenant — where this walk's
    // extraction lands them — never the conversation tenant a room row lives
    // under.
    let summary = match state.flow {
        GuidedFlow::Calibration => Some(
            completion::render(
                ctx,
                conv,
                &state,
                facts_tenant,
                subject_user_id,
                dossier,
                locale,
            )
            .await,
        ),
        GuidedFlow::Season => Some(
            completion::render_season(ctx, &state, facts_tenant, subject_user_id, locale).await,
        ),
        // The intake writes its own wrap-up when the platform closes it out,
        // and the fortnight's is the command's own reply — neither has a
        // wrap-up to render here.
        GuidedFlow::Pillars | GuidedFlow::Intake | GuidedFlow::Fortnight => None,
    };
    // Close the walk's window in the profile, so the next re-run of THIS flow
    // supersedes exactly this run's answers and a re-run of the OTHER
    // fixed-list flow leaves them alone. A failed write leaves the window
    // open, which is the pre-existing behaviour: superseded up to the re-run.
    if state.flow.profile_key().is_some() {
        close_walk_window(ctx, state.flow, facts_tenant, subject_user_id).await;
    }
    // Read before the state is consumed by `completed`: this turn carries the
    // athlete's answer to the interview's last question, and the deterministic
    // reply path still has to extract it.
    let answered = answered_target(&state);

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

    summary.map_or(GuidedResolution::Inactive, |summary| {
        GuidedResolution::WalkComplete { summary, answered }
    })
}

/// Stamp the walk's completion into the athlete's profile window.
///
/// Read-merge-write, because `upsert_profile` replaces the whole document;
/// a failed read is logged and leaves the profile untouched rather than
/// writing a bare window over the athlete's nutrition and equipment blocks.
async fn close_walk_window(
    ctx: &ChatPipelineContext,
    flow: GuidedFlow,
    facts_tenant: TenantId,
    subject_user_id: &str,
) {
    let Ok(user_id) = Uuid::parse_str(subject_user_id) else {
        return;
    };
    let profile = match ctx.repos.profiles.get_profile(user_id).await {
        Ok(profile) => profile,
        Err(e) => {
            tracing::warn!(error = %e, tenant_id = %facts_tenant, "could not read the profile to close the walk window");
            return;
        }
    };
    let merged = GuidedWindow::record_completion(profile, flow, &Utc::now().to_rfc3339());
    if let Err(e) = ctx.repos.profiles.upsert_profile(user_id, merged).await {
        tracing::warn!(error = %e, tenant_id = %facts_tenant, "failed to close the walk window");
    }
}

/// The topic the athlete's inbound message is answering.
///
/// A probe is recorded as delivered at the END of the turn that asks it, so by
/// the time the next turn loads this state its last ledger entry is the
/// question the athlete just replied to. [`OnboardingTurn::target`] is the
/// question this turn is about to ask, which is a different topic — stamping
/// the answer with it mis-files every guided answer by one. On the calibration
/// walk, where every topic forces a kind, that stored the availability answer
/// as [`FactKind::Injury`] and the injury answer as [`FactKind::Preference`]:
/// the dossier then hands the agent an "injury" fact reading "8 h/week,
/// Tuesdays protected", and `completion::assess` — which detects a missing
/// safety answer by its kind — reports a full house and names no gap.
///
/// `None` on the first guided turn, whose inbound message answers no probe, and
/// for a slug this build does not recognize (one written by a later build).
#[must_use]
pub fn answered_target(state: &OnboardingState) -> Option<GuidedTarget> {
    let slug = state.probed.last()?.as_str();
    if let Some(topic) = CalibrationTopic::parse(slug) {
        return Some(GuidedTarget::Calibration(topic));
    }
    if let Some(topic) = SeasonTopic::parse(slug) {
        return Some(GuidedTarget::Season(topic));
    }
    if slug == CoverageTarget::NorthStar.slug().as_str() {
        return Some(GuidedTarget::Coverage(CoverageTarget::NorthStar));
    }
    Pillar::parse(slug).map(|p| GuidedTarget::Coverage(CoverageTarget::Pillar(p)))
}

/// The state to write back at the end of a guided turn.
///
/// A turn that asked nothing records no question — the ledger is a
/// delivered-*question* history, and writing a flow that asks none into it is
/// what would poison [`answered_target`]'s parse. It still has to record that
/// it spent a turn, or a topic-less rail's budget never falls and it owns the
/// conversation for good.
fn probe_state(turn: &OnboardingTurn) -> OnboardingState {
    turn.target.map_or_else(
        || turn.state.clone().with_owned_turn(),
        |target| turn.state.clone().with_delivered_probe(target.slug()),
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
///
/// The write is a compare-and-set against the column as it stood at the start of
/// this turn. `turn.state` is a snapshot taken then, and an LLM turn runs for
/// tens of seconds while slash commands are handled synchronously in the webhook
/// path, outside the dispatch lock: an athlete who types `/calibrate` mid-turn
/// gets a fresh Calibration state written under this turn, and a blind write-back
/// would overwrite it with the stale pillars snapshot — silently reverting the
/// interview they just started and losing its load snapshot.
pub async fn record_delivered_probe(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    turn: &OnboardingTurn,
    tenant_id: TenantId,
) {
    let state = probe_state(turn);
    let column = match state.to_column() {
        Ok(column) => column,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize onboarding probe history");
            return;
        }
    };
    match ctx
        .repos
        .chat
        .compare_and_set_conversation_onboarding_state(
            &conv.id,
            conv.onboarding_state.as_deref(),
            Some(&column),
            tenant_id,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => tracing::info!(
            conversation_id = %conv.id,
            "a newer guided flow owns this conversation; not appending the delivered probe"
        ),
        Err(e) => tracing::warn!(error = %e, "failed to persist onboarding probe history"),
    }
}

/// Whether a just-finished flow is one whose rules need revoking.
///
/// Only an interview leaves a prohibition behind. The fortnight rail clears
/// its marker rather than retiring it, so this is normally false by absence —
/// but a stored row from a rail that failed to clear must still not be handed
/// the interview release, which would open with "the guided interview is
/// over" to an athlete who was never interviewed.
#[must_use]
pub fn just_completed_interview(raw: Option<&str>, now: DateTime<Utc>) -> bool {
    OnboardingState::just_completed(raw, now)
        && OnboardingState::retired_flow(raw).is_some_and(GuidedFlow::is_interview)
}

/// The system-prompt block the first turn after a guided flow ends carries.
///
/// What it says depends on which flow retired: three of the four walks get the
/// same revocation, and the season walk gets it plus the instruction that
/// answers its wrap-up. The fortnight rail never reaches here — it carries its
/// brief while it is ACTIVE, through [`directive`], because the drafting is
/// the flow's own work rather than something released after it.
///
/// The revocation is why the slot exists at all. [`directive`] is the most
/// forcefully worded block in the whole prompt: it
/// claims to override every other instruction, and it closes with "do not
/// build, propose, or save a training plan on this turn". On 2026-07-28 an
/// athlete finished a calibration interview that had carried that block on
/// eight consecutive turns; 48 seconds later, with the block gone and
/// `save_training_plan` back in the catalogue, the agent told him it could not
/// save his plan "cette fois-ci" — and the logs show it never called the tool.
/// Removing a prohibition does not retract it from the transcript that
/// prohibition already shaped, so it is retracted explicitly.
///
/// Placed with the same recency logic as [`directive`], and worded to revoke
/// rather than merely permit, because "you may now save" competes with eight
/// turns of "do not save" while "the earlier instruction no longer applies"
/// resolves them.
#[must_use]
pub fn release_directive(retired_column: Option<&str>) -> String {
    match OnboardingState::retired_flow(retired_column) {
        // The season wrap-up asked whether to lay the season out. A yes on
        // this turn is answered by the rule, not by the model's own
        // periodization: the tool reads the profile and the plan, takes the
        // walk's answers as arguments, and returns the ranked verdict the
        // agent then presents.
        Some(GuidedFlow::Season) => format!("{INTERVIEW_RELEASE}{SEASON_RELEASE_TAIL}"),
        _ => INTERVIEW_RELEASE.to_owned(),
    }
}

/// The block revoking a finished interview's no-writing rule.
const INTERVIEW_RELEASE: &str =
    "\n\n# Interview complete (this overrides the interview rules you were following)\n\
     The guided interview is over. The instruction not to build, propose or save a training \
     plan applied only while it was running and no longer applies — disregard it, along with \
     any statement you made under it about being unable to save.\n\
     Every tool listed under Available Tools is callable again this turn, including \
     save_training_plan.\n\
     Never tell the athlete that saving failed unless you called the tool on this turn and it \
     returned an error. If you intend to save, call the tool and report what it actually \
     returned.";

/// Appended when the retired walk was the season one, whose wrap-up offered to
/// lay the season out.
const SEASON_RELEASE_TAIL: &str =
    "\n\nThe wrap-up offered to lay out the athlete's season. If they accept, call \
     recommend_plan_flavour on this turn — hours_per_week and sessions_per_week from \
     their calibration availability, and event_class, weeks_to_goal, training_age, \
     measurements and interval_experience from what they said in the walk — and present \
     its verdict in your own words: the flavour it ranks first and why, what it ruled \
     out and why, and the phases it laid out. Never choose a flavour yourself, and never \
     describe a season you did not get from the tool.";

/// The brief `/fortnight` leaves for the turn that follows its go-ahead.
///
/// The command has already decided — there is an active plan, it states
/// phases, and the weeks the athlete can see stop short of the fortnight — and
/// told the athlete two weeks are being drafted and their readiness checked
/// first. That promise is kept here or nowhere: nothing else on an ordinary
/// turn would make the agent read the ladder before writing.
///
/// It names `include_state` because the readiness the command promised is
/// exactly what that flag returns, and the command deliberately did not read
/// it — the gather is the agent's, on the turn it needs it anyway to write the
/// days.
const FORTNIGHT_BRIEF: &str = "\n\n# This turn: write the next fortnight\n\
     The athlete asked for the next two weeks and the platform has already checked the \
     three things that would refuse it: they have an active plan, it states phases, and \
     the weeks they can currently see stop short of the fortnight ahead. Do not re-litigate \
     any of that.\n\
     Call get_training_plan with include_state true first. It returns the readiness ladder \
     for the coming weeks, the compliance verdict on what they have been doing, and where \
     the stored weeks stop — the readiness you were told would be checked before anything is \
     committed. Read it before you draft, and never state a readiness it did not report.\n\
     Then draft exactly two weeks, starting where the stored weeks stop, against the phase \
     each week falls in, and save them with save_training_plan on this same turn. Never \
     rewrite a week the athlete can already see. If the ladder blocks the fortnight, say so \
     and write nothing rather than writing two weeks you would have to unwrite.";

/// Retire the just-completed marker so [`release_directive`] fires once.
///
/// Called after the turn that carried the directive. Leaving it in place would
/// have every later turn told the interview "just" ended;
/// [`OnboardingState::just_completed`] bounds that to
/// [`COMPLETION_RELEASE_WINDOW_MINUTES`] anyway, so a failed clear costs a
/// repeat of the directive rather than a permanent one.
///
/// Cleared by compare-and-set against the marker this turn read, for the same
/// reason [`record_delivered_probe`] appends by compare-and-set: an athlete who
/// starts an interview mid-turn owns the column by the time this runs, and a
/// blind clear would delete the flow they just started.
pub async fn clear_completed_marker(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    tenant_id: TenantId,
) {
    match ctx
        .repos
        .chat
        .compare_and_set_conversation_onboarding_state(
            &conv.id,
            conv.onboarding_state.as_deref(),
            None,
            tenant_id,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => tracing::info!(
            conversation_id = %conv.id,
            "a newer guided flow owns this conversation; leaving the completed-interview marker alone"
        ),
        Err(e) => tracing::warn!(error = %e, "failed to clear the completed-interview marker"),
    }
}

/// The system-prompt directive steering the agent to probe the current topic
/// conversationally.
///
/// Appended at the very tail of the assembled prompt — after the channel
/// response constraints and the tool-discipline block — because an agent persona
/// may carry its own first-turn protocol ("your first reply in any conversation
/// MUST emit a plan"), and the model resolves that conflict by recency. The
/// wording therefore states the override explicitly rather than relying on
/// position alone.
#[must_use]
pub fn directive(turn: &OnboardingTurn) -> String {
    // A flow with no topic is not an interview and must not be handed the
    // interview's block — which forbids building or saving a plan, the one
    // thing the fortnight rail exists to do.
    let Some(target) = turn.target else {
        return FORTNIGHT_BRIEF.to_owned();
    };
    let (mode, purpose, topic, hint) = match target {
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
        GuidedTarget::Season(t) => (
            "Season mode",
            "You are laying out what this athlete's season is for, one question at a time",
            season_topic_label(t).to_owned(),
            t.probe_hint().to_owned(),
        ),
    };
    let baseline = calibration_baseline_line(turn);
    let room = room_audience_line(turn);
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
         plan on this turn, and do not list assumptions in place of one.{room}"
    )
}

/// The directive line a room walk appends: the interview belongs to its
/// subject alone, and the exchange is public to the room.
///
/// Prompt prose, deliberately not a locale string — it steers the model, the
/// athlete never reads it.
fn room_audience_line(turn: &OnboardingTurn) -> &'static str {
    match turn.state.audience {
        WalkAudience::Private => "",
        WalkAudience::Room => {
            "\nThis interview runs in a shared room the athlete chose. It belongs to the member \
             who started it alone: treat other members' messages as ordinary conversation, never \
             as interview answers, and do not re-ask them this question. The exchange is visible \
             to everyone in the room, so keep your questions on training territory and never \
             press for details the athlete would not say in front of a training partner."
        }
    }
}

/// What the agent is asking about, for a calibration topic.
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

/// What the agent is asking about, for a season topic.
const fn season_topic_label(topic: SeasonTopic) -> &'static str {
    match topic {
        SeasonTopic::RaceCalendar => "the events on their calendar and which one matters",
        SeasonTopic::GoalHorizon => "what a good season and a good few years would look like",
        SeasonTopic::PerformanceBaseline => "their recent best performances",
        SeasonTopic::Background => "how long they have trained and what they came from",
        SeasonTopic::MeasurementTools => "what they can steer a hard session by",
        SeasonTopic::CoachingFit => "what they want from coaching",
        SeasonTopic::FacilityAccess => "their access to a pool, a gym or a trainer",
    }
}

/// The inferred-baseline line injected on the topic that asks the athlete to
/// confirm it.
///
/// Only that topic gets it: quoting the figures on every turn would have the
/// agent reciting the athlete's training history back at them repeatedly, and
/// the numbers are only load-bearing for the confirmation question. Empty when
/// there is no snapshot — no provider connected — so the agent asks cold
/// instead of inventing figures.
fn calibration_baseline_line(turn: &OnboardingTurn) -> String {
    if turn.target != Some(GuidedTarget::Calibration(CalibrationTopic::BaselineConfirm)) {
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
/// Takes the topic the inbound message ANSWERS — [`answered_target`] — not the
/// one this turn is about to ask. Extraction reads the athlete's message, which
/// replies to the question the previous turn delivered.
///
/// Returns which pillar to tag, the provenance, and an optional forced kind
/// (North Star answers are stored as `FactKind::NorthStar` regardless of how
/// the extractor labels them; every calibration topic forces the kind its
/// answer means, so an injury answer can never be filed as a preference).
#[must_use]
pub fn extraction_params(answered: GuidedTarget) -> (Option<Pillar>, FactSource, Option<FactKind>) {
    match answered {
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
        // The calendar turn leaves the kind open: it also carries the
        // quoted-back availability, and a correction is a schedule fact
        // while the races are goals.
        GuidedTarget::Season(t) => (
            Some(t.pillar()),
            FactSource::Onboarding,
            t.fact_kind().map(FactKind::parse_lenient),
        ),
    }
}

/// Extraction stamping for a turn: the onboarding params when the inbound
/// message answers a guided probe, else the background-worker defaults (no
/// pillar, conversation source).
///
/// The defaults also cover the first turn of a guided flow, whose message
/// answers no probe: it arrived before any question was asked, so stamping it
/// with a topic would file whatever the athlete happened to open with as that
/// topic's answer.
#[must_use]
pub fn extraction_params_or_default(
    answered: Option<GuidedTarget>,
) -> (Option<Pillar>, FactSource, Option<FactKind>) {
    answered.map_or((None, FactSource::Conversation, None), extraction_params)
}
