// ABOUTME: The guided walks' completions — calibration, season and fortnight wrap-ups, and the pillars walk's record
// ABOUTME: Reports what was actually captured, and names the answer whose absence the next step cannot survive
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Deterministic completion for the calibration interview.
//!
//! The failure this exists to prevent: the interview runs its questions,
//! extraction lands nothing, nothing downstream notices, the next plan is
//! identical, and the athlete concludes the feature is theatre. So the wrap-up
//! is not written by the agent — it is rendered here from a count of the facts
//! that actually landed inside the interview's window, and it says so when the
//! count is short.
//!
//! Safety topics get named treatment. An interview that captured the athlete's
//! appetite for more load but not their injury history or recovery speed is
//! worse than no interview, because every answer it *did* land argues one way.
//! Those two topics are each the sole writer of their fact kind (see
//! [`CalibrationTopic::fact_kind`]), which is what makes their absence
//! detectable at all.
//!
//! The pillars walk is counted the same way, by the facts that landed inside
//! its window, and reported as `onboarding.completed` like the others — but
//! it closes on the agent's own reply rather than one written here.

use chrono::{DateTime, Utc};
use pierre_contremaitre::messaging_strings::{
    KEY_CALIBRATE_COMPLETE_HEADER, KEY_CALIBRATE_COMPLETE_MISSING, KEY_CALIBRATE_FOLLOWUP_NO_PLAN,
    KEY_CALIBRATE_FOLLOWUP_PLAN, KEY_CALIBRATE_TOPIC_INJURY, KEY_CALIBRATE_TOPIC_RECOVERY,
    KEY_FORTNIGHT_RECHECK_LANDED, KEY_FORTNIGHT_RECHECK_MISSING, KEY_SEASON_COMPLETE_HEADER,
    KEY_SEASON_COMPLETE_MISSING_GOAL, KEY_SEASON_FOLLOWUP_LAY_OUT,
};
use pierre_core::models::{
    CalibrationTopic, CoverageTarget, Dossier, OnboardingState, SeasonTopic, TenantId,
};
use pierre_memory::{FactKind, FactSource, UserFact};
use pierre_providers::ai_scope;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::training_plan_render::fortnight_is_covered;

use super::deterministic_reply::PlatformReply;
use super::onboarding::{calibration_conditions, season_conditions};
use crate::ChatPipelineContext;

/// Upper bound on facts pulled when counting what the interview landed. An
/// interview asks at most eight questions, so this leaves generous room for an
/// extractor that split one answer into several facts.
const LANDED_FETCH_LIMIT: i64 = 100;

/// When the walk started: the window its answers are credited inside.
///
/// An unparseable start stamp falls back to now, which credits nothing and
/// re-asks — the same safe direction as an unreadable fact store.
fn walk_started_at(state: &OnboardingState) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&state.started_at)
        .map_or_else(|_| Utc::now(), |d| d.with_timezone(&Utc))
}

/// The subject's interview-sourced facts this turn may read.
///
/// Fetched by source rather than by kind: a walk's answers span several
/// kinds, and a by-kind sweep would need one query per kind. An unreadable
/// store reports none, which re-asks — the safe direction. A fact derived
/// from first-party-only data stays out of an external turn, and stamps a
/// first-party one (carnet#769).
async fn landed_onboarding_facts(
    ctx: &ChatPipelineContext,
    facts_tenant: TenantId,
    subject_user_id: &str,
    flow: &'static str,
) -> Vec<UserFact> {
    let mut landed = ctx
        .repos
        .memory
        .list_user_facts_by_source(
            facts_tenant,
            subject_user_id,
            FactSource::Onboarding,
            LANDED_FETCH_LIMIT,
            ai_scope::readable_policy(),
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, flow, "could not read the walk's landed facts; reporting zero");
            Vec::new()
        });
    ai_scope::retain_admitted(&mut landed, |fact| fact.transport_policy);
    landed
}

