// ABOUTME: The athlete's first run — which first-run flows are done, the one write that marks a day, and Getting started's rows
// ABOUTME: flow.completed fires from record() alone, on the write that makes a flow done, so each flow reports once per athlete
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! First-run flows (carnet#1099, carnet#1100).
//!
//! The four first-run flows are use-case catalogue entries carrying a
//! `first_run:` order: meet the team, read the last workout, finish the
//! profile, the daily check-in. Each has a done rule met somewhere else — a
//! turn in the personal thread, a grounded answer, a complete dossier, a reply
//! on a day the coach checked in — and every one of those places calls
//! [`record`], the one write. It stores the day the rule was met and answers
//! whether the flow became done with it, which is the only moment
//! `flow.completed` is reported.
//!
//! [`FirstRun`] reads the days back for the ranker, which offers the next open
//! flow first, and for Home's Getting started list ([`getting_started`]).

use chrono::{DateTime, NaiveDate, Utc};
use pierre_contremaitre::use_case_catalogue::UseCaseCatalogue;
use pierre_core::civil_time::clock_date;
use pierre_core::errors::AppResult;
use pierre_core::models::{PostPurpose, TenantId};
use pierre_database::repositories::FirstRunDay;
use pierre_database::RepositoryRegistry;
use tracing::{info, warn};
use uuid::Uuid;

use crate::athlete_clock::athlete_zone;
use crate::athlete_state::{dossier_coverage, AthleteState};

/// Flow 1: the lead agent's first turn introduces the team.
pub const MEET_TEAM: &str = "meet_team";
/// Flow 2: the first answer grounded in the athlete's own data.
pub const LAST_WORKOUT: &str = "last_workout";
/// Flow 3: the pillars walk, done once the dossier covers every topic.
pub const FINISH_PROFILE: &str = "about_me_more";
/// Flow 4: the daily check-in habit.
pub const CHECK_IN: &str = "check_in";

/// Getting started's first row: a provider is connected. Not a catalogue
/// entry — connecting happens in Settings, not in a thread.
pub const CONNECT_ROW: &str = "connect";

/// The distinct local days a check-in must be answered on before the habit
/// counts as formed (activation, part 2).
pub const CHECK_IN_DAYS_TO_DONE: usize = 3;

/// The postback grammar of a Getting started row: `gs:<slot>:<id>`.
pub const GETTING_STARTED_POSTBACK_PREFIX: &str = "gs:";

/// The days on which `flow`'s done rule must be met before it is done.
#[must_use]
pub fn days_to_done(flow: &str) -> usize {
    if flow == CHECK_IN {
        CHECK_IN_DAYS_TO_DONE
    } else {
        1
    }
}

/// The postback a Getting started row shown in slot `position` sends.
#[must_use]
pub fn getting_started_postback(position: usize, id: &str) -> String {
    format!("{GETTING_STARTED_POSTBACK_PREFIX}{position}:{id}")
}

/// Where one flow stands after [`record`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowProgress {
    /// The flow's catalogue id.
    pub flow: String,
    /// Distinct days its done rule was met on.
    pub days: usize,
    /// The flow is done.
    pub done: bool,
    /// This call made it done: the one moment `flow.completed` is reported.
    pub became_done: bool,
}

/// What the athlete has done of their first run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirstRun {
    days: Vec<FirstRunDay>,
}

impl FirstRun {
    /// The first run the stored `days` describe.
    #[must_use]
    pub const fn from_days(days: Vec<FirstRunDay>) -> Self {
        Self { days }
    }

    /// Read `user`'s first run under `tenant`.
    ///
    /// # Errors
    ///
    /// Propagates the repository error.
    pub async fn read(repos: &RepositoryRegistry, tenant: TenantId, user: Uuid) -> AppResult<Self> {
        Ok(Self::from_days(
            repos
                .first_run_progress
                .first_run_days(tenant, user)
                .await?,
        ))
    }

