// ABOUTME: GET /api/coach/roster — the human coach's athletes, one row each, for Coach Home (À traiter, Tableau d'équipe)
// ABOUTME: The roster is pierre_services::coach_roster's and its reading pierre_tool_runtime::coach_roster's, shared with get_roster_overview
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach's roster read (carnet#1251).
//!
//! `GET /api/coach/roster?offset=&limit=` answers with every athlete of the
//! groups the caller coaches, in the order and with the consent statuses of
//! [`pierre_services::coach_roster`] — the one roster the agent's
//! `get_roster_overview` reads too. This module adds what a screen needs and
//! the agent does not: the plan summary, the coach's draft and when they last
//! published one, where the plan was pushed, and the agent's group replies
//! this week.
//!
//! - **Who may call it.** A user with `manages_roster` or platform admin who
//!   coaches at least one active group; anyone else gets 403.
//! - **Consent.** Only a `shared` row carries anything beyond the athlete's
//!   name, groups and status: no number, nothing inferred, for the others.
//!   A shared row carries nothing from a provider whose AI use the athlete
//!   withheld: the coach and the agent read one projection (carnet#1262).
//! - **Form** is the server's band on the athlete's own scale with the day it
//!   was read, never a raw TSB.
//! - **Bounded.** The roster is gathered and ordered from the database; the
//!   provider reads (one per shared athlete) happen only for the requested
//!   page, at most [`MAX_PAGE`] rows, a bounded number at a time.
//! - **Scope.** `scope: { visible_members, roster_members }` says how many of
//!   the athletes held share with the caller (ADR-002, 2026-10-23).
//!
//! No route resolves the caller's tenant: a coach reads the groups they coach
//! whatever tenant owns them (carnet#1210), and every per-athlete read is
//! scoped to the athlete's own tenant by the coaching gate.

use std::collections::{BTreeSet, HashMap};
use std::iter;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use pierre_config::coach_home::CoachHomeConfig;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::MemberFitnessSnapshot;
use pierre_core::models::TenantId;
use pierre_database::repositories::training_plans::DraftOwner;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, GoalRace, RacePriority, TrainingPlan};
use pierre_memory::{ClaimStatus, PlanPhase};
use pierre_middleware::AuthenticatedUser;
use pierre_providers::ai_scope;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_attention::{
    apply_dismissals, attention_reasons, season_view, AttentionInputs, AttentionReason,
    DraftSignals, PlanSignals, ReplySignal, SeasonRace, SeasonView,
};
use pierre_services::coach_plan_draft::{draft_outline, CoachDraftTarget};
use pierre_services::coach_replies::{self, CoachReplyView, ReplyPageQuery};
use pierre_services::coach_roster::{
    gather_roster, order_roster, roster_access, OrderedMember, RosterAccess, RosterAthleteStatus,
    RosterGroupRef, RosterGroupSummary, ROSTER_LOOKUP_CONCURRENCY,
};
use pierre_services::coached_athlete::coached_athlete_access;
use pierre_tool_runtime::coach_roster::{fetch_page_snapshots, read_snapshot, RosterReading};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::coach_attention::{
    page_dismissals, refused_since, sources_of, training_signals, PageDismissals,
};

/// Rows a page holds when the caller names no limit.
const DEFAULT_PAGE: usize = 50;

/// Most rows one page holds: each shared row is a provider read.
const MAX_PAGE: usize = 100;

/// Pages of the agent's replies read back to this week's Monday. Fifty
/// replies a page, so a coach whose groups heard more than this in one week
/// gets a count that says `capped`.
const MAX_REPLY_PAGES: usize = 20;

/// Replies read per page of the reply review.
const REPLY_PAGE_SIZE: i64 = 50;

/// Query of `GET /api/coach/roster`.
#[derive(Debug, Default, Deserialize)]
pub struct RosterQuery {
    /// Rows to skip: the `next_offset` of the previous page.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Rows per page, clamped to `1..=100`; 50 when absent.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// How many athletes the response covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RosterScope {
    /// Athletes who share their training with the caller.
    pub visible_members: usize,
    /// Athletes the caller's groups hold.
    pub roster_members: usize,
}

