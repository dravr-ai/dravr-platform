// ABOUTME: A coach refuses what their draft proposes on one date — every entry on it replaced by the live plan's entries, whole
// ABOUTME: A date that already reads as the live plan writes nothing and opens no draft; the coach's editor marks go with the entries

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A date of the draft put back to the live plan (carnet#1815)
//!
//! In the coach's workspace, a date where the draft differs from the live
//! plan — a session the thread's agent proposed, or one the coach edited —
//! offers « Refuser ». [`revert_draft_date`] puts the live plan's entries
//! back on that date of the draft: every entry the draft holds there is
//! replaced by the live plan's entries on the date, whole — template,
//! envelope, fuelling and the coach's call as the live plan holds them — in
//! their order. A date the live plan holds nothing on is left empty in the
//! draft. The coach's editor marks (the pre-edit snapshot and the added
//! flag) go with the entries they marked.
//!
//! Nothing is written when there is nothing to put back: the coach holds no
//! draft, the date sits in a week the draft never wrote (the live plan shows
//! through there), or the draft's entries on the date already are the live
//! plan's. No draft is opened to revert into.
//!
//! The revert names no session as the client last saw it (no `expect`):
//! it writes no session over another, it lands the date on what the
//! athlete already has, so a draft that moved since the coach looked comes
//! to the same known state, and the answer shows it. The live entries are
//! written as the live plan holds them, not held to today's entry bounds:
//! they are the athlete's plan as it stands.

use std::sync::Arc;

use dravr_tronc::dates::CivilDate;
use pierre_core::errors::AppResult;
use pierre_database::repositories::PlanChangeKind;
use pierre_memory::training_plans::PlannedDay;
use pierre_services::coach_plan_draft::coach_plan_view;
use tracing::info;

use super::day_edits::as_athletes_write;
use super::{holds, save_days_into_week, DayChange, DraftDayOutcome, EditorThread};
use crate::runtime::ToolRuntime;

/// `days` with every entry on `date` replaced by the entries `live` holds on
/// it, whole and in their order, carrying no editor mark; `None` when `days`
/// already holds exactly those entries on the date.
///
/// The live entries take the place of the date's first entry, or, where the
/// date holds none, the place the date falls in the week.
fn reverted(days: &[PlannedDay], live: &[PlannedDay], date: &str) -> Option<Vec<PlannedDay>> {
    let wanted: Vec<PlannedDay> = live
        .iter()
        .filter(|day| day.date == date)
        .map(|day| PlannedDay {
            coach_original: None,
            coach_added: false,
            ..day.clone()
        })
        .collect();
    let held = days.iter().filter(|day| day.date == date);
    if held.eq(wanted.iter()) {
        return None;
    }
    let at = days
        .iter()
        .position(|day| day.date == date)
        .unwrap_or_else(|| days.partition_point(|day| day.date.as_str() < date));
    let mut kept: Vec<PlannedDay> = days
        .iter()
        .filter(|day| day.date != date)
        .cloned()
        .collect();
    let tail = kept.split_off(at.min(kept.len()));
    kept.extend(wanted);
    kept.extend(tail);
    Some(kept)
}

/// Put the live plan's entries back on `date` of the coach's draft.
///
/// Every entry the draft of the athlete's plan holds on the date is replaced
/// by the live plan's, whole; a date the live plan holds nothing on is left
/// empty. The week's other dates stay as they stand.
///
/// `Unchanged`, with nothing written and no draft opened, when the coach
/// holds no draft, the date sits in a week the draft never wrote, or the
/// draft's entries on the date already are the live plan's.
///
/// # Errors
///
/// `UnavailableOverTransport` for a plan an external caller may not read,
/// and the repository errors of the reads and the save.
pub async fn revert_draft_date(
    runtime: &Arc<dyn ToolRuntime>,
    thread: &EditorThread,
    date: CivilDate,
) -> AppResult<DraftDayOutcome> {
    let repos = runtime.repos();
    let target = &thread.target;
    let write = async {
        super::day_edits::note_coach_read(thread);
        let view = coach_plan_view(repos, target).await?;
        let Some(draft) = view.draft else {
            return Ok(DraftDayOutcome::Unchanged);
        };
        let drafted = draft
            .weeks
            .into_iter()
            .find(|week| holds(week, date) && draft.drafted_weeks.contains(&week.week_start));
        let Some(week) = drafted else {
            return Ok(DraftDayOutcome::Unchanged);
        };
        let dated = date.to_string();
        let live: Vec<PlannedDay> = view.live.map_or_default(|live| {
            live.weeks
                .into_iter()
                .flat_map(|week| week.days)
                .filter(|day| day.date == dated)
                .collect()
        });
        let Some(days) = reverted(&week.days, &live, &dated) else {
            return Ok(DraftDayOutcome::Unchanged);
        };
        let change = DayChange {
            kind: PlanChangeKind::DateRevert,
            date,
        };
        let saved = save_days_into_week(repos, thread, &week, &days, change).await?;
        info!(
            athlete_user_id = %target.athlete(),
            coach_user_id = %target.coach(),
            draft_id = %saved.draft.plan.id,
            date = %dated,
            live_entries = live.len(),
            "coach put a date of their draft back to the live plan"
        );
        Ok(DraftDayOutcome::Saved)
    };
    as_athletes_write(repos, target, write).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_tronc::dates::Timestamp;
    use pierre_memory::training_plans::CoachCall;
    use serde_json::json;
    use uuid::Uuid;

    const DATE: &str = "2026-11-18";

    fn day(date: &str, sport: &str, workout: &str) -> PlannedDay {
        serde_json::from_value(json!({ "date": date, "sport": sport, "workout": workout })).unwrap()
    }

    fn workouts(days: &[PlannedDay]) -> Vec<&str> {
        days.iter().map(|day| day.workout.as_str()).collect()
    }

    #[test]
    fn a_date_takes_the_live_entries_whole_in_place_of_the_drafts() {
        let called = PlannedDay {
            coach_call: Some(CoachCall {
                by: Uuid::from_u128(7),
                at: Timestamp::now(),
            }),
            ..serde_json::from_value(json!({
                "date": DATE, "sport": "ride", "workout": "VO2max — 30/15",
                "template_slug": "vo2max_short", "template_preset": "30_15",
                "template_source": "catalogue", "envelope": { "generated": true },
                "fueling": { "carbs_g_per_h": 60.0, "fluid_ml_per_h": 500.0 }
            }))
            .unwrap()
        };
        let live = vec![
            day("2026-11-17", "run", "Easy"),
            called.clone(),
            day(DATE, "strength", "Core"),
            day("2026-11-19", "run", "Tempo"),
        ];
        let edited = PlannedDay {
            coach_original: Some(Box::new(called.clone())),
            ..day(DATE, "run", "Coach's run")
        };
        let added = PlannedDay {
            coach_added: true,
            ..day(DATE, "swim", "Drills")
        };
        let draft = vec![
            day("2026-11-17", "run", "Agent's easy"),
            edited,
            added,
            day("2026-11-19", "run", "Agent's tempo"),
        ];
        let back = reverted(&draft, &live, DATE).expect("the date differs from the live plan");
        assert_eq!(
            workouts(&back),
            ["Agent's easy", "VO2max — 30/15", "Core", "Agent's tempo"],
            "bug: only the date's entries are replaced, by the live ones in their order, in place"
        );
        assert_eq!(
            back[1], called,
            "bug: a live entry comes back whole: template, envelope, fuelling and call"
        );
        assert!(
            back.iter()
                .all(|entry| entry.coach_original.is_none() && !entry.coach_added),
            "bug: the coach's editor marks went with the entries they marked: {back:?}"
        );
        assert_eq!(
            reverted(&back, &live, DATE),
            None,
            "bug: a date already holding the live entries is written again"
        );
    }

    #[test]
    fn a_date_the_live_plan_leaves_empty_is_emptied_and_one_it_fills_is_filled_in_order() {
        let draft = vec![
            day("2026-11-17", "run", "Easy"),
            day(DATE, "ride", "Agent's ride"),
            day("2026-11-19", "run", "Tempo"),
        ];
        let emptied = reverted(&draft, &[day("2026-11-17", "run", "Easy")], DATE)
            .expect("the draft proposes a session where the live plan has none");
        assert_eq!(
            workouts(&emptied),
            ["Easy", "Tempo"],
            "bug: a date the live plan holds nothing on is left empty in the draft"
        );

        let filled = reverted(&emptied, &[day(DATE, "swim", "Live swim")], DATE)
            .expect("the live plan holds a session the draft does not");
        assert_eq!(
            workouts(&filled),
            ["Easy", "Live swim", "Tempo"],
            "bug: live entries on a date the draft left empty go where the date falls"
        );
        assert_eq!(
            reverted(&[day("2026-11-17", "run", "Easy")], &[], DATE),
            None,
            "bug: a date empty in both is no change"
        );
    }
}
