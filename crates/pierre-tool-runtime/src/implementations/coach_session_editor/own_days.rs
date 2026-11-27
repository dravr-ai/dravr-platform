// ABOUTME: The athlete's own day writes on their live plan — a family build, a hand edit, restore, remove, add and move of one session
// ABOUTME: Each goes through write_live_weeks, so their overlay is rebased; the first session added to no plan lays a season with no race

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The athlete edits their own plan (carnet#1841)
//!
//! A self-coached athlete's Plan tab edits their live plan directly: what
//! they ask for applies at once. The edits are the coach editor's own —
//! a session built from a family, written by hand or made a rest entry, put
//! back, removed or added, each naming the session as the client last saw
//! it — plus a move of one session to another date ([`move_own_session`]).
//! Where a coach's edit lands in their draft, the athlete's lands live,
//! through [`write_live_weeks`], which rebases their own overlay so a
//! proposal of their agent's never undoes what they just wrote.
//!
//! Every edit keeps the session as it stood before the athlete's first edit
//! (`coach_original`) and marks a session they added (`coach_added`), so
//! restoring puts the first back and removes the second. The marks are the
//! editor's alone: the agent's read drops them, and a coach's view of the
//! plan does not carry them.
//!
//! A plan starts at the first session added ([`add_own_session`]): with no
//! plan yet, the add lays a season with no goal race and the week holding
//! the date. A date with no week of the plan is given one, on the plan's own
//! week grid. Nothing is added or moved onto a date before the athlete's
//! today.

use std::future::Future;
use std::sync::Arc;

use dravr_tronc::dates::{CivilDate, Elapsed};
use pierre_core::errors::AppResult;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{
    PlanChangeKind, PlanChangeSource, PlanOutlineInput, PlanWeekInput,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, PlannedDay};
use pierre_providers::ai_scope;
use pierre_services::athlete_anchors::stored_athlete_anchors;
use pierre_services::coach_plan_draft::{coach_plan_view, PlanWithWeeks};
use pierre_services::own_plan::{write_live_weeks, LiveWrite};
use pierre_services::provider_notice;
use tracing::info;

use super::day_edits::{
    added, hand_day, hand_edited, next_position, removed, rest_label, restored, AddedSession,
    Edited, HandDay,
};
use super::sessions::{expected_index, placed, stamp_edit, without, SessionAt};
use super::{
    build_day, sport_label, writer_catalogue, DraftDayOutcome, EditorWords, OwnPlan, SessionBuild,
};
use crate::runtime::ToolRuntime;

/// The athlete's live season, or `None` with no plan.
async fn live_season(
    repos: &RepositoryRegistry,
    own: &OwnPlan,
) -> AppResult<Option<PlanWithWeeks>> {
    Ok(coach_plan_view(repos, &own.target()).await?.live)
}

/// The live week holding `date`, or the outcome that leaves nothing written.
fn week_of(season: Option<&PlanWithWeeks>, date: CivilDate) -> Result<PlanWeek, DraftDayOutcome> {
    let season = season.ok_or(DraftDayOutcome::NoPlan)?;
    season
        .weeks
        .iter()
        .find(|week| week.holds(date))
        .cloned()
        .ok_or(DraftDayOutcome::NoPlanWeek)
}

/// The first day of the week a new week holding `date` starts on: on the
/// grid of the plan's weeks when it has any, else the Monday of `date`'s
/// week. `None` when such a week would overlap a week the plan holds.
fn new_week_start(weeks: &[PlanWeek], date: CivilDate) -> Option<CivilDate> {
    let grid = weeks
        .iter()
        .filter_map(|week| parse_plan_date(&week.week_start))
        .min_by_key(|start| (date - *start).num_days().abs());
    let start = grid.map_or_else(
        || monday_of(date),
        |anchor| {
            let offset = (date - anchor).num_days().rem_euclid(7);
            date - Elapsed::days(offset)
        },
    );
    let end = start.checked_add_days(7)?;
    let overlaps = weeks
        .iter()
        .filter_map(|week| parse_plan_date(&week.week_start))
        .any(|held| {
            held.checked_add_days(7)
                .is_some_and(|held_end| held < end && start < held_end)
        });
    (!overlaps).then_some(start)
}

/// Run `write`, the athlete's own write on their plan: tracked so the plan is
/// stamped from what it read (carnet#769), under their AI consents.
pub(super) async fn as_own_write(
    repos: &RepositoryRegistry,
    own: &OwnPlan,
    write: impl Future<Output = AppResult<DraftDayOutcome>> + Send,
) -> AppResult<DraftDayOutcome> {
    let tracked = Box::pin(ai_scope::tracking(ai_scope::Provenance::new(), write));
    provider_notice::under_ai_consent(repos, own.tenant.as_uuid(), own.athlete, tracked).await
}

/// One week a write on the athlete's live plan saves: its days, the rest of
/// the week as it stands.
#[derive(Debug)]
struct WeekWrite {
    week_start: String,
    focus: String,
    adjustment_reason: String,
    phase_index: Option<u32>,
    days: Vec<PlannedDay>,
}

impl WeekWrite {
    /// `week` with `days` in place of its own.
    fn over(week: &PlanWeek, days: Vec<PlannedDay>) -> Self {
        Self {
            week_start: week.week_start.clone(),
            focus: week.focus.clone(),
            adjustment_reason: week.adjustment_reason.clone(),
            phase_index: week.phase_index,
            days,
        }
    }

    /// A week the plan does not hold yet, starting on `start`, holding none.
    fn new_at(start: CivilDate) -> Self {
        Self {
            week_start: start.to_string(),
            focus: String::new(),
            adjustment_reason: String::new(),
            phase_index: None,
            days: Vec::new(),
        }
    }

    fn input(&self) -> PlanWeekInput<'_> {
        PlanWeekInput {
            week_start: &self.week_start,
            focus: &self.focus,
            days: &self.days,
            adjustment_reason: &self.adjustment_reason,
            phase_index: self.phase_index,
        }
    }
}