/// The athlete's plan, as a roster row summarises it: today's place in it
/// for the roster table, and its phases and races for the season view
/// (carnet#1250).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RosterPlan {
    /// The phase today falls in (`base`, `build`, `peak`, …); absent between
    /// phases or outside the season.
    pub phase: Option<String>,
    /// The week of the season today falls in, from 1; absent outside it.
    pub week: Option<u32>,
    /// Weeks the season's phases span.
    pub weeks_total: u32,
    /// The goal race.
    pub goal_race: RosterRace,
    /// The rest of the race calendar, in date order: a second A race is a
    /// second peak; B and C races are context.
    pub races: Vec<RosterRace>,
    /// The season's last day, `YYYY-MM-DD`, when the plan sets one; absent
    /// when it ends with its goal race.
    pub season_end: Option<String>,
    /// The day after the plan's last day — its last phase, last race or
    /// season end, whichever is latest — `YYYY-MM-DD`.
    pub ends: Option<String>,
    /// The season's phases in order, each with its span.
    pub phases: Vec<RosterPhase>,
    /// The coaching platforms the plan's sessions from today on were pushed to.
    pub pushed_to: Vec<String>,
}

/// One phase of a plan, as the season view draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RosterPhase {
    /// What kind of phase (`base`, `build`, `specialty`, `peak`, `taper`,
    /// `race`, `transition`, `recovery`, `prep`).
    pub kind: String,
    /// Its first day, `YYYY-MM-DD`.
    pub start: String,
    /// The day after its last day, `YYYY-MM-DD`; absent when its start does
    /// not parse.
    pub end: Option<String>,
    /// Its length in weeks.
    pub weeks: u8,
}

/// A race on a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RosterRace {
    /// The race's name.
    pub name: String,
    /// The race day, `YYYY-MM-DD`.
    pub date: String,
    /// `A` (a goal the season peaks for), `B` or `C`.
    pub priority: String,
}

/// Where the caller's own work on the athlete's plan stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RosterPlanThread {
    /// When the caller last saved the draft they hold; absent with none open.
    pub draft_updated_at: Option<DateTime<Utc>>,
    /// Whether the open draft clashes with weeks the athlete changed since.
    pub draft_has_conflicts: bool,
    /// When the caller last published a draft of this plan; absent if never.
    pub published_at: Option<DateTime<Utc>>,
    /// The open draft's phases, for the season view's draft band; empty with
    /// no draft open, or one the caller may not read.
    pub draft_phases: Vec<RosterPhase>,
}

/// The agent's replies to the athlete in the caller's groups this week.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RosterAgentReplies {
    /// Replies sent since Monday 00:00 UTC.
    pub count: usize,
    /// Replies in which the checker found no contradicted or unsupported
    /// claim and no figure it could not match to the athlete's data.
    pub verified: usize,
    /// Figures, across those replies, with no source in the athlete's data.
    pub unmatched_figures: usize,
    /// True when the week held more replies than one read covers, so the
    /// counts are a floor.
    pub capped: bool,
}

/// One athlete on the roster.
#[derive(Debug, Clone, Serialize)]
pub struct RosterRow {
    /// The athlete.
    pub user_id: Uuid,
    /// Display name — never their account identifier.
    pub name: String,
    /// The caller's groups the athlete belongs to.
    pub groups: Vec<RosterGroupRef>,
    /// Whether their training is listed, and why not when it is not.
    pub status: RosterAthleteStatus,
    /// When they stopped sharing with the coach; only for `no_coach_consent`,
    /// and absent when the date is unknown (a revoke from before it was
    /// recorded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharing_stopped_at: Option<DateTime<Utc>>,
    /// Their training; only for `shared`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub training: Option<RosterReading>,
    /// Their live plan; only for `shared`, absent with none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<RosterPlan>,
    /// The caller's draft and publish history on that plan; only for `shared`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_thread: Option<RosterPlanThread>,
    /// The agent's group replies to them this week; only for `shared`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_replies: Option<RosterAgentReplies>,
    /// Where their season stands, for the season view; only for `shared`
    /// when the plan was read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season: Option<SeasonView>,
    /// The reasons to look at them today or this week, lowest `rank` first;
    /// empty for a calm athlete. A non-sharer carries the consent reason
    /// alone. A reason the caller marked handled for its current signal
    /// carries `dismissed_at`.
    pub attention: Vec<AttentionReason>,
}

