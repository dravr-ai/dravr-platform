// ABOUTME: A human coach's draft of a coached athlete's plan — the coach's view of it, where it stands, publish and discard
// ABOUTME: The one service the plan tools and the subject-plan routes both call, and the one DraftState every surface serialises

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coach's plan draft (carnet#1069)
//!
//! A group's human coach builds a coached athlete's plan in a draft the
//! athlete never sees, then publishes it in one step or throws it away. The
//! draft lives beside the athlete's season
//! ([`PlanDraft`](pierre_database::repositories::PlanDraft)) and is an
//! overlay on it: the coach reads the draft's outline over the live weeks,
//! with the weeks the coach drafted in their place ([`coach_plan_view`]).
//!
//! Every path that reads, publishes or discards a draft comes through here —
//! `get_training_plan` and `publish_training_plan` on the coach path, the
//! subject-plan routes behind the Publish and Discard buttons, and the chat
//! block of a coach's athlete thread — so they cannot disagree about what a
//! draft is or whether it can be published. Each caller runs the coaching
//! gates first ([`coached_athlete_access`](crate::coached_athlete)); nothing
//! here re-checks them.
//!
//! [`DraftState`] is the one serialisation of where a draft stands. A client
//! shows Publish and Discard on a card only when the card's `draft_id`
//! matches the subject-plan read's and its `updated_at` is not older, so both
//! must come from [`DraftState::of`].
//!
//! Publishing ([`publish_coach_draft`]) converges the athlete's goal fact on
//! a re-laid outline — the athlete's goal changes only when the plan they see
//! does — replays the draft in one transaction, retires the goal facts it
//! replaced, records `training_plan.published` and tells the athlete
//! ([`tell_plan_published`]). The rails a live plan
//! write runs (ramp, coverage, compliance, readiness) need the tool runtime,
//! so the tool runtime's publish entry point runs them on what this returns.

use std::collections::BTreeMap;
use std::sync::Arc;

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    DraftConflicts, DraftOwner, DraftPreview, DraftPublish, PlanChangeKind, PlanChangeSource,
    PublishDraftParams,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{self, PlanWeek, PlannedDay, TrainingPlan};
use pierre_notifications::NotificationService;
use pierre_providers::ai_scope;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::coach_plan_notice::tell_plan_published;
use crate::own_plan::rebase_own_overlay;
use crate::plan_change_history::{self, record_plan_change, PlanChange};
use crate::plan_goal_fact::{converge_goal_fact, retire_superseded_goal_facts};

/// Why publishing a drafted week would overwrite the athlete's own work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DraftConflictReason {
    /// The athlete's agent saved the week after the coach drafted over it,
    /// or saved it for the first time.
    AthleteChangedWeek,
}

/// One week the coach drafted whose live version the athlete changed since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DraftConflict {
    /// The week's first day, `YYYY-MM-DD`.
    pub week_start: String,
    /// Why publishing it would overwrite the athlete's work.
    pub reason: DraftConflictReason,
}

/// Where the coach's open draft stands: which draft it is, when the coach
/// last saved into it, and what publishing it now would overwrite.
///
/// Built only by [`DraftState::of`]. It deserializes too, so a plan card
/// stored on a reply carries the state it was built with back to the client
/// unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DraftState {
    /// The draft's id: the same through every save until it is published or
    /// discarded.
    pub draft_id: String,
    /// When the coach last saved into the draft.
    pub updated_at: Timestamp,
    /// The drafted weeks the athlete changed since the coach drafted them.
    /// Publishing refuses them unless the coach overwrites them; a week the
    /// coach never drafted is never here.
    pub conflicts: Vec<DraftConflict>,
    /// The athlete's season was re-laid (or laid, or removed) after the draft
    /// began, so the draft's weeks may no longer fit it. Publishing refuses
    /// it unless the coach overwrites it.
    pub season_relaid: bool,
}

impl DraftState {
    /// The one serialisation of a draft's standing, from the repository's
    /// read of it.
    #[must_use]
    pub fn of(preview: &DraftPreview) -> Self {
        Self {
            draft_id: preview.draft.plan.id.clone(),
            updated_at: preview.draft.plan.updated_at,
            conflicts: week_conflicts(&preview.conflicts),
            season_relaid: preview.conflicts.season_relaid,
        }
    }

