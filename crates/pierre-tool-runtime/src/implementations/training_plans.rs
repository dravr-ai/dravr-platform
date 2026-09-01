// ABOUTME: Training-plan tools — get_training_plan / save_training_plan (explicit coach persistence)
// ABOUTME: The durable home for coach prescriptions; replaces extraction minting plans as user_facts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Training-Plan Tools
//!
//! The coach persona persists plans it agrees with the athlete via
//! `save_training_plan` — an explicit tool call in the same turn the plan is
//! stated — and re-reads them next conversation via `get_training_plan`.
//! Explicit writes (not post-hoc extraction) are the durable-state pattern:
//! the plan the athlete was promised must survive context loss verbatim.
//!
//! Saving also closes the pillar loop: when the outline's goal race has no
//! linked pillar `Goal` fact, the save upserts one (`FactSource::Coach`,
//! pillar Training & Movement) so `/pillars` onboarding and conversational
//! goal-stating converge on one fact row.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use pierre_core::models::{ConversationRecord, Pillar, TenantId, WorkoutStep};
use pierre_database::repositories::{
    PlanOutlineInput, PlanWeekInput, SavePlanBundleParams, UpsertUserFactParams,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{
    parse_plan_date, GoalRace, PlanBlock, PlanWeek, PlannedDay, RacePriority, WeekStatus,
    MAX_DAYS_PER_WEEK,
};
use pierre_memory::{FactKind, FactSource, MemoryScope};
use pierre_services::ramp_check::assess_ramp;
use pierre_services::training_plan_render::plan_goal_is_stale;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;

use super::calendar::{bounded, validate_step, TargetRule, MAX_SESSION_STEPS, MAX_SHORT_TEXT_LEN};
use super::plan_scope::{resolve_plan_scope, PlanScopeRequest};
use super::training_plan_push::{calendar_block, calendar_preview_after_save};
use super::training_plan_schema::{
    athlete_prop, outline_schema, parse_payload_part, string_prop, weeks_schema,
};
use super::training_plan_telemetry::{
    athlete_today, emit_coverage_check, emit_plan_saved, emit_ramp_verdict, ramp_baseline,
};
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};

use crate::implementations::guided_flow::guided_flow_is_active;
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// Annotation set for the plan write tool.
fn write_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

/// Annotation set for the plan read tool.
fn read_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

fn optional_string_field(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|s| !s.trim().is_empty())
}

fn ctx_user_id(context: &ToolExecutionContext) -> String {
    context.user_id.to_string()
}

// ============================================================================
// Save payload deserialization
// ============================================================================

/// Outline half of the save payload.
#[derive(Deserialize)]
struct OutlinePayload {
    goal_race: GoalRace,
    #[serde(default)]
    races: Vec<GoalRace>,
    strategy: String,
    /// Optional: a short plan ("hold form for two weeks, then taper") has no
    /// mesocycle structure to describe, and requiring one made every such plan
    /// unsaveable until the coach invented phase/start/weeks/intent for it.
    #[serde(default)]
    blocks: Vec<PlanBlock>,
}

/// One week entry of the save payload.
#[derive(Deserialize)]
struct WeekPayload {
    week_start: String,
    #[serde(default)]
    focus: String,
    days: Vec<PlannedDay>,
    #[serde(default)]
    adjustment_reason: String,
}

/// Upper bounds on free-text and collection sizes in a save payload. A plan is
/// rendered verbatim into every future system prompt, so an unbounded (or
/// adversarial) save would inflate token cost on every turn; these caps keep a
/// single degenerate save from doing that. Generous enough that no real coach
/// plan hits them.
const MAX_STRATEGY_LEN: usize = 4_000;
const MAX_TEXT_LEN: usize = 1_000;
const MAX_RACES: usize = 12;
const MAX_BLOCKS: usize = 24;
/// Two full seasons. The longest real single-payload plan is one season (52
/// weeks to a late-year A race), and `MAX_BLOCKS` mesocycles of the usual three
/// to four weeks describe roughly 96 weeks — so the two caps agree, and one
/// save can still span a season boundary. Above this the payload is a
/// hallucination, and each week costs two statements in the save transaction.
const MAX_WEEKS: usize = 104;
const MAX_TARGET_HOURS: f32 = 60.0;
const MAX_DURATION_MIN: u32 = 24 * 60;
/// Fuelling ceilings, mirroring `$defs.FuelingProtocol` in the
/// structured-workout schema. The schema bounds what a coach may emit; nothing
/// bounded what a plan could store, so a rate no gut can absorb reached the
/// athlete's calendar unchallenged.
const MAX_CARBS_G_PER_H: f32 = 150.0;
const MAX_FLUID_ML_PER_H: f32 = 1_500.0;
const MAX_SODIUM_MG_PER_H: f32 = 2_000.0;

