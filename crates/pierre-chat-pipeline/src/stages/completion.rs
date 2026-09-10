// ABOUTME: The fixed-list walks' platform-rendered wrap-ups and facts-landed checks — calibration and season
// ABOUTME: Reports what was actually captured, and names the answer whose absence the next step cannot survive
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Deterministic completion for the calibration interview.
//!
//! The failure this exists to prevent: the interview runs its questions,
//! extraction lands nothing, nothing downstream notices, the next plan is
//! identical, and the athlete concludes the feature is theatre. So the wrap-up
//! is not written by the coach — it is rendered here from a count of the facts
//! that actually landed inside the interview's window, and it says so when the
//! count is short.
//!
//! Safety topics get named treatment. An interview that captured the athlete's
//! appetite for more load but not their injury history or recovery speed is
//! worse than no interview, because every answer it *did* land argues one way.
//! Those two topics are each the sole writer of their fact kind (see
//! [`CalibrationTopic::fact_kind`]), which is what makes their absence
//! detectable at all.

use chrono::{DateTime, Utc};
use pierre_contremaitre::messaging_strings::{
    KEY_CALIBRATE_COMPLETE_HEADER, KEY_CALIBRATE_COMPLETE_MISSING, KEY_CALIBRATE_FOLLOWUP_NO_PLAN,
    KEY_CALIBRATE_FOLLOWUP_PLAN, KEY_CALIBRATE_TOPIC_INJURY, KEY_CALIBRATE_TOPIC_RECOVERY,
    KEY_FORTNIGHT_RECHECK_LANDED, KEY_FORTNIGHT_RECHECK_MISSING, KEY_SEASON_COMPLETE_HEADER,
    KEY_SEASON_COMPLETE_MISSING_GOAL, KEY_SEASON_FOLLOWUP_LAY_OUT,
};
use pierre_core::models::{
    CalibrationTopic, ConversationRecord, Dossier, OnboardingState, SeasonTopic, TenantId,
};
use pierre_memory::{FactSource, UserFact};
use pierre_services::athlete_clock::athlete_today;
use pierre_services::fortnight::FORTNIGHT_WEEKS;
use pierre_services::training_plan_render::select_active_weeks;

use super::onboarding::{calibration_conditions, season_conditions};
use crate::ChatPipelineContext;