    /// Whether publishing now would overwrite nothing of the athlete's.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.conflicts.is_empty() && !self.season_relaid
    }
}

/// The conflicting weeks, each with its reason.
fn week_conflicts(conflicts: &DraftConflicts) -> Vec<DraftConflict> {
    conflicts
        .weeks
        .iter()
        .map(|week_start| DraftConflict {
            week_start: week_start.clone(),
            reason: DraftConflictReason::AthleteChangedWeek,
        })
        .collect()
}

/// Whose draft: the athlete — under their own tenant, where the coaching
/// gates found them — and the coach who owns it.
///
/// The athlete's own overlay is a draft they own themselves
/// ([`Self::own`], carnet#1841): the changes their agent made to their plan
/// unasked, which they accept or refuse date by date. It is a separate row
/// from any human coach's draft of the same plan, since one open draft is
/// kept per owner.
#[derive(Debug, Clone)]
pub struct CoachDraftTarget {
    tenant: TenantId,
    athlete: Uuid,
    coach: Uuid,
    tenant_id: String,
    athlete_id: String,
    coach_id: String,
}

impl CoachDraftTarget {
    /// The draft `coach` holds of `athlete`'s plan under `tenant`. The caller
    /// has run the coaching gates for this pair.
    #[must_use]
    pub fn new(tenant: TenantId, athlete: Uuid, coach: Uuid) -> Self {
        Self {
            tenant,
            athlete,
            coach,
            tenant_id: tenant.to_string(),
            athlete_id: athlete.to_string(),
            coach_id: coach.to_string(),
        }
    }

    /// The athlete's own overlay of their plan under `tenant`: the draft
    /// `athlete` owns, which their agent writes what it proposes unasked into
    /// (carnet#1841). The caller is the athlete, so no coaching gate applies.
    #[must_use]
    pub fn own(tenant: TenantId, athlete: Uuid) -> Self {
        Self::new(tenant, athlete, athlete)
    }

    /// Whether this is the athlete's own overlay rather than a coach's draft.
    #[must_use]
    pub fn is_own(&self) -> bool {
        self.coach == self.athlete
    }

    /// The athlete's tenant.
    #[must_use]
    pub const fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// The athlete.
    #[must_use]
    pub const fn athlete(&self) -> Uuid {
        self.athlete
    }

    /// The coach.
    #[must_use]
    pub const fn coach(&self) -> Uuid {
        self.coach
    }

    /// The repository's key for this draft.
    #[must_use]
    pub fn owner(&self) -> DraftOwner<'_> {
        DraftOwner {
            tenant_id: &self.tenant_id,
            user_id: &self.athlete_id,
            coach_user_id: &self.coach_id,
        }
    }
}

/// A plan with its active weeks in calendar order.
#[derive(Debug, Clone)]
pub struct PlanWithWeeks {
    /// The outline.
    pub plan: TrainingPlan,
    /// Its weeks.
    pub weeks: Vec<PlanWeek>,
}

/// The coach's draft as the coach reads it.
#[derive(Debug, Clone)]
pub struct DraftView {
    /// The draft's outline (status `draft`): the one the coach re-laid, or a
    /// copy of the live one.
    pub plan: TrainingPlan,
    /// The live weeks, with each week the coach drafted in its place, in
    /// calendar order.
    pub weeks: Vec<PlanWeek>,
    /// The `week_start` of each week the coach drafted.
    pub drafted_weeks: Vec<String>,
    /// Where the draft stands.
    pub state: DraftState,
}

/// What the coach sees of the athlete's plan: the live season as the athlete
/// has it, and the coach's draft over it.
#[derive(Debug, Clone)]
pub struct CoachPlanView {
    /// The athlete's live season, or `None` when they have none.
    pub live: Option<PlanWithWeeks>,
    /// The coach's open draft, or `None` when the coach holds none.
    pub draft: Option<DraftView>,
}