/// Calendar domain a plan date must fall in.
///
/// `parse_plan_date` is a format check: chrono's `%Y` accepts a signed,
/// unlimited-digit year and `NaiveDate`'s `Display` re-emits it, so
/// `+262142-12-31` round-trips and persists. Every renderer that walks a plan
/// adds `Days` to a stored date, which panics past `NaiveDate::MAX` — and a
/// stored plan is re-rendered into the system prompt on every subsequent turn,
/// so one such date breaks the athlete's chat until the row is removed.
/// `9999-12-31` plus the longest offset any renderer adds stays far inside
/// `NaiveDate::MAX`, which is what makes those additions unreachable from
/// overflow rather than merely unlikely.
const MIN_PLAN_YEAR: i32 = 1000;
const MAX_PLAN_YEAR: i32 = 9999;

/// Parse a plan date, accepting only a canonical `YYYY-MM-DD` inside the
/// calendar domain plans are rendered in (see [`MIN_PLAN_YEAR`]).
///
/// `field` names the payload field in the rejection so the model's next
/// iteration knows which date to fix.
fn plan_date(field: &str, raw: &str) -> AppResult<NaiveDate> {
    let Some(date) = parse_plan_date(raw) else {
        return Err(AppError::invalid_input(format!(
            "{field} must be YYYY-MM-DD, got '{raw}'"
        )));
    };
    let year = date.year();
    if !(MIN_PLAN_YEAR..=MAX_PLAN_YEAR).contains(&year) {
        return Err(AppError::invalid_input(format!(
            "{field} year {year} is outside {MIN_PLAN_YEAR}-{MAX_PLAN_YEAR}, got '{raw}'"
        )));
    }
    Ok(date)
}

/// Validate every date/shape/size constraint BEFORE any write so a rejected
/// payload can never begin a partial save.
fn validate_outline(outline: &OutlinePayload) -> AppResult<()> {
    plan_date("goal_race.date", &outline.goal_race.date)?;
    bounded(
        "goal_race.name",
        &outline.goal_race.name,
        MAX_SHORT_TEXT_LEN,
    )?;
    bounded(
        "goal_race.discipline",
        &outline.goal_race.discipline,
        MAX_SHORT_TEXT_LEN,
    )?;
    if outline.races.len() > MAX_RACES {
        return Err(AppError::invalid_input(format!(
            "too many races ({}; max {MAX_RACES})",
            outline.races.len()
        )));
    }
    for race in &outline.races {
        plan_date(&format!("race '{}' date", race.name), &race.date)?;
        bounded("race.name", &race.name, MAX_SHORT_TEXT_LEN)?;
        bounded("race.discipline", &race.discipline, MAX_SHORT_TEXT_LEN)?;
    }
    if outline.strategy.trim().is_empty() {
        return Err(AppError::invalid_input(
            "outline.strategy must state the coach's plan in prose",
        ));
    }
    bounded("outline.strategy", &outline.strategy, MAX_STRATEGY_LEN)?;
    // No minimum on blocks: a plan can legitimately have no mesocycle
    // structure ("hold form for two weeks, then taper"), and the previous
    // at-least-one rule made such a plan unsaveable until the coach invented
    // one. The upper bound stays — it is the token-cost guard, not a demand.
    if outline.blocks.len() > MAX_BLOCKS {
        return Err(AppError::invalid_input(format!(
            "too many blocks ({}; max {MAX_BLOCKS})",
            outline.blocks.len()
        )));
    }
    for block in &outline.blocks {
        plan_date("block start", &block.start)?;
        if block.weeks == 0 {
            return Err(AppError::invalid_input("block weeks must be >= 1"));
        }
        bounded("block.intent", &block.intent, MAX_TEXT_LEN)?;
        if let Some(hours) = block.target_hours {
            if !(0.0..=MAX_TARGET_HOURS).contains(&hours) {
                return Err(AppError::invalid_input(format!(
                    "block target_hours must be between 0 and {MAX_TARGET_HOURS}, got {hours}"
                )));
            }
        }
    }
    Ok(())
}