/// `GET /api/coach/roster`.
#[derive(Debug, Serialize)]
pub struct RosterResponse {
    /// How many athletes are held and how many share with the caller.
    pub scope: RosterScope,
    /// The groups covered.
    pub groups: Vec<RosterGroupSummary>,
    /// This page of athletes: those who share first, the longest without a
    /// session at the top; then the others, by name.
    pub athletes: Vec<RosterRow>,
    /// The `offset` of the next page; absent on the last.
    pub next_offset: Option<usize>,
}

/// Mount-helper for the coach roster route.
#[derive(Debug)]
pub struct CoachRosterRoutes;

impl CoachRosterRoutes {
    /// `GET /api/coach/roster`. Requires a valid session; who may read it is
    /// decided by [`roster_access`].
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/coach/roster", get(handle_roster))
            .with_state(resources)
    }
}

/// The refusal for a caller who may not read a roster, shared by every
/// coach-roster route.
pub(crate) fn not_a_coach() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "Only a coach attached to a group can read a roster",
    )
}

/// `GET /api/coach/roster` — the caller's athletes, one page.
///
/// # Errors
///
/// 403 when the caller may not read a roster; the repository error otherwise.
pub async fn handle_roster(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<RosterQuery>,
) -> Result<Response, AppError> {
    let coach = auth.into_inner().user_id;
    let repos = &resources.common.repos;
    let RosterAccess::Granted(groups) = roster_access(repos, coach).await? else {
        return Err(not_a_coach());
    };

    let gathered = gather_roster(repos, coach, &groups).await?;
    let scope = RosterScope {
        visible_members: gathered.visible_members(),
        roster_members: gathered.roster_members(),
    };
    let ordered = order_roster(repos, gathered.members).await?;
    let offset = query.offset.unwrap_or(0).min(ordered.len());
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let end = offset.saturating_add(limit).min(ordered.len());
    let page = &ordered[offset..end];

    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);
    // A human coach's read: each athlete's AI consents apply to it as to the
    // agent's, and no other AI rule (carnet#1262, ADR-002 amendment 2026-10-30).
    let snapshots = ai_scope::for_human_coach(fetch_page_snapshots(&runtime, page)).await;
    // Only an athlete who shares can carry replies (their counts on a
    // `shared` row, an unmatched figure on any sharer's): a page with none
    // reads none.
    let replies = if page
        .iter()
        .any(|m| m.member.status != RosterAthleteStatus::NoCoachConsent)
    {
        replies_this_week(&resources, coach).await?
    } else {
        WeekReplies::default()
    };

    let dismissals = page_dismissals(repos, coach, page).await?;
    let read = PageRead {
        coach,
        snapshots: &snapshots,
        replies: &replies,
        dismissals: &dismissals,
        thresholds: resources.common.config.coach_home,
    };
    let rows: Vec<_> = page
        .iter()
        .map(|member| row(repos, &read, member))
        .collect();
    let athletes: Vec<RosterRow> = stream::iter(rows)
        .buffered(ROSTER_LOOKUP_CONCURRENCY)
        .try_collect()
        .await?;
    info!(
        coach = %coach,
        roster = scope.roster_members,
        visible = scope.visible_members,
        offset,
        rows = athletes.len(),
        "coach roster read"
    );
    Ok((
        StatusCode::OK,
        Json(RosterResponse {
            scope,
            groups: gathered.groups,
            athletes,
            next_offset: (end < ordered.len()).then_some(end),
        }),
    )
        .into_response())
}

/// What one page read shares across its rows.
struct PageRead<'a> {
    coach: Uuid,
    snapshots: &'a HashMap<Uuid, MemberFitnessSnapshot>,
    replies: &'a WeekReplies,
    dismissals: &'a PageDismissals,
    thresholds: CoachHomeConfig,
}