impl CoachPlanView {
    /// The dates whose sessions the draft changes over the live plan.
    ///
    /// Sorted, and empty with no draft. Each week of the draft is compared
    /// with the live week of the same `week_start`, by the rule the publish's
    /// room post and the plan's change history name changed days with
    /// ([`plan_change_history::dates_changed`]), so the workspace proposes
    /// exactly the dates a publish announces. The draft's weeks are the live
    /// ones with the drafted ones in their place, so a week the coach never
    /// drafted compares equal and proposes nothing; with no live plan, every
    /// date the drafted weeks hold is proposed.
    #[must_use]
    pub fn proposed_dates(&self) -> Vec<CivilDate> {
        let Some(draft) = &self.draft else {
            return Vec::new();
        };
        let live = self
            .live
            .as_ref()
            .map_or(&[][..], |season| season.weeks.as_slice());
        plan_change_history::dates_changed(live, &draft.weeks)
    }

    /// The proposed dates grouped as they are accepted or refused: a session
    /// the draft moves from one date to another is one proposal over both
    /// dates, every other proposed date its own (carnet#1841).
    ///
    /// Each proposed date is in exactly one group. A pair is `[from, to]`:
    /// date `from` lost exactly one session and gained none, date `to`
    /// gained exactly one and lost none, the two sessions are the same
    /// training apart from their date, and neither could pair with any other
    /// date. Every ambiguous case stays unpaired. Groups follow the first
    /// date of each, in date order.
    #[must_use]
    pub fn proposal_groups(&self) -> Vec<Vec<CivilDate>> {
        let proposed = self.proposed_dates();
        let Some(draft) = &self.draft else {
            return Vec::new();
        };
        let live: &[PlanWeek] = self
            .live
            .as_ref()
            .map_or(&[][..], |season| season.weeks.as_slice());
        let changes: Vec<DateChange> = proposed
            .iter()
            .map(|date| DateChange::of(*date, live, &draft.weeks))
            .collect();
        group_moves(&changes)
    }
}

/// What the draft changed on one proposed date: the sessions it took off the
/// live plan's entries there and the ones it put on, each as training
/// content with no date.
#[derive(Debug, Clone)]
struct DateChange {
    date: CivilDate,
    lost: Vec<PlannedDay>,
    gained: Vec<PlannedDay>,
}

impl DateChange {
    fn of(date: CivilDate, live: &[PlanWeek], draft: &[PlanWeek]) -> Self {
        let dated = date.to_string();
        let on = |weeks: &[PlanWeek]| -> Vec<PlannedDay> {
            weeks
                .iter()
                .flat_map(|week| &week.days)
                .filter(|day| day.date == dated)
                .map(undated)
                .collect()
        };
        let before = on(live);
        let after = on(draft);
        Self {
            date,
            lost: multiset_minus(&before, &after),
            gained: multiset_minus(&after, &before),
        }
    }

    /// The one session this date lost while gaining none.
    fn lost_one(&self) -> Option<&PlannedDay> {
        match (self.lost.as_slice(), self.gained.is_empty()) {
            ([only], true) => Some(only),
            _ => None,
        }
    }

    /// The one session this date gained while losing none.
    fn gained_one(&self) -> Option<&PlannedDay> {
        match (self.gained.as_slice(), self.lost.is_empty()) {
            ([only], true) => Some(only),
            _ => None,
        }
    }
}

/// `day`'s training content with its date blanked, so a session moved to
/// another date compares equal to itself.
fn undated(day: &PlannedDay) -> PlannedDay {
    PlannedDay {
        date: String::new(),
        ..training_plans::training_content(day)
    }
}

/// The entries of `from` left once each entry of `take` is matched away once.
fn multiset_minus(from: &[PlannedDay], take: &[PlannedDay]) -> Vec<PlannedDay> {
    let mut left: Vec<PlannedDay> = from.to_vec();
    for day in take {
        if let Some(at) = left.iter().position(|held| held == day) {
            left.remove(at);
        }
    }
    left
}

