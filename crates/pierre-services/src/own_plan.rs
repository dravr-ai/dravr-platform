// ABOUTME: Every write to an athlete's live plan made for them — their own day writes, a move, an accept, their agent's asked-for save
// ABOUTME: One function writes the weeks and rebases the athlete's own overlay over them, so a proposal never undoes the athlete's edit

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The athlete's live plan and their overlay (carnet#1841)
//!
//! A self-coached athlete's plan lives in two places: the live plan they
//! read and send, and their own overlay ([`CoachDraftTarget::own`]), where
//! their agent writes what it changes unasked, shown « Proposé » until they
//! accept or refuse it date by date.
//!
//! The overlay stores whole weeks, and a date is proposed when the overlay's
//! entries on it differ from the live plan's. So when the live plan changes
//! under a week the overlay holds, the overlay's copy of every other date of
//! that week would still carry the old live entries: the athlete's own edit
//! would read as a proposal, and accepting it would undo the edit. Every
//! live write made for the athlete therefore goes through
//! [`write_live_weeks`], which writes the weeks and then rebases the
//! overlay's copy of each week it wrote ([`rebase_days`]):
//!
//! - a date the overlay did not change takes the new live entries;
//! - a proposed date the live write left alone keeps its proposal;
//! - a proposed date the live write changed drops its proposal: the athlete
//!   wins.
//!
//! A coach's publish writes the live plan in its own transaction and rebases
//! the overlay the same way afterwards ([`rebase_own_overlay`]).
//!
//! A write that changed a date is recorded in the plan's change history as
//! the athlete's own workspace's, naming the dates it changed.

use std::collections::BTreeSet;

use dravr_tronc::dates::CivilDate;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{
    PlanChangeKind, PlanChangeSource, PlanOutlineInput, PlanWeekInput, SaveDraftParams,
    SavePlanBundleParams, SavedPlanBundle,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{training_content, PlanWeek, PlannedDay};
use tracing::info;
use uuid::Uuid;

use crate::coach_plan_draft::{coach_plan_view, CoachDraftTarget};
use crate::plan_change_history::{self, record_plan_change, PlanChange};

/// A write to the athlete's live plan: the bundle the live save path
/// persists, and who made it for the history.
#[derive(Debug)]
pub struct LiveWrite<'a> {
    /// The agent the write records as the author: `none` for the athlete's
    /// own day writes.
    pub author: PlanAuthor<'a>,
    /// The goal fact an outline links to, when one was converged.
    pub goal_fact_id: Option<&'a str>,
    /// A new outline, or `None` to put the weeks on the live season.
    pub outline: Option<PlanOutlineInput<'a>>,
    /// Whether an outline may replace a season another agent laid.
    pub replace_season: bool,
    /// The weeks, each superseding the live week of its `week_start`.
    pub weeks: &'a [PlanWeekInput<'a>],
    /// The stamp of what the write was derived from (carnet#769).
    pub transport_policy: TransportPolicy,
    /// Who made it and what kind of change it was, as the history records it.
    pub source: PlanChangeSource,
    /// The kind of change.
    pub kind: PlanChangeKind,
}

/// Write `write` to the live plan of `athlete` under `tenant`, then rebase
/// their own overlay over the weeks it wrote.
///
/// The change is recorded in the plan's change history as made in the
/// athlete's own workspace, naming the dates whose sessions it changed; a
/// write that changed none and laid no outline records nothing.
///
/// # Errors
///
/// The repository errors of the live save, and of the overlay's read and
/// rebase. A rebase that fails leaves the live write standing.
pub async fn write_live_weeks(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    athlete: Uuid,
    write: LiveWrite<'_>,
) -> AppResult<SavedPlanBundle> {
    let tenant_id = tenant.to_string();
    let user_id = athlete.to_string();
    let starts: BTreeSet<&str> = write.weeks.iter().map(|week| week.week_start).collect();
    let before = live_weeks_at(repos, &tenant_id, &user_id, &starts).await?;
    let laid_outline = write.outline.is_some();
    let saved = repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant_id,
            user_id: &user_id,
            author: write.author,
            goal_fact_id: write.goal_fact_id,
            outline: write.outline,
            replace_season: write.replace_season,
            weeks: write.weeks,
            transport_policy: write.transport_policy,
        })
        .await?;
    rebase_own_overlay(
        repos,
        tenant,
        athlete,
        &before,
        &saved.weeks,
        saved.plan.transport_policy,
    )
    .await?;
    let dates = plan_change_history::dates_changed(&before, &saved.weeks);
    if laid_outline || !dates.is_empty() {
        let change = PlanChange {
            source: write.source,
            kind: write.kind,
            dates: &dates,
        };
        record_plan_change(repos, &CoachDraftTarget::own(tenant, athlete), change).await;
    }
    Ok(saved)
}

