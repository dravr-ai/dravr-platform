// ABOUTME: Endurance MCP tools — list_workout_templates, prescribe_workout, withdraw_prescribed_workout
// ABOUTME: Lists the training catalogue's workout bank by purpose, phase and sport, and writes, replaces, or removes one prescription on the athlete's calendar
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{BTreeSet, HashMap};
use std::slice;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::periodization::{
    EvidenceTier, PhaseFit, PhaseKind, Progression, WorkoutFilter, WorkoutParams, WorkoutPurpose,
};
use pierre_core::models::{
    CalendarEventSource, CalendarKey, IntensityDistribution, PlannedSession, PrescribedWorkout,
    SportType, TenantId, WorkoutStep, WorkoutTargetZones, WorkoutTemplate,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use pierre_services::plan_calendar_push::CALENDAR_PROVIDER;

use super::calendar::{
    calendar_provider, destructive_annotations, required_text, step_schema, validate_step,
    TargetRule, MAX_SESSION_STEPS,
};
use super::training_plan_telemetry::{emit_calendar_sync_completed, emit_calendar_sync_failed};
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// Tool names, as the notify events name the trigger.
const PRESCRIBE_TOOL: &str = "prescribe_workout";
const WITHDRAW_TOOL: &str = "withdraw_prescribed_workout";

/// Upper bounds on an inline session beyond the per-step ones in
/// [`super::calendar`]: its name, and its total once the steps are summed. A
/// session is serialized into the audit row and rendered into the calendar
/// event body, so an unbounded payload inflates both.
const MAX_SESSION_DURATION_MINUTES: u64 = 24 * 60;
const MAX_NAME_LEN: usize = 200;

fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        open_world_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

fn write_safe_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

fn require_tenant(context: &ToolExecutionContext) -> AppResult<TenantId> {
    context.tenant_id.map(TenantId::from_uuid).ok_or_else(|| {
        AppError::new(
            ErrorCode::AuthInvalid,
            "Endurance workout tools require an active tenant context",
        )
    })
}

/// A structured session the coach authored in conversation, rather than one
/// the catalogue's workout bank carries.
///
/// `structure` deserializes straight into [`WorkoutStep`] so the argument shape
/// and the stored shape cannot drift: the step defaults (`repeat` of 1, absent
/// distance and note) are the model's own.
#[derive(Deserialize)]
struct SessionPayload {
    /// Session name as the coach states it to the athlete.
    name: String,
    /// Sport the session is for.
    sport: SportType,
    /// Seiler-style distribution label for downstream coaching cues.
    intensity_distribution: IntensityDistribution,
    /// What the session is for, in the catalogue's purpose vocabulary. When
    /// absent the purpose follows from the intensity distribution.
    #[serde(default)]
    purpose: Option<WorkoutPurpose>,
    /// Ordered steps that make up the session.
    structure: Vec<WorkoutStep>,
}

/// Validate an inline session and return its total duration in minutes.
///
/// The total is derived from the steps rather than accepted as an argument so
/// the stored `duration_minutes` can never contradict the structure it
/// summarizes. It rounds up, so a session never understates what it asks of the
/// athlete.
fn session_duration_minutes(session: &SessionPayload) -> AppResult<u32> {
    required_text("session.name", &session.name, MAX_NAME_LEN)?;
    if session.structure.is_empty() {
        return Err(AppError::invalid_input(
            "session.structure must contain at least one step",
        ));
    }
    if session.structure.len() > MAX_SESSION_STEPS {
        return Err(AppError::invalid_input(format!(
            "session.structure has {} steps; max {MAX_SESSION_STEPS}",
            session.structure.len()
        )));
    }

    let mut total_seconds: u64 = 0;
    for (index, step) in session.structure.iter().enumerate() {
        total_seconds += validate_step(
            &format!("session.structure[{index}]"),
            step,
            TargetRule::AnyLabel,
        )?;
    }

    let minutes = total_seconds.div_ceil(60);
    if minutes > MAX_SESSION_DURATION_MINUTES {
        return Err(AppError::invalid_input(format!(
            "session totals {minutes} minutes; max {MAX_SESSION_DURATION_MINUTES}"
        )));
    }
    u32::try_from(minutes)
        .map_err(|e| AppError::internal(format!("session duration does not fit in u32: {e}")))
}

/// Derive the storage slug for an inline session from its name.
///
/// Two prescriptions of the same named session converge on one stored template
/// rather than accumulating near-duplicates, which is what makes re-prescribing
/// "Trail technique 55 min" idempotent at the library level.
fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('_') {
            slug.push('_');
        }
    }
    slug.trim_end_matches('_').to_owned()
}

