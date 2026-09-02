// ABOUTME: Handlers for /plan and /plan share — deterministic read-only display of the athlete's stored training plan
// ABOUTME: Bare = goal + countdown + today & tomorrow; `week` = current week; `today` = today only; share posts it to the room
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PLAN_BLOCK_LINE, KEY_PLAN_DAY_LINE, KEY_PLAN_EMPTY,
    KEY_PLAN_GOAL_LINE, KEY_PLAN_NO_COVERAGE, KEY_PLAN_NO_SESSION, KEY_PLAN_REST, KEY_PLAN_RESUMES,
    KEY_PLAN_SHARED_HEADER, KEY_PLAN_STALE_GOAL, KEY_PLAN_TODAY, KEY_PLAN_TOMORROW,
    KEY_PLAN_WEEK_HEADER,
};
use pierre_core::errors::AppError;
use pierre_core::models::User;
use pierre_memory::training_plans::{
    parse_plan_date, PlanBlock, PlanWeek, PlannedDay, TrainingPlan,
};
use pierre_messaging::commands::CommandResponse;
use pierre_messaging::rich_text::escape_markdown;
use pierre_services::training_plan_render::{
    plan_goal_is_stale, resolve_plan_coach_slug, select_active_weeks, SelectedWeek,
};
use std::fmt::Write as _;

use crate::{CallerGroupStanding, CommandHandler, PlatformCommandContext};

/// Weeks the selection may return — `/plan` never shows more than the current
/// and next, matching the prompt-side block.
const PLAN_WEEK_LIMIT: usize = 2;

/// What the athlete asked to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanView {
    /// Bare `/plan` — goal, countdown, current block, today and tomorrow.
    Compact,
    /// `/plan week` — the current week day by day.
    Week,
    /// `/plan today` — today's session only.
    Today,
}

impl PlanView {
    /// Parse the subcommand; anything unrecognized reads as the compact view.
    fn parse(arg: Option<&str>) -> Self {
        match arg {
            Some("week") => Self::Week,
            Some("today") => Self::Today,
            _ => Self::Compact,
        }
    }
}

/// Handler for `/plan` — show the stored training plan.
///
/// Deterministic and read-only: no LLM round-trip, per [[ADR-003]]'s rule that a
/// slash command answers from data. Plan *generation* stays conversational, so
/// this command never writes and never asks a coach to build anything.
pub struct PlanShowHandler;

/// Handler for `/plan share` — the same read as `/plan`, posted to the room.
///
/// Bare `/plan` is answered privately in a shared room because a plan is the
/// caller's own state. Typing the share variant is the athlete's consent to
/// post it, granted per invocation and legible in the room as their own turn;
/// on a messaging room the reply opens with a header naming whose plan it is.
/// Only ever the caller's own plan: nobody can publish another athlete's plan
/// into a room, which is the leak the private default exists to prevent.
pub struct PlanShareHandler;

/// What the selected weeks know about one date.
///
/// The distinction the athlete needs is between a plan that deliberately asks
/// for nothing and a plan that simply does not reach this date — the two used
/// to collapse into the same "nothing scheduled" line, which is how a hole in
/// the stored weeks read as a working plan with an easy day in it.
#[derive(Debug, Clone, Copy)]
enum DayStatus<'a> {
    /// A stored week spans the date and prescribes this session.
    Session(&'a PlannedDay),
    /// A stored week spans the date and prescribes nothing on it.
    Unscheduled,
    /// No stored week spans the date at all.
    Uncovered,
}

