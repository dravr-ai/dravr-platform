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
//!
//! Getting started also carries the questions the onboarding wizard stopped
//! asking (carnet#1530): the health check when it was put off, the goal and
//! the why, the check-in cadence. Each completes in a sheet that writes the
//! onboarding step row its wizard screen wrote, so [`SheetAnswers`] reads
//! those rows, not first-run days.

use std::iter::once;

use chrono::{DateTime, NaiveDate, Utc};
use pierre_contremaitre::use_case_catalogue::UseCaseCatalogue;
use pierre_core::civil_time::clock_date;
use pierre_core::errors::AppResult;
use pierre_core::models::{PostPurpose, TenantId};
use pierre_database::repositories::{FirstRunDay, OnboardingStepRecord};
use pierre_database::RepositoryRegistry;
use tracing::{info, warn};
use uuid::Uuid;

use crate::athlete_clock::athlete_zone;
use crate::athlete_state::{dossier_coverage, AthleteState};
use crate::intake::{
    athlete_steps_waived, STATUS_COMPLETE, STEP_ABOUT_YOU, STEP_CHECK_IN, STEP_COACH_PROPOSAL,
    STEP_PARQ,
};

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

/// Getting started's last row: a chat app is linked (carnet#1525).
///
/// Linking Telegram, `WhatsApp` or another app lets the athlete write to their
/// agent from it. Not a catalogue entry either — linking happens in a sheet,
/// not in a thread — and listed only when the workspace offers an app
/// ([`ChatApp`]) and a row before it is still to do ([`getting_started`]).
pub const CHAT_APP_ROW: &str = "chat_app";

/// Getting started's health-check row: the PAR-Q+ put off at onboarding's
/// third screen, or never asked (carnet#1530).
///
/// Not a catalogue entry — it completes in a sheet. Listed first while the
/// list is open and the questions are still to answer, and kept, ticked, once
/// they are answered from Home; one answered in the wizard is never listed.
pub const HEALTH_CHECK_ROW: &str = "health_check";

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

/// Where the athlete stands on linking a chat app, for Getting started's last
/// row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatApp {
    /// The workspace offers no chat app: the row is not listed, so the count
    /// never holds a step nobody can take.
    NotOffered,
    /// An app is offered and none is linked yet.
    Unlinked,
    /// An app is linked.
    Linked,
}

/// Whether one of the questions Home's sheets ask has been answered, and when.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Answer {
    /// Not answered: put off, or never asked.
    #[default]
    Open,
    /// Answered, at the time its step row was last written when that reads.
    Answered {
        /// When the step row was written.
        at: Option<DateTime<Utc>>,
    },
}

impl Answer {
    /// The answer a step row holds: `complete` is answered, anything else open.
    fn of(step: Option<&OnboardingStepRecord>) -> Self {
        match step {
            Some(step) if step.status == STATUS_COMPLETE => Self::Answered {
                at: step.updated_at,
            },
            _ => Self::Open,
        }
    }

    const fn answered(self) -> bool {
        matches!(self, Self::Answered { .. })
    }

    const fn at(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Answered { at } => at,
            Self::Open => None,
        }
    }
}

/// Where the health-check row stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HealthCheck {
    /// Not the athlete's row: answered during onboarding, or the athlete
    /// steps are waived (a coach who does not train).
    #[default]
    NotListed,
    /// Put off at the third screen, or never asked.
    Open,
    /// Answered from Home after the wizard: the row stays, ticked, so the
    /// count moves on by one instead of losing a row.
    Answered {
        /// When the PAR-Q step row was written.
        at: Option<DateTime<Utc>>,
    },
}