/// Validate one week payload (dates, day count, day dates inside the week,
/// step bounds), and complete a structured day's `duration_min` from its
/// steps when the coach left it out — the stored day never contradicts its
/// own structure, and a stated duration that does is refused.
fn validate_week(week: &mut WeekPayload) -> AppResult<()> {
    let start = plan_date("week_start", &week.week_start)?;
    if week.days.is_empty() {
        return Err(AppError::invalid_input(format!(
            "week {} has no days",
            week.week_start
        )));
    }
    if week.days.len() > MAX_DAYS_PER_WEEK {
        return Err(AppError::invalid_input(format!(
            "week {} has {} days; max {MAX_DAYS_PER_WEEK}",
            week.week_start,
            week.days.len()
        )));
    }
    bounded("week.focus", &week.focus, MAX_TEXT_LEN)?;
    bounded(
        "week.adjustment_reason",
        &week.adjustment_reason,
        MAX_TEXT_LEN,
    )?;
    for day in &mut week.days {
        let date = plan_date("day date", &day.date)?;
        let offset = (date - start).num_days();
        if !(0..7).contains(&offset) {
            return Err(AppError::invalid_input(format!(
                "day {} falls outside week starting {}",
                day.date, week.week_start
            )));
        }
        if day.workout.trim().is_empty() {
            return Err(AppError::invalid_input(format!(
                "day {} has an empty workout",
                day.date
            )));
        }
        bounded("day.sport", &day.sport, MAX_SHORT_TEXT_LEN)?;
        bounded("day.workout", &day.workout, MAX_TEXT_LEN)?;
        bounded("day.intensity", &day.intensity, MAX_SHORT_TEXT_LEN)?;
        if let Some(mins) = day.duration_min {
            if mins > MAX_DURATION_MIN {
                return Err(AppError::invalid_input(format!(
                    "day {} duration_min {mins} exceeds {MAX_DURATION_MIN}",
                    day.date
                )));
            }
        }
        if let Some(fuel) = day.fueling.as_ref() {
            // Rates are per hour and cannot be negative, and each ceiling is
            // the schema's own. Sodium is checked like the rest even though it
            // is a loss estimate rather than a target — an implausible estimate
            // produces an implausible plan just as surely.
            for (field, value, max) in [
                ("carbs_g_per_h", fuel.carbs_g_per_h, MAX_CARBS_G_PER_H),
                ("fluid_ml_per_h", fuel.fluid_ml_per_h, MAX_FLUID_ML_PER_H),
            ] {
                if !value.is_finite() || !(0.0..=max).contains(&value) {
                    return Err(AppError::invalid_input(format!(
                        "day {} fueling.{field} {value} is outside 0..={max}",
                        day.date
                    )));
                }
            }
            if let Some(sodium) = fuel.sodium_mg_per_h {
                if !sodium.is_finite() || !(0.0..=MAX_SODIUM_MG_PER_H).contains(&sodium) {
                    return Err(AppError::invalid_input(format!(
                        "day {} fueling.sodium_mg_per_h {sodium} is outside 0..={MAX_SODIUM_MG_PER_H}",
                        day.date
                    )));
                }
            }
            if let Some(source) = fuel.carb_source.as_deref() {
                bounded("day.fueling.carb_source", source, MAX_SHORT_TEXT_LEN)?;
            }
            if day.is_rest() {
                return Err(AppError::invalid_input(format!(
                    "day {} is a rest day and carries a fuelling protocol — there is no \
                     session to fuel",
                    day.date
                )));
            }
        }
        if day.steps.is_empty() {
            continue;
        }
        if day.is_rest() {
            return Err(AppError::invalid_input(format!(
                "day {} is a rest day and carries steps — drop the steps or give the day a sport",
                day.date
            )));
        }
        if day.steps.len() > MAX_SESSION_STEPS {
            return Err(AppError::invalid_input(format!(
                "day {} has {} steps; max {MAX_SESSION_STEPS}",
                day.date,
                day.steps.len()
            )));
        }
        for (index, step) in day.steps.iter().enumerate() {
            validate_step(
                &format!("day {} steps[{index}]", day.date),
                step,
                TargetRule::Resolvable,
            )?;
        }
        let minutes = WorkoutStep::total_seconds(&day.steps).div_ceil(60);
        if minutes > u64::from(MAX_DURATION_MIN) {
            return Err(AppError::invalid_input(format!(
                "day {} steps total {minutes} minutes; max {MAX_DURATION_MIN}",
                day.date
            )));
        }
        let minutes = u32::try_from(minutes)
            .map_err(|e| AppError::internal(format!("day duration does not fit in u32: {e}")))?;
        match day.duration_min {
            Some(stated) if stated != minutes => {
                return Err(AppError::invalid_input(format!(
                    "day {} duration_min {stated} contradicts its steps, which total {minutes} \
                     minutes — omit duration_min or make it {minutes}",
                    day.date
                )));
            }
            Some(_) => {}
            None => day.duration_min = Some(minutes),
        }
    }
    Ok(())
}

