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
    SavePlanWeekParams, SaveTrainingPlanParams, UpsertUserFactParams,
};
use pierre_memory::training_plans::{
    parse_plan_date, GoalRace, PlanBlock, PlannedDay, MAX_DAYS_PER_WEEK,
};
use pierre_memory::{FactKind, FactSource, MemoryScope};
use serde::Deserialize;
use serde_json::{json, Value};

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
    p.insert("weeks".to_owned(), number_prop("Block length in weeks."));
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
        number_prop("Planned duration in minutes; omit for rest days."),
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

/// Validate every date/shape constraint BEFORE any write so a rejected week
/// can never leave a half-saved plan behind (the repo calls are separate
/// transactions; validation is the atomicity line).
fn validate_outline(outline: &OutlinePayload) -> AppResult<()> {
    if parse_plan_date(&outline.goal_race.date).is_none() {
        return Err(AppError::invalid_input(format!(
            "goal_race.date must be YYYY-MM-DD, got '{}'",
            outline.goal_race.date
        )));
    }
    for race in &outline.races {
        if parse_plan_date(&race.date).is_none() {
            return Err(AppError::invalid_input(format!(
                "race '{}' date must be YYYY-MM-DD, got '{}'",
                race.name, race.date
            )));
        }
    }
    if outline.strategy.trim().is_empty() {
        return Err(AppError::invalid_input(
            "outline.strategy must state the coach's plan in prose",
        ));
    }
    if outline.blocks.is_empty() {
        return Err(AppError::invalid_input(
            "outline.blocks must contain at least one block",
        ));
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
    }
    Ok(())
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
            let tenant_id = TenantId::from(context.require_tenant()?).to_string();
            let user_id = ctx_user_id(&context);
            let coach = optional_string_field(&args, "coach_id");
            let include_history = args
                .get("include_history")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let repos = context.resources.repos();
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

            Ok(ToolResult::ok(json!({
                "plan": plan,
                "weeks": weeks,
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
            let coach = optional_string_field(&args, "coach_id");
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

            // Close the pillar loop: an outline whose goal race has no linked
            // Goal fact writes one, so the living goal and the plan snapshot
            // stay connected (and /pillars re-screens converge on this row).
            if let (Some(o), None) = (&outline, &goal_fact_id) {
                let object = format!(
                    "{} ({}) on {} — priority {}",
                    o.goal_race.name,
                    o.goal_race.discipline,
                    o.goal_race.date,
                    serde_json::to_value(o.goal_race.priority)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                );
                // The goal is the ATHLETE's truth, not coach-scoped: any coach
                // (and the /pillars walk) must converge on this same fact row,
                // so it is written coach-agnostic.
                let fact = repos
                    .memory
                    .upsert_user_fact(&UpsertUserFactParams {
                        tenant_id: tenant,
                        user_id: &user_id,
                        coach_id: None,
                        scope: MemoryScope::User,
                        kind: FactKind::Goal,
                        pillar: Some(Pillar::TrainingAndMovement),
                        subject: "you",
                        predicate: "target race",
                        object: &object,
                        confidence: 0.95,
                        source: FactSource::Coach,
                        valid_until: None,
                        source_msg_id: None,
                        embedding: None,
                    })
                    .await?;
                goal_fact_id = Some(fact.id);
            }

            // Resolve the plan the weeks attach to: a fresh outline save, or
            // the athlete's existing active plan.
            let (plan_id, superseded_plan_id, race_summary) = if let Some(o) = &outline {
                let saved = repos
                    .training_plans
                    .save_training_plan(&SaveTrainingPlanParams {
                        tenant_id: &tenant_id,
                        user_id: &user_id,
                        coach_slug: coach.as_deref(),
                        goal_fact_id: goal_fact_id.as_deref(),
                        goal_race: &o.goal_race,
                        races: &o.races,
                        strategy: &o.strategy,
                        blocks: &o.blocks,
                        source_conversation_id: conversation_id.as_deref(),
                    })
                    .await?;
                let summary = format!("{} on {}", saved.goal_race.name, saved.goal_race.date);
                (saved.id, saved.supersedes_id, summary)
            } else {
                let existing = repos
                    .training_plans
                    .get_active_plan(&tenant_id, &user_id, coach.as_deref())
                    .await?
                    .ok_or_else(|| {
                        AppError::invalid_input(
                            "no active plan to attach weeks to — save an outline first",
                        )
                    })?;
                let summary = format!("{} on {}", existing.goal_race.name, existing.goal_race.date);
                (existing.id, None, summary)
            };

            let mut week_ids = Vec::with_capacity(weeks.len());
            for week in &weeks {
                let saved = repos
                    .training_plans
                    .save_plan_week(&SavePlanWeekParams {
                        tenant_id: &tenant_id,
                        user_id: &user_id,
                        plan_id: &plan_id,
                        week_start: &week.week_start,
                        focus: &week.focus,
                        days: &week.days,
                        adjustment_reason: &week.adjustment_reason,
                    })
                    .await?;
                week_ids.push(json!({
                    "week_start": saved.week_start,
                    "week_id": saved.id,
                    "superseded": saved.supersedes_id.is_some(),
                }));
            }

            Ok(ToolResult::ok(json!({
                "plan_id": plan_id,
                "goal_race": race_summary,
                "superseded_plan_id": superseded_plan_id,
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

crate::declare_security!(GetTrainingPlanTool => empty);
crate::declare_security!(SaveTrainingPlanTool => empty);