/// Persist an inline session as a user-authored template and return it.
///
/// The slug is unique per (`tenant_id`, `user_id`), so an existing row's id is
/// reused when one matches — minting a fresh id would collide with that unique
/// index on the second prescription of the same session.
async fn store_session(
    context: &ToolExecutionContext,
    tenant_id: TenantId,
    user_id: Uuid,
    session: SessionPayload,
) -> AppResult<WorkoutTemplate> {
    let duration_minutes = session_duration_minutes(&session)?;
    let slug = slugify(&session.name);
    if slug.is_empty() {
        return Err(AppError::invalid_input(format!(
            "session.name '{}' has no letters or digits to build a template slug from",
            session.name
        )));
    }
    let repos = context.resources.repos();
    let existing = repos
        .workout_templates
        .get_user_workout_template(tenant_id, user_id, &slug)
        .await?;
    // An inline session's readiness floor follows from its intensity
    // distribution, and so does its purpose unless the coach named one. It is
    // the coach's own judgement with no citation behind it.
    let (default_purpose, readiness_min) =
        WorkoutTemplate::inline_defaults(session.intensity_distribution);
    let purpose = session.purpose.unwrap_or(default_purpose);
    let template = WorkoutTemplate {
        id: existing.map_or_else(Uuid::new_v4, |t| t.id),
        tenant_id: Some(tenant_id.as_uuid()),
        user_id: Some(user_id),
        slug,
        name: session.name,
        sport: session.sport,
        duration_minutes,
        intensity_distribution: session.intensity_distribution,
        purpose,
        sport_variants: Vec::new(),
        evidence_tier: EvidenceTier::CoachJudgement,
        caveat: None,
        structure: session.structure,
        // An inline session carries no zone overlay: its steps name the zones
        // directly, and the athlete's own zones apply underneath.
        target_zones: WorkoutTargetZones {
            hr_pct_of_lt2: None,
            power_pct_of_ftp: None,
        },
        params: WorkoutParams::default(),
        progression: Progression::default(),
        fit: PhaseFit {
            readiness_min,
            ..PhaseFit::default()
        },
        evidence_refs: Vec::new(),
        is_compiled_in: false,
        updated_at: Utc::now(),
    };
    repos
        .workout_templates
        .upsert_workout_template(&template)
        .await?;
    Ok(template)
}

/// Resolve the workout this call prescribes: a named template or an inline
/// session, never both and never neither.
async fn resolve_template(
    context: &ToolExecutionContext,
    tenant_id: TenantId,
    user_id: Uuid,
    args: &Value,
) -> AppResult<WorkoutTemplate> {
    let slug = args.get("template_slug").and_then(Value::as_str);
    let session = args.get("session").filter(|v| !v.is_null());

    match (slug, session) {
        (Some(_), Some(_)) => Err(AppError::invalid_input(
            "provide either template_slug or session, not both",
        )),
        (None, None) => Err(AppError::invalid_input(
            "provide template_slug (a stored template) or session (a structured session you authored)",
        )),
        (None, Some(raw)) => {
            let session: SessionPayload = serde_json::from_value(raw.clone()).map_err(|e| {
                AppError::invalid_input(format!(
                    "`session` does not match the schema: {e}. Every step needs label, \
                     duration_seconds and target_zone; sport and intensity_distribution are \
                     lowercase enums."
                ))
            })?;
            store_session(context, tenant_id, user_id, session).await
        }
        (Some(slug), None) => {
            if let Some(template) = context.resources.training_catalogue().workout(slug) {
                return Ok(template);
            }
            context
                .resources
                .repos()
                .workout_templates
                .get_user_workout_template(tenant_id, user_id, slug)
                .await?
                .ok_or_else(|| {
                    AppError::not_found(format!(
                        "no workout template with slug '{slug}' in the catalogue or among \
                         this athlete's saved sessions — list_workout_templates names both"
                    ))
                })
        }
    }
}