/// Subject/predicate the coach-agnostic goal `user_fact` is written under. The
/// save converges every outline on a single fact with this identity so
/// `/pillars` and conversational goal-stating never fork into duplicates.
const GOAL_SUBJECT: &str = "you";
const GOAL_PREDICATE: &str = "target race";

/// Render a race priority (`A`/`B`/`C`) as its serialized string.
fn race_priority_str(priority: RacePriority) -> String {
    serde_json::to_value(priority)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The `object` phrase stored for a goal-race `user_fact`.
fn goal_object(race: &GoalRace) -> String {
    format!(
        "{} ({}) on {} — priority {}",
        race.name,
        race.discipline,
        race.date,
        race_priority_str(race.priority)
    )
}

/// Resolve the coach the plan is bound to. The conversation's coach is
/// authoritative when the call originates in a Pierre conversation — the plan
/// injection (Stage 7f.2) keys on it, so trusting an LLM-supplied `coach_id`
/// instead would save a plan under a slug the injection never reads (a plan
/// "saved but not showing"). Only MCP-direct / A2A calls with no conversation
/// fall back to the argument.
pub(super) async fn load_conversation(
    repos: &RepositoryRegistry,
    conversation_id: Option<&str>,
    tenant: TenantId,
    user_id: &str,
) -> AppResult<Option<ConversationRecord>> {
    match conversation_id {
        Some(conv_id) => repos.chat.get_conversation(conv_id, user_id, tenant).await,
        None => Ok(None),
    }
}

/// The coach slug this plan is saved under.
///
/// The conversation's coach is authoritative, so save and the Stage 7f.2 plan
/// injection agree on the slug; a conversation with no coach yields `None`
/// rather than falling back. The LLM-supplied argument is used only when there
/// is no conversation at all (a direct MCP call).
pub(super) fn resolve_coach_slug(
    conv: Option<&ConversationRecord>,
    arg_coach: Option<String>,
) -> Option<String> {
    conv.map_or(arg_coach, |conv| conv.coach_id.clone())
}

/// `true` when `fact_id` is a real `Goal` fact of this tenant + user. Guards
/// against an LLM-supplied `goal_fact_id` that never existed, points at another
/// athlete's fact, or is a non-`Goal` fact — a plan links only to a Goal fact,
/// and [`plan_goal_is_stale`] can see only Goal facts, so a non-Goal link would
/// read stale forever. Such a value is dropped rather than persisted.
///
/// A point lookup, not a scan of a capped list: an athlete whose Goal facts
/// outnumber any list cap would have a legitimate id below the cut treated as
/// foreign and silently replaced.
async fn fact_belongs_to_user(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    fact_id: &str,
) -> AppResult<bool> {
    Ok(repos
        .memory
        .get_user_fact(fact_id, tenant, user_id)
        .await?
        .is_some_and(|fact| fact.kind == FactKind::Goal))
}

/// The goal fact a plan links to, together with the coach-agnostic goal facts
/// it replaces.
///
/// Two halves because they belong on opposite sides of the plan write: the id
/// has to exist *before* the save (the plan row stores it), while retiring the
/// facts it supersedes must wait until the save has committed — a hard delete
/// before a failed transaction erases the athlete's standing goal for a plan
/// that was never stored.
struct GoalFactConvergence {
    /// The fact the plan links to.
    fact_id: String,
    /// Prior coach-agnostic goal facts, to retire once the plan is stored.
    superseded: Vec<String>,
}

/// Converge the athlete's coach-agnostic goal `user_fact` on the outline's goal
/// race: reuse an identical stored goal (no churn on a re-save), otherwise write
/// the new one. Every *other* coach-agnostic goal fact is reported as
/// superseded, in both cases, so the pillar view converges on one row even after
/// a save that failed between the write and the retirement.
async fn converge_goal_fact(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    goal_race: &GoalRace,
) -> AppResult<GoalFactConvergence> {
    let object = goal_object(goal_race);
    let facts = repos
        .memory
        .list_user_facts(tenant, user_id, None, Some(FactKind::Goal), 200)
        .await?;
    let agnostic_targets: Vec<&_> = facts
        .iter()
        .filter(|f| f.coach_id.is_none() && f.predicate == GOAL_PREDICATE)
        .collect();
    let fact_id = match agnostic_targets.iter().find(|f| f.object == object) {
        Some(existing) => existing.id.clone(),
        None => {
            repos
                .memory
                .upsert_user_fact(&UpsertUserFactParams {
                    tenant_id: tenant,
                    user_id,
                    coach_id: None,
                    scope: MemoryScope::User,
                    kind: FactKind::Goal,
                    pillar: Some(Pillar::TrainingAndMovement),
                    subject: GOAL_SUBJECT,
                    predicate: GOAL_PREDICATE,
                    object: &object,
                    confidence: 0.95,
                    source: FactSource::Coach,
                    valid_until: None,
                    source_msg_id: None,
                    embedding: None,
                })
                .await?
                .id
        }
    };
    // The linked fact is never in the superseded set, so the retirement pass
    // cannot delete the very row the plan points at.
    let superseded = agnostic_targets
        .iter()
        .map(|f| f.id.clone())
        .filter(|id| *id != fact_id)
        .collect();
    Ok(GoalFactConvergence {
        fact_id,
        superseded,
    })
}

/// Delete the goal facts a stored plan's goal has replaced.
///
/// Runs only after the plan bundle has committed. `delete_user_fact` is the
/// erase path, so calling it earlier would destroy the athlete's previous goal
/// on behalf of a plan that may never be stored.
///
/// A failure here is logged rather than returned: the plan IS saved, and
/// answering the coach with an error would have it tell the athlete a save
/// failed that did not. The leftover fact is retired by the next save, which
/// reports every non-linked agnostic goal fact as superseded.
async fn retire_superseded_goal_facts(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    superseded: &[String],
) {
    for fact_id in superseded {
        if let Err(e) = repos
            .memory
            .delete_user_fact(fact_id, tenant, user_id)
            .await
        {
            warn!(error = %e, "save_training_plan: superseded goal fact not retired");
        }
    }
}

// ============================================================================
// GetTrainingPlanTool
// ============================================================================

/// Read the athlete's active training plan (outline + weeks).
pub struct GetTrainingPlanTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetTrainingPlanTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            string_prop(
                "Coach persona slug asking; falls back to the athlete's coach-agnostic plan.",
            ),
        );
        properties.insert(
            "include_history".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some(
                    "Include superseded week versions (the adjustment audit trail).".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert("athlete".to_owned(), athlete_prop());
        let schema = object_schema(properties, None);
        tool_definition(
            "get_training_plan",
            "Fetch the athlete's active training plan: goal race, block strategy, and the day-by-day weeks. Use before answering any 'what's my plan / what am I doing this week' question — the stored plan, not memory of the conversation, is the source of truth. The calendar block lists what Dravr has on the athlete's Intervals.icu calendar (each entry's prescription_id is what prescribe_workout's replaces and withdraw_prescribed_workout take) and whether push_training_plan would change it. A group's human coach reads a consenting athlete's plan by passing `athlete` from their own direct chat — the athlete shares it into the room with `/plan share`, the coach reads and edits it from their DM.",
            schema,
            Some(read_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::REQUIRES_TENANT)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let requester_tenant = TenantId::from_uuid(context.require_tenant()?);
            let requester = ctx_user_id(&context);
            let arg_coach = optional_string_field(&args, "coach_id");
            let athlete = optional_string_field(&args, "athlete");
            let include_history = args
                .get("include_history")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let repos = context.resources.repos();
            let conv = load_conversation(
                repos,
                context.conversation_id.as_deref(),
                requester_tenant,
                &requester,
            )
            .await?;
            // Whose plan: the caller's own, or — for the group's human coach,
            // from a direct chat, with the athlete's consent — a coached
            // athlete's. Everything below reads under the resolved scope.
            let scope = match resolve_plan_scope(PlanScopeRequest {
                context: &context,
                requester_tenant,
                conversation: conv.as_ref(),
                arg_coach,
                athlete: athlete.as_deref(),
                tool_name: "get_training_plan",
            })
            .await?
            {
                Ok(scope) => scope,
                Err(refused) => return Ok(refused),
            };
            let tenant = scope.tenant;
            let tenant_id = tenant.to_string();
            let user_id = scope.user_id.to_string();
            let coach = scope.coach_slug.clone();
            let today = athlete_today(repos, &user_id).await;
            let Some(plan) = repos
                .training_plans
                .get_active_plan(&tenant_id, &user_id, coach.as_deref())
                .await?
            else {
                // No plan, but the calendar may still hold single prescriptions
                // — and plan entries of a plan since abandoned, which the
                // block's `pending.remove` counts.
                let calendar = calendar_block(repos, tenant, scope.user_id, &[], today).await?;
                return Ok(ToolResult::ok(json!({
                    "plan": Value::Null,
                    "athlete": scope.acting_for,
                    "message": "no active training plan — build one with the athlete and persist it via save_training_plan",
                    "calendar": calendar,
                })));
            };
            let weeks = repos
                .training_plans
                .list_plan_weeks(&tenant_id, &user_id, &plan.id, include_history)
                .await?;
            // The plan snapshots the goal at save time; flag it stale if the
            // living goal fact has since expired so the coach re-confirms.
            let goal_stale = match plan.goal_fact_id.as_deref() {
                Some(fid) => plan_goal_is_stale(repos, tenant, &user_id, fid).await?,
                None => false,
            };
            let active_weeks: Vec<PlanWeek> = weeks
                .iter()
                .filter(|week| week.status == WeekStatus::Active)
                .cloned()
                .collect();
            let calendar =
                calendar_block(repos, tenant, scope.user_id, &active_weeks, today).await?;

            Ok(ToolResult::ok(json!({
                "plan": plan,
                "athlete": scope.acting_for,
                "weeks": weeks,
                "goal_stale": goal_stale,
                "calendar": calendar,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SaveTrainingPlanTool
// ============================================================================

/// Persist a plan outline and/or day-by-day weeks, superseding prior
/// versions prospectively.
pub struct SaveTrainingPlanTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SaveTrainingPlanTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            string_prop("Coach persona slug saving the plan."),
        );
        properties.insert("outline".to_owned(), outline_schema());
        properties.insert("weeks".to_owned(), weeks_schema());
        properties.insert(
            "goal_fact_id".to_owned(),
            string_prop("Existing pillar Goal fact this plan serves, when known."),
        );
        properties.insert(
            "conversation_id".to_owned(),
            string_prop("Originating conversation ID for provenance."),
        );
        properties.insert("athlete".to_owned(), athlete_prop());
        let schema = object_schema(properties, None);
        tool_definition(
            "save_training_plan",
            "Persist the training plan you agreed with the athlete — outline (goal race, blocks, strategy) and/or day-by-day weeks — in the SAME turn you state it. Saved plans are re-injected into future conversations; an unsaved plan is forgotten. Adjustments re-save only the changed week(s) and supersede prospectively; past weeks stay immutable. For a day with interval structure, give steps (same shape as prescribe_workout's session.structure) — that is what puts workout-builder steps and a planned load on the calendar; prose alone reaches it as a timed entry. Saving never writes to the athlete's calendar: when the reply's calendar.stale is true, their Intervals.icu calendar no longer matches the plan — tell them and offer push_training_plan. A group's human coach edits a consenting athlete's plan by passing `athlete` from their own direct chat, never in a room: the athlete shares the plan into the room with `/plan share`, the coach saves the change from their DM, and the athlete's next `/plan` shows it.",
            schema,
            Some(write_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let requester_tenant = TenantId::from_uuid(context.require_tenant()?);
            let requester = ctx_user_id(&context);
            let arg_coach = optional_string_field(&args, "coach_id");
            let athlete = optional_string_field(&args, "athlete");
            let conversation_id = optional_string_field(&args, "conversation_id");
            let mut goal_fact_id = optional_string_field(&args, "goal_fact_id");

            // Both halves are parsed before either can reject, so a payload
            // that got outline *and* weeks wrong — which is what a model
            // guessing the shape actually does — learns both in one reply
            // rather than spending a second LLM iteration to discover the
            // next mismatch.
            let mut shape_errors: Vec<String> = Vec::new();
            let outline: Option<OutlinePayload> =
                match parse_payload_part(args.get("outline"), "outline", &outline_schema()) {
                    Ok(parsed) => parsed,
                    Err(message) => {
                        shape_errors.push(message);
                        None
                    }
                };
            let mut weeks: Vec<WeekPayload> =
                match parse_payload_part(args.get("weeks"), "weeks", &weeks_schema()) {
                    Ok(parsed) => parsed.unwrap_or_default(),
                    Err(message) => {
                        shape_errors.push(message);
                        Vec::new()
                    }
                };
            if !shape_errors.is_empty() {
                return Err(AppError::invalid_input(shape_errors.join(" ")));
            }

            if outline.is_none() && weeks.is_empty() {
                return Err(AppError::invalid_input(
                    "nothing to save: provide an outline, weeks, or both",
                ));
            }
            // Validate EVERYTHING before the first write (see validate_week).
            if let Some(o) = &outline {
                validate_outline(o)?;
            }
            if weeks.len() > MAX_WEEKS {
                return Err(AppError::invalid_input(format!(
                    "too many weeks ({}; max {MAX_WEEKS})",
                    weeks.len()
                )));
            }
            for week in &mut weeks {
                validate_week(week)?;
            }

            let repos = context.resources.repos();
            let conv = load_conversation(
                repos,
                context.conversation_id.as_deref(),
                requester_tenant,
                &requester,
            )
            .await?;
            // Whose plan: the caller's own, or — for the group's human coach,
            // from a direct chat, with the athlete's consent — a coached
            // athlete's. Every write below lands under the resolved scope.
            let scope = match resolve_plan_scope(PlanScopeRequest {
                context: &context,
                requester_tenant,
                conversation: conv.as_ref(),
                arg_coach,
                athlete: athlete.as_deref(),
                tool_name: "save_training_plan",
            })
            .await?
            {
                Ok(scope) => scope,
                Err(refused) => return Ok(refused),
            };
            let tenant = scope.tenant;
            let tenant_id = tenant.to_string();
            let user_id = scope.user_id.to_string();

            // Enforcement half of the guided-flow tool withhold. The pipeline
            // already drops this tool from the prompt's tool list and from the
            // native function declarations while a profile walk is active, but
            // neither covers the native-MCP path, where an ACP subprocess reads
            // `tools/list` straight off the `/mcp` endpoint. That path carries
            // no conversation, so the walk is resolved from the athlete there
            // (see `guided_flow_is_active`) and the withhold holds on every
            // surface, including a direct MCP call. Self scope only: a coach
            // saving for an athlete is not inside that athlete's profile walk.
            if scope.acting_for.is_none()
                && guided_flow_is_active(
                    repos,
                    conv.as_ref(),
                    context.conversation_ref(),
                    tenant,
                    &user_id,
                )
                .await?
            {
                return Err(AppError::invalid_input(
                    "this conversation is building the athlete's profile one topic at a time — \
                     finish the profile walk before saving a training plan",
                ));
            }

            let coach = scope.coach_slug.clone();

            // Don't trust an LLM-supplied goal_fact_id that isn't a real fact of
            // this athlete — drop it and let the outline path mint/reuse one.
            if let Some(fid) = goal_fact_id.clone() {
                if !fact_belongs_to_user(repos, tenant, &user_id, &fid).await? {
                    warn!("save_training_plan: dropping unknown goal_fact_id");
                    goal_fact_id = None;
                }
            }

            // Close the pillar loop: an outline whose goal race has no linked
            // Goal fact converges on one coach-agnostic Goal fact (the athlete's
            // truth, shared across coaches and the /pillars walk) — idempotent
            // so a re-save never mints a duplicate. The plan row stores the id,
            // so the fact is written here; the facts it replaces are erased only
            // once the plan is safely stored.
            let converged = match (&outline, &goal_fact_id) {
                (Some(o), None) => {
                    Some(converge_goal_fact(repos, tenant, &user_id, &o.goal_race).await?)
                }
                _ => None,
            };
            let superseded_goal_facts: Vec<String> = match converged {
                Some(c) => {
                    goal_fact_id = Some(c.fact_id);
                    c.superseded
                }
                None => Vec::new(),
            };

            // Persist the outline and every week in ONE transaction so a
            // mid-payload failure can't strand a superseded plan with a
            // half-saved successor.
            let week_inputs: Vec<PlanWeekInput<'_>> = weeks
                .iter()
                .map(|w| PlanWeekInput {
                    week_start: &w.week_start,
                    focus: &w.focus,
                    days: &w.days,
                    adjustment_reason: &w.adjustment_reason,
                })
                .collect();
            let outline_input = outline.as_ref().map(|o| PlanOutlineInput {
                goal_race: &o.goal_race,
                races: &o.races,
                strategy: &o.strategy,
                blocks: &o.blocks,
                source_conversation_id: conversation_id.as_deref(),
            });
            let bundle = repos
                .training_plans
                .save_plan_bundle(&SavePlanBundleParams {
                    tenant_id: &tenant_id,
                    user_id: &user_id,
                    coach_slug: coach.as_deref(),
                    goal_fact_id: goal_fact_id.as_deref(),
                    outline: outline_input,
                    weeks: &week_inputs,
                })
                .await?;

            // The plan is stored, so the goal facts its goal replaced can go.
            retire_superseded_goal_facts(repos, tenant, &user_id, &superseded_goal_facts).await;

            // The one enforced rail on plan difficulty: does the plan's opening
            // week sit far above what this athlete actually does? A warning,
            // never a block — the coach may have good reason, and refusing a
            // save would strand a plan the athlete already agreed to. The event
            // also reports when the comparison could not be made, so a quiet
            // log means "measured and fine" rather than "never looked".
            //
            // Only a save carrying an outline is a new plan with an opening
            // week. A week-only save is an adjustment to one week of a plan
            // that was already measured, and grading it as an opening week
            // reports a ramp the athlete is not being asked to make.
            if outline.is_some() {
                emit_ramp_check(
                    repos,
                    tenant,
                    &user_id,
                    &bundle.plan.id,
                    earliest_week(&weeks),
                )
                .await;
            }

            // Every committed write is reported, so a weeks-only adjustment is
            // no longer silent, and the plan is then checked for whether it
            // actually covers the athlete it was just written for.
            emit_plan_saved(&bundle.plan.id, outline.is_some(), &bundle.weeks);
            emit_coverage_check(
                repos,
                &tenant_id,
                &user_id,
                &bundle.plan.id,
                &bundle.plan.blocks,
            )
            .await;

            // The save is committed; if the calendar already carries this
            // plan, say what a push would now change so the athlete learns
            // the calendar is behind — without pushing on anyone's behalf.
            let today = athlete_today(repos, &user_id).await;
            let calendar =
                calendar_preview_after_save(repos, tenant, scope.user_id, &bundle.plan.id, today)
                    .await;

            let race_summary = format!(
                "{} on {}",
                bundle.plan.goal_race.name, bundle.plan.goal_race.date
            );
            let week_ids: Vec<Value> = bundle
                .weeks
                .iter()
                .map(|saved| {
                    json!({
                        "week_start": saved.week_start,
                        "week_id": saved.id,
                        "superseded": saved.supersedes_id.is_some(),
                    })
                })
                .collect();

            Ok(ToolResult::ok(json!({
                "plan_id": bundle.plan.id,
                "athlete": scope.acting_for,
                "goal_race": race_summary,
                "superseded_plan_id": bundle.superseded_plan_id,
                "goal_fact_id": goal_fact_id,
                "weeks_saved": week_ids.len(),
                "weeks": week_ids,
                "calendar": calendar,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The chronologically first week of a payload — the plan's opening week.
///
/// Payload order is the model's, not the calendar's: nothing sorts `weeks`, and
/// a plan may be sent newest-first or in any order at all. The ramp check grades
/// the week the athlete starts on, so it is selected by date.
fn earliest_week(weeks: &[WeekPayload]) -> Option<&WeekPayload> {
    weeks
        .iter()
        .filter_map(|w| parse_plan_date(&w.week_start).map(|date| (date, w)))
        .min_by_key(|(date, _)| *date)
        .map(|(_, week)| week)
}

/// Measure the saved plan's opening week against the athlete's real recent
/// load and emit the result.
///
/// Best-effort by design: a plan the athlete already agreed to must not fail to
/// save because the activity cache was unreadable, so every failure path here
/// degrades to an unmeasurable verdict rather than an error.
async fn emit_ramp_check(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    plan_id: &str,
    opening_week: Option<&WeekPayload>,
) {
    let baseline = ramp_baseline(repos, tenant, user_id).await;
    let durations: Vec<Option<u32>> = opening_week
        .map(|w| w.days.iter().map(|d| d.duration_min).collect())
        .unwrap_or_default();
    emit_ramp_verdict(plan_id, &assess_ramp(&durations, baseline.as_ref()));
}

/// Factory for the training-plan tool set.
#[must_use]
pub fn create_training_plan_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(GetTrainingPlanTool),
        Box::new(SaveTrainingPlanTool),
    ]
}

// A stored plan is conversation-derived text (coach/LLM-authored strategy,
// block intents, day workouts) re-entering the LLM context — the same
// untrusted-content class as recalled memory, so a read taints the turn.
crate::declare_security!(GetTrainingPlanTool => UNTRUSTED_OUTPUT);
crate::declare_security!(SaveTrainingPlanTool => empty);
