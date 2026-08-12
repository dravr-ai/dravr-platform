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
use pierre_core::models::{ConversationRecord, LoadSnapshot, OnboardingState, Pillar, TenantId};
use pierre_database::repositories::{
    PlanOutlineInput, PlanWeekInput, SavePlanBundleParams, UpsertUserFactParams,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{
    parse_plan_date, GoalRace, PlanBlock, PlannedDay, RacePriority, MAX_DAYS_PER_WEEK,
};
use pierre_memory::{FactKind, FactSource, MemoryScope};
use pierre_services::ramp_check::{assess_ramp, RampVerdict};
use pierre_services::recent_load::recent_load_snapshot;
use pierre_services::training_plan_render::plan_goal_is_stale;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

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

/// Schema for the outline half of the save payload.
///
/// A named function rather than an inline block so the rejection skeleton in
/// [`shape_hint`] is generated from the very schema the tool advertises. A
/// hand-written copy of the expected shape would be free to drift from what
/// the deserializer actually accepts, which is the whole defect being fixed.
fn outline_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "goal_race".to_owned(),
        race_schema("The goal (A) race this plan builds toward."),
    );
    p.insert(
        "races".to_owned(),
        array_prop(
            "Other races on the calendar (B/C priorities).",
            race_schema("A secondary race."),
        ),
    );
    p.insert(
        "strategy".to_owned(),
        string_prop(
            "The coach's strategy in prose — what the athlete sees as the long-term direction.",
        ),
    );
    p.insert(
        "blocks".to_owned(),
        array_prop(
            "Ordered training blocks from now to the goal race. Omit for a short plan that has no mesocycle structure.",
            block_schema(),
        ),
    );
    // `blocks` is deliberately absent from the required list: a two-week
    // "hold form then taper" plan has no mesocycle structure, and demanding
    // one forced the coach to invent phase/start/weeks/intent before any plan
    // could be saved at all. The outline still needs a race to aim at and a
    // stated strategy.
    object_prop(
        "The plan outline (goal race + strategy, optionally blocks). Required when creating a plan; omit to adjust weeks of the existing active plan. Re-sending an outline supersedes the athlete's current plan.",
        p,
        vec!["goal_race".to_owned(), "strategy".to_owned()],
    )
}

/// Schema for the `weeks` half of the save payload.
fn weeks_schema() -> PropertySchema {
    array_prop(
        "Day-by-day weeks to save. Send the full multi-week detail when the athlete asks to see the whole plan; send a single adjusted week for 'move Tuesday to Wednesday' changes.",
        week_schema(),
    )
}

/// A shape-correct JSON skeleton for `prop`, each leaf carrying its own
/// description as the placeholder value.
///
/// Handed back when a payload fails to deserialize so the model's next
/// iteration can see the exact field names it got wrong — the tool loop runs
/// another LLM turn after a tool error, so an actionable rejection can convert
/// into a successful save within the same turn. Using the description as the
/// placeholder keeps the format hints ("Race date, YYYY-MM-DD.") in the hint
/// itself rather than stripping them to an empty string.
fn shape_hint(prop: &PropertySchema) -> Value {
    match prop.property_type.as_str() {
        "object" => {
            let mut map = serde_json::Map::new();
            if let Some(props) = prop.properties.as_ref() {
                for (name, child) in props {
                    map.insert(name.clone(), shape_hint(child));
                }
            }
            Value::Object(map)
        }
        "array" => prop
            .items
            .as_ref()
            .map_or_else(|| json!([]), |item| json!([shape_hint(item)])),
        "integer" => json!(0),
        "number" => json!(0.0),
        "boolean" => json!(false),
        _ => Value::String(prop.description.clone().unwrap_or_default()),
    }
}

/// Deserialize one half of the save payload, turning a shape mismatch into a
/// message that carries the schema the tool actually accepts.
///
/// `Ok(None)` when the field was simply not sent — both halves are optional on
/// their own, and "nothing to save" is caught separately.
fn parse_payload_part<T: DeserializeOwned>(
    raw: Option<&Value>,
    field: &str,
    schema: &PropertySchema,
) -> Result<Option<T>, String> {
    let Some(value) = raw.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|e| {
            format!(
                "`{field}` does not match the schema: {e}. Send exactly this shape \
                 (each value below describes the field): {}",
                shape_hint(schema)
            )
        })
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
const MAX_SHORT_TEXT_LEN: usize = 200;
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