/// The week a session added or moved onto `date` lands in: the live week
/// holding it, else a new one on the plan's week grid; `NoPlanWeek` when a
/// new one would overlap a week the plan holds.
fn landing_week(weeks: &[PlanWeek], date: CivilDate) -> Result<WeekWrite, DraftDayOutcome> {
    if let Some(week) = weeks.iter().find(|week| week.holds(date)) {
        return Ok(WeekWrite::over(week, week.days.clone()));
    }
    new_week_start(weeks, date)
        .map(WeekWrite::new_at)
        .ok_or(DraftDayOutcome::NoPlanWeek)
}

/// Save `weeks` to the athlete's live plan as their own `kind` of write —
/// laying a season with no goal race first when `lay_season` — through
/// [`write_live_weeks`].
///
/// # Errors
///
/// The errors of [`write_live_weeks`].
async fn save_live(
    repos: &RepositoryRegistry,
    own: &OwnPlan,
    weeks: &[WeekWrite],
    kind: PlanChangeKind,
    lay_season: bool,
) -> AppResult<()> {
    let inputs: Vec<PlanWeekInput<'_>> = weeks.iter().map(WeekWrite::input).collect();
    // The season the athlete's first session starts: no race yet, no
    // phases, nothing said of it; a race or an agent's outline comes later.
    let outline = lay_season.then(|| PlanOutlineInput {
        goal_race: None,
        races: Some(&[]),
        strategy: "",
        flavour: None,
        season_start: None,
        season_end: None,
        phases: &[],
        source_conversation_id: None,
    });
    let saved = write_live_weeks(
        repos,
        own.tenant,
        own.athlete,
        LiveWrite {
            author: PlanAuthor::none(),
            goal_fact_id: None,
            outline,
            replace_season: false,
            weeks: &inputs,
            transport_policy: ai_scope::derived_policy(),
            source: PlanChangeSource::Athlete,
            kind,
        },
    )
    .await?;
    info!(
        user_id = %own.athlete,
        plan_id = %saved.plan.id,
        kind = kind.as_str(),
        weeks = saved.weeks.len(),
        laid_season = lay_season,
        "athlete wrote their own live plan"
    );
    Ok(())
}

/// Save `edited` over `week` as the athlete's `kind` of write, or answer the
/// outcome that leaves nothing written.
async fn save_edited(
    repos: &RepositoryRegistry,
    own: &OwnPlan,
    week: &PlanWeek,
    edited: Edited,
    kind: PlanChangeKind,
) -> AppResult<DraftDayOutcome> {
    match edited {
        Ok(days) => {
            save_live(repos, own, &[WeekWrite::over(week, days)], kind, false).await?;
            Ok(DraftDayOutcome::Saved)
        }
        Err(outcome) => Ok(outcome),
    }
}

