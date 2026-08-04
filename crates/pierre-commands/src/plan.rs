// ABOUTME: Handler for /plan — deterministic read-only display of the athlete's stored training plan
// ABOUTME: Bare = goal + countdown + today & tomorrow; `week` = current week; `today` = today only
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PLAN_BLOCK_LINE, KEY_PLAN_DAY_LINE, KEY_PLAN_EMPTY,
    KEY_PLAN_GOAL_LINE, KEY_PLAN_NO_SESSION, KEY_PLAN_REST, KEY_PLAN_STALE_GOAL, KEY_PLAN_TODAY,
    KEY_PLAN_TOMORROW, KEY_PLAN_TRUNCATED, KEY_PLAN_WEEK_HEADER,
};
use pierre_core::errors::AppError;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, PlannedDay, TrainingPlan};
use pierre_messaging::commands::CommandResponse;
use pierre_services::training_plan_render::{
    plan_goal_is_stale, select_active_weeks, SelectedWeek,
};
use std::fmt::Write as _;

use crate::{CommandHandler, PlatformCommandContext};

/// Weeks the selection may return — `/plan` never shows more than the current
/// and next, matching the prompt-side block.
const PLAN_WEEK_LIMIT: usize = 2;

/// Outbound text budget for a `/plan` reply.
///
/// The cross-channel floor: Discord and Messenger both cap text at 2000
/// characters, against Telegram/WhatsApp's 4096 and Slack's 40000 (canot's
/// `ChannelDescriptor::max_message_length`). No chunking infrastructure exists
/// anywhere in the send path, so a reply over the channel's limit is dropped by
/// the platform rather than split — truncating at the floor is what makes
/// `/plan` safe on all five channels.
///
/// Deliberately the floor and not the per-channel value: the descriptor lookup
/// is feature-gated per channel and the resolved limit belongs to the transport,
/// not to this handler. Threading it through `PlatformCommandContext` is the
/// follow-up that lets Telegram and Slack use their real headroom.
const PLAN_TEXT_BUDGET: usize = 2_000;

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

/// Render one day as `"<label>: <session>"`.
fn render_day(
    reg: &MessagingStringsRegistry,
    locale: &str,
    label: &str,
    day: Option<&PlannedDay>,
) -> String {
    let session = match day {
        None => reg.render(KEY_PLAN_NO_SESSION, locale, &[]),
        Some(d) if d.is_rest() => reg.render(KEY_PLAN_REST, locale, &[]),
        Some(d) => {
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
    // The block containing today, when the outline has one. A block whose span
    // runs past `NaiveDate::MAX` holds no calendar date, today included, so it
    // is passed over the same way an unparseable start is — `parse_plan_date`
    // is a format check and `weeks` reaches 255, so the end is not always a
    // date that exists.
    if let Some(block) = plan.blocks.iter().find(|b| {
        parse_plan_date(&b.start).is_some_and(|start| {
            start
                .checked_add_days(chrono::Days::new(u64::from(b.weeks) * 7))
                .is_some_and(|end| (start..end).contains(&today))
        })
    }) {
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
    let day_across = |date: NaiveDate| weeks.iter().find_map(|w| w.day_on(date));
    match view {
        PlanView::Today => {
            let label = reg.render(KEY_PLAN_TODAY, locale, &[]);
            let _ = writeln!(
                out,
                "{}",
                render_day(reg, locale, &label, day_across(today))
            );
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
        }
        PlanView::Week => {
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
                let _ = writeln!(out, "{}", render_day(reg, locale, &day.date, Some(day)));
            }
        }
    }
    out
}

/// Truncate to the outbound budget on a line boundary, appending the marker.
///
/// Cutting mid-line would leave a half-written session that reads as real
/// prescription, so the cut lands between days.
fn cap_to_budget(reg: &MessagingStringsRegistry, locale: &str, body: String) -> String {
    if body.chars().count() <= PLAN_TEXT_BUDGET {
        return body;
    }
    let marker = reg.render(KEY_PLAN_TRUNCATED, locale, &[]);
    let room = PLAN_TEXT_BUDGET.saturating_sub(marker.chars().count());
    let mut kept = String::with_capacity(room);
    for line in body.lines() {
        if kept.chars().count() + line.chars().count() + 1 > room {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    format!("{}{marker}", kept.trim_end())
}

#[async_trait]
impl CommandHandler for PlanShowHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let repos = ctx.ctx.repos();
        let user = ctx.user_id.to_string();
        let tenant = ctx.tenant_id.to_string();

        // "Today" is the athlete's civil date, not the server's UTC one — a
        // 23:30 EDT `/plan` must show today's session, not tomorrow's.
        let timezone = repos
            .users
            .get_global(ctx.user_id)
            .await
            .ok()
            .flatten()
            .and_then(|u| u.timezone);
        let today = timezone
            .as_deref()
            .and_then(|tz| tz.parse::<chrono_tz::Tz>().ok())
            .map_or_else(
                || chrono::Utc::now().date_naive(),
                |tz| chrono::Utc::now().with_timezone(&tz).date_naive(),
            );

        // The conversation's coach is authoritative, matching how the plan was
        // saved; a conversation without one falls back to the coach-agnostic plan.
        let coach = match ctx.conversation_id.as_deref() {
            Some(cid) => repos
                .chat
                .get_conversation(cid, &user, ctx.tenant_id)
                .await?
                .and_then(|c| c.coach_id),
            None => None,
        };

        let Some(plan) = repos
            .training_plans
            .get_active_plan(&tenant, &user, coach.as_deref())
            .await?
        else {
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_PLAN_EMPTY,
                &ctx.locale,
                &[],
            )));
        };

        let stored: Vec<PlanWeek> = repos
            .training_plans
            .list_plan_weeks(&tenant, &user, &plan.id, false)
            .await?;
        let selection = select_active_weeks(&stored, today, PLAN_WEEK_LIMIT);

        let view = PlanView::parse(ctx.args.first().map(|s| s.trim().to_lowercase()).as_deref());
        let mut body = render_header(reg, &ctx.locale, &plan, today);
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

        Ok(CommandResponse::rich_text(cap_to_budget(
            reg,
            &ctx.locale,
            body,
        )))
    }
}