/// One row: name, groups, status and the consent reason for everyone; the
/// rest only for a sharer.
async fn row(
    repos: &RepositoryRegistry,
    read: &PageRead<'_>,
    member: &OrderedMember,
) -> AppResult<RosterRow> {
    let athlete = member.member.user_id;
    let mut row = RosterRow {
        user_id: athlete,
        name: member.name.clone(),
        groups: member.member.groups.clone(),
        status: member.member.status,
        // The service already leaves it `None` for any other status.
        sharing_stopped_at: member.member.sharing_stopped_at,
        training: None,
        plan: None,
        plan_thread: None,
        agent_replies: None,
        season: None,
        attention: Vec::new(),
    };
    let today = athlete_today(repos, athlete).await;
    let snapshot = read.snapshots.get(&athlete);
    let replies = read.replies.signals_for(athlete);
    let plan = if member.member.status == RosterAthleteStatus::Shared {
        row.training = snapshot.map(read_snapshot);
        row.agent_replies = Some(read.replies.for_athlete(athlete));
        shared_plan(repos, read.coach, member, today, &mut row).await?
    } else {
        SharedPlan::default()
    };

    let sources = snapshot.map(sources_of).unwrap_or_default();
    let refusals = refused_since(repos, athlete, row.training.as_ref()).await?;
    let inputs = AttentionInputs {
        status: member.member.status,
        sharing_stopped_at: member.member.sharing_stopped_at,
        joined: &member.member.joined,
        now: Utc::now(),
        today,
        training: snapshot
            .zip(row.training.as_ref())
            .map(|(snapshot, reading)| training_signals(snapshot, reading, &sources, &refusals)),
        plan_read: plan.read,
        plan: plan.plan.as_ref(),
        draft: plan.draft.as_ref(),
        replies,
    };
    row.season = season_view(&inputs, &read.thresholds);
    row.attention = attention_reasons(&inputs, &read.thresholds);
    if let Some(dismissed) = read.dismissals.get(&athlete) {
        apply_dismissals(&mut row.attention, dismissed);
    }
    Ok(row)
}

/// What a sharer's plan read gave the queue.
#[derive(Debug, Default)]
struct SharedPlan {
    /// The coaching gate let the coach read the plan.
    read: bool,
    plan: Option<PlanSignals>,
    draft: Option<DraftSignals>,
}

/// Fill a sharer's plan and plan thread on `row`.
///
/// The by-id coaching gate, run again: it resolves the athlete's own tenant,
/// where their plan lives, and refuses anything consent no longer allows. A
/// refusal here leaves the plan out rather than failing the page.
async fn shared_plan(
    repos: &RepositoryRegistry,
    coach: Uuid,
    member: &OrderedMember,
    today: NaiveDate,
    row: &mut RosterRow,
) -> AppResult<SharedPlan> {
    let athlete = member.member.user_id;
    let Some(group_id) = member.member.shared_via else {
        return Ok(SharedPlan::default());
    };
    let access = match coached_athlete_access(repos, coach, athlete, group_id).await? {
        Ok(access) => access,
        Err(refusal) => {
            warn!(athlete = %athlete, reason = refusal.reason(), "roster row: no plan read");
            return Ok(SharedPlan::default());
        }
    };
    let (plan, signals) = plan_summary(repos, access.tenant, athlete, today)
        .await?
        .unzip();
    row.plan = plan;
    let (thread, draft) = plan_thread(repos, access.tenant, athlete, coach).await?;
    row.plan_thread = Some(thread);
    Ok(SharedPlan {
        read: true,
        plan: signals,
        draft,
    })
}