/// The athlete's live weeks whose `week_start` is in `starts`.
async fn live_weeks_at(
    repos: &RepositoryRegistry,
    tenant_id: &str,
    user_id: &str,
    starts: &BTreeSet<&str>,
) -> AppResult<Vec<PlanWeek>> {
    let Some(plan) = repos
        .training_plans
        .get_active_plan(tenant_id, user_id)
        .await?
    else {
        return Ok(Vec::new());
    };
    Ok(repos
        .training_plans
        .list_plan_weeks(tenant_id, user_id, &plan.id, false)
        .await?
        .into_iter()
        .filter(|week| starts.contains(week.week_start.as_str()))
        .collect())
}

/// Rebase the athlete's own overlay over a live write that turned `before`
/// into `after`: each week the overlay holds of those written is rewritten
/// by [`rebase_days`], and saved back when that changed it. A week the
/// overlay never wrote shows the live plan through and is left alone.
///
/// `policy` is the live plan's stamp after the write: the rebased week
/// carries its entries.
///
/// # Errors
///
/// The repository errors of the overlay's read and save.
pub async fn rebase_own_overlay(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    athlete: Uuid,
    before: &[PlanWeek],
    after: &[PlanWeek],
    policy: TransportPolicy,
) -> AppResult<()> {
    let target = CoachDraftTarget::own(tenant, athlete);
    let Some(draft) = repos.training_plans.get_draft_plan(target.owner()).await? else {
        return Ok(());
    };
    let drafted = repos
        .training_plans
        .list_draft_weeks(target.owner(), &draft.plan.id)
        .await?;
    for written in after {
        let Some(overlay) = drafted
            .iter()
            .find(|week| week.week_start == written.week_start)
        else {
            continue;
        };
        let old = before
            .iter()
            .find(|week| week.week_start == written.week_start);
        let old_days = old.map_or(&[][..], |week| week.days.as_slice());
        let days = rebase_days(&overlay.days, old_days, &written.days);
        let focus = rebased_field(
            &overlay.focus,
            old.map(|w| w.focus.as_str()),
            &written.focus,
        );
        let reason = rebased_field(
            &overlay.adjustment_reason,
            old.map(|w| w.adjustment_reason.as_str()),
            &written.adjustment_reason,
        );
        if days == overlay.days && focus == overlay.focus && reason == overlay.adjustment_reason {
            continue;
        }
        repos
            .training_plans
            .save_draft_bundle(&SaveDraftParams {
                owner: target.owner(),
                author: PlanAuthor::from_agent(overlay.author_agent_id.as_deref()),
                goal_fact_id: None,
                outline: None,
                weeks: &[PlanWeekInput {
                    week_start: &overlay.week_start,
                    focus,
                    days: &days,
                    adjustment_reason: reason,
                    phase_index: overlay.phase_index,
                }],
                transport_policy: TransportPolicy::strictest_of([
                    policy,
                    draft.plan.transport_policy,
                ]),
            })
            .await?;
    }
    Ok(())
}

/// A week-level field of the overlay after a live write turned `old` into
/// `new`: the new live value where the overlay held the old one, else the
/// overlay's own.
fn rebased_field<'a>(overlay: &'a str, old: Option<&str>, new: &'a str) -> &'a str {
    if old.is_none_or(|old| old == overlay) {
        new
    } else {
        overlay
    }
}

/// The training content of `days`' entries on `date`, in their order.
fn on(days: &[PlannedDay], date: &str) -> Vec<PlannedDay> {
    days.iter()
        .filter(|day| day.date == date)
        .map(training_content)
        .collect()
}