/// Classify `date` against the selected weeks.
///
/// Looks for a prescribed day first, then falls back to asking whether any
/// selected week merely *spans* the date — a week can cover a date without
/// listing it, which is a real rest day rather than a gap.
fn day_status<'a>(weeks: &'a [SelectedWeek<'a>], date: NaiveDate) -> DayStatus<'a> {
    weeks.iter().find_map(|w| w.day_on(date)).map_or_else(
        || {
            if weeks.iter().any(|w| (w.start..=w.end).contains(&date)) {
                DayStatus::Unscheduled
            } else {
                DayStatus::Uncovered
            }
        },
        DayStatus::Session,
    )
}

/// The outline block whose span contains `date`, when the outline has one.
///
/// A block whose span runs past `NaiveDate::MAX` holds no calendar date, so it
/// is passed over the same way an unparseable start is — `parse_plan_date` is a
/// format check and `weeks` reaches 255, so the end is not always a date that
/// exists.
fn block_on(plan: &TrainingPlan, date: NaiveDate) -> Option<&PlanBlock> {
    plan.blocks.iter().find(|b| {
        parse_plan_date(&b.start).is_some_and(|start| {
            start
                .checked_add_days(chrono::Days::new(u64::from(b.weeks) * 7))
                .is_some_and(|end| (start..end).contains(&date))
        })
    })
}

/// The first selected week that starts after `today`, i.e. where the plan picks
/// up again when today itself is not covered.
fn resume_week<'a>(
    weeks: &'a [SelectedWeek<'a>],
    today: NaiveDate,
) -> Option<&'a SelectedWeek<'a>> {
    weeks.iter().find(|w| w.start > today)
}

/// Render one day as `"<label>: <session>"`.
fn render_day(
    reg: &MessagingStringsRegistry,
    locale: &str,
    label: &str,
    day: DayStatus<'_>,
) -> String {
    let session = match day {
        DayStatus::Uncovered => reg.render(KEY_PLAN_NO_COVERAGE, locale, &[]),
        DayStatus::Unscheduled => reg.render(KEY_PLAN_NO_SESSION, locale, &[]),
        DayStatus::Session(d) if d.is_rest() => reg.render(KEY_PLAN_REST, locale, &[]),
        DayStatus::Session(d) => {
            let duration = d
                .duration_min
                .map_or_else(String::new, |m| format!(" {m}min"));
            let intensity = if d.intensity.is_empty() {
                String::new()
            } else {
                format!(" [{}]", d.intensity)
            };
            format!("{}{duration}{intensity} — {}", d.sport, d.workout)
        }
    };
    reg.render(KEY_PLAN_DAY_LINE, locale, &[label, &session])
}