/// `changes` grouped: each unique move as `[from, to]`, every other date
/// alone, ordered by each group's first date.
fn group_moves(changes: &[DateChange]) -> Vec<Vec<CivilDate>> {
    let partners = |date: CivilDate, session: &PlannedDay, losing: bool| -> Vec<CivilDate> {
        changes
            .iter()
            .filter(|other| other.date != date)
            .filter(|other| {
                let theirs = if losing {
                    other.gained_one()
                } else {
                    other.lost_one()
                };
                theirs == Some(session)
            })
            .map(|other| other.date)
            .collect()
    };
    let mut pairs: Vec<(CivilDate, CivilDate)> = Vec::new();
    for change in changes {
        let Some(session) = change.lost_one() else {
            continue;
        };
        let [to] = partners(change.date, session, true)[..] else {
            continue;
        };
        let Some(gained) = changes
            .iter()
            .find(|other| other.date == to)
            .and_then(DateChange::gained_one)
        else {
            continue;
        };
        if partners(to, gained, false) == [change.date] {
            pairs.push((change.date, to));
        }
    }
    let paired = |date: CivilDate| pairs.iter().any(|(from, to)| *from == date || *to == date);
    let mut groups: Vec<Vec<CivilDate>> = pairs
        .iter()
        .map(|(from, to)| vec![*from, *to])
        .chain(
            changes
                .iter()
                .filter(|change| !paired(change.date))
                .map(|change| vec![change.date]),
        )
        .collect();
    groups.sort_by_key(|group| group.iter().min().copied());
    groups
}

/// The athlete's live season and the coach's draft over it.
///
/// # Errors
///
/// Returns the repository error from any read, and
/// [`ErrorCode::UnavailableOverTransport`](pierre_core::errors::ErrorCode)
/// when either plan was derived from first-party-only data and the caller is
/// external (carnet#769) — never "no plan", which would invite the caller to
/// lay one over it.
pub async fn coach_plan_view(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
) -> AppResult<CoachPlanView> {
    let mut live = live_season(repos, target).await?;
    // An athlete's own edits mark the days they changed on their live plan
    // (carnet#1841); a coach reads the plan without them, and their own
    // editor marks only their own edits.
    if !target.is_own() {
        if let Some(season) = live.as_mut() {
            for week in &mut season.weeks {
                week.drop_coach_marks();
            }
        }
    }
    let draft = draft_over(
        repos,
        target,
        live.as_ref().map_or(&[][..], |l| l.weeks.as_slice()),
    )
    .await?;
    Ok(CoachPlanView { live, draft })
}

/// The coach's draft over `live_weeks`, or `None` when the coach holds none.
///
/// `live_weeks` are the athlete's live weeks as the caller already read
/// them: this is for a caller that reads the live season its own way, where
/// [`coach_plan_view`] reads both.
///
/// # Errors
///
/// Returns the repository error from the draft reads, and
/// [`ErrorCode::UnavailableOverTransport`](pierre_core::errors::ErrorCode)
/// for a draft an external caller may not read.
pub async fn draft_over(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    live_weeks: &[PlanWeek],
) -> AppResult<Option<DraftView>> {
    let Some(preview) = repos
        .training_plans
        .preview_draft_publish(target.owner())
        .await?
    else {
        return Ok(None);
    };
    admit(&preview.draft.plan)?;
    let drafted = repos
        .training_plans
        .list_draft_weeks(target.owner(), &preview.draft.plan.id)
        .await?;
    let state = DraftState::of(&preview);
    Ok(Some(DraftView {
        drafted_weeks: drafted.iter().map(|w| w.week_start.clone()).collect(),
        weeks: overlay(live_weeks, drafted),
        plan: preview.draft.plan,
        state,
    }))
}

/// Where the coach's open draft stands, or `None` when the coach holds none.
///
/// # Errors
///
/// Returns the repository error from the draft or season read.
pub async fn draft_state(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
) -> AppResult<Option<DraftState>> {
    Ok(draft_outline(repos, target).await?.map(|(state, _)| state))
}

/// Where the coach's open draft stands and the draft's outline, or `None`
/// when the coach holds none — one read for a caller that shows both, as
/// the coach's roster does for its season view.
///
/// The outline is `None` when it was derived from first-party-only data and
/// the caller is external (carnet#769); its standing is still reported.
///
/// # Errors
///
/// Returns the repository error from the draft or season read.
pub async fn draft_outline(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
) -> AppResult<Option<(DraftState, Option<TrainingPlan>)>> {
    Ok(repos
        .training_plans
        .preview_draft_publish(target.owner())
        .await?
        .map(|preview| {
            let state = DraftState::of(&preview);
            let plan = preview.draft.plan;
            (state, admit(&plan).is_ok().then_some(plan))
        }))
}