/// The overlay's entries of one week after a live write turned `old` into
/// `new`, date by date: a date the overlay did not change takes `new`'s
/// entries; a date the overlay proposes keeps its proposal while the write
/// left the date alone, and takes `new`'s entries when the write changed it.
/// Entries are compared by their training content, and come back in date
/// order, each date's in its own order.
#[must_use]
pub fn rebase_days(
    overlay: &[PlannedDay],
    old: &[PlannedDay],
    new: &[PlannedDay],
) -> Vec<PlannedDay> {
    let dates: BTreeSet<&str> = overlay
        .iter()
        .chain(old)
        .chain(new)
        .map(|day| day.date.as_str())
        .collect();
    let mut rebased = Vec::new();
    for date in dates {
        let proposed = on(overlay, date) != on(old, date);
        let edited = on(new, date) != on(old, date);
        let source = if proposed && !edited { overlay } else { new };
        rebased.extend(source.iter().filter(|day| day.date == date).cloned());
    }
    rebased
}

/// `days` with every entry on `date` replaced by `entries`, in their order:
/// the date's entries take the place of its first one, or the place the date
/// falls in the week where it holds none.
#[must_use]
pub fn with_entries_on(days: &[PlannedDay], entries: &[PlannedDay], date: &str) -> Vec<PlannedDay> {
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
    kept.extend(entries.iter().cloned());
    kept.extend(tail);
    kept
}

/// What accepting a proposal on the athlete's own plan came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalAccept {
    /// The proposal's dates — the date asked for, and the other date of a
    /// moved session — are on the live plan now.
    Accepted(Vec<CivilDate>),
    /// The date proposes nothing: already accepted or refused, or never
    /// proposed. Nothing was written.
    NotProposed,
}