/// The athlete's live plan, summarised on their own `today`, with what the
/// queue and the season view read off it.
async fn plan_summary(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    athlete: Uuid,
    today: NaiveDate,
) -> AppResult<Option<(RosterPlan, PlanSignals)>> {
    let Some(plan) = repos
        .training_plans
        .get_active_plan(&tenant.to_string(), &athlete.to_string())
        .await?
    else {
        return Ok(None);
    };
    // A plan derived from first-party-only data is not described to an
    // external caller, as every plan read does (carnet#769).
    if !ai_scope::admit_derived(plan.transport_policy) {
        return Ok(None);
    }
    let phase = plan
        .phases
        .iter()
        .find(|p| p.covers(today))
        .map(|p| p.kind.as_str().to_owned());
    let weeks_total: u32 = plan.phases.iter().map(|p| u32::from(p.weeks)).sum();
    // The weeks are the phases' weeks, so they count from the first phase:
    // a season opened before it would number them off by the gap.
    let start = plan.phases.first().and_then(PlanPhase::start_date);
    let week = start.and_then(|start| season_week(start, today, weeks_total));

    // Each provider once, however many connections name it.
    let providers: BTreeSet<String> = repos
        .provider_connections
        .get_for_user(athlete, None)
        .await?
        .into_iter()
        .map(|connection| connection.provider)
        .collect();
    let mut pushed_to = Vec::new();
    for provider in providers {
        let events = repos
            .prescribed_workouts
            .list_live_calendar_events(tenant, athlete, &provider, Some(today))
            .await?;
        if !events.is_empty() {
            pushed_to.push(provider);
        }
    }

    let ends = plan_ends(&plan);
    let mut races: Vec<&GoalRace> = plan.races.iter().collect();
    races.sort_by(|a, b| a.date.cmp(&b.date));
    let signals = PlanSignals {
        plan_id: plan.id.clone(),
        updated_at: plan.updated_at,
        ends,
        races: iter::once(&plan.goal_race)
            .chain(races.iter().copied())
            .filter_map(|race| {
                Some(SeasonRace {
                    name: race.name.clone(),
                    date: parse_plan_date(&race.date)?,
                    a_race: race.priority == RacePriority::A,
                })
            })
            .collect(),
    };
    let summary = RosterPlan {
        phase,
        week,
        weeks_total,
        goal_race: roster_race(&plan.goal_race),
        races: races.into_iter().map(roster_race).collect(),
        season_end: plan.season_end.clone(),
        ends: ends.map(|day| day.format("%Y-%m-%d").to_string()),
        phases: roster_phases(&plan.phases),
        pushed_to,
    };
    Ok(Some((summary, signals)))
}

fn roster_race(race: &GoalRace) -> RosterRace {
    RosterRace {
        name: race.name.clone(),
        date: race.date.clone(),
        priority: race.priority.as_str().to_owned(),
    }
}

fn roster_phases(phases: &[PlanPhase]) -> Vec<RosterPhase> {
    phases
        .iter()
        .map(|p| RosterPhase {
            kind: p.kind.as_str().to_owned(),
            start: p.start.clone(),
            end: p
                .end_exclusive()
                .map(|day| day.format("%Y-%m-%d").to_string()),
            weeks: p.weeks,
        })
        .collect()
}

/// The day after the plan's last day: its last phase's end, the day after
/// its last race or after its season end, whichever is latest.
fn plan_ends(plan: &TrainingPlan) -> Option<NaiveDate> {
    let after = |date: &str| parse_plan_date(date).and_then(|d| d.succ_opt());
    plan.phases
        .iter()
        .filter_map(PlanPhase::end_exclusive)
        .chain(
            iter::once(&plan.goal_race)
                .chain(&plan.races)
                .filter_map(|race| after(&race.date)),
        )
        .chain(plan.season_end.as_deref().and_then(after))
        .max()
}

/// The week of a `weeks_total`-week season starting `start` that `today`
/// falls in, from 1; `None` before or after it.
fn season_week(start: NaiveDate, today: NaiveDate, weeks_total: u32) -> Option<u32> {
    let days = (today - start).num_days();
    if days < 0 {
        return None;
    }
    let week = u32::try_from(days / 7).ok()?.checked_add(1)?;
    (week <= weeks_total).then_some(week)
}

/// The caller's open draft and last publish on the athlete's plan.
async fn plan_thread(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    athlete: Uuid,
    coach: Uuid,
) -> AppResult<(RosterPlanThread, Option<DraftSignals>)> {
    let target = CoachDraftTarget::new(tenant, athlete, coach);
    let draft = draft_outline(repos, &target).await?;
    let (tenant_key, athlete_key, coach_key) =
        (tenant.to_string(), athlete.to_string(), coach.to_string());
    let published_at = repos
        .training_plans
        .last_published_draft_at(DraftOwner {
            tenant_id: &tenant_key,
            user_id: &athlete_key,
            coach_user_id: &coach_key,
        })
        .await?;
    let thread = RosterPlanThread {
        draft_updated_at: draft.as_ref().map(|(state, _)| state.updated_at),
        draft_has_conflicts: draft.as_ref().is_some_and(|(state, _)| !state.is_clear()),
        published_at,
        draft_phases: draft
            .as_ref()
            .and_then(|(_, plan)| plan.as_ref())
            .map(|plan| roster_phases(&plan.phases))
            .unwrap_or_default(),
    };
    let signals = draft.map(|(state, _)| DraftSignals {
        has_conflicts: !state.is_clear(),
        draft_id: state.draft_id,
        updated_at: state.updated_at,
    });
    Ok((thread, signals))
}

