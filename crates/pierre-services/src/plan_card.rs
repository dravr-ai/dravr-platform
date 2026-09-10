// ABOUTME: The workout_plan block's payload — the saved plan projected for a card: phases with the current one flagged, weeks of dated days with their steps
// ABOUTME: One vocabulary for a session's structure, WorkoutStep, shared with the prompt's plan block and the calendar push
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The plan card
//!
//! What a client renders when a plan was saved on the turn: the outline's
//! season — goal race, flavour in the athlete's words, the phases laid on a
//! timeline with the current one flagged — and the weeks worth showing today,
//! each day with the steps `save_training_plan` persisted.
//!
//! The payload is a projection of the stored plan, never a document the model
//! wrote: the same `PlannedDay { steps, fueling, template_slug }` the prompt's
//! plan block summarises and the calendar push writes to a provider, so the
//! card, the prompt and the calendar cannot disagree about a session. A
//! `PlanWeek` row carries tenant and user ids the card has no use for, which
//! is why this is a projection and not the row's own serde shape. A
//! [`GoalRace`] carries neither, so the card serialises the stored race
//! itself rather than a same-shaped copy of it.

use chrono::NaiveDate;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::periodization::{PhaseKind, WorkoutStep};
use pierre_core::models::{FuelingProtocol, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{
    parse_plan_date, GoalRace, PlanWeek, PlannedDay, SelectedBy, TemplateSource, TrainingPlan,
};
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

use crate::training_plan_render::{select_active_weeks, ACTIVE_WEEKS};

/// The tool whose call produced the plan the card projects, named as the
/// block's `source_tool`.
pub const PLAN_CARD_SOURCE_TOOL: &str = "save_training_plan";

/// The tool that reads a plan back, so an athlete who asks to see their
/// season gets the card rather than the agent's paraphrase of it.
pub const PLAN_CARD_READ_TOOL: &str = "get_training_plan";

/// `true` when one of this turn's tool calls puts the card on the reply.
///
/// The save and the read both do: a plan the athlete just agreed to and a
/// plan they asked to see are the same object, and the card is how the
/// platform shows it either way.
#[must_use]
pub fn turn_shows_the_plan(tools_called: &[String]) -> bool {
    tools_called
        .iter()
        .any(|tool| tool == PLAN_CARD_SOURCE_TOOL || tool == PLAN_CARD_READ_TOOL)
}

/// The card, serialised as the `plan` of a `workout_plan` block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanCard {
    /// The race the season is aimed at.
    pub goal_race: GoalRace,
    /// The other races on the calendar, in the order the outline stores
    /// them. The athlete named them, so the card shows them: a season is a
    /// calendar, not a single date.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub races: Vec<GoalRace>,
    /// First day of the season, when the outline states one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season_start: Option<String>,
    /// Day after the last day of the season, when the outline states one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season_end: Option<String>,
    /// The flavour the season runs on, in the athlete's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flavour: Option<FlavourCard>,
    /// The season's phases in order.
    pub phases: Vec<PhaseCard>,
    /// Index into `phases` of the one covering `today`, when one does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_phase_index: Option<usize>,
    /// The weeks worth showing today, in calendar order.
    pub weeks: Vec<WeekCard>,
    /// Future weeks the plan holds beyond `weeks`.
    pub weeks_deferred: usize,
}

/// The flavour, id and athlete-facing label side by side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlavourCard {
    /// Catalogue id.
    pub id: String,
    /// The plain-words label in the athlete's locale, or the id when the
    /// catalogue has no row for it.
    pub label: String,
    /// Who chose it.
    pub selected_by: SelectedBy,
}

/// One phase on the season timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhaseCard {
    /// The phase kind.
    pub kind: PhaseKind,
    /// First day, `YYYY-MM-DD`.
    pub start: String,
    /// Day after the last day, `YYYY-MM-DD`, when the start parses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// Length in weeks.
    pub weeks: u8,
    /// What the phase is for.
    pub purpose: String,
    /// The coach's intent for it.
    pub intent: String,
    /// Weekly hours the phase targets, when stated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_hours: Option<f32>,
    /// Hard sessions a week the phase allows, when stated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hard_sessions_max: Option<u32>,
    /// `true` for the phase covering today.
    pub current: bool,
}

/// One week of the card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeekCard {
    /// Monday, `YYYY-MM-DD`.
    pub week_start: String,
    /// The week's focus in the coach's words.
    pub focus: String,
    /// Index into `phases`, when the week names its phase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_index: Option<u32>,
    /// `true` when today falls in this week.
    pub current: bool,
    /// The week's days in order.
    pub days: Vec<DayCard>,
}

/// One day of a week.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayCard {
    /// Civil date, `YYYY-MM-DD`.
    pub date: String,
    /// Sport label.
    pub sport: String,
    /// The session in the coach's words.
    pub workout: String,
    /// Planned duration in minutes, when stated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_min: Option<u32>,
    /// Intensity label as the coach wrote it.
    pub intensity: String,
    /// `true` for a rest day.
    pub rest: bool,
    /// The session's structure, when the day carries one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkoutStep>,
    /// What to take in, when prescribed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fueling: Option<FuelingProtocol>,
    /// The template the day instantiates, when it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_slug: Option<String>,
    /// Which tier the template came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_source: Option<TemplateSource>,
}