/// The live prescription `prescription_id` names, owned by this athlete,
/// with the provider event id that holds it.
///
/// Only a single prescription qualifies: an entry the training plan put on
/// the calendar is changed by adjusting the plan and pushing it again, so the
/// plan and the calendar cannot drift apart through this door.
async fn live_prescription(
    context: &ToolExecutionContext,
    tenant_id: TenantId,
    user_id: Uuid,
    prescription_id: Uuid,
) -> AppResult<(PrescribedWorkout, String)> {
    let row = context
        .resources
        .repos()
        .prescribed_workouts
        .get_prescribed_workout(tenant_id, user_id, prescription_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "no prescription {prescription_id} for this athlete — get_training_plan lists \
                 the calendar entries with their ids"
            ))
        })?;
    if row.source != CalendarEventSource::Prescription {
        return Err(AppError::invalid_input(format!(
            "prescription {prescription_id} is an entry of the training plan — adjust the plan \
             with save_training_plan and push it with push_training_plan instead"
        )));
    }
    if !row.is_live() {
        return Err(AppError::invalid_input(format!(
            "prescription {prescription_id} is {} — it is no longer on the calendar",
            row.status
        )));
    }
    let event_id = row.provider_event_id.clone().ok_or_else(|| {
        AppError::internal(format!(
            "prescription {prescription_id} is recorded as pushed but carries no calendar \
             event id"
        ))
    })?;
    Ok((row, event_id))
}

/// Parse an optional prescription-id argument.
fn optional_prescription_id(args: &Value, key: &str) -> AppResult<Option<Uuid>> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            Uuid::parse_str(s).map_err(|e| {
                AppError::invalid_input(format!("{key} must be a prescription id (UUID): {e}"))
            })
        })
        .transpose()
}

/// The two shapes a listed template comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListDetail {
    /// The fields a coach picks a session by.
    Summary,
    /// The whole template, steps and target zones included.
    Full,
}

impl ListDetail {
    const ALL: &[Self] = &[Self::Summary, Self::Full];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Full => "full",
        }
    }
}

/// A trimmed optional string argument; `None` when absent, null or blank.
fn optional_text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The vocabulary names, comma-separated, for an error that has to name
/// every allowed value.
fn vocabulary<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> String {
    all.iter()
        .map(|value| name(*value))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve an optional closed-vocabulary argument, refusing a value outside
/// the vocabulary with the whole vocabulary in the message.
fn vocab_arg<T: Copy>(
    args: &Value,
    key: &str,
    all: &[T],
    name: fn(T) -> &'static str,
) -> AppResult<Option<T>> {
    let Some(raw) = optional_text(args, key) else {
        return Ok(None);
    };
    all.iter()
        .copied()
        .find(|value| name(*value) == raw)
        .map(Some)
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "{key} '{raw}' is not in the vocabulary; one of: {}",
                vocabulary(all, name)
            ))
        })
}

/// Resolve the optional `sport` argument: a `snake_case` [`SportType`] name.
///
/// An unknown name is refused with the sports the bank is written for, so
/// the coach can pick one that returns something.
fn sport_arg(args: &Value, bank: &[WorkoutTemplate]) -> AppResult<Option<SportType>> {
    let Some(raw) = optional_text(args, "sport") else {
        return Ok(None);
    };
    serde_json::from_value::<SportType>(Value::String(raw.to_owned()))
        .map(Some)
        .map_err(|_| {
            let mut sports = BTreeSet::new();
            for template in bank {
                for sport in slice::from_ref(&template.sport)
                    .iter()
                    .chain(&template.sport_variants)
                {
                    if let Ok(Value::String(name)) = serde_json::to_value(sport) {
                        sports.insert(name);
                    }
                }
            }
            AppError::invalid_input(format!(
                "sport '{raw}' is not a sport name; the bank is written for: {}",
                sports.into_iter().collect::<Vec<_>>().join(", ")
            ))
        })
}