/// Save `build` over the session `at` names on `date` of the athlete's live
/// plan. A position that no longer holds that session writes nothing.
///
/// # Errors
///
/// The refusals of [`build_day`] and [`placed`], and the repository errors
/// of the reads and the write.
pub async fn save_own_day(
    runtime: &Arc<dyn ToolRuntime>,
    own: &OwnPlan,
    date: CivilDate,
    at: &SessionAt,
    build: &SessionBuild,
) -> AppResult<DraftDayOutcome> {
    let repos = runtime.repos();
    let write = async {
        let season = live_season(repos, own).await?;
        let week = match week_of(season.as_ref(), date) {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let index = match expected_index(&week.days, &date.to_string(), at) {
            Ok(index) => index,
            Err(outcome) => return Ok(outcome),
        };
        let catalogue = writer_catalogue(runtime, &own.writer()).await?;
        let anchors = stored_athlete_anchors(repos, own.tenant, own.athlete).await?;
        let replaced = week.days.get(index);
        let label = sport_label(replaced, build.sport);
        let mut made = build_day(
            &catalogue,
            build,
            date,
            at.position,
            label,
            anchors.as_ref(),
        )?;
        stamp_edit(&mut made.day, replaced);
        let days = placed(&week.days, Some(index), made.day)?;
        save_edited(repos, own, &week, Ok(days), PlanChangeKind::DayEdit).await
    };
    as_own_write(repos, own, write).await
}

/// Save the session the athlete wrote by hand over the session `at` names on
/// `date` of their live plan, a rest entry's session named in their words.
///
/// # Errors
///
/// The refusals of a hand-written day ([`hand_day`]), and the repository
/// errors of the reads and the write.
pub async fn save_own_hand_day(
    runtime: &Arc<dyn ToolRuntime>,
    own: &OwnPlan,
    date: CivilDate,
    at: &SessionAt,
    request: &HandDay,
    words: EditorWords<'_>,
) -> AppResult<DraftDayOutcome> {
    let rest_label = rest_label(words)?;
    let repos = runtime.repos();
    let write = async {
        let season = live_season(repos, own).await?;
        let week = match week_of(season.as_ref(), date) {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let edited = hand_edited(&week.days, date, at, request, &rest_label)?;
        save_edited(repos, own, &week, edited, PlanChangeKind::DayEdit).await
    };
    as_own_write(repos, own, write).await
}

/// Put the session `at` names on `date` of the athlete's live plan back: a
/// session they added is removed, one they edited is put back as it stood
/// before their first edit.
///
/// # Errors
///
/// The refusal of a second rest entry on the date, and the repository errors
/// of the reads and the write.
pub async fn restore_own_day(
    runtime: &Arc<dyn ToolRuntime>,
    own: &OwnPlan,
    date: CivilDate,
    at: &SessionAt,
) -> AppResult<DraftDayOutcome> {
    let repos = runtime.repos();
    let write = async {
        let season = live_season(repos, own).await?;
        let week = match week_of(season.as_ref(), date) {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let edited = restored(&week.days, &date.to_string(), at)?;
        save_edited(repos, own, &week, edited, PlanChangeKind::DayRestore).await
    };
    as_own_write(repos, own, write).await
}

/// Remove the session `at` names from `date` of the athlete's live plan;
/// never the date's last entry, which is made a rest day instead.
///
/// # Errors
///
/// The repository errors of the reads and the write.
pub async fn remove_own_day(
    runtime: &Arc<dyn ToolRuntime>,
    own: &OwnPlan,
    date: CivilDate,
    at: &SessionAt,
) -> AppResult<DraftDayOutcome> {
    let repos = runtime.repos();
    let write = async {
        let season = live_season(repos, own).await?;
        let week = match week_of(season.as_ref(), date) {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let edited = removed(&week.days, &date.to_string(), at);
        save_edited(repos, own, &week, edited, PlanChangeKind::DayRemove).await
    };
    as_own_write(repos, own, write).await
}

/// Add `session` after the entries on `date` of the athlete's live plan,
/// marked as one they added.
///
/// With no plan yet, the add lays the season — no goal race — and the week
/// holding the date; a date no week of the plan holds is given a week on the
/// plan's grid. Nothing is added before `today`, the athlete's civil date.
///
/// # Errors
///
/// The refusals of [`build_day`] or [`hand_day`], and the repository errors
/// of the reads and the write.
pub async fn add_own_session(
    runtime: &Arc<dyn ToolRuntime>,
    own: &OwnPlan,
    date: CivilDate,
    session: &AddedSession,
    words: EditorWords<'_>,
    today: CivilDate,
) -> AppResult<DraftDayOutcome> {
    if date < today {
        return Ok(DraftDayOutcome::PastDate);
    }
    let repos = runtime.repos();
    let write = async {
        let season = live_season(repos, own).await?;
        let (week, lay_season) = match season.as_ref() {
            None => (WeekWrite::new_at(monday_of(date)), true),
            Some(season) => match landing_week(&season.weeks, date) {
                Ok(week) => (week, false),
                Err(outcome) => return Ok(outcome),
            },
        };
        let position = match next_position(&week.days, &date.to_string()) {
            Ok(position) => position,
            Err(outcome) => return Ok(outcome),
        };
        let day = match session {
            AddedSession::Build(build) => {
                let catalogue = writer_catalogue(runtime, &own.writer()).await?;
                let anchors = stored_athlete_anchors(repos, own.tenant, own.athlete).await?;
                let label = build.sport.as_str().to_owned();
                build_day(&catalogue, build, date, position, label, anchors.as_ref())?.day
            }
            AddedSession::Day(request) => {
                hand_day(request, date, position, None, &rest_label(words)?)?
            }
        };
        let days = added(&week.days, day)?;
        let week = WeekWrite { days, ..week };
        save_live(repos, own, &[week], PlanChangeKind::DayAdd, lay_season).await?;
        Ok(DraftDayOutcome::Saved)
    };
    as_own_write(repos, own, write).await
}

/// The Monday of `date`'s week.
fn monday_of(date: CivilDate) -> CivilDate {
    date - Elapsed::days(i64::from(date.weekday().num_days_from_monday()))
}

/// Move the session `at` names on `date` of the athlete's live plan to
/// `to`, after the entries `to` holds, in one write over the one or two
/// weeks it touches.
///
/// The session moves whole; a snapshot of an earlier edit stays behind, as
/// its date is the old one. `SameDate` for a move onto its own date,
/// `PastDate` onto a date before `today`, `DateFull` onto a date holding as
/// many entries as a date holds; nothing is written on any of them.
///
/// # Errors
///
/// The refusal of a second rest entry on `to`, and the repository errors of
/// the reads and the write.
pub async fn move_own_session(
    runtime: &Arc<dyn ToolRuntime>,
    own: &OwnPlan,
    date: CivilDate,
    at: &SessionAt,
    to: CivilDate,
    today: CivilDate,
) -> AppResult<DraftDayOutcome> {
    if to == date {
        return Ok(DraftDayOutcome::SameDate);
    }
    if to < today {
        return Ok(DraftDayOutcome::PastDate);
    }
    let repos = runtime.repos();
    let write = async {
        let season = live_season(repos, own).await?;
        let source = match week_of(season.as_ref(), date) {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let index = match expected_index(&source.days, &date.to_string(), at) {
            Ok(index) => index,
            Err(outcome) => return Ok(outcome),
        };
        let Some(session) = source.days.get(index) else {
            return Ok(DraftDayOutcome::SessionMoved);
        };
        let moved = PlannedDay {
            date: to.to_string(),
            coach_original: None,
            ..session.clone()
        };
        let left = without(&source.days, index);
        let to_dated = to.to_string();
        let weeks = if source.holds(to) {
            if let Err(outcome) = next_position(&left, &to_dated) {
                return Ok(outcome);
            }
            vec![WeekWrite::over(&source, placed(&left, None, moved)?)]
        } else {
            let weeks = season.as_ref().map_or(&[][..], |s| s.weeks.as_slice());
            let landing = match landing_week(weeks, to) {
                Ok(week) => week,
                Err(outcome) => return Ok(outcome),
            };
            if let Err(outcome) = next_position(&landing.days, &to_dated) {
                return Ok(outcome);
            }
            let days = placed(&landing.days, None, moved)?;
            vec![
                WeekWrite::over(&source, left),
                WeekWrite { days, ..landing },
            ]
        };
        save_live(repos, own, &weeks, PlanChangeKind::DayMove, false).await?;
        Ok(DraftDayOutcome::Saved)
    };
    as_own_write(repos, own, write).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_tronc::dates::Timestamp;
    use pierre_memory::training_plans::WeekStatus;

    fn date(text: &str) -> CivilDate {
        parse_plan_date(text).unwrap()
    }

    fn week(start: &str) -> PlanWeek {
        PlanWeek {
            id: format!("week-{start}"),
            tenant_id: "tenant".to_owned(),
            user_id: "athlete".to_owned(),
            plan_id: "plan".to_owned(),
            week_start: start.to_owned(),
            focus: String::new(),
            phase_index: None,
            days: Vec::new(),
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            author_agent_id: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        }
    }

    #[test]
    fn a_new_week_takes_the_plans_week_grid_or_the_monday() {
        // The plan's weeks start on Saturdays.
        let weeks = vec![week("2026-11-14"), week("2026-11-21")];
        assert_eq!(
            new_week_start(&weeks, date("2026-12-01")),
            Some(date("2026-11-28")),
            "bug: a date past the plan's weeks takes a week on the plan's own grid"
        );
        assert_eq!(
            new_week_start(&weeks, date("2026-11-10")),
            Some(date("2026-11-07")),
            "bug: a date before the plan's weeks takes a week on the plan's own grid"
        );
        assert_eq!(
            new_week_start(&[], date("2026-11-19")),
            Some(date("2026-11-16")),
            "bug: with no week to follow, a new week starts on the date's Monday"
        );
        // A plan whose weeks keep no single grid: a week on the nearest one's
        // grid would overlap the other, so none is laid.
        let crooked = vec![week("2026-11-14"), week("2026-11-23")];
        assert_eq!(
            new_week_start(&crooked, date("2026-11-22")),
            None,
            "bug: a new week never overlaps a week the plan holds"
        );
    }
}