/// The locale string naming a safety-critical topic in the wrap-up.
const fn topic_label_key(topic: CalibrationTopic) -> Option<&'static str> {
    match topic {
        CalibrationTopic::Injury => Some(KEY_CALIBRATE_TOPIC_INJURY),
        CalibrationTopic::RecoverySpeed => Some(KEY_CALIBRATE_TOPIC_RECOVERY),
        _ => None,
    }
}

/// How many of `asked` produced at least one fact, and which safety-critical
/// topics produced none.
///
/// Counting is by fact kind, which is exact for the two safety topics (each is
/// the sole writer of its kind) and approximate for the rest: five topics share
/// `preference`, so several answers landing under it count once. The header
/// therefore under-reports rather than over-reports — the honest direction for
/// a message whose job is to admit gaps.
fn assess(
    landed: &[UserFact],
    asked: &[CalibrationTopic],
    started_at: DateTime<Utc>,
) -> (usize, Vec<CalibrationTopic>) {
    let kinds: Vec<&str> = asked.iter().map(|t| t.fact_kind()).collect();
    let (captured, missing) = credit_by_kind(landed, &kinds, started_at);
    let missing_safety = missing
        .into_iter()
        .map(|i| asked[i])
        .filter(|t| t.is_safety_critical())
        .collect();
    (captured, missing_safety)
}

/// The kind-crediting core both walks share: how many of the asked kinds
/// produced at least one fact inside the window, and the indexes of the
/// topics that produced none.
fn credit_by_kind(
    landed: &[UserFact],
    asked_kinds: &[&str],
    started_at: DateTime<Utc>,
) -> (usize, Vec<usize>) {
    let kinds_landed: Vec<&str> = landed
        .iter()
        .filter(|f| f.created_at >= started_at)
        .map(|f| f.kind.as_str())
        .collect();

    let mut captured = 0;
    let mut missing = Vec::new();
    let mut counted_kinds: Vec<&str> = Vec::new();
    for (index, kind) in asked_kinds.iter().copied().enumerate() {
        let present = kinds_landed.contains(&kind);
        if present && !counted_kinds.contains(&kind) {
            counted_kinds.push(kind);
            captured += 1;
        } else if present {
            // A kind shared by several topics: each additional topic counts
            // only if another fact of that kind landed beyond the ones already
            // credited.
            let landed_of_kind = kinds_landed.iter().filter(|k| **k == kind).count();
            let credited = counted_kinds.iter().filter(|k| **k == kind).count();
            if landed_of_kind > credited {
                counted_kinds.push(kind);
                captured += 1;
            }
        }
        if !present {
            missing.push(index);
        }
    }
    (captured, missing)
}

/// How many of the season walk's topics produced a fact, and whether the
/// calendar — the one answer the season layout cannot do without — did.
///
/// The calendar credits on a `goal` fact, whichever other kinds its turn
/// produced (a corrected availability lands as `schedule`).
fn assess_season(
    landed: &[UserFact],
    asked: &[SeasonTopic],
    started_at: DateTime<Utc>,
) -> (usize, bool) {
    let kinds: Vec<&str> = asked.iter().map(|t| t.landed_kind()).collect();
    let (captured, missing) = credit_by_kind(landed, &kinds, started_at);
    let goal_missing = missing
        .into_iter()
        .any(|i| asked[i] == SeasonTopic::RaceCalendar);
    (captured, goal_missing)
}