    /// The days `flow`'s done rule was met on, oldest first.
    fn days_of<'a>(&'a self, flow: &'a str) -> impl Iterator<Item = &'a FirstRunDay> {
        self.days.iter().filter(move |day| day.flow_id == flow)
    }

    /// When `flow` became done: the day that completed its count, or `None`
    /// while it is open.
    #[must_use]
    pub fn done_at(&self, flow: &str) -> Option<DateTime<Utc>> {
        self.days_of(flow)
            .nth(days_to_done(flow).saturating_sub(1))
            .map(|day| day.recorded_at)
    }

    /// Whether `flow` is done.
    #[must_use]
    pub fn is_done(&self, flow: &str) -> bool {
        self.done_at(flow).is_some()
    }

    /// Whether `flow`'s done rule was already met on `date`.
    #[must_use]
    pub fn met_on(&self, flow: &str, date: NaiveDate) -> bool {
        self.days_of(flow).any(|day| day.local_date == date)
    }
}

/// Record that `user` met `flow`'s done rule on `local_date`, and report
/// `flow.completed` when that made the flow done.
///
/// `surface` is the chat surface the completing turn ran on
/// (`SurfaceId::as_str`), or the app surface that observed it. A day already
/// recorded changes nothing, so every caller may call this on every
/// qualifying event.
///
/// # Errors
///
/// Propagates the repository error.
pub async fn record(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
    flow: &str,
    local_date: NaiveDate,
    surface: &str,
) -> AppResult<FlowProgress> {
    let written = repos
        .first_run_progress
        .record_first_run_day(tenant, user, flow, local_date, Utc::now())
        .await?;
    let days = FirstRun::read(repos, tenant, user)
        .await?
        .days_of(flow)
        .count();
    let needed = days_to_done(flow);
    let became_done = written && days == needed;
    if became_done {
        info!(
            target: "notify",
            event = "flow.completed",
            user_id = %user,
            tenant_id = %tenant,
            flow = flow,
            surface = surface,
            "first-run flow completed"
        );
    }
    Ok(FlowProgress {
        flow: flow.to_owned(),
        days,
        done: days >= needed,
        became_done,
    })
}

/// [`record`] for a caller that cannot act on its failure: logged, never
/// raised. A day missed only delays a flow's tick to its next qualifying
/// event.
async fn record_best_effort(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
    flow: &str,
    local_date: NaiveDate,
    surface: &str,
) {
    if let Err(e) = record(repos, user, tenant, flow, local_date, surface).await {
        warn!(error = %e, user_id = %user, flow, "first-run day not recorded");
    }
}

/// One athlete turn that may meet a first-run done rule.
#[derive(Debug, Clone, Copy)]
pub struct AthleteTurn<'a> {
    /// The athlete who spoke.
    pub user: Uuid,
    /// The athlete's own tenant, where their first run is kept.
    pub tenant: TenantId,
    /// The turn ran in the athlete's personal thread.
    pub personal_thread: bool,
    /// The reply was grounded in the athlete's own data.
    pub grounded: bool,
    /// The chat surface the turn ran on.
    pub surface: &'a str,
}

/// Mark every first-run flow an athlete turn completes.
///
/// - Meet your team: any turn of theirs in the personal thread — it opens on
///   the lead's welcome, so a reply or a tapped starter answers it.
/// - Read your last workout: the first grounded answer, anywhere.
/// - Finish your profile: the dossier covers every topic (read only while the
///   flow is open).
/// - Daily check-in: a turn in the personal thread on a local day the coach
///   checked in on, at or before it.
///
/// Best-effort throughout: the turn is already answered.
pub async fn after_athlete_turn(repos: &RepositoryRegistry, turn: AthleteTurn<'_>) {
    let first_run = match FirstRun::read(repos, turn.tenant, turn.user).await {
        Ok(first_run) => first_run,
        Err(e) => {
            warn!(error = %e, user_id = %turn.user, "first-run progress unreadable; this turn marks nothing");
            return;
        }
    };
    let now = Utc::now();
    let zone = athlete_zone(repos, turn.user).await;
    let today = clock_date(now, zone);
    let mark = |flow| record_best_effort(repos, turn.user, turn.tenant, flow, today, turn.surface);
    if turn.personal_thread && !first_run.is_done(MEET_TEAM) {
        mark(MEET_TEAM).await;
    }
    if turn.grounded && !first_run.is_done(LAST_WORKOUT) {
        mark(LAST_WORKOUT).await;
    }
    if turn.personal_thread
        && !first_run.is_done(CHECK_IN)
        && !first_run.met_on(CHECK_IN, today)
        && checked_in_on(repos, turn.tenant, turn.user, today, zone, now).await
    {
        mark(CHECK_IN).await;
    }
    observe_profile(repos, turn.user, turn.tenant, &first_run, turn.surface).await;
}