/// The questions Home's « Pour bien démarrer » sheets ask, as the onboarding
/// step rows record them (carnet#1530).
///
/// Each sheet answers through the endpoint its wizard screen used, then
/// writes that screen's step row `complete`, so a question answered on any
/// surface — the old wizard, the chat intake, a sheet — reads the same.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SheetAnswers {
    /// The PAR-Q+, when it is Home's to ask.
    pub health_check: HealthCheck,
    /// The goal, the why and the sport (the `about_you` step).
    pub about_you: Answer,
    /// The check-in cadence (the `check_in` step); `off` is an answer.
    pub check_in: Answer,
}

impl SheetAnswers {
    /// Read the answers from the user's onboarding step rows.
    ///
    /// A PAR-Q answered after the team step was answered from Home, and keeps
    /// its row; one answered before it was answered in the wizard, and never
    /// had one. When either time does not read, the row is not listed: an
    /// answered health check is never shown as a question.
    #[must_use]
    pub fn from_steps(steps: &[OnboardingStepRecord]) -> Self {
        let find = |id: &str| steps.iter().find(|step| step.step_id == id);
        let parq = Answer::of(find(STEP_PARQ));
        let team_at = find(STEP_COACH_PROPOSAL).and_then(|step| step.updated_at);
        let health_check = if athlete_steps_waived(steps) {
            HealthCheck::NotListed
        } else {
            match parq {
                Answer::Open => HealthCheck::Open,
                Answer::Answered { at } => match (at, team_at) {
                    (Some(at), Some(team_at)) if at > team_at => {
                        HealthCheck::Answered { at: Some(at) }
                    }
                    _ => HealthCheck::NotListed,
                },
            }
        };
        Self {
            health_check,
            about_you: Answer::of(find(STEP_ABOUT_YOU)),
            check_in: Answer::of(find(STEP_CHECK_IN)),
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
    /// Connect, the health check when it is Home's to ask, the goal and the
    /// why, the check-in cadence, the chat app when one is offered — the
    /// health check and the chat app only while the list is open — then the
    /// first-run flows.
    pub rows: Vec<GettingStartedRow>,
    /// Rows done.
    pub done_count: usize,
    /// Rows in all.
    pub total: usize,
    /// When the last row became done; `None` until every row is.
    pub folded_at: Option<DateTime<Utc>>,
}

/// The first-run flows whose rows complete in a sheet (carnet#1530): Finish
/// your profile became « Ton objectif et ton pourquoi », the check-in habit
/// « Ton point du jour ». Their flow still ticks them when it is done.
const SHEET_FLOWS: [&str; 2] = [FINISH_PROFILE, CHECK_IN];

/// A row before its place in the list and its postback are settled.
struct DraftRow {
    id: String,
    eligible: bool,
    done: bool,
    /// When it became done, where that is known.
    done_at: Option<DateTime<Utc>>,
    /// A first-run flow whose tap sends its starter into the thread.
    starter: bool,
}

/// The earlier of two instants, either of which may be unknown.
fn earliest(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The Getting started list for an athlete in `state` with `first_run` and
/// the onboarding `answers` Home's sheets ask for.
///
/// The board's order (P4·D): connect first, then the questions the onboarding
/// wizard stopped asking, each completing in a sheet (carnet#1530) — the
/// health check when it was put off (`answers.health_check`), the goal and
/// the why, the check-in cadence — then the chat app when `chat_app` says the
/// workspace offers one, then `catalogue`'s other first-run flows. The health
/// check and the chat app ride along with an open list: neither holds a
/// finished list open, and neither brings back one already folded or hidden.
/// A sheet row is done once its question is answered or its first-run flow is
/// done, whichever came first; it carries no postback, since a tap opens its
/// sheet.
#[must_use]
pub fn getting_started(
    catalogue: &UseCaseCatalogue,
    state: &AthleteState,
    first_run: &FirstRun,
    answers: &SheetAnswers,
    chat_app: ChatApp,
) -> GettingStarted {
    let sheet = |id: &str, answer: Answer| {
        let flow_done_at = first_run.done_at(id);
        DraftRow {
            id: id.to_owned(),
            eligible: true,
            done: answer.answered() || flow_done_at.is_some(),
            done_at: earliest(answer.at(), flow_done_at),
            starter: false,
        }
    };
    let questions = [
        sheet(FINISH_PROFILE, answers.about_you),
        sheet(CHECK_IN, answers.check_in),
    ];
    let connect = DraftRow {
        id: CONNECT_ROW.to_owned(),
        eligible: true,
        done: state.has_provider,
        done_at: None,
        starter: false,
    };
    let mut later = Vec::new();
    for flow in catalogue
        .first_run()
        .iter()
        .filter(|flow| !SHEET_FLOWS.contains(&flow.id.as_str()))
    {
        let done_at = first_run.done_at(&flow.id);
        later.push(DraftRow {
            id: flow.id.clone(),
            eligible: flow.eligible(|predicate| state.holds(predicate)),
            done: done_at.is_some(),
            done_at,
            starter: true,
        });
    }
    let core = || once(&connect).chain(&questions).chain(&later);
    let list_open = core().any(|row| !row.done);
    // The moment the last of these rows became done is when the list folds:
    // the two that ride along are never listed on a finished list.
    let last_done_at = core().filter_map(|row| row.done_at).max();

    let mut drafts = vec![connect];
    if list_open {
        match answers.health_check {
            HealthCheck::NotListed => {}
            HealthCheck::Open => drafts.push(DraftRow {
                id: HEALTH_CHECK_ROW.to_owned(),
                eligible: true,
                done: false,
                done_at: None,
                starter: false,
            }),
            HealthCheck::Answered { at } => drafts.push(DraftRow {
                id: HEALTH_CHECK_ROW.to_owned(),
                eligible: true,
                done: true,
                done_at: at,
                starter: false,
            }),
        }
    }
    drafts.extend(questions);
    if list_open && chat_app != ChatApp::NotOffered {
        drafts.push(DraftRow {
            id: CHAT_APP_ROW.to_owned(),
            eligible: true,
            done: chat_app == ChatApp::Linked,
            done_at: None,
            starter: false,
        });
    }
    drafts.extend(later);

    let mut rows: Vec<GettingStartedRow> = drafts
        .into_iter()
        .enumerate()
        .map(|(slot, row)| GettingStartedRow {
            postback: (row.starter && row.eligible && !row.done)
                .then(|| getting_started_postback(slot, &row.id)),
            id: row.id,
            order: slot + 1,
            eligible: row.eligible,
            done: row.done,
            current: false,
        })
        .collect();
    if let Some(current) = rows.iter_mut().find(|row| row.eligible && !row.done) {
        current.current = true;
    }
    let done_count = rows.iter().filter(|row| row.done).count();
    let total = rows.len();
    let folded_at = (done_count == total).then_some(last_done_at).flatten();
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
    use crate::intake::STATUS_NOT_APPLICABLE;
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

    fn instant(date: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, date, hour, 0, 0)
            .single()
            .expect("instant")
    }

    fn day(flow: &str, date: u32, hour: u32) -> FirstRunDay {
        FirstRunDay {
            flow_id: flow.to_owned(),
            local_date: NaiveDate::from_ymd_opt(2026, 10, date).expect("date"),
            recorded_at: instant(date, hour),
        }
    }

    fn step(step_id: &str, status: &str, at: Option<DateTime<Utc>>) -> OnboardingStepRecord {
        OnboardingStepRecord {
            step_id: step_id.to_owned(),
            status: status.to_owned(),
            chosen_channel: None,
            updated_at: at,
        }
    }

    fn ids(list: &GettingStarted) -> Vec<&str> {
        list.rows.iter().map(|row| row.id.as_str()).collect()
    }

    fn row<'a>(list: &'a GettingStarted, id: &str) -> &'a GettingStartedRow {
        list.rows
            .iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("a {id} row"))
    }

    /// Every flow done, the check-in on its third day.
    fn every_flow() -> FirstRun {
        FirstRun::from_days(vec![
            day(MEET_TEAM, 1, 8),
            day(LAST_WORKOUT, 1, 9),
            day(FINISH_PROFILE, 2, 10),
            day(CHECK_IN, 2, 9),
            day(CHECK_IN, 3, 9),
            day(CHECK_IN, 5, 9),
        ])
    }

    const NOTHING_ANSWERED: SheetAnswers = SheetAnswers {
        health_check: HealthCheck::NotListed,
        about_you: Answer::Open,
        check_in: Answer::Open,
    };

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
    fn getting_started_lists_connect_then_the_sheet_questions_then_the_flows() {
        let list = getting_started(
            &catalogue(),
            &state(),
            &FirstRun::default(),
            &NOTHING_ANSWERED,
            ChatApp::NotOffered,
        );
        assert_eq!(
            ids(&list),
            [
                "connect",
                "about_me_more",
                "check_in",
                "meet_team",
                "last_workout"
            ],
            "the board's order: connect first"
        );
        let orders: Vec<usize> = list.rows.iter().map(|row| row.order).collect();
        assert_eq!(orders, [1, 2, 3, 4, 5]);
        assert_eq!((list.done_count, list.total), (1, 5));
        assert!(
            list.rows[1].current,
            "connected, the goal and the why are the first open row"
        );
        assert_eq!(list.rows.iter().filter(|row| row.current).count(), 1);
        for sheet in ["about_me_more", "check_in", "connect"] {
            assert_eq!(
                row(&list, sheet).postback,
                None,
                "{sheet} opens its own place"
            );
        }
        assert_eq!(
            row(&list, "meet_team").postback.as_deref(),
            Some("gs:3:meet_team"),
            "a flow's postback names the slot it is shown in"
        );
        assert_eq!(list.folded_at, None);
    }

    #[test]
    fn a_sheet_row_ticks_on_its_answer_or_on_its_flow() {
        let answered = SheetAnswers {
            about_you: Answer::Answered {
                at: Some(instant(4, 8)),
            },
            check_in: Answer::Answered { at: None },
            ..NOTHING_ANSWERED
        };
        let list = getting_started(
            &catalogue(),
            &state(),
            &FirstRun::default(),
            &answered,
            ChatApp::NotOffered,
        );
        assert!(row(&list, "about_me_more").done);
        assert!(
            row(&list, "check_in").done,
            "a cadence of any kind is an answer"
        );
        assert!(row(&list, "meet_team").current);

        let profile_walked = FirstRun::from_days(vec![day(FINISH_PROFILE, 2, 10)]);
        let list = getting_started(
            &catalogue(),
            &state(),
            &profile_walked,
            &NOTHING_ANSWERED,
            ChatApp::NotOffered,
        );
        assert!(
            row(&list, "about_me_more").done,
            "a complete dossier holds the goal and the why"
        );
    }

    #[test]
    fn an_unconnected_athlete_can_answer_the_sheets_and_waits_on_data_for_the_flows() {
        let mut state = state();
        state.has_provider = false;
        state.activities = ActivityHistory::Absent;
        state.check_in_on = false;
        let list = getting_started(
            &catalogue(),
            &state,
            &FirstRun::default(),
            &NOTHING_ANSWERED,
            ChatApp::NotOffered,
        );
        assert!(
            row(&list, "check_in").eligible,
            "the cadence is chosen before any check-in runs"
        );
        assert!(!row(&list, "connect").done);
        let last_workout = row(&list, "last_workout");
        assert!(!last_workout.eligible);
        assert_eq!(
            last_workout.postback, None,
            "a row waiting on a prerequisite links nowhere"
        );
        assert_eq!(list.done_count, 0);
    }

    #[test]
    fn the_list_folds_when_its_last_row_is_done_at_that_rows_time() {
        let mut flows_but_check_in = every_flow().days;
        flows_but_check_in.retain(|day| day.flow_id != CHECK_IN);
        let first_run = FirstRun::from_days(flows_but_check_in);
        let list = getting_started(
            &catalogue(),
            &state(),
            &first_run,
            &NOTHING_ANSWERED,
            ChatApp::NotOffered,
        );
        assert!(
            row(&list, "check_in").current,
            "the cadence is still to choose"
        );
        assert_eq!(list.folded_at, None);

        let cadence_chosen = SheetAnswers {
            check_in: Answer::Answered {
                at: Some(instant(20, 7)),
            },
            ..NOTHING_ANSWERED
        };
        let list = getting_started(
            &catalogue(),
            &state(),
            &first_run,
            &cadence_chosen,
            ChatApp::NotOffered,
        );
        assert_eq!((list.done_count, list.total), (5, 5));
        assert!(list.rows.iter().all(|row| !row.current));
        assert_eq!(
            list.folded_at,
            Some(instant(20, 7)),
            "the list folds when the cadence was chosen, not when the flows were done"
        );

        let list = getting_started(
            &catalogue(),
            &state(),
            &every_flow(),
            &NOTHING_ANSWERED,
            ChatApp::NotOffered,
        );
        assert_eq!(list.done_count, 5);
        assert_eq!(list.folded_at, Some(day(CHECK_IN, 5, 9).recorded_at));
    }

    #[test]
    fn a_health_check_put_off_follows_connect_and_stays_ticked_once_answered() {
        let skipped = SheetAnswers {
            health_check: HealthCheck::Open,
            ..NOTHING_ANSWERED
        };
        let list = getting_started(
            &catalogue(),
            &state(),
            &FirstRun::default(),
            &skipped,
            ChatApp::NotOffered,
        );
        assert_eq!(&ids(&list)[..2], [CONNECT_ROW, HEALTH_CHECK_ROW]);
        let health = row(&list, HEALTH_CHECK_ROW);
        assert!(
            health.current && health.eligible && !health.done,
            "connected, the health check is the first open row"
        );
        assert_eq!(health.postback, None, "the row opens its sheet");
        assert_eq!((list.done_count, list.total), (1, 6));

        let answered = SheetAnswers {
            health_check: HealthCheck::Answered {
                at: Some(instant(6, 9)),
            },
            ..NOTHING_ANSWERED
        };
        let list = getting_started(
            &catalogue(),
            &state(),
            &FirstRun::default(),
            &answered,
            ChatApp::NotOffered,
        );
        assert!(
            row(&list, HEALTH_CHECK_ROW).done,
            "answered from Home, the row ticks"
        );
        assert_eq!(
            (list.done_count, list.total),
            (2, 6),
            "the count moves on by one"
        );

        let list = getting_started(
            &catalogue(),
            &state(),
            &FirstRun::default(),
            &NOTHING_ANSWERED,
            ChatApp::NotOffered,
        );
        assert!(!ids(&list).contains(&HEALTH_CHECK_ROW));
    }

    #[test]
    fn the_health_check_and_the_chat_app_never_reopen_a_finished_list() {
        let finished = SheetAnswers {
            health_check: HealthCheck::Open,
            about_you: Answer::Answered { at: None },
            check_in: Answer::Answered { at: None },
        };
        for chat_app in [ChatApp::Unlinked, ChatApp::Linked] {
            let list = getting_started(&catalogue(), &state(), &every_flow(), &finished, chat_app);
            assert!(
                !ids(&list).contains(&HEALTH_CHECK_ROW) && !ids(&list).contains(&CHAT_APP_ROW),
                "a finished list gets neither row ({chat_app:?})"
            );
            assert_eq!((list.done_count, list.total), (5, 5));
            assert_eq!(
                list.folded_at,
                Some(day(CHECK_IN, 5, 9).recorded_at),
                "the list folds when its last first-run row was done"
            );
        }
    }

    #[test]
    fn the_chat_app_row_follows_the_questions_only_when_an_app_is_offered() {
        let without = getting_started(
            &catalogue(),
            &state(),
            &FirstRun::default(),
            &NOTHING_ANSWERED,
            ChatApp::NotOffered,
        );
        assert!(!ids(&without).contains(&CHAT_APP_ROW), "no app, no row");

        let list = getting_started(
            &catalogue(),
            &state(),
            &FirstRun::default(),
            &NOTHING_ANSWERED,
            ChatApp::Unlinked,
        );
        assert_eq!(
            ids(&list),
            [
                "connect",
                "about_me_more",
                "check_in",
                CHAT_APP_ROW,
                "meet_team",
                "last_workout"
            ]
        );
        let chat = row(&list, CHAT_APP_ROW);
        assert_eq!(chat.order, 4);
        assert!(chat.eligible && !chat.done);
        assert_eq!(chat.postback, None, "the row opens a sheet, not a thread");
        assert!(
            !chat.current,
            "the goal and the why are still the current row"
        );
        assert_eq!((list.done_count, list.total), (1, 6));

        let linked = getting_started(
            &catalogue(),
            &state(),
            &FirstRun::default(),
            &NOTHING_ANSWERED,
            ChatApp::Linked,
        );
        assert!(row(&linked, CHAT_APP_ROW).done);
        assert_eq!((linked.done_count, linked.total), (2, 6));
    }

    #[test]
    fn sheet_answers_read_the_step_rows_the_wizard_and_the_sheets_write() {
        let team_at = instant(3, 12);
        let team = step(STEP_COACH_PROPOSAL, STATUS_COMPLETE, Some(team_at));

        let none = SheetAnswers::from_steps(&[]);
        assert_eq!(
            none.health_check,
            HealthCheck::Open,
            "never asked is still to ask"
        );
        assert_eq!(
            (none.about_you, none.check_in),
            (Answer::Open, Answer::Open)
        );

        let skipped = SheetAnswers::from_steps(&[
            step(STEP_PARQ, "skipped", Some(instant(3, 11))),
            team.clone(),
        ]);
        assert_eq!(skipped.health_check, HealthCheck::Open);

        let in_the_wizard = SheetAnswers::from_steps(&[
            step(STEP_PARQ, STATUS_COMPLETE, Some(instant(3, 11))),
            team.clone(),
        ]);
        assert_eq!(
            in_the_wizard.health_check,
            HealthCheck::NotListed,
            "answered at the third screen, Home never asks"
        );

        let from_home = SheetAnswers::from_steps(&[
            step(STEP_PARQ, STATUS_COMPLETE, Some(instant(4, 9))),
            team.clone(),
        ]);
        assert_eq!(
            from_home.health_check,
            HealthCheck::Answered {
                at: Some(instant(4, 9))
            }
        );

        let untimed = SheetAnswers::from_steps(&[step(STEP_PARQ, STATUS_COMPLETE, None), team]);
        assert_eq!(
            untimed.health_check,
            HealthCheck::NotListed,
            "an answer is never shown as a question"
        );

        let coach = SheetAnswers::from_steps(&[step(STEP_PARQ, STATUS_NOT_APPLICABLE, None)]);
        assert_eq!(coach.health_check, HealthCheck::NotListed);

        let answered = SheetAnswers::from_steps(&[
            step(STEP_ABOUT_YOU, STATUS_COMPLETE, Some(instant(5, 8))),
            step(STEP_CHECK_IN, "skipped", Some(instant(5, 8))),
        ]);
        assert_eq!(
            answered.about_you,
            Answer::Answered {
                at: Some(instant(5, 8))
            }
        );
        assert_eq!(answered.check_in, Answer::Open, "a skip is no cadence");
    }

    #[test]
    fn done_flows_names_only_the_flows_that_are_done() {
        let first_run = FirstRun::from_days(vec![day(MEET_TEAM, 1, 8), day(CHECK_IN, 1, 9)]);
        assert_eq!(done_flows(&catalogue(), &first_run), ["meet_team"]);
    }
}