/// A draft that is now the athlete's live plan.
#[derive(Debug, Clone)]
pub struct PublishedCoachDraft {
    /// The draft that was published.
    pub draft_id: String,
    /// The athlete's live season after the publish.
    pub plan: TrainingPlan,
    /// The live weeks the publish wrote, one per drafted week.
    pub weeks: Vec<PlanWeek>,
    /// The season the publish superseded, when it re-laid one.
    pub superseded_plan_id: Option<String>,
    /// Whether the publish laid an outline: the season was re-laid.
    pub outline_published: bool,
    /// Whether the athlete was sent a notice of the publish
    /// (`coach_plan_published`): `false` when the deployment has no
    /// notification service or the coach's account could not be read.
    pub athlete_notified: bool,
    /// The athlete's live season as it stood just before the publish, or
    /// `None` when they had none — what the room post compares the published
    /// weeks with to name the days that changed (carnet#1249). Read for that
    /// comparison only, never served.
    pub previous: Option<PlanWithWeeks>,
}

/// What a publish came to.
#[derive(Debug, Clone)]
pub enum CoachDraftPublish {
    /// The draft is the athlete's live plan.
    Published(Box<PublishedCoachDraft>),
    /// The athlete's plan moved under the draft and the coach did not ask to
    /// overwrite it. No plan was written ([`publish_coach_draft`] names the
    /// goal fact a race can leave); the state names what collided.
    Conflict(DraftState),
    /// The coach holds no open draft of this athlete's plan.
    NoDraft,
}

/// Publish the coach's draft as the athlete's live plan.
///
/// Refuses, writing nothing, when the athlete's plan moved under the draft —
/// a week both changed, or a re-laid season — unless `overwrite`. Otherwise a
/// re-laid outline converges the athlete's goal fact on its race first (the
/// published plan links to it), the draft is replayed onto the season in one
/// transaction, the goal facts it replaced are retired, the publish is
/// recorded (`training_plan.published`) and the athlete is told through
/// `notifications`. Weeks the coach never drafted keep the athlete's version.
/// A refused publish writes no plan and tells no one. One refused only by the
/// transaction's own check — the athlete's plan moved, or the draft was
/// discarded, after the read here passed — leaves behind the goal fact a
/// re-laid outline converged, which the athlete's next live outline write
/// reuses or retires.
///
/// A publish that landed is recorded in the plan's change history as made by
/// `source`, naming the dates it changed over the athlete's live plan
/// (carnet#1815).
///
/// # Errors
///
/// Returns the repository error from the goal-fact or plan writes, and
/// [`ErrorCode::UnavailableOverTransport`](pierre_core::errors::ErrorCode)
/// for a draft an external caller may not read.
pub async fn publish_coach_draft(
    repos: &RepositoryRegistry,
    notifications: Option<&Arc<NotificationService>>,
    target: &CoachDraftTarget,
    overwrite: bool,
    source: PlanChangeSource,
) -> AppResult<CoachDraftPublish> {
    let Some(preview) = repos
        .training_plans
        .preview_draft_publish(target.owner())
        .await?
    else {
        return Ok(CoachDraftPublish::NoDraft);
    };
    admit(&preview.draft.plan)?;
    // Refused before anything is written — the goal fact included. The
    // publish checks again inside its transaction.
    if !preview.conflicts.is_empty() && !overwrite {
        return Ok(CoachDraftPublish::Conflict(DraftState::of(&preview)));
    }

    // What the athlete's plan said before the publish, so the room can be
    // told which days changed (carnet#1249).
    let previous = season_before_publish(repos, target).await?;

    // A re-laid outline is the athlete's new goal: the plan row stores the
    // fact it links to, so the fact is written first, and the facts it
    // replaces are erased only once the plan is stored.
    let draft = &preview.draft;
    // A season with no race yet has no goal to converge.
    let converged = match (
        draft.outline_drafted,
        &draft.plan.goal_fact_id,
        &draft.plan.goal_race,
    ) {
        (true, None, Some(race)) => {
            Some(converge_goal_fact(repos, target.tenant(), &target.athlete_id, race).await?)
        }
        _ => None,
    };

    let published = match repos
        .training_plans
        .publish_draft(&PublishDraftParams {
            owner: target.owner(),
            overwrite,
            goal_fact_id: converged.as_ref().map(|c| c.fact_id.as_str()),
        })
        .await?
    {
        DraftPublish::Published(published) => published,
        // The athlete's plan moved between the read above and the publish's
        // own check: no plan was written, though a goal fact converged above
        // stays.
        DraftPublish::Conflicted(raced) => {
            return Ok(CoachDraftPublish::Conflict(DraftState::of(&raced)))
        }
        DraftPublish::NoDraft => return Ok(CoachDraftPublish::NoDraft),
    };

    if let Some(converged) = converged {
        retire_superseded_goal_facts(
            repos,
            target.tenant(),
            &target.athlete_id,
            &converged.superseded,
        )
        .await;
    }

    info!(
        target: "notify",
        event = "training_plan.published",
        tenant_id = %target.tenant(),
        user_id = %target.athlete(),
        plan_id = %published.plan.id,
        coach_user_id = %target.coach(),
        draft_id = %published.draft_id,
        outline_published = published.outline_published,
        weeks_published = published.weeks.len(),
        overwrite = overwrite,
        "a human coach published a coached athlete's plan draft"
    );
    let athlete_notified = tell_plan_published(repos, notifications, target, &published.plan).await;
    let previous_weeks = previous
        .as_ref()
        .map_or(&[][..], |season| season.weeks.as_slice());
    let dates = plan_change_history::dates_changed(previous_weeks, &published.weeks);
    let change = PlanChange {
        source,
        kind: PlanChangeKind::Publish,
        dates: &dates,
    };
    record_plan_change(repos, target, change).await;
    // The athlete's own overlay is rebased over what the publish wrote, as
    // every live write rebases it (carnet#1841). The publish has landed and
    // the athlete was told, so a rebase that fails is logged, not answered.
    if let Err(error) = rebase_own_overlay(
        repos,
        target.tenant(),
        target.athlete(),
        previous_weeks,
        &published.weeks,
        published.plan.transport_policy,
    )
    .await
    {
        warn!(
            error = %error,
            athlete_user_id = %target.athlete(),
            "the athlete's own overlay was not rebased over a coach's publish"
        );
    }

    Ok(CoachDraftPublish::Published(Box::new(
        PublishedCoachDraft {
            draft_id: published.draft_id,
            plan: published.plan,
            weeks: published.weeks,
            superseded_plan_id: published.superseded_plan_id,
            outline_published: published.outline_published,
            athlete_notified,
            previous,
        },
    )))
}