/// Mark Flow 1 for a starter tapped in the personal thread — a command
/// starter runs no turn, so [`after_athlete_turn`] never sees it.
pub async fn after_personal_tap(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
    surface: &str,
) {
    match FirstRun::read(repos, tenant, user).await {
        Ok(first_run) if first_run.is_done(MEET_TEAM) => return,
        Ok(_) => {}
        Err(e) => {
            warn!(error = %e, user_id = %user, "first-run progress unreadable; this tap marks nothing");
            return;
        }
    }
    let today = clock_date(Utc::now(), athlete_zone(repos, user).await);
    record_best_effort(repos, user, tenant, MEET_TEAM, today, surface).await;
}

/// Mark Finish your profile once the dossier covers every topic.
///
/// Read only while the flow is open in `first_run`. The dossier fills after a
/// reply, from fact extraction, and from Settings, so the next turn or Home
/// read is the first to see it.
pub async fn observe_profile(
    repos: &RepositoryRegistry,
    user: Uuid,
    tenant: TenantId,
    first_run: &FirstRun,
    surface: &str,
) {
    if first_run.is_done(FINISH_PROFILE) {
        return;
    }
    match dossier_coverage(repos, tenant, user).await {
        Ok(coverage) if coverage.complete => {
            let today = clock_date(Utc::now(), athlete_zone(repos, user).await);
            record_best_effort(repos, user, tenant, FINISH_PROFILE, today, surface).await;
        }
        Ok(_) => {}
        Err(e) => {
            warn!(error = %e, user_id = %user, "dossier unreadable; the profile flow is not checked");
        }
    }
}

/// Whether the coach's check-in landed on `today` in `zone`, at or before
/// `now`. An unreadable claim reads as no check-in.
async fn checked_in_on(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: Uuid,
    today: NaiveDate,
    zone: chrono_tz::Tz,
    now: DateTime<Utc>,
) -> bool {
    match repos
        .platform_post_claims
        .platform_post_posted_at(tenant, user, PostPurpose::CheckIn.as_str())
        .await
    {
        Ok(Some(posted_at)) => posted_at <= now && clock_date(posted_at, zone) == today,
        Ok(None) => false,
        Err(e) => {
            warn!(error = %e, user_id = %user, "check-in post unreadable; the turn is not counted as an answer");
            false
        }
    }
}

/// One row of Home's Getting started list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GettingStartedRow {
    /// `connect`, or the flow's catalogue id.
    pub id: String,
    /// The row's place in the list, from 1.
    pub order: usize,
    /// Its prerequisites hold: the catalogue entry's `requires`.
    pub eligible: bool,
    /// Done.
    pub done: bool,
    /// The first row neither done nor waiting on a prerequisite.
    pub current: bool,
    /// What a tap posts into the thread, `gs:<slot>:<id>`; `None` for a row
    /// that is done, waits on a prerequisite, or is not a catalogue entry.
    pub postback: Option<String>,
}

/// Home's Getting started list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GettingStarted {
    /// Connect, then the first-run flows in flow order.
    pub rows: Vec<GettingStartedRow>,
    /// Rows done.
    pub done_count: usize,
    /// Rows in all.
    pub total: usize,
    /// When the last row became done; `None` until every row is.
    pub folded_at: Option<DateTime<Utc>>,
}

