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
//!
//! A day's fuelling rates are shown only to an athlete with no medical flag
//! on file. For a flagged athlete a day that stores a protocol carries
//! `fueling_withheld` in its place — why the rates are absent and that a
//! clinician sets them — through the same [`FuelingDisclosure`] the prompt
//! block and the calendar push read.
//!
//! In a human coach's thread bound to one athlete (carnet#1070) the card is
//! that athlete's: the coach's draft over their live weeks, or their live
//! season while the coach holds no draft ([`try_load_subject_plan_card`]).
//! Its block carries [`PlanCardDraft`] beside `plan` — the athlete, and where
//! the draft stands — so a client can offer Publish and Discard on it.

use chrono::{Days, NaiveDate};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::periodization::{PhaseKind, WorkoutStep};
use pierre_core::models::{FuelingProtocol, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{
    parse_plan_date, GoalRace, PlanWeek, PlannedDay, SelectedBy, TemplateSource, TrainingPlan,
};
use pierre_providers::ai_scope;
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

use crate::coach_plan_draft::{coach_plan_view, CoachDraftTarget, DraftState};
use crate::medical_flag::FiguresWithheld;
use crate::plan_fueling::FuelingDisclosure;
use crate::training_plan_render::{select_active_weeks, ACTIVE_WEEKS};
use pierre_core::json_value::to_value_as_written;

/// The `type` of a plan card's entry in a `content_blocks` array.
pub const WORKOUT_PLAN_BLOCK_TYPE: &str = "workout_plan";

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
    /// The agent's intent for it.
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
    /// The week's focus in the agent's words.
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
    /// The session in the agent's words.
    pub workout: String,
    /// Planned duration in minutes, when stated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_min: Option<u32>,
    /// Intensity label as the agent wrote it.
    pub intensity: String,
    /// `true` for a rest day.
    pub rest: bool,
    /// The session's structure, when the day carries one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkoutStep>,
    /// What to take in, when prescribed and this athlete may see the rates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fueling: Option<FuelingProtocol>,
    /// In place of `fueling`, for a day that stores a protocol while a
    /// medical flag is on file: why the rates are absent and who sets them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fueling_withheld: Option<FiguresWithheld>,
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
    /// words; they are read only when the plan carries a flavour. `fueling`
    /// decides whether a day's stored rates are shown or withheld.
    #[must_use]
    pub fn build(
        plan: &TrainingPlan,
        weeks: &[PlanWeek],
        today: NaiveDate,
        registry: &MessagingStringsRegistry,
        locale: &str,
        fueling: &FuelingDisclosure,
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
                .map(|w| week_card(w.week, w.is_current, fueling))
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
        self.block(source_tool, None)
    }

    /// The `workout_plan` block carrying this card in a human coach's thread
    /// about one athlete, with `subject` and `draft` beside `plan`.
    ///
    /// # Errors
    ///
    /// The serialisation error of [`Self::as_block`].
    pub fn as_subject_block(
        &self,
        source_tool: &str,
        draft: &PlanCardDraft,
    ) -> Result<serde_json::Value, serde_json::Error> {
        self.block(source_tool, Some(draft))
    }

    fn block(
        &self,
        source_tool: &str,
        draft: Option<&PlanCardDraft>,
    ) -> Result<serde_json::Value, serde_json::Error> {
        to_value_as_written(&WorkoutPlanBlock {
            block_type: WORKOUT_PLAN_BLOCK_TYPE,
            source_tool,
            plan: self,
            draft,
        })
    }
}

/// One `workout_plan` entry of a `content_blocks` array.
#[derive(Serialize)]
struct WorkoutPlanBlock<'a> {
    #[serde(rename = "type")]
    block_type: &'static str,
    source_tool: &'a str,
    plan: &'a PlanCard,
    /// `subject` and `draft`, on a card in a coach's athlete thread only.
    #[serde(flatten)]
    draft: Option<&'a PlanCardDraft>,
}

/// The athlete a card in a human coach's thread shows, by id and roster
/// name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSubject {
    /// The athlete.
    pub user_id: Uuid,
    /// Their roster name
    /// ([`roster_display_name`](pierre_core::models::roster_display_name)).
    pub display_name: String,
}