impl PlanCard {
    /// Project a stored plan and its weeks for `today`.
    ///
    /// `registry` and `locale` name the plan's flavour in the athlete's own
    /// words; they are read only when the plan carries a flavour.
    #[must_use]
    pub fn build(
        plan: &TrainingPlan,
        weeks: &[PlanWeek],
        today: NaiveDate,
        registry: &MessagingStringsRegistry,
        locale: &str,
    ) -> Self {
        let phases: Vec<PhaseCard> = plan
            .phases
            .iter()
            .map(|p| PhaseCard {
                kind: p.kind,
                start: p.start.clone(),
                end: p.end_exclusive().map(|d| d.format("%Y-%m-%d").to_string()),
                weeks: p.weeks,
                purpose: p.purpose.clone(),
                intent: p.intent.clone(),
                target_hours: p.target_hours,
                hard_sessions_max: p.hard_sessions_max,
                current: p.covers(today),
            })
            .collect();
        let current_phase_index = phases.iter().position(|p| p.current);
        let selection = select_active_weeks(weeks, today, ACTIVE_WEEKS);
        Self {
            goal_race: plan.goal_race.clone(),
            // Races still ahead only: the calendar outlives a re-save now,
            // so a race that has been run stays stored, and the card would
            // otherwise offer it to the athlete as one to come.
            races: plan
                .races
                .iter()
                .filter(|race| parse_plan_date(&race.date).is_none_or(|date| date >= today))
                .cloned()
                .collect(),
            season_start: plan.season_start.clone(),
            season_end: plan.season_end.clone(),
            flavour: plan.flavour.as_ref().map(|f| FlavourCard {
                id: f.id.clone(),
                label: flavour_label(registry, locale, &f.id),
                selected_by: f.selected_by,
            }),
            phases,
            current_phase_index,
            weeks: selection
                .weeks
                .iter()
                .map(|w| WeekCard {
                    week_start: w.week.week_start.clone(),
                    focus: w.week.focus.clone(),
                    phase_index: w.week.phase_index,
                    current: w.is_current,
                    days: w.week.days.iter().map(day_card).collect(),
                })
                .collect(),
            weeks_deferred: selection.deferred,
        }
    }

    /// The `workout_plan` block carrying this card, as one entry of a
    /// `content_blocks` array.
    ///
    /// # Errors
    ///
    /// Returns the serialisation error, which no card built by [`Self::build`]
    /// produces.
    /// `source_tool` names the tool that actually ran, which is the whole
    /// point of the field: a block whose `source_tool` did not run in its
    /// turn is refused elsewhere precisely because its numbers would be
    /// unattributable. Claiming the save on a turn that only read the plan
    /// asserted a provenance that was false every time.
    pub fn as_block(&self, source_tool: &str) -> Result<serde_json::Value, serde_json::Error> {
        Ok(serde_json::json!({
            "type": "workout_plan",
            "source_tool": source_tool,
            "plan": serde_json::to_value(self)?,
        }))
    }
}

fn day_card(day: &PlannedDay) -> DayCard {
    DayCard {
        date: day.date.clone(),
        sport: day.sport.clone(),
        workout: day.workout.clone(),
        duration_min: day.duration_min,
        intensity: day.intensity.clone(),
        rest: day.is_rest(),
        steps: day.steps.clone(),
        fueling: day.fueling.clone(),
        template_slug: day.template_slug.clone(),
        template_source: day.template_source,
    }
}

/// The athlete-facing label of a flavour, in one locale.
///
/// Plain words — "mostly easy with two hard days", never "polarized" — from
/// the string catalogue under `messaging.flavour.<id>`, the hyphens of the id
/// folded to underscores. A flavour the catalogue has no words for (a coach
/// package's house flavour) is named by its id, which is at least honest.
///
/// The card and `recommend_plan_flavour` both call this, so the label the
/// tool ranks a flavour under and the label the card prints are the same
/// words by construction.
#[must_use]
pub fn flavour_label(registry: &MessagingStringsRegistry, locale: &str, id: &str) -> String {
    let key = format!("messaging.flavour.{}", id.replace('-', "_"));
    let label = registry.get(&key, locale);
    if label.is_empty() {
        id.to_owned()
    } else {
        label
    }
}

/// Load the athlete's active plan under `coach` and project it for the card.
///
/// `None` when there is no active plan or the store cannot be read — a card
/// is a courtesy on the reply, never a reason to fail the turn.
pub async fn load_plan_card(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    coach: Option<&str>,
    today: NaiveDate,
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> Option<PlanCard> {
    let tenant_id = tenant.to_string();
    let user = user_id.to_string();
    let plan = match repos
        .training_plans
        .get_active_plan(&tenant_id, &user, coach)
        .await
    {
        Ok(Some(plan)) => plan,
        Ok(None) => return None,
        Err(e) => {
            warn!(error = %e, "plan card: active plan unreadable");
            return None;
        }
    };
    let weeks = match repos
        .training_plans
        .list_plan_weeks(&tenant_id, &user, &plan.id, false)
        .await
    {
        Ok(weeks) => weeks,
        Err(e) => {
            warn!(error = %e, "plan card: plan weeks unreadable");
            return None;
        }
    };
    Some(PlanCard::build(&plan, &weeks, today, registry, locale))
}