/// Throw the coach's draft away, returning its id, or `None` when the coach
/// holds none. The athlete's plan is untouched and nothing reaches them.
///
/// A discard that threw a draft away is recorded in the plan's change history
/// as made by `source`, naming no date: the whole draft goes (carnet#1815).
///
/// # Errors
///
/// Returns the repository error from the write.
pub async fn discard_coach_draft(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    source: PlanChangeSource,
) -> AppResult<Option<String>> {
    let discarded = repos.training_plans.discard_draft(target.owner()).await?;
    if let Some(draft_id) = discarded.as_deref() {
        info!(
            athlete_user_id = %target.athlete(),
            coach_user_id = %target.coach(),
            draft_id = %draft_id,
            "a human coach discarded their draft of a coached athlete's plan"
        );
        let change = PlanChange {
            source,
            kind: PlanChangeKind::Discard,
            dates: &[],
        };
        record_plan_change(repos, target, change).await;
    }
    Ok(discarded)
}

/// The athlete's live season with its active weeks, refused to an external
/// caller when it is first-party-only.
async fn live_season(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
) -> AppResult<Option<PlanWithWeeks>> {
    let season = season_before_publish(repos, target).await?;
    if let Some(season) = &season {
        admit(&season.plan)?;
    }
    Ok(season)
}

/// The athlete's live season with its active weeks, whatever the caller's
/// transport.
///
/// [`live_season`] serves it and so refuses a first-party-only season to an
/// external caller. A publish reads it here too, before it replaces part of
/// it: nothing read for that is served to the caller, and the room post
/// built from it carries the season's stamp.
async fn season_before_publish(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
) -> AppResult<Option<PlanWithWeeks>> {
    let Some(plan) = repos
        .training_plans
        .get_active_plan(&target.tenant_id, &target.athlete_id)
        .await?
    else {
        return Ok(None);
    };
    let weeks = repos
        .training_plans
        .list_plan_weeks(&target.tenant_id, &target.athlete_id, &plan.id, false)
        .await?;
    Ok(Some(PlanWithWeeks { plan, weeks }))
}