/// The goal line plus the current block, shared by every view.
fn render_header(
    reg: &MessagingStringsRegistry,
    locale: &str,
    plan: &TrainingPlan,
    weeks: &[SelectedWeek<'_>],
    today: NaiveDate,
) -> String {
    let mut out = String::with_capacity(256);
    let days_out = parse_plan_date(&plan.goal_race.date)
        .map_or_else(String::new, |race| (race - today).num_days().to_string());
    let _ = writeln!(
        out,
        "{}",
        reg.render(
            KEY_PLAN_GOAL_LINE,
            locale,
            &[&plan.goal_race.name, &plan.goal_race.date, &days_out],
        )
    );
    // The block containing today. When today sits in a gap the outline does not
    // phase, fall back to the block covering the first week actually being
    // shown: the header describes the plan the athlete is about to read, and
    // dropping the line entirely is how the phase context went silently missing
    // whenever the plan resumed in the future.
    if let Some(block) =
        block_on(plan, today).or_else(|| weeks.first().and_then(|w| block_on(plan, w.start)))
    {
        let phase = block.phase.as_str();
        let hours = block
            .target_hours
            .map_or_else(String::new, |h| format!(", ~{h}h/wk"));
        let _ = writeln!(
            out,
            "{}",
            reg.render(KEY_PLAN_BLOCK_LINE, locale, &[phase, &hours])
        );
    }
    out
}

/// Render the requested view over the selected weeks.
fn render_view(
    reg: &MessagingStringsRegistry,
    locale: &str,
    view: PlanView,
    weeks: &[SelectedWeek<'_>],
    today: NaiveDate,
) -> String {
    let mut out = String::with_capacity(512);
    // Today and tomorrow can straddle a week boundary, so look them up across
    // the whole selection rather than only the current week.
    let day_across = |date: NaiveDate| day_status(weeks, date);
    // Naming the date the plan picks up again is what turns "not covered" from
    // a dead end into an answer. Only worth saying when today itself is the
    // uncovered date — a plan that already has today in hand needs no pointer.
    let append_resume = |out: &mut String| {
        if matches!(day_across(today), DayStatus::Uncovered) {
            if let Some(week) = resume_week(weeks, today) {
                let _ = writeln!(
                    out,
                    "{}",
                    reg.render(KEY_PLAN_RESUMES, locale, &[&week.week.week_start])
                );
            }
        }
    };
    match view {
        PlanView::Today => {
            let label = reg.render(KEY_PLAN_TODAY, locale, &[]);
            let _ = writeln!(
                out,
                "{}",
                render_day(reg, locale, &label, day_across(today))
            );
            append_resume(&mut out);
        }
        PlanView::Compact => {
            let today_label = reg.render(KEY_PLAN_TODAY, locale, &[]);
            let tomorrow_label = reg.render(KEY_PLAN_TOMORROW, locale, &[]);
            let tomorrow = today + chrono::Days::new(1);
            let _ = writeln!(
                out,
                "{}",
                render_day(reg, locale, &today_label, day_across(today))
            );
            let _ = writeln!(
                out,
                "{}",
                render_day(reg, locale, &tomorrow_label, day_across(tomorrow))
            );
            append_resume(&mut out);
        }
        PlanView::Week => {
            // A week block carries a date, not a tense, so on its own it cannot
            // tell the athlete whether they are reading the week they are in or
            // one that has not started — seven sessions under a future date
            // read exactly like this week's. Leading with today's status says
            // which, and it is the only thing this view has to say at all when
            // every stored week has ended and the selection comes back empty.
            if matches!(day_across(today), DayStatus::Uncovered) {
                let label = reg.render(KEY_PLAN_TODAY, locale, &[]);
                let _ = writeln!(
                    out,
                    "{}",
                    render_day(reg, locale, &label, DayStatus::Uncovered)
                );
                append_resume(&mut out);
            }
            // The current week if today falls in one, else the first upcoming.
            let Some(week) = weeks
                .iter()
                .find(|w| w.is_current)
                .or_else(|| weeks.first())
            else {
                return out;
            };
            let focus = if week.week.focus.is_empty() {
                String::new()
            } else {
                format!(" — {}", week.week.focus)
            };
            let _ = writeln!(
                out,
                "{}",
                reg.render(
                    KEY_PLAN_WEEK_HEADER,
                    locale,
                    &[&week.week.week_start, &focus]
                )
            );
            for day in &week.week.days {
                let _ = writeln!(
                    out,
                    "{}",
                    render_day(reg, locale, &day.date, DayStatus::Session(day))
                );
            }
        }
    }
    out
}

/// The caller's global user row, when it can be read. `None` leaves every
/// per-user default in place: UTC for "today", "Unknown" for the name.
async fn caller_record(ctx: &PlatformCommandContext) -> Option<User> {
    ctx.ctx
        .repos()
        .users
        .get_global(ctx.user_id)
        .await
        .ok()
        .flatten()
}

/// The caller's civil date. "Today" is the athlete's, not the server's UTC
/// one — a 23:30 EDT `/plan` must show today's session, not tomorrow's.
fn caller_today(user: Option<&User>) -> NaiveDate {
    user.and_then(|u| u.timezone.as_deref())
        .and_then(|tz| tz.parse::<chrono_tz::Tz>().ok())
        .map_or_else(
            || chrono::Utc::now().date_naive(),
            |tz| chrono::Utc::now().with_timezone(&tz).date_naive(),
        )
}

/// The name a room already knows the caller by: their display name, else the
/// local part of their e-mail — the same fallback the group roster renders,
/// so the header adds no name the room has not seen.
fn caller_display_name(user: Option<&User>) -> String {
    user.map_or_else(
        || "Unknown".to_owned(),
        |u| {
            u.display_name
                .clone()
                .unwrap_or_else(|| u.email.split('@').next().unwrap_or("Unknown").to_owned())
        },
    )
}

/// Render the caller's stored plan in the requested view, or `None` when
/// nothing is saved. Shared by `/plan` and `/plan share`: one body, two
/// deliveries.
///
/// The coach the plan is read under follows the same ladder on every
/// surface: the conversation's coach, read under the tenant that owns the
/// conversation row (a shared room files it under the bot tenant, where the
/// caller's own tenant never finds it); else the coach the athlete selected in
/// their own tenant, which is what their DM binds; else the coach-agnostic
/// plan. Without the second rung, an athlete whose plan was built in their DM
/// under coach X read "no plan saved yet" in the room.
async fn render_plan_reply(
    ctx: &PlatformCommandContext,
    view: PlanView,
    today: NaiveDate,
) -> Result<Option<String>, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let repos = ctx.ctx.repos();
    let user = ctx.user_id.to_string();
    let tenant = ctx.tenant_id.to_string();

    let conversation_coach = match ctx.conversation_id.as_deref() {
        Some(cid) => repos
            .chat
            .get_conversation(cid, &user, ctx.conversation_tenant_id)
            .await?
            .and_then(|c| c.coach_id),
        None => None,
    };
    let coach =
        resolve_plan_coach_slug(repos, conversation_coach, ctx.tenant_id, ctx.user_id).await?;

    let Some(plan) = repos
        .training_plans
        .get_active_plan(&tenant, &user, coach.as_deref())
        .await?
    else {
        return Ok(None);
    };

    let stored: Vec<PlanWeek> = repos
        .training_plans
        .list_plan_weeks(&tenant, &user, &plan.id, false)
        .await?;
    let selection = select_active_weeks(&stored, today, PLAN_WEEK_LIMIT);

    let mut body = render_header(reg, &ctx.locale, &plan, &selection.weeks, today);
    body.push_str(&render_view(
        reg,
        &ctx.locale,
        view,
        &selection.weeks,
        today,
    ));

    // Flag a plan whose goal fact has since been superseded, using the same
    // predicate `get_training_plan` reports `goal_stale` from.
    if let Some(fact_id) = plan.goal_fact_id.as_deref() {
        if plan_goal_is_stale(repos, ctx.tenant_id, &user, fact_id).await? {
            body.push_str(&reg.render(KEY_PLAN_STALE_GOAL, &ctx.locale, &[]));
        }
    }
    Ok(Some(body))
}

/// The view the caller asked for, from the first argument.
fn requested_view(ctx: &PlatformCommandContext) -> PlanView {
    PlanView::parse(ctx.args.first().map(|s| s.trim().to_lowercase()).as_deref())
}

/// The plan body, or the empty-state line when nothing is saved.
async fn plan_body(ctx: &PlatformCommandContext, today: NaiveDate) -> Result<String, AppError> {
    let rendered = render_plan_reply(ctx, requested_view(ctx), today).await?;
    Ok(rendered.unwrap_or_else(|| {
        ctx.ctx
            .messaging_strings_registry()
            .render(KEY_PLAN_EMPTY, &ctx.locale, &[])
    }))
}

#[async_trait]
impl CommandHandler for PlanShowHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let user = caller_record(ctx).await;
        let body = plan_body(ctx, caller_today(user.as_ref())).await?;
        // The whole plan, however long. A body past what one message on this
        // channel carries is split into ordered messages by the egress
        // (`messaging_ingress::block_render::fan_out`), so a twelve-week plan
        // arrives complete instead of stopping at a week boundary with a
        // "truncated" marker.
        Ok(CommandResponse::rich_text(body))
    }
}