/// Render the interview's closing message.
///
/// Never claims success it cannot evidence: the header states the real count,
/// and a missing safety answer is named with an instruction to redo it.
///
/// LIMITATION(registre#830): `render`, `render_season` and `render_fortnight` close with `PlatformReply::text_only` — no next steps, and no line naming the command a step runs — because the transition table that picks each walk's steps from the use-case catalogue, filtered by its `requires` predicates, does not exist.
pub async fn render(
    ctx: &ChatPipelineContext,
    state: &OnboardingState,
    facts_tenant: TenantId,
    subject_user_id: &str,
    dossier: &Dossier,
    locale: &str,
) -> PlatformReply {
    let reg = &ctx.messaging_strings_registry;
    let asked =
        CalibrationTopic::for_conditions(calibration_conditions(dossier, state.snapshot.as_ref()));
    let started_at = walk_started_at(state);
    let landed = landed_onboarding_facts(ctx, facts_tenant, subject_user_id, "calibration").await;

    let (captured, missing_safety) = assess(&landed, &asked, started_at);

    tracing::info!(
        target: "notify",
        event = "onboarding.completed",
        flow = "calibration",
        topics_answered = captured,
        topics_asked = asked.len(),
        facts_landed = landed.len(),
        "guided interview completed"
    );

    let mut out = reg.render(
        KEY_CALIBRATE_COMPLETE_HEADER,
        locale,
        &[&captured.to_string(), &asked.len().to_string()],
    );

    for topic in &missing_safety {
        if let Some(key) = topic_label_key(*topic) {
            let label = reg.render(key, locale, &[]);
            out.push_str("\n\n");
            out.push_str(&reg.render(KEY_CALIBRATE_COMPLETE_MISSING, locale, &[&label]));
        }
    }

    // Follow-up: offer a rebuild when the athlete already has a plan, a build
    // when they do not. Both are questions, never actions — the athlete
    // approves the change.
    //
    // The athlete's one season, whichever agent laid it: an athlete who built
    // a plan with any agent is offered the rebuild, never told to build one.
    let has_plan = ctx
        .repos
        .training_plans
        .get_active_plan(&facts_tenant.to_string(), subject_user_id)
        .await
        .ok()
        .flatten()
        .is_some_and(|plan| ai_scope::admit_derived(plan.transport_policy));
    let followup = if has_plan {
        KEY_CALIBRATE_FOLLOWUP_PLAN
    } else {
        KEY_CALIBRATE_FOLLOWUP_NO_PLAN
    };
    out.push_str("\n\n");
    out.push_str(&reg.render(followup, locale, &[]));

    PlatformReply::text_only(out)
}

/// Render the season walk's closing message.
///
/// Same contract as [`render`]: the header states the real count, and a
/// missing goal race is named — the layout cannot run without one — with
/// the offer to lay the season out deferred until the athlete names it.
/// Otherwise the message closes on the offer, and a yes on the next turn
/// runs `recommend_plan_flavour` under the release directive.
pub async fn render_season(
    ctx: &ChatPipelineContext,
    state: &OnboardingState,
    facts_tenant: TenantId,
    subject_user_id: &str,
    locale: &str,
) -> PlatformReply {
    let reg = &ctx.messaging_strings_registry;
    let asked = SeasonTopic::for_conditions(season_conditions(state.snapshot.as_ref()));
    let started_at = walk_started_at(state);
    let landed = landed_onboarding_facts(ctx, facts_tenant, subject_user_id, "season").await;

    let (captured, goal_missing) = assess_season(&landed, &asked, started_at);

    tracing::info!(
        target: "notify",
        event = "onboarding.completed",
        flow = "season",
        topics_answered = captured,
        topics_asked = asked.len(),
        facts_landed = landed.len(),
        "guided interview completed"
    );

    let mut out = reg.render(
        KEY_SEASON_COMPLETE_HEADER,
        locale,
        &[&captured.to_string(), &asked.len().to_string()],
    );
    out.push_str("\n\n");
    let followup = if goal_missing {
        KEY_SEASON_COMPLETE_MISSING_GOAL
    } else {
        KEY_SEASON_FOLLOWUP_LAY_OUT
    };
    out.push_str(&reg.render(followup, locale, &[]));
    PlatformReply::text_only(out)
}