/// Upper bound on facts pulled when counting what the interview landed. An
/// interview asks at most eight questions, so this leaves generous room for an
/// extractor that split one answer into several facts.
const LANDED_FETCH_LIMIT: i64 = 100;

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
pub async fn render(
    ctx: &ChatPipelineContext,
    conv: &ConversationRecord,
    state: &OnboardingState,
    facts_tenant: TenantId,
    subject_user_id: &str,
    dossier: &Dossier,
    locale: &str,
) -> String {
    let reg = &ctx.messaging_strings_registry;
    let asked =
        CalibrationTopic::for_conditions(calibration_conditions(dossier, state.snapshot.as_ref()));

    // An unparseable start stamp falls back to now, which credits nothing and
    // re-asks — the same safe direction as an unreadable fact store.
    let started_at = DateTime::parse_from_rfc3339(&state.started_at)
        .map_or_else(|_| Utc::now(), |d| d.with_timezone(&Utc));

    // Fetch by source rather than by kind: the interview's answers span four
    // kinds, and a by-kind sweep would need one query per kind. An unreadable
    // store reports zero captured, which re-asks — the safe direction.
    let landed = ctx
        .repos
        .memory
        .list_user_facts_by_source(
            facts_tenant,
            subject_user_id,
            FactSource::Onboarding,
            LANDED_FETCH_LIMIT,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read landed calibration facts; reporting zero");
            Vec::new()
        });

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
    // Scoped to this conversation's coach, which is the slug `save_training_plan`
    // binds a plan to. The lookup falls back to a coach-agnostic plan on its own
    // (`coach_slug IN (slug, '')`), so passing the coach only widens what counts:
    // a `None` here matches agnostic plans alone, and calibration runs inside
    // coach-bound messaging conversations — it would tell the athletes most
    // likely to hold a plan, the ones who built one with a coach, to build one.
    let has_plan = ctx
        .repos
        .training_plans
        .get_active_plan(
            &facts_tenant.to_string(),
            subject_user_id,
            conv.coach_id.as_deref(),
        )
        .await
        .ok()
        .flatten()
        .is_some();
    let followup = if has_plan {
        KEY_CALIBRATE_FOLLOWUP_PLAN
    } else {
        KEY_CALIBRATE_FOLLOWUP_NO_PLAN
    };
    out.push_str("\n\n");
    out.push_str(&reg.render(followup, locale, &[]));

    out
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
) -> String {
    let reg = &ctx.messaging_strings_registry;
    let asked = SeasonTopic::for_conditions(season_conditions(state.snapshot.as_ref()));
    let started_at = DateTime::parse_from_rfc3339(&state.started_at)
        .map_or_else(|_| Utc::now(), |d| d.with_timezone(&Utc));

    let landed = ctx
        .repos
        .memory
        .list_user_facts_by_source(
            facts_tenant,
            subject_user_id,
            FactSource::Onboarding,
            LANDED_FETCH_LIMIT,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read landed season facts; reporting zero");
            Vec::new()
        });

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
    out
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
    agent: Option<&str>,
    locale: &str,
) -> String {
    let reg = &ctx.messaging_strings_registry;
    let today = athlete_today(&ctx.repos, subject_user_id.parse().unwrap_or_default()).await;

    let covered = match ctx
        .repos
        .training_plans
        .get_active_plan(&facts_tenant.to_string(), subject_user_id, agent)
        .await
    {
        Ok(Some(plan)) => {
            let weeks = ctx
                .repos
                .training_plans
                .list_plan_weeks(&facts_tenant.to_string(), subject_user_id, &plan.id, false)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "fortnight wrap-up could not read the plan weeks");
                    Vec::new()
                });
            select_active_weeks(&weeks, today, FORTNIGHT_WEEKS)
                .weeks
                .len()
                >= FORTNIGHT_WEEKS
        }
        Ok(None) => false,
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
    reg.render(key, locale, &[])
}

