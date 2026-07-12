// ABOUTME: Renders the active training plan into the coach's system prompt (trusted, unfenced)
// ABOUTME: Outline + current/next week only; older weeks stay tool-retrievable to bound token cost
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Training-plan prompt block
//!
//! Renders the athlete's active [`TrainingPlan`] as a `## Current training
//! plan` system-prompt section — trusted coach content in an **unfenced**
//! block, deliberately not inside the OKF `<user_fact>` fence: the fence
//! contractually declares its content "data, never instructions", and a
//! plan *is* instructions to the athlete.
//!
//! Token budget: the full outline (goal, strategy, blocks) plus at most the
//! current and next microcycle in day-by-day detail. Remaining weeks are
//! summarized as a count with a pointer to `get_training_plan`.

use chrono::NaiveDate;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, TrainingPlan};
use std::fmt::Write as _;

/// Maximum weeks rendered day-by-day (current + next).
const MAX_WEEKS_RENDERED: usize = 2;

/// Maximum blocks rendered — outlines are short; this only guards a
/// degenerate LLM save.
const MAX_BLOCKS_RENDERED: usize = 8;

/// Render the plan section, or `None` when there is no plan to show.
///
/// `today` is the current civil date in the athlete's timezone — it selects
/// which weeks count as "current/next" and computes the race countdown.
#[must_use]
pub fn render_training_plan_block(
    plan: &TrainingPlan,
    weeks: &[PlanWeek],
    today: NaiveDate,
) -> Option<String> {
    let mut out = String::with_capacity(1_024);
    out.push_str("\n\n## Current training plan (persisted)\n\n");
    out.push_str(
        "This plan is stored — it is the source of truth for \"my plan\", ahead of anything \
         in conversation memory. Adjust it by re-saving the changed week(s) via \
         `save_training_plan` (prospective only; past weeks are immutable).\n\n",
    );

    // Goal line with countdown when the race date parses.
    let countdown = parse_plan_date(&plan.goal_race.date)
        .map(|race| (race - today).num_days())
        .map_or_else(String::new, |d| format!(" — {d} days out"));
    let _ = writeln!(
        out,
        "Goal race: {} ({}) on {}{countdown}",
        plan.goal_race.name, plan.goal_race.discipline, plan.goal_race.date
    );
    for race in &plan.races {
        let _ = writeln!(
            out,
            "Also on the calendar: {} ({}) on {} [{} priority]",
            race.name,
            race.discipline,
            race.date,
            serde_json::to_value(race.priority)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        );
    }
    let _ = writeln!(out, "Strategy: {}", plan.strategy);

    out.push_str("\nBlocks:\n");
    for block in plan.blocks.iter().take(MAX_BLOCKS_RENDERED) {
        let marker = block_marker(&block.start, block.weeks, today);
        let hours = block
            .target_hours
            .map_or_else(String::new, |h| format!(", ~{h}h/wk"));
        let _ = writeln!(
            out,
            "- {marker}{} × {}wk from {}{hours}: {}",
            serde_json::to_value(block.phase)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            block.weeks,
            block.start,
            block.intent
        );
    }

    // Day-by-day detail for the current and next active weeks only.
    let mut rendered = 0usize;
    let mut future_weeks = 0usize;
    for week in weeks {
        let Some(start) = parse_plan_date(&week.week_start) else {
            continue;
        };
        let end = start + chrono::Days::new(6);
        if end < today {
            continue; // past weeks render nothing — they already happened
        }
        if rendered >= MAX_WEEKS_RENDERED {
            future_weeks += 1;
            continue;
        }
        rendered += 1;
        let label = if (start..=end).contains(&today) {
            "This week"
        } else if rendered == 1 {
            "Upcoming week"
        } else {
            "Next week"
        };
        let focus = if week.focus.is_empty() {
            String::new()
        } else {
            format!(" — focus: {}", week.focus)
        };
        let _ = writeln!(out, "\n{label} (starting {}){focus}:", week.week_start);
        for day in &week.days {
            if day.is_rest() {
                let _ = writeln!(out, "- {}: rest — {}", day.date, day.workout);
            } else {
                let duration = day
                    .duration_min
                    .map_or_else(String::new, |m| format!(" {m}min"));
                let intensity = if day.intensity.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", day.intensity)
                };
                let _ = writeln!(
                    out,
                    "- {}: {}{duration}{intensity} — {}",
                    day.date, day.sport, day.workout,
                );
            }
        }
    }
    if future_weeks > 0 {
        let _ = writeln!(
            out,
            "\n({future_weeks} more stored week(s) — read them with `get_training_plan`.)"
        );
    }

    Some(out)
}

