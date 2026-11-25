// ABOUTME: The coach's calls an agent's week save keeps — on each session it saves unchanged over the week it replaces
// ABOUTME: That week is the coach's draft over the live plan on the coach's path, else the athlete's live week
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coach's call through an agent's save (carnet#1814)
//!
//! A coach marks a session of their draft as their call, « C'est mon
//! choix », and the mark goes live with the draft. An agent's save never
//! takes a call from its payload (`validate_week` drops it); it carries the
//! call of the week it replaces onto each session it writes unchanged, at
//! the same date and position ([`carry_coach_calls`]). A session it changes
//! loses the call, so the coach's pre-publish check asks about it again.

use pierre_core::errors::AppResult;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{carry_coach_calls, PlanWeek, TrainingPlan};
use pierre_services::coach_plan_draft::{coach_plan_view, CoachDraftTarget};

use super::WeekPayload;

/// The weeks a save writes over, in calendar order: on the coach's path
/// (`draft_target`), the coach's draft — the live weeks with each week the
/// coach drafted in its place — else the live weeks the draft would open
/// from; otherwise the weeks of the athlete's season `active`. Empty when
/// there is none.
///
/// # Errors
///
/// The repository errors of the reads, and `UnavailableOverTransport` for a
/// plan an external caller may not read.
pub(super) async fn weeks_written_over(
    repos: &RepositoryRegistry,
    active: Option<&TrainingPlan>,
    draft_target: Option<&CoachDraftTarget>,
) -> AppResult<Vec<PlanWeek>> {
    if let Some(target) = draft_target {
        let view = coach_plan_view(repos, target).await?;
        return Ok(view
            .draft
            .map(|draft| draft.weeks)
            .or_else(|| view.live.map(|live| live.weeks))
            .unwrap_or_default());
    }
    let Some(plan) = active else {
        return Ok(Vec::new());
    };
    repos
        .training_plans
        .list_plan_weeks(&plan.tenant_id, &plan.user_id, &plan.id, false)
        .await
}

/// Keep, on each week of `weeks`, the coach's calls the week of the same
/// `week_start` in `prior` holds on the sessions saved unchanged; every other
/// session is saved with none.
pub(super) fn carry_onto_weeks(weeks: &mut [WeekPayload], prior: &[PlanWeek]) {
    for week in weeks {
        let replaced = prior
            .iter()
            .find(|held| held.week_start == week.week_start)
            .map_or(&[][..], |held| held.days.as_slice());
        carry_coach_calls(&mut week.days, replaced);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_tronc::dates::Timestamp;
    use pierre_memory::training_plans::{CoachCall, PlannedDay, WeekStatus};
    use serde_json::{json, Value};
    use uuid::Uuid;

    fn day(date: &str, workout: &str) -> Value {
        json!({ "date": date, "sport": "run", "workout": workout, "duration_min": 40, "intensity": "Z2" })
    }

    fn prior_week(week_start: &str, days: Vec<PlannedDay>) -> PlanWeek {
        PlanWeek {
            id: format!("week-{week_start}"),
            tenant_id: "tenant".to_owned(),
            user_id: "athlete".to_owned(),
            plan_id: "plan".to_owned(),
            week_start: week_start.to_owned(),
            focus: String::new(),
            phase_index: None,
            days,
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            author_agent_id: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        }
    }

    fn called(value: Value, call: CoachCall) -> PlannedDay {
        PlannedDay {
            coach_call: Some(call),
            ..serde_json::from_value(value).unwrap()
        }
    }

    #[test]
    fn a_save_keeps_each_weeks_calls_on_the_sessions_it_writes_unchanged() {
        let call = CoachCall {
            by: Uuid::from_u128(7),
            at: Timestamp::now(),
        };
        let prior = vec![
            prior_week(
                "2026-11-16",
                vec![
                    called(day("2026-11-17", "Tempo"), call),
                    called(day("2026-11-18", "Easy"), call),
                ],
            ),
            prior_week("2026-11-23", vec![called(day("2026-11-24", "Tempo"), call)]),
        ];
        let mut weeks: Vec<WeekPayload> = serde_json::from_value(json!([
            { "week_start": "2026-11-16", "days": [day("2026-11-17", "Tempo"), day("2026-11-18", "Hills")] },
            { "week_start": "2026-11-30", "days": [day("2026-12-01", "Tempo")] },
        ]))
        .unwrap();
        carry_onto_weeks(&mut weeks, &prior);
        assert_eq!(
            weeks[0].days[0].coach_call,
            Some(call),
            "bug: a session saved unchanged keeps the coach's call from the week it replaces"
        );
        assert!(
            weeks[0].days[1].coach_call.is_none(),
            "bug: a session the save changed keeps a call: {:?}",
            weeks[0].days[1]
        );
        assert!(
            weeks[1].days[0].coach_call.is_none(),
            "bug: a week that replaces none takes another week's call: {:?}",
            weeks[1].days[0]
        );
    }
}