#[async_trait]
impl CommandHandler for PlanShareHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let user = caller_record(ctx).await;
        let body = plan_body(ctx, caller_today(user.as_ref())).await?;
        // The header renders only where the reply is actually posted to a
        // room: a messaging channel (a sender id) in a shared chat. A DM has
        // no room to share with, and an in-app group thread persists the
        // reply into the caller's own conversation alone, so claiming "shared
        // with the room" there would misstate its audience.
        if ctx.is_direct_message || ctx.sender_id.is_none() {
            return Ok(CommandResponse::rich_text(body));
        }
        // The header is inline markdown and the name is user-set free text, so
        // it is escaped: one `*` in a display name would otherwise open a run
        // and swallow the rest of the header. Angle brackets need nothing here;
        // each channel escapes its own text nodes when it renders.
        let name = escape_markdown(&caller_display_name(user.as_ref()));
        let header = ctx.ctx.messaging_strings_registry().render(
            KEY_PLAN_SHARED_HEADER,
            &ctx.locale,
            &[&name],
        );
        Ok(CommandResponse::rich_text(format!("{header}\n{body}")))
    }

    /// Listed only where a room exists to share into. In a DM — and in a
    /// palette opened outside any group-bound conversation — the reply
    /// renders exactly like `/plan`, so listing the share variant there
    /// offers two names for one behaviour. What the command ACTS ON is
    /// unchanged: `execute` still answers a DM caller with their plan; this
    /// predicate exists for listing coherence alone.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        !standing.is_direct_message
    }
}