/// The Getting started list for an athlete in `state` with `first_run`, from
/// `catalogue`'s first-run flows.
#[must_use]
pub fn getting_started(
    catalogue: &UseCaseCatalogue,
    state: &AthleteState,
    first_run: &FirstRun,
) -> GettingStarted {
    let mut rows = vec![GettingStartedRow {
        id: CONNECT_ROW.to_owned(),
        order: 1,
        eligible: true,
        done: state.has_provider,
        current: false,
        postback: None,
    }];
    for flow in catalogue.first_run() {
        let position = rows.len();
        let eligible = flow.eligible(|predicate| state.holds(predicate));
        let done = first_run.is_done(&flow.id);
        rows.push(GettingStartedRow {
            id: flow.id.clone(),
            order: position + 1,
            eligible,
            done,
            current: false,
            postback: (eligible && !done).then(|| getting_started_postback(position, &flow.id)),
        });
    }
    if let Some(current) = rows.iter_mut().find(|row| row.eligible && !row.done) {
        current.current = true;
    }
    let done_count = rows.iter().filter(|row| row.done).count();
    let total = rows.len();
    let folded_at = (done_count == total)
        .then(|| {
            catalogue
                .first_run()
                .iter()
                .filter_map(|flow| first_run.done_at(&flow.id))
                .max()
        })
        .flatten();
    GettingStarted {
        rows,
        done_count,
        total,
        folded_at,
    }
}