/// Validate one week payload (dates, day count, day dates inside the week).
fn validate_week(week: &WeekPayload) -> AppResult<()> {
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
    for day in &week.days {
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
async fn load_conversation(
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
fn resolve_coach_slug(
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
            let tenant = TenantId::from_uuid(context.require_tenant()?);
            let tenant_id = tenant.to_string();
            let user_id = ctx_user_id(&context);
            let arg_coach = optional_string_field(&args, "coach_id");
            let include_history = args
                .get("include_history")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let repos = context.resources.repos();
            let conv =
                load_conversation(repos, context.conversation_id.as_deref(), tenant, &user_id)
                    .await?;
            let coach = resolve_coach_slug(conv.as_ref(), arg_coach);
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
            let tenant = TenantId::from_uuid(context.require_tenant()?);
            let tenant_id = tenant.to_string();
            let user_id = ctx_user_id(&context);
            let arg_coach = optional_string_field(&args, "coach_id");
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
            let weeks: Vec<WeekPayload> =
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
            for week in &weeks {
                validate_week(week)?;
            }

            let repos = context.resources.repos();
            let conv =
                load_conversation(repos, context.conversation_id.as_deref(), tenant, &user_id)
                    .await?;

            // Enforcement half of the guided-flow tool withhold. The pipeline
            // already drops this tool from the prompt's tool list and from the
            // native function declarations while a profile walk is active, but
            // neither covers the native-MCP path, where an ACP subprocess reads
            // `tools/list` straight off the `/mcp` endpoint. That path carries
            // no conversation, so the walk is resolved from the athlete there
            // (see `guided_flow_is_active`) and the withhold holds on every
            // surface, including a direct MCP call.
            if guided_flow_is_active(repos, conv.as_ref(), tenant, &user_id).await? {
                return Err(AppError::invalid_input(
                    "this conversation is building the athlete's profile one topic at a time — \
                     finish the profile walk before saving a training plan",
                ));
            }

            let coach = resolve_coach_slug(conv.as_ref(), arg_coach);

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

/// Newest conversations consulted when resolving an athlete's guided-flow state
/// without a conversation in scope. An interview keeps its conversation the
/// most recently updated one, so a running walk is always inside this window.
const GUIDED_FLOW_SCAN_LIMIT: i64 = 50;

/// `true` when a guided interview owns the turn and write tools are withheld.
///
/// The conversation is authoritative when the call arrives through the chat
/// pipeline, which puts its id in scope. A `tools/call` on the `/mcp` endpoint
/// has none, so the state is resolved from the athlete's conversations instead —
/// the `conversation_id` argument is model-supplied and cannot be trusted to
/// answer a question about whether this same model may write.
async fn guided_flow_is_active(
    repos: &RepositoryRegistry,
    conv: Option<&ConversationRecord>,
    tenant: TenantId,
    user_id: &str,
) -> AppResult<bool> {
    if let Some(conv) = conv {
        return Ok(OnboardingState::from_column(conv.onboarding_state.as_deref()).is_some());
    }
    let states = repos
        .chat
        .list_user_onboarding_states(user_id, tenant, GUIDED_FLOW_SCAN_LIMIT)
        .await?;
    Ok(states
        .iter()
        .any(|raw| OnboardingState::from_column(Some(raw)).is_some()))
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

/// The athlete's recent-load baseline for the ramp check, or `None` when it
/// cannot be read. Never propagates an error: the plan is already saved.
async fn ramp_baseline(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: &str,
) -> Option<LoadSnapshot> {
    let uuid = match Uuid::parse_str(user_id) {
        Ok(uuid) => uuid,
        Err(e) => {
            warn!(error = %e, "ramp check: unparseable user id");
            return None;
        }
    };
    recent_load_snapshot(repos.activity_cache.as_ref(), uuid, &tenant)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "ramp check: activity cache unreadable");
            None
        })
}

/// Emit the ramp verdict, including the explicit "could not measure" case so a
/// quiet log means the comparison ran and passed.
fn emit_ramp_verdict(plan_id: &str, verdict: &RampVerdict) {
    match verdict {
        RampVerdict::Exceeded {
            planned_hours,
            baseline_hours,
            increase,
        } => {
            warn!(
                target: "notify",
                event = "training_plan.ramp_warning",
                plan_id = %plan_id,
                planned_hours = format!("{planned_hours:.1}"),
                baseline_hours = format!("{baseline_hours:.1}"),
                increase_pct = format!("{:.0}", increase * 100.0),
                "planned opening week exceeds the athlete's recent weekly load"
            );
        }
        RampVerdict::WithinThreshold {
            planned_hours,
            baseline_hours,
        } => {
            info!(
                target: "notify",
                event = "training_plan.ramp_checked",
                plan_id = %plan_id,
                planned_hours = format!("{planned_hours:.1}"),
                baseline_hours = format!("{baseline_hours:.1}"),
                "planned opening week is within the athlete's recent weekly load"
            );
        }
        RampVerdict::Unmeasurable(reason) => {
            info!(
                target: "notify",
                event = "training_plan.ramp_unmeasurable",
                plan_id = %plan_id,
                reason = reason.as_str(),
                "could not compare the planned opening week against recent load"
            );
        }
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
