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
use pierre_core::models::{Pillar, TenantId};
use pierre_database::repositories::{
    PlanOutlineInput, PlanWeekInput, SavePlanBundleParams, UpsertUserFactParams,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{
    parse_plan_date, GoalRace, PlanBlock, PlannedDay, RacePriority, MAX_DAYS_PER_WEEK,
};
use pierre_memory::{FactKind, FactSource, MemoryScope};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
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

/// String schema property with a description.
fn string_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// Number schema property with a description.
fn number_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "number".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// Integer schema property with a description. Whole-valued floats are
/// still accepted at deserialization (LLM callers emit 60.0 for integers).
fn integer_prop(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "integer".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

/// Object schema property with nested fields.
fn object_prop(
    description: &str,
    properties: HashMap<String, PropertySchema>,
    required: Vec<String>,
) -> PropertySchema {
    PropertySchema {
        property_type: "object".to_owned(),
        description: Some(description.to_owned()),
        properties: Some(properties),
        required: Some(required),
        ..Default::default()
    }
}

/// Array schema property with an item schema.
fn array_prop(description: &str, items: PropertySchema) -> PropertySchema {
    PropertySchema {
        property_type: "array".to_owned(),
        description: Some(description.to_owned()),
        items: Some(Box::new(items)),
        ..Default::default()
    }
}

/// Schema for one race entry (goal or secondary).
fn race_schema(description: &str) -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "name".to_owned(),
        string_prop("Race name as the athlete calls it."),
    );
    p.insert("date".to_owned(), string_prop("Race date, YYYY-MM-DD."));
    p.insert(
        "discipline".to_owned(),
        string_prop("Discipline: gravel, xco, road, trail run, …"),
    );
    p.insert(
        "priority".to_owned(),
        string_prop("A = goal race, B = tune-up, C = training race."),
    );
    object_prop(
        description,
        p,
        vec![
            "name".to_owned(),
            "date".to_owned(),
            "discipline".to_owned(),
            "priority".to_owned(),
        ],
    )
}

/// Schema for one outline block.
fn block_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "phase".to_owned(),
        string_prop("One of: rest | base | build | peak | taper."),
    );
    p.insert(
        "start".to_owned(),
        string_prop("Block start date, YYYY-MM-DD."),
    );
    p.insert("weeks".to_owned(), integer_prop("Block length in weeks."));
    p.insert(
        "intent".to_owned(),
        string_prop("What this block is for, in coach voice."),
    );
    p.insert(
        "target_hours".to_owned(),
        number_prop("Optional target weekly volume in hours."),
    );
    object_prop(
        "One training block (mesocycle).",
        p,
        vec![
            "phase".to_owned(),
            "start".to_owned(),
            "weeks".to_owned(),
            "intent".to_owned(),
        ],
    )
}

/// Schema for one planned day.
fn day_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert("date".to_owned(), string_prop("Day date, YYYY-MM-DD."));
    p.insert(
        "sport".to_owned(),
        string_prop("Sport (mtb, gravel, run, …) or 'rest'."),
    );
    p.insert(
        "workout".to_owned(),
        string_prop("What to do, in coach voice."),
    );
    p.insert(
        "duration_min".to_owned(),
        integer_prop("Planned duration in minutes; omit for rest days."),
    );
    p.insert(
        "intensity".to_owned(),
        string_prop(
            "Intensity RELATIVE to thresholds ('Z2', '3x8min @ 88-93% FTP'). Never absolute watts.",
        ),
    );
    object_prop(
        "One prescribed day.",
        p,
        vec!["date".to_owned(), "sport".to_owned(), "workout".to_owned()],
    )
}

