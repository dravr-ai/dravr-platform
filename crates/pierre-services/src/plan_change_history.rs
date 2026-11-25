// ABOUTME: Records each change a coach or the thread's agent makes to an athlete's plan in the coach's workspace
// ABOUTME: Written after the change landed; a write that fails is logged and never fails the change, and no plan content is logged

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coach's plan workspace history (carnet#1815)
//!
//! Every change to a coached athlete's plan made in the coach's workspace —
//! each day write of the coach's, each save the thread's agent makes into
//! the draft, the publish and the discard — records one event once it
//! landed ([`record_plan_change`]): who made it, what kind of change it was
//! and the dates it touched. The coach's thread lists the coach's own events
//! newest first (`GET …/subject-plan/history`).
//!
//! The record is a side effect of a change that already succeeded, so it
//! never fails that change: a write the repository refuses is logged with
//! the kind, the people and how many dates it named, never what the plan
//! says, and the change is answered as it landed.
//!
//! Which dates a change names:
//!
//! - a coach's day write: the date it wrote;
//! - an agent's save into the draft: the dates whose sessions it changed
//!   over the weeks it wrote over ([`dates_changed`]);
//! - a publish: the dates it changed over the athlete's live plan, or every
//!   date it holds when the athlete had none;
//! - a discard: none — the whole draft goes, and the events before it name
//!   the dates the draft held.

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_database::repositories::{NewPlanChangeEvent, PlanChangeKind, PlanChangeSource};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{self, parse_plan_date, PlanWeek};
use tracing::warn;

use crate::coach_plan_draft::CoachDraftTarget;

/// One change to record: who made it, its kind, and the dates it touched.
#[derive(Debug, Clone, Copy)]
pub struct PlanChange<'change> {
    /// Who made it.
    pub source: PlanChangeSource,
    /// What kind of change it was.
    pub kind: PlanChangeKind,
    /// The dates it touched, in any order.
    pub dates: &'change [CivilDate],
}

/// Record `change` to the plan of `target`'s athlete, as landed now.
///
/// The change was made in `target`'s coach's workspace. Never fails: the
/// change has already landed, so a write the repository refuses is logged —
/// the kind, the source, the athlete, the coach and how many dates it named,
/// never the plan's content — and the caller answers the change as it
/// stands.
pub async fn record_plan_change(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    change: PlanChange<'_>,
) {
    let event = NewPlanChangeEvent {
        tenant: target.tenant(),
        athlete: target.athlete(),
        coach: target.coach(),
        source: change.source,
        kind: change.kind,
        dates: change.dates,
        created_at: Timestamp::now(),
    };
    if let Err(error) = repos.plan_change_events.record_plan_change(&event).await {
        warn!(
            error = %error,
            kind = change.kind.as_str(),
            source = change.source.as_str(),
            athlete_user_id = %target.athlete(),
            coach_user_id = %target.coach(),
            dates = change.dates.len(),
            "plan change history: the change landed but its event was not recorded"
        );
    }
}

/// The dates whose sessions `after` changed over `before`, in date order.
///
/// `after` is the weeks a save or publish wrote, each compared with the week
/// of the same `week_start` it wrote over
/// ([`training_plans::changed_dates`]). With no `before`, every date `after`
/// holds. A stored date that does not parse cannot be named, and is left out
/// (logged).
#[must_use]
pub fn dates_changed(before: &[PlanWeek], after: &[PlanWeek]) -> Vec<CivilDate> {
    training_plans::changed_dates(before, after)
        .into_iter()
        .filter_map(|date| {
            parse_plan_date(date).or_else(|| {
                warn!(
                    date,
                    "plan change: a changed date does not parse; not named"
                );
                None
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_memory::training_plans::{PlannedDay, WeekStatus};
    use serde_json::json;

    fn week(week_start: &str, days: &[(&str, &str)]) -> PlanWeek {
        PlanWeek {
            id: format!("week-{week_start}"),
            tenant_id: "tenant".to_owned(),
            user_id: "athlete".to_owned(),
            plan_id: "plan".to_owned(),
            week_start: week_start.to_owned(),
            focus: String::new(),
            phase_index: None,
            days: days
                .iter()
                .map(|(date, workout)| {
                    serde_json::from_value::<PlannedDay>(
                        json!({ "date": date, "sport": "run", "workout": workout }),
                    )
                    .unwrap()
                })
                .collect(),
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            author_agent_id: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        }
    }

    fn date(text: &str) -> CivilDate {
        parse_plan_date(text).unwrap()
    }

    #[test]
    fn the_dates_named_are_those_changed_or_every_one_over_no_plan() {
        let live = [week(
            "2026-11-16",
            &[("2026-11-17", "Easy"), ("2026-11-19", "Tempo")],
        )];
        let saved = [week(
            "2026-11-16",
            &[("2026-11-17", "Easy"), ("2026-11-19", "Hills")],
        )];
        assert_eq!(
            dates_changed(&live, &saved),
            [date("2026-11-19")],
            "bug: only the date whose session changed is named"
        );
        assert_eq!(
            dates_changed(&[], &saved),
            [date("2026-11-17"), date("2026-11-19")],
            "bug: over no plan, every date the weeks hold is named"
        );
        let unparsable = [week("2026-11-16", &[("2026-11-7", "Easy")])];
        assert!(
            dates_changed(&[], &unparsable).is_empty(),
            "bug: a stored date that does not parse is never named as another date"
        );
    }
}