/// Refuse a plan derived from first-party-only data to an external caller,
/// as every plan read does (carnet#769).
fn admit(plan: &TrainingPlan) -> AppResult<()> {
    if ai_scope::admit_derived(plan.transport_policy) {
        Ok(())
    } else {
        Err(AppError::unavailable_over_transport(
            "this athlete's training plan is not available over this interface",
        ))
    }
}

/// The live weeks with each drafted week in place of the live one of its
/// `week_start`, in calendar order.
fn overlay(live: &[PlanWeek], drafted: Vec<PlanWeek>) -> Vec<PlanWeek> {
    let mut by_start: BTreeMap<String, PlanWeek> = live
        .iter()
        .map(|week| (week.week_start.clone(), week.clone()))
        .collect();
    for week in drafted {
        by_start.insert(week.week_start.clone(), week);
    }
    by_start.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_memory::training_plans::{self, WeekStatus};

    fn week(plan: &str, week_start: &str, focus: &str) -> PlanWeek {
        PlanWeek {
            id: format!("{plan}-{week_start}"),
            tenant_id: "tenant".to_owned(),
            user_id: "athlete".to_owned(),
            plan_id: plan.to_owned(),
            week_start: week_start.to_owned(),
            focus: focus.to_owned(),
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

    /// The coach reads the live weeks with their drafted ones in place, in
    /// calendar order, whatever order either list came in.
    #[test]
    fn the_overlay_replaces_live_weeks_by_week_start_and_keeps_the_rest() {
        let live = vec![
            week("live", "2026-07-20", "live two"),
            week("live", "2026-07-13", "live one"),
        ];
        let drafted = vec![
            week("draft", "2026-07-27", "draft three"),
            week("draft", "2026-07-13", "draft one"),
        ];
        let merged = overlay(&live, drafted);
        let merged: Vec<(&str, &str)> = merged
            .iter()
            .map(|w| (w.week_start.as_str(), w.focus.as_str()))
            .collect();
        assert_eq!(
            merged,
            vec![
                ("2026-07-13", "draft one"),
                ("2026-07-20", "live two"),
                ("2026-07-27", "draft three"),
            ]
        );
    }

    /// The wire shape the clients parse: each conflict names its week and
    /// the one reason, and the timestamp is RFC 3339.
    #[test]
    fn a_draft_state_serialises_in_the_shape_the_clients_parse() {
        let state = DraftState {
            draft_id: "draft-1".to_owned(),
            updated_at: "2026-10-16T09:30:00Z".parse().unwrap(),
            conflicts: week_conflicts(&DraftConflicts {
                season_relaid: false,
                weeks: vec!["2026-07-13".to_owned()],
            }),
            season_relaid: true,
        };
        assert!(!state.is_clear());
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::json!({
                "draft_id": "draft-1",
                "updated_at": "2026-10-16T09:30:00Z",
                "conflicts": [{ "week_start": "2026-07-13", "reason": "athlete_changed_week" }],
                "season_relaid": true,
            })
        );
    }

    fn with_days(mut held: PlanWeek, days: &[(&str, &str)]) -> PlanWeek {
        held.days = days
            .iter()
            .map(|(date, workout)| {
                serde_json::from_value(
                    serde_json::json!({ "date": date, "sport": "run", "workout": workout }),
                )
                .unwrap()
            })
            .collect();
        held
    }

    fn outline(id: &str, status: &str) -> TrainingPlan {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "tenant_id": "tenant",
            "user_id": "athlete",
            "goal_race": { "name": "Race", "date": "2027-05-16", "discipline": "trail", "priority": "A" },
            "races": [],
            "strategy": "",
            "phases": [],
            "status": status,
            "created_at": "2026-11-16T00:00:00Z",
            "updated_at": "2026-11-16T00:00:00Z",
        }))
        .unwrap()
    }

    /// The coach's draft over `live`, with `drafted` written in it.
    fn draft_over_weeks(live: &[PlanWeek], drafted: Vec<PlanWeek>) -> DraftView {
        DraftView {
            plan: outline("draft", "draft"),
            drafted_weeks: drafted.iter().map(|w| w.week_start.clone()).collect(),
            weeks: overlay(live, drafted),
            state: DraftState {
                draft_id: "draft".to_owned(),
                updated_at: Timestamp::now(),
                conflicts: Vec::new(),
                season_relaid: false,
            },
        }
    }

    fn civil(text: &str) -> CivilDate {
        training_plans::parse_plan_date(text).unwrap()
    }

    /// The workspace proposes the dates whose sessions the draft changes
    /// over the live plan: none with no draft, none in a week the coach
    /// never drafted, and every drafted date over no live plan.
    #[test]
    fn the_proposed_dates_are_those_the_draft_changes_over_the_live_plan() {
        let live_weeks = vec![
            with_days(
                week("live", "2026-11-16", "build"),
                &[("2026-11-17", "Easy"), ("2026-11-19", "Tempo")],
            ),
            with_days(
                week("live", "2026-11-23", "build"),
                &[("2026-11-24", "Long")],
            ),
        ];
        let rewritten = || {
            vec![with_days(
                week("draft", "2026-11-16", "build"),
                &[("2026-11-17", "Easy"), ("2026-11-19", "Hills")],
            )]
        };
        let mut view = CoachPlanView {
            draft: None,
            live: Some(PlanWithWeeks {
                plan: outline("live", "active"),
                weeks: live_weeks.clone(),
            }),
        };
        assert!(
            view.proposed_dates().is_empty(),
            "bug: with no draft open, no date is proposed"
        );

        view.draft = Some(draft_over_weeks(&live_weeks, rewritten()));
        assert_eq!(
            view.proposed_dates(),
            [civil("2026-11-19")],
            "bug: only the date whose session the draft rewrote is proposed, and nothing of the week it never drafted"
        );

        let laid = CoachPlanView {
            live: None,
            draft: Some(draft_over_weeks(&[], rewritten())),
        };
        assert_eq!(
            laid.proposed_dates(),
            [civil("2026-11-17"), civil("2026-11-19")],
            "bug: over no live plan, every date the draft holds is proposed"
        );
    }

    /// The athlete's view of a draft written over `live_days` as
    /// `drafted_days`, both in the week of 2026-11-16.
    fn own_view(live_days: &[(&str, &str)], drafted_days: &[(&str, &str)]) -> CoachPlanView {
        let live_weeks = vec![with_days(week("live", "2026-11-16", "build"), live_days)];
        let drafted = vec![with_days(
            week("draft", "2026-11-16", "build"),
            drafted_days,
        )];
        CoachPlanView {
            draft: Some(draft_over_weeks(&live_weeks, drafted)),
            live: Some(PlanWithWeeks {
                plan: outline("live", "active"),
                weeks: live_weeks,
            }),
        }
    }

    /// A session the draft moves from one date to another is one proposal
    /// over both, and a date it rewrote in place stays its own.
    #[test]
    fn a_moved_session_pairs_its_two_dates_and_a_rewrite_stays_alone() {
        let view = own_view(
            &[("2026-11-17", "Tempo"), ("2026-11-18", "Easy")],
            &[("2026-11-18", "Hills"), ("2026-11-19", "Tempo")],
        );
        assert_eq!(
            view.proposal_groups(),
            vec![
                vec![civil("2026-11-17"), civil("2026-11-19")],
                vec![civil("2026-11-18")],
            ],
            "bug: the move pairs its two dates, from first, and the rewrite is alone"
        );
    }

    /// A move that could pair two ways is no move: every date of it is its
    /// own proposal, never guessed.
    #[test]
    fn an_ambiguous_move_stays_unpaired() {
        let view = own_view(
            &[("2026-11-17", "Tempo")],
            &[("2026-11-19", "Tempo"), ("2026-11-20", "Tempo")],
        );
        assert_eq!(
            view.proposal_groups(),
            vec![
                vec![civil("2026-11-17")],
                vec![civil("2026-11-19")],
                vec![civil("2026-11-20")],
            ],
            "bug: one lost session two dates gained is ambiguous and pairs with neither"
        );

        let changed = own_view(&[("2026-11-17", "Tempo")], &[("2026-11-19", "Tempo 50")]);
        assert_eq!(
            changed.proposal_groups(),
            vec![vec![civil("2026-11-17")], vec![civil("2026-11-19")]],
            "bug: a session that changed as it moved is no move"
        );
        assert!(
            CoachPlanView {
                live: None,
                draft: None
            }
            .proposal_groups()
            .is_empty(),
            "bug: with no draft nothing is proposed"
        );
    }
}