/// Schema for one week entry in the save payload.
fn week_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "week_start".to_owned(),
        string_prop("Date of the week's first day, YYYY-MM-DD."),
    );
    p.insert(
        "focus".to_owned(),
        string_prop("The week's intent in one line."),
    );
    p.insert(
        "days".to_owned(),
        array_prop("The day rows, in date order (max 7).", day_schema()),
    );
    p.insert(
        "adjustment_reason".to_owned(),
        string_prop("Why this week is being re-saved; omit on first save."),
    );
    object_prop(
        "One week of day-by-day prescriptions. Re-saving a week_start supersedes the previous version (prospective adjustment).",
        p,
        vec!["week_start".to_owned(), "days".to_owned()],
    )
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
const MAX_SHORT_TEXT_LEN: usize = 200;
const MAX_RACES: usize = 12;
const MAX_BLOCKS: usize = 24;
const MAX_TARGET_HOURS: f32 = 60.0;
const MAX_DURATION_MIN: u32 = 24 * 60;

/// Reject a text field longer than `max` Unicode scalar values.
fn bounded(field: &str, value: &str, max: usize) -> AppResult<()> {
    let len = value.chars().count();
    if len > max {
        return Err(AppError::invalid_input(format!(
            "{field} is too long ({len} chars; max {max})"
        )));
    }
    Ok(())
}