#[cfg(test)]
mod tests {
    use super::{assess, assess_season};
    use chrono::{Duration, Utc};
    use pierre_core::models::{CalibrationTopic, SeasonTopic};
    use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode, UserFact};

    fn fact(kind: FactKind, age_minutes: i64) -> UserFact {
        let ts = Utc::now() - Duration::minutes(age_minutes);
        UserFact {
            id: format!("{kind:?}-{age_minutes}"),
            tenant_id: "t".to_owned(),
            user_id: "u".to_owned(),
            coach_id: None,
            scope: MemoryScope::User,
            kind,
            pillar: None,
            predicate_code: PredicateCode::States,
            object: "o".to_owned(),
            confidence: 0.9,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn a_silent_interview_reports_zero_and_names_both_safety_gaps() {
        // The failure this whole module exists for: every question asked,
        // nothing extracted. It must not read as success.
        let (captured, missing) = assess(
            &[],
            &CalibrationTopic::CORE,
            Utc::now() - Duration::hours(1),
        );
        assert_eq!(captured, 0);
        assert_eq!(
            missing,
            vec![CalibrationTopic::Injury, CalibrationTopic::RecoverySpeed]
        );
    }

    #[test]
    fn facts_from_before_the_interview_do_not_count() {
        // A pillars walk months ago also wrote `source=onboarding` facts. If
        // those counted, an interview that landed nothing would report a full
        // house and the athlete would never be asked again.
        let started = Utc::now() - Duration::hours(1);
        let stale = vec![
            fact(FactKind::Injury, 60 * 24 * 30),
            fact(FactKind::Physiology, 60 * 24 * 30),
        ];
        let (captured, missing) = assess(&stale, &CalibrationTopic::CORE, started);
        assert_eq!(captured, 0, "month-old facts predate this interview");
        assert_eq!(missing.len(), 2);
    }

    #[test]
    fn a_landed_injury_answer_clears_only_that_safety_gap() {
        let started = Utc::now() - Duration::hours(1);
        let landed = vec![fact(FactKind::Injury, 10)];
        let (captured, missing) = assess(&landed, &CalibrationTopic::CORE, started);
        assert_eq!(captured, 1);
        assert_eq!(
            missing,
            vec![CalibrationTopic::RecoverySpeed],
            "recovery speed is still unanswered and must still be named"
        );
    }

    #[test]
    fn a_complete_interview_names_no_gaps() {
        let started = Utc::now() - Duration::hours(1);
        let landed = vec![
            fact(FactKind::Preference, 50), // progression intent
            fact(FactKind::Preference, 40), // baseline confirm
            fact(FactKind::Schedule, 30),   // availability
            fact(FactKind::Injury, 20),     // injury
            fact(FactKind::Preference, 15), // rpe headroom
            fact(FactKind::Physiology, 10), // recovery speed
        ];
        let (captured, missing) = assess(&landed, &CalibrationTopic::CORE, started);
        assert_eq!(captured, 6, "all six core topics produced a fact");
        assert!(missing.is_empty());
    }

    #[test]
    fn shared_kinds_are_credited_once_per_fact_not_once_per_topic() {
        // Three topics write `preference`. One preference fact must credit one
        // topic, not three — over-reporting is the direction that makes the
        // wrap-up a lie.
        let started = Utc::now() - Duration::hours(1);
        let landed = vec![fact(FactKind::Preference, 10)];
        let (captured, _) = assess(&landed, &CalibrationTopic::CORE, started);
        assert_eq!(captured, 1);

        let landed = vec![
            fact(FactKind::Preference, 10),
            fact(FactKind::Preference, 9),
        ];
        let (captured, _) = assess(&landed, &CalibrationTopic::CORE, started);
        assert_eq!(captured, 2);
    }

    #[test]
    fn a_conditional_topic_that_was_asked_is_counted_in_the_denominator() {
        let started = Utc::now() - Duration::hours(1);
        let mut asked = CalibrationTopic::CORE.to_vec();
        asked.push(CalibrationTopic::EventDemand);
        let landed = vec![fact(FactKind::Goal, 10)];
        let (captured, missing) = assess(&landed, &asked, started);
        assert_eq!(captured, 1, "only the event-demand answer landed");
        assert_eq!(missing.len(), 2, "both safety topics are still missing");
    }

    #[test]
    fn a_season_walk_with_no_goal_fact_is_short_a_calendar() {
        let started = Utc::now() - Duration::minutes(30);
        let landed = vec![
            fact(FactKind::Physiology, 20),
            fact(FactKind::Preference, 15),
            fact(FactKind::Equipment, 10),
            fact(FactKind::Preference, 5),
        ];
        let (captured, goal_missing) = assess_season(&landed, &SeasonTopic::CORE, started);
        assert_eq!(
            captured, 4,
            "bests, background, tools and coaching fit landed"
        );
        assert!(
            goal_missing,
            "no goal fact means no calendar to lay a season on"
        );
    }

    #[test]
    fn a_goal_fact_credits_the_calendar_and_the_horizon_separately() {
        let started = Utc::now() - Duration::minutes(30);
        let landed = vec![fact(FactKind::Goal, 25), fact(FactKind::Goal, 20)];
        let (captured, goal_missing) = assess_season(&landed, &SeasonTopic::CORE, started);
        assert_eq!(captured, 2, "two goal facts credit both goal topics");
        assert!(!goal_missing);

        let one = vec![fact(FactKind::Goal, 25)];
        let (captured, goal_missing) = assess_season(&one, &SeasonTopic::CORE, started);
        assert_eq!(
            captured, 1,
            "one goal fact credits the calendar, asked first"
        );
        assert!(!goal_missing);
    }

    #[test]
    fn season_facts_before_the_window_do_not_count() {
        let started = Utc::now() - Duration::minutes(10);
        let stale = vec![fact(FactKind::Goal, 60), fact(FactKind::Equipment, 45)];
        let (captured, goal_missing) = assess_season(&stale, &SeasonTopic::CORE, started);
        assert_eq!(captured, 0);
        assert!(goal_missing);
    }
}