/// The ids of the first-run flows `first_run` holds done, for the ranker.
#[must_use]
pub fn done_flows(catalogue: &UseCaseCatalogue, first_run: &FirstRun) -> Vec<String> {
    catalogue
        .first_run()
        .iter()
        .filter(|flow| first_run.is_done(&flow.id))
        .map(|flow| flow.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::athlete_state::{ActivityHistory, DossierCoverage};
    use chrono::TimeZone;
    use pierre_contremaitre::use_case_catalogue::Stage;

    const FLOWS: &str = "
- {id: connect, run: command, command: /connect, requires: [no_provider], stage: first_session, domains: [any], repeat: once, first_run: null}
- {id: meet_team, run: prompt, command: null, requires: [], stage: first_session, domains: [any], repeat: once, first_run: 1}
- {id: about_me_more, run: command, command: /pillars, requires: [dossier_empty | pillars_open], stage: any, domains: [any], repeat: recurring, first_run: 3}
- {id: last_workout, run: prompt, command: null, requires: [has_activities], stage: first_session, domains: [analysis], repeat: recurring, first_run: 2}
- {id: check_in, run: prompt, command: null, requires: [check_in_on], stage: any, domains: [any], repeat: recurring, first_run: 4}
";

    fn catalogue() -> UseCaseCatalogue {
        UseCaseCatalogue::parse(FLOWS).expect("the fixture parses")
    }

    fn state() -> AthleteState {
        AthleteState {
            stage: Stage::FirstSession,
            has_provider: true,
            activities: ActivityHistory::Present,
            weeks_of_data: 1,
            dossier: DossierCoverage {
                covered: 2,
                complete: false,
                empty: false,
            },
            season_set: false,
            plan_active: false,
            recovery_source: false,
            calendar_writable: false,
            check_in_on: true,
        }
    }

    fn day(flow: &str, date: u32, hour: u32) -> FirstRunDay {
        FirstRunDay {
            flow_id: flow.to_owned(),
            local_date: NaiveDate::from_ymd_opt(2026, 10, date).expect("date"),
            recorded_at: Utc
                .with_ymd_and_hms(2026, 10, date, hour, 0, 0)
                .single()
                .expect("instant"),
        }
    }

    fn ids(list: &GettingStarted) -> Vec<&str> {
        list.rows.iter().map(|row| row.id.as_str()).collect()
    }

    #[test]
    fn the_check_in_needs_three_days_and_every_other_flow_one() {
        let one_each = FirstRun::from_days(vec![
            day(MEET_TEAM, 1, 8),
            day(LAST_WORKOUT, 1, 9),
            day(FINISH_PROFILE, 1, 10),
            day(CHECK_IN, 1, 11),
        ]);
        for flow in [MEET_TEAM, LAST_WORKOUT, FINISH_PROFILE] {
            assert!(one_each.is_done(flow), "{flow} is done on its first day");
        }
        assert!(
            !one_each.is_done(CHECK_IN),
            "one check-in day is not a habit"
        );
        let two = FirstRun::from_days(vec![day(CHECK_IN, 1, 9), day(CHECK_IN, 2, 9)]);
        assert!(!two.is_done(CHECK_IN));
        assert!(two.met_on(
            CHECK_IN,
            NaiveDate::from_ymd_opt(2026, 10, 2).expect("date")
        ));
        let three = FirstRun::from_days(vec![
            day(CHECK_IN, 1, 9),
            day(CHECK_IN, 2, 9),
            day(CHECK_IN, 4, 9),
        ]);
        assert_eq!(
            three.done_at(CHECK_IN),
            Some(day(CHECK_IN, 4, 9).recorded_at)
        );
    }

    #[test]
    fn getting_started_lists_connect_then_the_flows_in_flow_order() {
        let list = getting_started(&catalogue(), &state(), &FirstRun::default());
        assert_eq!(
            ids(&list),
            [
                "connect",
                "meet_team",
                "last_workout",
                "about_me_more",
                "check_in"
            ]
        );
        let orders: Vec<usize> = list.rows.iter().map(|row| row.order).collect();
        assert_eq!(orders, [1, 2, 3, 4, 5]);
        assert_eq!((list.done_count, list.total), (1, 5));
        assert_eq!(list.rows[0].postback, None, "connect is a Settings link");
        assert_eq!(list.rows[1].postback.as_deref(), Some("gs:1:meet_team"));
        assert!(list.rows[1].current, "the first open flow is current");
        assert_eq!(list.rows.iter().filter(|row| row.current).count(), 1);
        assert_eq!(list.folded_at, None);
    }

    #[test]
    fn an_unconnected_athlete_is_current_on_connect_and_waits_on_data() {
        let mut state = state();
        state.has_provider = false;
        state.activities = ActivityHistory::Absent;
        state.check_in_on = false;
        let list = getting_started(&catalogue(), &state, &FirstRun::default());
        assert!(list.rows[0].current);
        let last_workout = &list.rows[2];
        assert!(!last_workout.eligible);
        assert_eq!(
            last_workout.postback, None,
            "a row waiting on a prerequisite links nowhere"
        );
        assert!(!list.rows[4].eligible, "no cadence, no check-in");
        assert_eq!(list.done_count, 0);
    }

    #[test]
    fn a_done_row_has_no_link_and_the_list_folds_when_the_last_flow_is_done() {
        let done = FirstRun::from_days(vec![
            day(MEET_TEAM, 1, 8),
            day(LAST_WORKOUT, 1, 9),
            day(FINISH_PROFILE, 2, 10),
            day(CHECK_IN, 2, 9),
            day(CHECK_IN, 3, 9),
        ]);
        let list = getting_started(&catalogue(), &state(), &done);
        assert_eq!(list.rows[1].postback, None);
        assert!(list.rows[4].current, "two check-in days are not three");
        assert_eq!(list.done_count, 4);
        assert_eq!(list.folded_at, None);

        let mut days = done.days;
        days.push(day(CHECK_IN, 5, 9));
        let list = getting_started(&catalogue(), &state(), &FirstRun::from_days(days));
        assert_eq!(list.done_count, 5);
        assert!(list.rows.iter().all(|row| !row.current));
        assert_eq!(list.folded_at, Some(day(CHECK_IN, 5, 9).recorded_at));
    }

    #[test]
    fn done_flows_names_only_the_flows_that_are_done() {
        let first_run = FirstRun::from_days(vec![day(MEET_TEAM, 1, 8), day(CHECK_IN, 1, 9)]);
        assert_eq!(done_flows(&catalogue(), &first_run), ["meet_team"]);
    }
}