/// The fortnight rail's wrap-up: did the two weeks actually land?
///
/// This is the re-check the rail exists to close. The command decided, the
/// agent drafted and saved, and until now nothing read the result back — the
/// command had ended by the time `save_training_plan` returned, so "I've
/// written your fortnight" was the last word whether or not it was true.
///
/// It reads coverage rather than the full state block on purpose. The
/// question at the end of the rail is "are the next two weeks on the plan",
/// which the stored weeks answer directly; readiness and compliance are the
/// drafting turn's inputs, and re-gathering them here would spend the rails'
/// whole gather to say something the athlete did not ask.
pub async fn render_fortnight(
    ctx: &ChatPipelineContext,
    facts_tenant: TenantId,
    subject_user_id: &str,
    locale: &str,
) -> PlatformReply {
    let reg = &ctx.messaging_strings_registry;
    let today = athlete_today(&ctx.repos, subject_user_id.parse().unwrap_or_default()).await;

    let covered = match ctx
        .repos
        .training_plans
        .get_active_plan(&facts_tenant.to_string(), subject_user_id)
        .await
    {
        Ok(Some(plan)) if ai_scope::admit_derived(plan.transport_policy) => {
            let weeks = ctx
                .repos
                .training_plans
                .list_plan_weeks(&facts_tenant.to_string(), subject_user_id, &plan.id, false)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "fortnight wrap-up could not read the plan weeks");
                    Vec::new()
                });
            fortnight_is_covered(&weeks, today)
        }
        // No plan, or one an external turn is not served.
        Ok(_) => false,
        Err(e) => {
            // An unreadable plan is not an empty one, and claiming the weeks
            // landed on a failed read is the false confirmation this wrap-up
            // exists to remove.
            tracing::warn!(error = %e, "fortnight wrap-up could not read the active plan");
            false
        }
    };

    tracing::info!(
        target: "notify",
        event = "onboarding.completed",
        flow = "fortnight",
        topics_answered = u32::from(covered),
        topics_asked = 1,
        facts_landed = 0,
        "guided interview completed"
    );

    let key = if covered {
        KEY_FORTNIGHT_RECHECK_LANDED
    } else {
        KEY_FORTNIGHT_RECHECK_MISSING
    };
    PlatformReply::text_only(reg.render(key, locale, &[]))
}

/// Record that the pillars walk reached its end, as `onboarding.completed`
/// with `flow = pillars`.
///
/// Counted like the fixed-list walks, from the facts that landed inside the
/// walk's window. The topics asked are the distinct ones the delivered-probe
/// ledger names — a topic already covered when the walk began was never asked,
/// so it counts neither way — and one is answered when a fact in the window
/// is about it: the North Star by the kind extraction forces on that answer,
/// a pillar by the pillar extraction stamps on it. `facts_landed` is the
/// window's count, so it reports what this walk captured and nothing an
/// earlier walk did.
///
/// It sends nothing, so the walk closes on the agent's own reply.
///
/// LIMITATION(registre#830): `record_pillars` writes no wrap-up and offers no next step — contremaitre has no pillars completion string for the platform to send in the agent's place.
pub async fn record_pillars(
    ctx: &ChatPipelineContext,
    state: &OnboardingState,
    facts_tenant: TenantId,
    subject_user_id: &str,
) {
    let asked = pillars_asked(state);
    let started_at = walk_started_at(state);
    let landed = landed_onboarding_facts(ctx, facts_tenant, subject_user_id, "pillars").await;
    let in_window: Vec<&UserFact> = landed
        .iter()
        .filter(|fact| fact.created_at >= started_at)
        .collect();
    let answered = asked
        .iter()
        .filter(|topic| in_window.iter().any(|fact| answers(fact, **topic)))
        .count();

    tracing::info!(
        target: "notify",
        event = "onboarding.completed",
        flow = "pillars",
        topics_answered = answered,
        topics_asked = asked.len(),
        facts_landed = in_window.len(),
        "guided interview completed"
    );
}

/// The distinct pillars-walk topics the ledger records a delivered probe
/// for, in the order they were first asked.
fn pillars_asked(state: &OnboardingState) -> Vec<CoverageTarget> {
    let mut asked = Vec::new();
    for target in state
        .probed
        .iter()
        .filter_map(|slug| CoverageTarget::parse(slug.as_str()))
    {
        if !asked.contains(&target) {
            asked.push(target);
        }
    }
    asked
}

/// Whether `fact` answers the pillars-walk topic `target`.
fn answers(fact: &UserFact, target: CoverageTarget) -> bool {
    match target {
        CoverageTarget::NorthStar => fact.kind == FactKind::NorthStar,
        CoverageTarget::Pillar(pillar) => fact.pillar == Some(pillar),
    }
}