/// Validate every date/shape/size constraint BEFORE any write so a rejected
/// payload can never begin a partial save.
fn validate_outline(outline: &OutlinePayload) -> AppResult<()> {
    if parse_plan_date(&outline.goal_race.date).is_none() {
        return Err(AppError::invalid_input(format!(
            "goal_race.date must be YYYY-MM-DD, got '{}'",
            outline.goal_race.date
        )));
    }
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
        if parse_plan_date(&race.date).is_none() {
            return Err(AppError::invalid_input(format!(
                "race '{}' date must be YYYY-MM-DD, got '{}'",
                race.name, race.date
            )));
        }
        bounded("race.name", &race.name, MAX_SHORT_TEXT_LEN)?;
        bounded("race.discipline", &race.discipline, MAX_SHORT_TEXT_LEN)?;
    }
    if outline.strategy.trim().is_empty() {
        return Err(AppError::invalid_input(
            "outline.strategy must state the coach's plan in prose",
        ));
    }
    bounded("outline.strategy", &outline.strategy, MAX_STRATEGY_LEN)?;
    if outline.blocks.is_empty() {
        return Err(AppError::invalid_input(
            "outline.blocks must contain at least one block",
        ));
    }
    if outline.blocks.len() > MAX_BLOCKS {
        return Err(AppError::invalid_input(format!(
            "too many blocks ({}; max {MAX_BLOCKS})",
            outline.blocks.len()
        )));
    }
    for block in &outline.blocks {
        if parse_plan_date(&block.start).is_none() {
            return Err(AppError::invalid_input(format!(
                "block start must be YYYY-MM-DD, got '{}'",
                block.start
            )));
        }
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

/// Validate one week payload (dates, day count, day dates inside the week).
fn validate_week(week: &WeekPayload) -> AppResult<()> {
    let Some(start) = parse_plan_date(&week.week_start) else {
        return Err(AppError::invalid_input(format!(
            "week_start must be YYYY-MM-DD, got '{}'",
            week.week_start
        )));
    };
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
    for day in &week.days {
        let Some(date) = parse_plan_date(&day.date) else {
            return Err(AppError::invalid_input(format!(
                "day date must be YYYY-MM-DD, got '{}'",
                day.date
            )));
        };
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
async fn resolve_coach_slug(
    repos: &RepositoryRegistry,
    conversation_id: Option<&str>,
    tenant: TenantId,
    user_id: &str,
    arg_coach: Option<String>,
) -> AppResult<Option<String>> {
    if let Some(conv_id) = conversation_id {
        if let Some(conv) = repos
            .chat
            .get_conversation(conv_id, user_id, tenant)
            .await?
        {
            return Ok(conv.coach_id);
        }
    }
    Ok(arg_coach)
}

/// `true` when `fact_id` is a real `Goal` fact of this tenant + user. Guards
/// against an LLM-supplied `goal_fact_id` that never existed, points at another
/// athlete's fact, or is a non-`Goal` fact — a plan links only to a Goal fact,
/// and [`plan_goal_is_stale`] can see only Goal facts, so a non-Goal link would
/// read stale forever. Such a value is dropped rather than persisted.
async fn fact_belongs_to_user(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    fact_id: &str,
) -> AppResult<bool> {
    let facts = repos
        .memory
        .list_user_facts(tenant, user_id, None, Some(FactKind::Goal), 500)
        .await?;
    Ok(facts.iter().any(|f| f.id == fact_id))
}

/// Ensure exactly one current coach-agnostic goal `user_fact` for the outline's
/// goal race, returning its id. Idempotent: an identical goal already stored is
/// reused (no churn on a re-save); a changed goal replaces the prior agnostic
/// goal fact(s) so the pillar view never accumulates duplicates.
async fn ensure_goal_fact(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    goal_race: &GoalRace,
) -> AppResult<String> {
    let object = goal_object(goal_race);
    let facts = repos
        .memory
        .list_user_facts(tenant, user_id, None, Some(FactKind::Goal), 200)
        .await?;
    let agnostic_targets: Vec<&_> = facts
        .iter()
        .filter(|f| f.coach_id.is_none() && f.predicate == GOAL_PREDICATE)
        .collect();
    if let Some(existing) = agnostic_targets.iter().find(|f| f.object == object) {
        return Ok(existing.id.clone());
    }
    for stale in &agnostic_targets {
        repos
            .memory
            .delete_user_fact(&stale.id, tenant, user_id)
            .await?;
    }
    let fact = repos
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
        .await?;
    Ok(fact.id)
}

/// `true` when the plan's linked goal fact has expired (its `valid_until` is in
/// the past), meaning the living goal moved on and the plan snapshot is stale.
/// Backs the migration's "goal superseded => plan flagged stale on read".
async fn plan_goal_is_stale(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
    goal_fact_id: &str,
) -> AppResult<bool> {
    let facts = repos
        .memory
        .list_user_facts(tenant, user_id, None, Some(FactKind::Goal), 200)
        .await?;
    let now = chrono::Utc::now();
    // A missing linked fact (deleted / replaced) means the snapshot no longer
    // reflects a living goal, so treat it as stale.
    Ok(facts
        .iter()
        .find(|f| f.id == goal_fact_id)
        .is_none_or(|fact| fact.valid_until.is_some_and(|until| until < now)))
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };
        tool_definition(
            "get_training_plan",
            "Fetch the athlete's active training plan: goal race, block strategy, and the day-by-day weeks. Use before answering any 'what's my plan / what am I doing this week' question — the stored plan, not memory of the conversation, is the source of truth.",
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
            let tenant = TenantId::from(context.require_tenant()?);
            let tenant_id = tenant.to_string();
            let user_id = ctx_user_id(&context);
            let arg_coach = optional_string_field(&args, "coach_id");
            let include_history = args
                .get("include_history")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let repos = context.resources.repos();
            let coach = resolve_coach_slug(
                repos,
                context.conversation_id.as_deref(),
                tenant,
                &user_id,
                arg_coach,
            )
            .await?;
            let Some(plan) = repos
                .training_plans
                .get_active_plan(&tenant_id, &user_id, coach.as_deref())
                .await?
            else {
                return Ok(ToolResult::ok(json!({
                    "plan": Value::Null,
                    "message": "no active training plan — build one with the athlete and persist it via save_training_plan",
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

            Ok(ToolResult::ok(json!({
                "plan": plan,
                "weeks": weeks,
                "goal_stale": goal_stale,
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
        let mut outline_props = HashMap::new();
        outline_props.insert(
            "goal_race".to_owned(),
            race_schema("The goal (A) race this plan builds toward."),
        );
        outline_props.insert(
            "races".to_owned(),
            array_prop(
                "Other races on the calendar (B/C priorities).",
                race_schema("A secondary race."),
            ),
        );
        outline_props.insert(
            "strategy".to_owned(),
            string_prop(
                "The coach's strategy in prose — what the athlete sees as the long-term direction.",
            ),
        );
        outline_props.insert(
            "blocks".to_owned(),
            array_prop(
                "Ordered training blocks from now to the goal race.",
                block_schema(),
            ),
        );

        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            string_prop("Coach persona slug saving the plan."),
        );
        properties.insert(
            "outline".to_owned(),
            object_prop(
                "The plan outline (goal race + blocks + strategy). Required when creating a plan; omit to adjust weeks of the existing active plan. Re-sending an outline supersedes the athlete's current plan.",
                outline_props,
                vec!["goal_race".to_owned(), "strategy".to_owned(), "blocks".to_owned()],
            ),
        );
        properties.insert(
            "weeks".to_owned(),
            array_prop(
                "Day-by-day weeks to save. Send the full multi-week detail when the athlete asks to see the whole plan; send a single adjusted week for 'move Tuesday to Wednesday' changes.",
                week_schema(),
            ),
        );
        properties.insert(
            "goal_fact_id".to_owned(),
            string_prop("Existing pillar Goal fact this plan serves, when known."),
        );
        properties.insert(
            "conversation_id".to_owned(),
            string_prop("Originating conversation ID for provenance."),
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };
        tool_definition(
            "save_training_plan",
            "Persist the training plan you agreed with the athlete — outline (goal race, blocks, strategy) and/or day-by-day weeks — in the SAME turn you state it. Saved plans are re-injected into future conversations; an unsaved plan is forgotten. Adjustments re-save only the changed week(s) and supersede prospectively; past weeks stay immutable.",
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
            let tenant = TenantId::from(context.require_tenant()?);
            let tenant_id = tenant.to_string();
            let user_id = ctx_user_id(&context);
            let arg_coach = optional_string_field(&args, "coach_id");
            let conversation_id = optional_string_field(&args, "conversation_id");
            let mut goal_fact_id = optional_string_field(&args, "goal_fact_id");

            let outline: Option<OutlinePayload> = args
                .get("outline")
                .filter(|v| !v.is_null())
                .map(|v| {
                    serde_json::from_value(v.clone()).map_err(|e| {
                        AppError::invalid_input(format!("outline does not match the schema: {e}"))
                    })
                })
                .transpose()?;
            let weeks: Vec<WeekPayload> = args
                .get("weeks")
                .filter(|v| !v.is_null())
                .map(|v| {
                    serde_json::from_value(v.clone()).map_err(|e| {
                        AppError::invalid_input(format!("weeks do not match the schema: {e}"))
                    })
                })
                .transpose()?
                .unwrap_or_default();

            if outline.is_none() && weeks.is_empty() {
                return Err(AppError::invalid_input(
                    "nothing to save: provide an outline, weeks, or both",
                ));
            }
            // Validate EVERYTHING before the first write (see validate_week).
            if let Some(o) = &outline {
                validate_outline(o)?;
            }
            for week in &weeks {
                validate_week(week)?;
            }

            let repos = context.resources.repos();

            // The conversation's coach — not the LLM's argument — is authoritative
            // so save and plan injection agree on the coach slug.
            let coach = resolve_coach_slug(
                repos,
                context.conversation_id.as_deref(),
                tenant,
                &user_id,
                arg_coach,
            )
            .await?;

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
            // so a re-save never mints a duplicate.
            if let (Some(o), None) = (&outline, &goal_fact_id) {
                goal_fact_id = Some(ensure_goal_fact(repos, tenant, &user_id, &o.goal_race).await?);
            }

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
                "goal_race": race_summary,
                "superseded_plan_id": bundle.superseded_plan_id,
                "goal_fact_id": goal_fact_id,
                "weeks_saved": week_ids.len(),
                "weeks": week_ids,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
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