/// The fields a coach picks a session by, without the steps.
fn summary_row(template: &WorkoutTemplate) -> AppResult<Value> {
    let params = serde_json::to_value(&template.params)
        .map_err(|e| AppError::internal(format!("serialize template params: {e}")))?;
    Ok(json!({
        "slug": template.slug,
        "name": template.name,
        "purpose": template.purpose,
        "sport": template.sport,
        "sport_variants": template.sport_variants,
        "duration_minutes": template.duration_minutes,
        "intensity_distribution": template.intensity_distribution,
        "evidence_tier": template.evidence_tier,
        "caveat": template.caveat,
        "params": params,
        "fit": {
            "phases": template.fit.phases,
            "readiness_min": template.fit.readiness_min,
            "max_per_week": template.fit.max_per_week,
            "min_spacing_hours": template.fit.min_spacing_hours,
        },
        "is_compiled_in": template.is_compiled_in,
    }))
}

/// `list_workout_templates` — the workout bank, filtered by purpose, phase
/// and sport, with the athlete's own saved sessions after it.
pub struct ListWorkoutTemplatesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListWorkoutTemplatesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "purpose".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(format!(
                    "Only templates with this purpose. One of: {}.",
                    vocabulary(WorkoutPurpose::ALL, WorkoutPurpose::as_str)
                )),
                ..Default::default()
            },
        );
        properties.insert(
            "phase".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(format!(
                    "Only templates that fit this season phase; a template written for \
                     any phase always matches. One of: {}.",
                    vocabulary(PhaseKind::ALL, PhaseKind::as_str)
                )),
                ..Default::default()
            },
        );
        properties.insert(
            "sport".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Only templates written for this sport, as the primary sport or a \
                     variant. A snake_case sport name: run, ride, swim, strength_training, …"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "detail".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "How much of each template to return: summary (default) — slug, name, \
                     purpose, sport, duration, evidence tier, parameter ranges and phase fit — \
                     or full, the whole template with its steps and target zones."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(Vec::new()));
        tool_definition(
            "list_workout_templates",
            "List the workout template bank by what a session is for. Every \
             template carries a purpose (recovery, endurance, endurance_long, \
             tempo, sweet_spot, threshold, vo2max_long, vo2max_short, sprint, \
             neuromuscular, race_specific, brick, strength_aa, strength_max, \
             strength_maint, plyometric, mobility), the season phases it fits, \
             the readiness level it needs, the sports it is written for, its \
             evidence tier, and parameter ranges (reps, work and rest seconds, \
             duration, RPE, intensity per sport) with a default the coach fills \
             in for the athlete. Filter with purpose, phase and sport; the reply \
             lists the athlete's own saved sessions after the bank. Pass detail = \
             full for the structured steps and target zones prescribe_workout \
             pushes to the athlete's Intervals.icu calendar.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let catalogue = context.resources.training_catalogue();
            let bank = catalogue.workouts();
            let filter = WorkoutFilter {
                purpose: vocab_arg(
                    &args,
                    "purpose",
                    WorkoutPurpose::ALL,
                    WorkoutPurpose::as_str,
                )?,
                phase: vocab_arg(&args, "phase", PhaseKind::ALL, PhaseKind::as_str)?,
                sport: sport_arg(&args, &bank)?,
            };
            let detail = vocab_arg(&args, "detail", ListDetail::ALL, ListDetail::as_str)?
                .unwrap_or(ListDetail::Summary);

            let mut templates = catalogue.workouts_matching(&filter);
            // The athlete's own sessions are scoped by tenant; a call with no
            // tenant context has no athlete rows to read.
            if let Some(tenant_id) = context.tenant_id.map(TenantId::from_uuid) {
                let own = context
                    .resources
                    .repos()
                    .workout_templates
                    .list_user_workout_templates(tenant_id, context.user_id)
                    .await?;
                templates.extend(own.into_iter().filter(|t| filter.matches(t)));
            }

            let rows = match detail {
                ListDetail::Summary => templates
                    .iter()
                    .map(summary_row)
                    .collect::<AppResult<Vec<Value>>>()?,
                ListDetail::Full => templates
                    .iter()
                    .map(|t| {
                        serde_json::to_value(t)
                            .map_err(|e| AppError::internal(format!("serialize template: {e}")))
                    })
                    .collect::<AppResult<Vec<Value>>>()?,
            };
            Ok(ToolResult::ok(json!({
                "count": rows.len(),
                "filters": {
                    "purpose": filter.purpose,
                    "phase": filter.phase,
                    "sport": filter.sport,
                    "detail": detail.as_str(),
                },
                "templates": rows,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Schema for an inline structured session.
fn session_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "name".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some("Session name as you state it to the athlete.".to_owned()),
            ..Default::default()
        },
    );
    p.insert(
        "sport".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some("Sport: run, ride, swim, walk, hike, ski, yoga, …".to_owned()),
            ..Default::default()
        },
    );
    p.insert(
        "intensity_distribution".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "One of: polarized, threshold, vo2max, recovery, pyramid.".to_owned(),
            ),
            ..Default::default()
        },
    );
    p.insert(
        "purpose".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(format!(
                "Optional: what the session is for, one of: {}. Omitted, it follows from \
                 intensity_distribution.",
                vocabulary(WorkoutPurpose::ALL, WorkoutPurpose::as_str)
            )),
            ..Default::default()
        },
    );
    p.insert(
        "structure".to_owned(),
        PropertySchema {
            property_type: "array".to_owned(),
            description: Some("The steps in order. Total duration is summed from them.".to_owned()),
            items: Some(Box::new(step_schema())),
            ..Default::default()
        },
    );
    PropertySchema {
        property_type: "object".to_owned(),
        description: Some(
            "A structured session you authored, for anything the workout bank does not express."
                .to_owned(),
        ),
        properties: Some(p.into_iter().collect()),
        required: Some(vec![
            "name".to_owned(),
            "sport".to_owned(),
            "intensity_distribution".to_owned(),
            "structure".to_owned(),
        ]),
        ..Default::default()
    }
}