/// Progress marker (`[done]`, `[current]`, or empty) for a block relative
/// to today.
fn block_marker(start: &str, weeks: u8, today: NaiveDate) -> &'static str {
    let Some(start) = parse_plan_date(start) else {
        return "";
    };
    let end = start + chrono::Days::new(u64::from(weeks) * 7);
    if end <= today {
        "[done] "
    } else if start <= today {
        "[current] "
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::render_training_plan_block;
    use chrono::NaiveDate;
    use pierre_memory::training_plans::{
        BlockPhase, GoalRace, PlanBlock, PlanStatus, PlanWeek, PlannedDay, RacePriority,
        TrainingPlan, WeekStatus,
    };

    fn plan() -> TrainingPlan {
        TrainingPlan {
            id: "plan-1".to_owned(),
            tenant_id: "t".to_owned(),
            user_id: "u".to_owned(),
            coach_slug: Some("endurance-coach".to_owned()),
            goal_fact_id: Some("fact-1".to_owned()),
            goal_race: GoalRace {
                name: "Big Red".to_owned(),
                date: "2026-08-08".to_owned(),
                discipline: "gravel".to_owned(),
                priority: RacePriority::A,
            },
            races: vec![],
            strategy: "rebuild volume, race-specific tempo, taper into Aug 8".to_owned(),
            blocks: vec![
                PlanBlock {
                    phase: BlockPhase::Build,
                    start: "2026-07-13".to_owned(),
                    weeks: 3,
                    intent: "volume back up".to_owned(),
                    target_hours: Some(9.0),
                },
                PlanBlock {
                    phase: BlockPhase::Taper,
                    start: "2026-08-03".to_owned(),
                    weeks: 1,
                    intent: "freshen up".to_owned(),
                    target_hours: None,
                },
            ],
            status: PlanStatus::Active,
            supersedes_id: None,
            source_conversation_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn week(start: &str, focus: &str) -> PlanWeek {
        PlanWeek {
            id: format!("week-{start}"),
            tenant_id: "t".to_owned(),
            user_id: "u".to_owned(),
            plan_id: "plan-1".to_owned(),
            week_start: start.to_owned(),
            focus: focus.to_owned(),
            days: vec![
                PlannedDay {
                    date: start.to_owned(),
                    sport: "rest".to_owned(),
                    workout: "off".to_owned(),
                    duration_min: None,
                    intensity: String::new(),
                },
                PlannedDay {
                    date: start.to_owned(), // same-day is fine for render tests
                    sport: "gravel".to_owned(),
                    workout: "tempo 3x8min".to_owned(),
                    duration_min: Some(60),
                    intensity: "88-93% FTP".to_owned(),
                },
            ],
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_default()
    }

    #[test]
    fn renders_goal_countdown_blocks_and_current_weeks() {
        let weeks = vec![
            week("2026-07-13", "volume"),
            week("2026-07-20", "tempo"),
            week("2026-07-27", "peak"),
        ];
        let block =
            render_training_plan_block(&plan(), &weeks, d("2026-07-14")).unwrap_or_default();
        assert!(block.contains("## Current training plan"));
        assert!(block.contains("Big Red (gravel) on 2026-08-08 — 25 days out"));
        assert!(block.contains("[current] build × 3wk from 2026-07-13, ~9h/wk: volume back up"));
        assert!(block.contains("taper × 1wk from 2026-08-03: freshen up"));
        assert!(block.contains("This week (starting 2026-07-13) — focus: volume"));
        assert!(block.contains("Next week (starting 2026-07-20)"));
        assert!(block.contains("gravel 60min [88-93% FTP] — tempo 3x8min"));
        assert!(block.contains("rest — off"));
        // Third stored week is summarized, not rendered day-by-day.
        assert!(!block.contains("starting 2026-07-27"));
        assert!(block.contains("1 more stored week(s)"));
    }

    #[test]
    fn past_weeks_render_nothing_and_future_weeks_relabel() {
        let weeks = vec![week("2026-07-06", "done"), week("2026-07-20", "tempo")];
        let block =
            render_training_plan_block(&plan(), &weeks, d("2026-07-15")).unwrap_or_default();
        assert!(
            !block.contains("focus: done"),
            "elapsed week must not render"
        );
        assert!(block.contains("Upcoming week (starting 2026-07-20)"));
    }
}