/// The agent's replies this week, per athlete.
#[derive(Debug, Default)]
struct WeekReplies {
    by_athlete: HashMap<Uuid, RosterAgentReplies>,
    /// Each reply, as the attention queue reads it.
    signals: HashMap<Uuid, Vec<ReplySignal>>,
    capped: bool,
}

impl WeekReplies {
    fn for_athlete(&self, athlete: Uuid) -> RosterAgentReplies {
        let mut counts = self.by_athlete.get(&athlete).copied().unwrap_or_default();
        counts.capped = self.capped;
        counts
    }

    fn signals_for(&self, athlete: Uuid) -> &[ReplySignal] {
        self.signals.get(&athlete).map_or(&[], Vec::as_slice)
    }

    fn add(&mut self, reply: &CoachReplyView) {
        let Ok(athlete) = Uuid::parse_str(&reply.athlete_user_id) else {
            return;
        };
        if let Ok(sent_at) = DateTime::parse_from_rfc3339(&reply.created_at) {
            self.signals.entry(athlete).or_default().push(ReplySignal {
                reply_id: reply.id.clone(),
                group_id: reply.group_id.clone(),
                group_name: reply.group_name.clone(),
                sent_at: sent_at.with_timezone(&Utc),
                figures: reply.unmatched_figure_claims.clone(),
                corrected: !reply.corrections.is_empty(),
            });
        }
        let flagged = reply.verdicts.iter().any(|v| {
            v.status == ClaimStatus::Contradicted.as_str()
                || v.status == ClaimStatus::Unsupported.as_str()
        });
        let counts = self.by_athlete.entry(athlete).or_default();
        counts.count += 1;
        counts.unmatched_figures += reply.unmatched_figures;
        // A withheld reply carries no verdicts to this caller: unread, never
        // verified.
        if !reply.withheld && !flagged && reply.unmatched_figures == 0 {
            counts.verified += 1;
        }
    }
}

/// Read the reply review newest first, back to this week's Monday, through
/// the same service and checker rules as `GET /api/coach/replies`.
async fn replies_this_week(resources: &ServerContext, coach: Uuid) -> AppResult<WeekReplies> {
    let since = coach_replies::week_start(Utc::now().date_naive())
        .and_time(NaiveTime::MIN)
        .and_utc();
    let mut week = WeekReplies::default();
    let mut before = None;
    for _ in 0..MAX_REPLY_PAGES {
        let page = coach_replies::list_replies(
            &resources.common.repos,
            &resources.mcp.evidence_registry,
            coach,
            ReplyPageQuery {
                before,
                limit: Some(REPLY_PAGE_SIZE),
                ..ReplyPageQuery::default()
            },
        )
        .await?;
        for reply in &page.replies {
            let sent = DateTime::parse_from_rfc3339(&reply.created_at)
                .map(|t| t.with_timezone(&Utc))
                .ok();
            if sent.is_some_and(|t| t < since) {
                return Ok(week);
            }
            week.add(reply);
        }
        match page.next_before.as_deref().map(Uuid::parse_str) {
            Some(Ok(next)) => before = Some(next),
            _ => return Ok(week),
        }
    }
    week.capped = true;
    Ok(week)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_season_week_counts_from_one_and_stops_at_the_season() {
        let start = day("2026-10-05");
        assert_eq!(
            season_week(start, day("2026-10-04"), 10),
            None,
            "the day before the season is in no week"
        );
        assert_eq!(
            season_week(start, day("2026-10-05"), 10),
            Some(1),
            "the season's first day is week 1"
        );
        assert_eq!(
            season_week(start, day("2026-10-11"), 10),
            Some(1),
            "the seventh day is still week 1"
        );
        assert_eq!(
            season_week(start, day("2026-10-12"), 10),
            Some(2),
            "the eighth day opens week 2"
        );
        assert_eq!(
            season_week(start, day("2026-12-13"), 10),
            Some(10),
            "the season's last day is its last week"
        );
        assert_eq!(
            season_week(start, day("2026-12-14"), 10),
            None,
            "the day after the season is in no week"
        );
    }
}