/// `prescribe_workout` — push a workout to the athlete's Intervals.icu calendar
/// and record the prescription.
pub struct PrescribeWorkoutTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for PrescribeWorkoutTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "template_slug".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Slug of a stored template: one from the workout bank \
                     (list_workout_templates), or a session you prescribed this athlete \
                     before. Use `session` instead for anything new."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert("session".to_owned(), session_schema());
        properties.insert(
            "date".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Calendar date the workout is scheduled for (YYYY-MM-DD).".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Optional coach id stamped onto the audit row.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "replaces".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "prescription_id of an earlier prescription to change in place — the \
                     calendar entry keeps its slot and gets this workout. Omit to add a new \
                     entry."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["date".to_owned()]));
        tool_definition(
            "prescribe_workout",
            "Write one workout onto the athlete's Intervals.icu calendar for a \
             given date, and record it in the prescribed_workouts ledger. \
             Requires a connected Intervals.icu account. Pass EITHER \
             template_slug — a slug from the workout bank (list_workout_templates \
             filters it by purpose, phase and sport) or a session you prescribed \
             this athlete before — OR session, a structured session you authored \
             for anything those do not express. Args: date (YYYY-MM-DD), \
             template_slug or session, \
             optional coach_id, optional replaces. Without replaces every call \
             adds a new calendar entry; with replaces = a prescription_id (from an \
             earlier call, or from get_training_plan's calendar block) that entry \
             is changed in place instead. withdraw_prescribed_workout removes one.",
            schema,
            Some(write_safe_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::REQUIRES_PROVIDER
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
            let tenant_id = require_tenant(&context)?;
            let user_id = context.user_id;
            let date_str = args
                .get("date")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("date is required"))?;
            let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .map_err(|e| AppError::invalid_input(format!("date must be YYYY-MM-DD: {e}")))?;
            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .map(str::to_owned);

            let replaces = optional_prescription_id(&args, "replaces")?;

            let template = resolve_template(&context, tenant_id, user_id, &args).await?;
            // The previous entry is resolved before the provider is built, so a
            // bad id is refused without a credential lookup.
            let previous = match replaces {
                Some(id) => Some(live_prescription(&context, tenant_id, user_id, id).await?),
                None => None,
            };
            let provider = calendar_provider(&context, tenant_id, user_id).await?;

            let prescription_id = Uuid::new_v4();
            let session = PlannedSession::from_template(
                &template,
                date,
                CalendarKey::prescription(prescription_id),
            );
            let payload_json = serde_json::to_string(&session)
                .map_err(|e| AppError::internal(format!("serialize session: {e}")))?;
            let payload_hash = session
                .payload_hash()
                .map_err(|e| AppError::internal(format!("hash session: {e}")))?;

            // With `replaces`, the calendar entry keeps its provider id and gets
            // the new workout; without it, a new entry is created.
            let push: AppResult<String> = match &previous {
                Some((_, event_id)) => provider
                    .update_planned_session(event_id, &session)
                    .await
                    .map(|()| event_id.clone()),
                None => provider.push_planned_session(&session).await,
            };
            let (status, provider_event_id) = push.as_ref().map_or_else(
                |_| (PrescribedWorkout::STATUS_FAILED, None),
                |id| (PrescribedWorkout::STATUS_PUSHED, Some(id.clone())),
            );
            match &push {
                Ok(_) => emit_calendar_sync_completed(
                    tenant_id,
                    user_id,
                    CALENDAR_PROVIDER,
                    PRESCRIBE_TOOL,
                    1,
                ),
                Err(e) => emit_calendar_sync_failed(
                    tenant_id,
                    user_id,
                    CALENDAR_PROVIDER,
                    PRESCRIBE_TOOL,
                    &e.to_string(),
                ),
            }

            // The ledger records the attempt either way. A prescription the
            // provider refused is a fact the coach and the athlete both need,
            // and a ledger that holds only successes cannot answer the one
            // question it exists for: did this workout reach the athlete?
            let repos = context.resources.repos();
            // A landed replacement supersedes the previous row first, so the
            // one-live-row-per-key index never sees two live rows for the
            // same calendar entry.
            let superseded: AppResult<()> = match (&push, &previous) {
                (Ok(_), Some((prev, _))) => {
                    repos
                        .prescribed_workouts
                        .set_prescribed_workout_status(
                            tenant_id,
                            prev.id,
                            PrescribedWorkout::STATUS_REPLACED,
                        )
                        .await
                }
                _ => Ok(()),
            };
            let now = Utc::now();
            let prescribed = PrescribedWorkout {
                id: prescription_id,
                tenant_id: tenant_id.as_uuid(),
                user_id,
                coach_id,
                template_slug: Some(template.slug.clone()),
                sport: template.sport.clone(),
                prescribed_for_date: date,
                provider: CALENDAR_PROVIDER.to_owned(),
                provider_event_id: provider_event_id.clone(),
                external_id: Some(session.external_id.clone()),
                source: CalendarEventSource::Prescription,
                plan_week_id: None,
                replaces_id: previous.as_ref().map(|(prev, _)| prev.id),
                payload_hash: Some(payload_hash),
                payload_json,
                status: status.to_owned(),
                created_at: now,
                updated_at: now,
            };
            let audit: AppResult<()> = if let Err(e) = superseded {
                Err(e)
            } else {
                repos
                    .prescribed_workouts
                    .upsert_prescribed_workout(&prescribed)
                    .await
            };
            if let Err(e) = &audit {
                warn!(error = %e, "prescribe_workout: ledger row not saved");
            }

            // A push failure is the root cause and outranks a failed ledger
            // write. Only once the push is known to have landed does a ledger
            // failure surface — and then it must say the event IS on the
            // calendar, because a coach told "the prescription failed" would
            // retry, and a retry adds a second entry.
            let event_id = push?;
            audit.map_err(|e| {
                AppError::internal(format!(
                    "the workout IS on the athlete's Intervals.icu calendar (event {event_id}) \
                     but the prescription ledger row failed to save — do not prescribe it again: {e}"
                ))
            })?;

            Ok(ToolResult::ok(json!({
                "prescription_id": prescription_id,
                "provider": CALENDAR_PROVIDER,
                "provider_event_id": event_id,
                "replaced_prescription_id": previous.as_ref().map(|(prev, _)| prev.id),
                "template_slug": template.slug,
                "name": template.name,
                "duration_minutes": template.duration_minutes,
                "scheduled_for": date.format("%Y-%m-%d").to_string(),
                "status": PrescribedWorkout::STATUS_PUSHED,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// `withdraw_prescribed_workout` — remove one prescription from the athlete's
/// calendar and mark it withdrawn in the ledger.
pub struct WithdrawPrescribedWorkoutTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for WithdrawPrescribedWorkoutTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "prescription_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "prescription_id of the entry to remove — from the prescribe_workout \
                     reply, or from get_training_plan's calendar block."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["prescription_id".to_owned()]));
        tool_definition(
            "withdraw_prescribed_workout",
            "Remove a workout that prescribe_workout wrote to the athlete's Intervals.icu \
             calendar: deletes the calendar entry and marks the prescription withdrawn. \
             Only for single prescriptions — an entry the training plan put there is \
             removed by adjusting the plan (save_training_plan) and pushing it \
             (push_training_plan). Args: prescription_id.",
            schema,
            Some(destructive_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::REQUIRES_PROVIDER
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
            let tenant_id = require_tenant(&context)?;
            let user_id = context.user_id;
            let prescription_id = optional_prescription_id(&args, "prescription_id")?
                .ok_or_else(|| AppError::invalid_input("prescription_id is required"))?;
            let (row, event_id) =
                live_prescription(&context, tenant_id, user_id, prescription_id).await?;
            let provider = calendar_provider(&context, tenant_id, user_id).await?;

            let deleted = provider
                .delete_planned_sessions(slice::from_ref(&event_id))
                .await;
            match &deleted {
                Ok(_) => emit_calendar_sync_completed(
                    tenant_id,
                    user_id,
                    CALENDAR_PROVIDER,
                    WITHDRAW_TOOL,
                    1,
                ),
                Err(e) => emit_calendar_sync_failed(
                    tenant_id,
                    user_id,
                    CALENDAR_PROVIDER,
                    WITHDRAW_TOOL,
                    &e.to_string(),
                ),
            }
            deleted?;

            // The entry is gone from the calendar. If the ledger cannot record
            // that, say so in those terms — a retry would find nothing to
            // delete, which the provider ignores, so the row can be repaired
            // by calling again.
            context
                .resources
                .repos()
                .prescribed_workouts
                .set_prescribed_workout_status(
                    tenant_id,
                    row.id,
                    PrescribedWorkout::STATUS_WITHDRAWN,
                )
                .await
                .map_err(|e| {
                    AppError::internal(format!(
                        "the entry is gone from the athlete's Intervals.icu calendar (event \
                         {event_id}) but the ledger could not record the withdrawal — call \
                         again to repair the row: {e}"
                    ))
                })?;

            Ok(ToolResult::ok(json!({
                "prescription_id": prescription_id,
                "provider": CALENDAR_PROVIDER,
                "provider_event_id": event_id,
                "name": row.template_slug,
                "scheduled_for": row.prescribed_for_date.format("%Y-%m-%d").to_string(),
                "status": PrescribedWorkout::STATUS_WITHDRAWN,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Build the Endurance workout-tool list for registry registration.
#[must_use]
pub fn create_endurance_workout_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ListWorkoutTemplatesTool),
        Box::new(PrescribeWorkoutTool),
        Box::new(WithdrawPrescribedWorkoutTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
// A prescription — including one that replaces another — is recoverable from
// the ledger row it superseded, so it is a plain write; a withdrawal deletes
// the calendar entry on the provider and is the destructive one.
crate::declare_security!(ListWorkoutTemplatesTool => empty);
crate::declare_security!(PrescribeWorkoutTool => empty);
crate::declare_security!(WithdrawPrescribedWorkoutTool => IRREVERSIBLE);