/// What a card in a human coach's thread about one athlete carries beside
/// its plan: the athlete, and where the coach's draft of their plan stands.
///
/// With a draft the card shows it over the athlete's live weeks; with
/// `draft: null` it shows their live season, no draft open. A client offers
/// Publish and Discard on the first only, and only while its `draft` matches
/// the subject-plan read's — [`DraftState::of`] builds both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCardDraft {
    /// Whose plan the card shows.
    pub subject: PlanSubject,
    /// Where the coach's draft stands; `None` with no draft open.
    #[serde(default)]
    pub draft: Option<DraftState>,
}

/// The card a human coach's thread about one athlete shows, and what it
/// carries beside its plan.
#[derive(Debug, Clone, PartialEq)]
pub struct SubjectPlanCard {
    /// The coach's draft over the athlete's live weeks, or their live season.
    pub card: PlanCard,
    /// The athlete and where the draft stands.
    pub draft: PlanCardDraft,
}

/// The card for a human coach's thread about one athlete.
///
/// The coach's draft over the athlete's live weeks, or the athlete's live
/// season while the coach holds no draft, projected on the athlete's `today`
/// under their own fuelling disclosure. `target` is the pair the coaching gates just passed
/// ([`coached_athlete_access`](crate::coached_athlete::coached_athlete_access));
/// nothing here re-checks them. `Ok(None)` when the athlete has no live
/// season and the coach no draft.
///
/// # Errors
///
/// The errors of [`coach_plan_view`] — a store that cannot be read, a plan
/// an external caller may not read — and the medical flag's read.
pub async fn try_load_subject_plan_card(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    subject: PlanSubject,
    today: NaiveDate,
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> AppResult<Option<SubjectPlanCard>> {
    let view = coach_plan_view(repos, target).await?;
    let (plan, weeks, state) = match (&view.draft, &view.live) {
        (Some(draft), _) => (&draft.plan, &draft.weeks, Some(draft.state.clone())),
        (None, Some(live)) => (&live.plan, &live.weeks, None),
        (None, None) => return Ok(None),
    };
    let fueling = FuelingDisclosure::for_athlete(repos, target.tenant(), target.athlete()).await?;
    Ok(Some(SubjectPlanCard {
        card: PlanCard::build(plan, weeks, today, registry, locale, &fueling),
        draft: PlanCardDraft {
            subject,
            draft: state,
        },
    }))
}

fn week_card(week: &PlanWeek, current: bool, fueling: &FuelingDisclosure) -> WeekCard {
    WeekCard {
        week_start: week.week_start.clone(),
        focus: week.focus.clone(),
        phase_index: week.phase_index,
        current,
        days: week.days.iter().map(|d| day_card(d, fueling)).collect(),
    }
}

fn day_card(day: &PlannedDay, fueling: &FuelingDisclosure) -> DayCard {
    let (fueling, fueling_withheld) = fueling.day_fueling(day);
    DayCard {
        date: day.date.clone(),
        sport: day.sport.clone(),
        workout: day.workout.clone(),
        duration_min: day.duration_min,
        intensity: day.intensity.clone(),
        rest: day.is_rest(),
        steps: day.steps.clone(),
        fueling,
        fueling_withheld,
        template_slug: day.template_slug.clone(),
        template_source: day.template_source,
    }
}

/// The athlete-facing label of a flavour, in one locale.
///
/// Plain words — "mostly easy with two hard days", never "polarized" — from
/// the string catalogue under `messaging.flavour.<id>`, the hyphens of the id
/// folded to underscores. A flavour the catalogue has no words for (an agent
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

/// Load the athlete's active plan — their one season, whichever agent laid
/// it — and project it for the card.
///
/// `None` when there is no active plan or the store cannot be read — a card
/// is a courtesy on the reply, never a reason to fail the turn.
pub async fn load_plan_card(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    today: NaiveDate,
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> Option<PlanCard> {
    match try_load_plan_card(repos, tenant, user_id, today, registry, locale).await {
        Ok(card) => card,
        // Not served over this transport: the courtesy card is simply absent.
        Err(e) if e.code == ErrorCode::UnavailableOverTransport => None,
        Err(e) => {
            warn!(error = %e, "plan card: active plan unreadable");
            None
        }
    }
}

/// Load the athlete's active plan and project it for the card, reporting a
/// store that cannot be read.
///
/// The read behind [`load_plan_card`] for a caller where the plan *is* the
/// answer rather than a courtesy on one: a page that shows the plan must tell
/// "no plan" (`Ok(None)`) from "the plan could not be read" (`Err`), or an
/// outage reads to the athlete as a plan that was deleted.
///
/// # Errors
///
/// Returns the repository error when the active plan, its weeks or the
/// athlete's medical flag cannot be read, and
/// [`ErrorCode::UnavailableOverTransport`] when the plan was derived from
/// first-party-only data and the caller is external (carnet#769) — never
/// "no plan", which would invite the caller to lay one over it.
pub async fn try_load_plan_card(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    today: NaiveDate,
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> AppResult<Option<PlanCard>> {
    let Some(active) = load_active_plan(repos, tenant, user_id).await? else {
        return Ok(None);
    };
    Ok(Some(PlanCard::build(
        &active.plan,
        &active.weeks,
        today,
        registry,
        locale,
        &active.fueling,
    )))
}

/// Load the athlete's active plan and project its weeks overlapping `from..=to`.
///
/// Past weeks are included, in calendar order — the weeks a calendar draws,
/// where the card shows only the current and the next. Each week is projected
/// exactly as the card projects its weeks, fuelling disclosure included, and
/// flagged `current` when `today` falls in it.
/// `Ok(None)` when the athlete has no active plan, so a calendar can tell
/// "no plan" from "the plan says nothing about these days" (`Ok(Some(vec![]))`).
///
/// # Errors
///
/// The errors of [`try_load_plan_card`]: a store that cannot be read, and
/// [`ErrorCode::UnavailableOverTransport`] for a plan an external caller may
/// not read.
pub async fn try_load_plan_weeks_between(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    today: NaiveDate,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Option<Vec<WeekCard>>> {
    let Some(active) = load_active_plan(repos, tenant, user_id).await? else {
        return Ok(None);
    };
    Ok(Some(
        weeks_between(&active.weeks, today, from, to)
            .map(|(week, current)| week_card(week, current, &active.fueling))
            .collect(),
    ))
}

/// The stored weeks that overlap `from..=to`, each with whether `today`
/// falls in it. A week whose start does not parse, or cannot be closed six
/// days later, is skipped rather than guessed at, as the card skips it.
fn weeks_between(
    weeks: &[PlanWeek],
    today: NaiveDate,
    from: NaiveDate,
    to: NaiveDate,
) -> impl Iterator<Item = (&PlanWeek, bool)> {
    weeks.iter().filter_map(move |week| {
        let start = parse_plan_date(&week.week_start)?;
        let end = start.checked_add_days(Days::new(DAYS_AFTER_WEEK_START))?;
        (start <= to && end >= from).then(|| (week, (start..=end).contains(&today)))
    })
}

/// Days from a week's Monday to its Sunday.
const DAYS_AFTER_WEEK_START: u64 = 6;

/// The athlete's active plan with its weeks and the fuelling disclosure its
/// days are projected under.
struct ActivePlan {
    plan: TrainingPlan,
    weeks: Vec<PlanWeek>,
    fueling: FuelingDisclosure,
}

/// The one read behind both the card and the calendar: the athlete's active
/// plan, refused over a transport it may not cross, with its active weeks.
async fn load_active_plan(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
) -> AppResult<Option<ActivePlan>> {
    let tenant_id = tenant.to_string();
    let user = user_id.to_string();
    let Some(plan) = repos
        .training_plans
        .get_active_plan(&tenant_id, &user)
        .await?
    else {
        return Ok(None);
    };
    if !ai_scope::admit_derived(plan.transport_policy) {
        return Err(AppError::unavailable_over_transport(
            "this athlete's training plan is not available over this interface",
        ));
    }
    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant_id, &user, &plan.id, false)
        .await?;
    let fueling = FuelingDisclosure::for_athlete(repos, tenant, user_id).await?;
    Ok(Some(ActivePlan {
        plan,
        weeks,
        fueling,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use pierre_memory::training_plans::{RacePriority, WeekStatus};

    fn week(week_start: &str) -> PlanWeek {
        PlanWeek {
            id: format!("week-{week_start}"),
            tenant_id: "tenant".to_owned(),
            user_id: "user".to_owned(),
            plan_id: "plan".to_owned(),
            week_start: week_start.to_owned(),
            focus: String::new(),
            phase_index: None,
            days: Vec::new(),
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            author_agent_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn date(text: &str) -> NaiveDate {
        parse_plan_date(text).unwrap_or(NaiveDate::MIN)
    }

    fn bare_card() -> PlanCard {
        PlanCard {
            goal_race: GoalRace {
                name: "Parkrun PB".to_owned(),
                date: "2027-05-16".to_owned(),
                discipline: "run_5k".to_owned(),
                priority: RacePriority::A,
            },
            races: Vec::new(),
            season_start: None,
            season_end: None,
            flavour: None,
            phases: Vec::new(),
            current_phase_index: None,
            weeks: Vec::new(),
            weeks_deferred: 0,
        }
    }

    /// A card in a coach's athlete thread carries `subject` and `draft`
    /// beside its plan, `draft: null` included; every other card carries
    /// neither (carnet#1070).
    #[test]
    fn only_a_card_in_an_athlete_thread_carries_subject_and_draft() {
        let card = bare_card();
        let plan = to_value_as_written(&card).unwrap();
        assert_eq!(
            card.as_block("get_training_plan").unwrap(),
            serde_json::json!({
                "type": "workout_plan",
                "source_tool": "get_training_plan",
                "plan": plan,
            })
        );

        let subject = PlanSubject {
            user_id: Uuid::nil(),
            display_name: "Mira".to_owned(),
        };
        let live = PlanCardDraft {
            subject: subject.clone(),
            draft: None,
        };
        let block = card.as_subject_block("get_training_plan", &live).unwrap();
        assert_eq!(block["plan"], plan);
        assert_eq!(
            block["subject"],
            serde_json::json!({ "user_id": Uuid::nil(), "display_name": "Mira" })
        );
        assert_eq!(block["draft"], serde_json::Value::Null);
        assert!(block.as_object().unwrap().contains_key("draft"));

        let drafted = PlanCardDraft {
            subject,
            draft: Some(DraftState {
                draft_id: "draft-1".to_owned(),
                updated_at: "2026-10-16T09:30:00Z".parse().unwrap(),
                conflicts: Vec::new(),
                season_relaid: true,
            }),
        };
        let block = card
            .as_subject_block("save_training_plan", &drafted)
            .unwrap();
        assert_eq!(block["source_tool"], "save_training_plan");
        assert_eq!(block["draft"]["draft_id"], "draft-1");
        assert_eq!(block["draft"]["season_relaid"], true);
        // What the block carries reads back as what was put on it.
        assert_eq!(
            serde_json::from_value::<PlanCardDraft>(block).unwrap(),
            drafted
        );
    }

    #[test]
    fn weeks_between_keeps_every_week_overlapping_the_range_past_ones_included() {
        let weeks = [
            week("2026-08-31"),
            week("2026-09-07"),
            week("2026-09-14"),
            week("2026-09-21"),
            week("2026-09-28"),
        ];
        // A range that starts on a Wednesday and ends on a Tuesday overlaps
        // the weeks on either side of it.
        let picked: Vec<(&str, bool)> = weeks_between(
            &weeks,
            date("2026-09-24"),
            date("2026-09-09"),
            date("2026-09-22"),
        )
        .map(|(week, current)| (week.week_start.as_str(), current))
        .collect();
        assert_eq!(
            picked,
            vec![
                ("2026-09-07", false),
                ("2026-09-14", false),
                ("2026-09-21", true)
            ]
        );
    }

    #[test]
    fn weeks_between_skips_a_week_whose_start_is_not_a_calendar_day() {
        let weeks = [week("2026-9-7"), week("2026-09-14")];
        let picked: Vec<&str> = weeks_between(
            &weeks,
            date("2026-09-14"),
            date("2026-09-01"),
            date("2026-09-30"),
        )
        .map(|(week, _)| week.week_start.as_str())
        .collect();
        assert_eq!(picked, vec!["2026-09-14"]);
    }
}