/// Accept what the athlete's own overlay proposes on `date`, and on the other
/// date of a session it moves (carnet#1841): each date's overlay entries go
/// onto the live plan in place of its live entries, through
/// [`write_live_weeks`], recorded as the athlete's `proposal_accept`.
///
/// Idempotent: a date that proposes nothing is answered as such and writes
/// nothing. A proposed date in a week the live plan does not hold yet is
/// given that week, from the overlay's.
///
/// # Errors
///
/// The errors of the plan view and of [`write_live_weeks`].
pub async fn accept_proposal(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    athlete: Uuid,
    date: CivilDate,
) -> AppResult<ProposalAccept> {
    let view = coach_plan_view(repos, &CoachDraftTarget::own(tenant, athlete)).await?;
    let Some(group) = view
        .proposal_groups()
        .into_iter()
        .find(|group| group.contains(&date))
    else {
        return Ok(ProposalAccept::NotProposed);
    };
    let (Some(live), Some(draft)) = (view.live.as_ref(), view.draft.as_ref()) else {
        return Ok(ProposalAccept::NotProposed);
    };
    // The weeks the proposal touches, by `week_start`, each as it will be
    // written: the live week, else the overlay's.
    let mut weeks: Vec<PlanWeek> = Vec::new();
    let mut author: Option<String> = None;
    for day in &group {
        let Some(proposed) = draft.weeks.iter().find(|week| week.holds(*day)) else {
            continue;
        };
        author = author.or_else(|| proposed.author_agent_id.clone());
        let dated = day.to_string();
        let entries: Vec<PlannedDay> = proposed
            .days
            .iter()
            .filter(|entry| entry.date == dated)
            .map(|entry| PlannedDay {
                coach_original: None,
                coach_added: false,
                ..entry.clone()
            })
            .collect();
        let at = match weeks
            .iter()
            .position(|week| week.week_start == proposed.week_start)
        {
            Some(at) => at,
            None => {
                let base = live
                    .weeks
                    .iter()
                    .find(|week| week.week_start == proposed.week_start)
                    .cloned()
                    .unwrap_or_else(|| PlanWeek {
                        days: Vec::new(),
                        ..proposed.clone()
                    });
                weeks.push(base);
                weeks.len() - 1
            }
        };
        if let Some(week) = weeks.get_mut(at) {
            week.days = with_entries_on(&week.days, &entries, &dated);
        }
    }
    if weeks.is_empty() {
        return Ok(ProposalAccept::NotProposed);
    }
    let inputs: Vec<PlanWeekInput<'_>> = weeks
        .iter()
        .map(|week| PlanWeekInput {
            week_start: &week.week_start,
            focus: &week.focus,
            days: &week.days,
            adjustment_reason: &week.adjustment_reason,
            phase_index: week.phase_index,
        })
        .collect();
    write_live_weeks(
        repos,
        tenant,
        athlete,
        LiveWrite {
            author: PlanAuthor::from_agent(author.as_deref()),
            goal_fact_id: None,
            outline: None,
            replace_season: false,
            weeks: &inputs,
            transport_policy: TransportPolicy::strictest_of([
                live.plan.transport_policy,
                draft.plan.transport_policy,
            ]),
            source: PlanChangeSource::Athlete,
            kind: PlanChangeKind::ProposalAccept,
        },
    )
    .await?;
    info!(
        user_id = %athlete,
        dates = group.len(),
        "athlete accepted a proposal of their agent's"
    );
    Ok(ProposalAccept::Accepted(group))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn day(date: &str, workout: &str) -> PlannedDay {
        serde_json::from_value(json!({ "date": date, "sport": "run", "workout": workout })).unwrap()
    }

    fn workouts(days: &[PlannedDay]) -> Vec<(&str, &str)> {
        days.iter()
            .map(|day| (day.date.as_str(), day.workout.as_str()))
            .collect()
    }

    /// The athlete's live week before their edit: Tuesday tempo, Wednesday
    /// easy, Friday long.
    fn live() -> Vec<PlannedDay> {
        vec![
            day("2026-11-17", "Tempo"),
            day("2026-11-18", "Easy"),
            day("2026-11-20", "Long"),
        ]
    }

    /// The overlay's copy of the week: the agent proposed hills on Wednesday.
    fn overlay() -> Vec<PlannedDay> {
        vec![
            day("2026-11-17", "Tempo"),
            day("2026-11-18", "Hills"),
            day("2026-11-20", "Long"),
        ]
    }

    #[test]
    fn a_date_the_overlay_did_not_change_takes_the_athletes_edit() {
        let mut edited = live();
        edited[2] = day("2026-11-20", "Long 2h");
        let rebased = rebase_days(&overlay(), &live(), &edited);
        assert_eq!(
            workouts(&rebased),
            [
                ("2026-11-17", "Tempo"),
                ("2026-11-18", "Hills"),
                ("2026-11-20", "Long 2h"),
            ],
            "bug: the athlete's edit of Friday reached the overlay, and Wednesday's proposal stayed"
        );
    }

    #[test]
    fn a_proposed_date_the_athlete_edited_drops_its_proposal() {
        let mut edited = live();
        edited[1] = day("2026-11-18", "Easy 30");
        let rebased = rebase_days(&overlay(), &live(), &edited);
        assert_eq!(
            workouts(&rebased),
            [
                ("2026-11-17", "Tempo"),
                ("2026-11-18", "Easy 30"),
                ("2026-11-20", "Long"),
            ],
            "bug: the athlete wins on a date they edited: the proposal is gone"
        );
    }

    #[test]
    fn a_session_moved_onto_an_empty_date_reaches_the_overlay_in_date_order() {
        let moved = vec![
            day("2026-11-18", "Easy"),
            day("2026-11-19", "Tempo"),
            day("2026-11-20", "Long"),
        ];
        let rebased = rebase_days(&overlay(), &live(), &moved);
        assert_eq!(
            workouts(&rebased),
            [
                ("2026-11-18", "Hills"),
                ("2026-11-19", "Tempo"),
                ("2026-11-20", "Long"),
            ],
            "bug: both dates of the move read as live, the untouched proposal stays, in date order"
        );
        assert_eq!(
            rebase_days(&live(), &live(), &moved),
            moved,
            "bug: an overlay with no proposal on the week becomes the new live week"
        );
    }

    #[test]
    fn a_week_field_follows_the_live_write_unless_the_overlay_changed_it() {
        assert_eq!(rebased_field("build", Some("build"), "recover"), "recover");
        assert_eq!(rebased_field("taper", Some("build"), "recover"), "taper");
        assert_eq!(rebased_field("taper", None, "recover"), "recover");
    }
}
