// ABOUTME: Endurance Phase 5 MCP tools — list_workout_templates + prescribe_workout
// ABOUTME: Surfaces the cornerstone workouts and pushes a prescription to the athlete's Intervals.icu calendar
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{
    IntensityDistribution, PrescribedWorkout, SportType, TenantId, WorkoutStep, WorkoutTargetZones,
    WorkoutTemplate,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use pierre_services::workout_library::{cornerstone_by_slug, cornerstone_templates};

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// The provider a prescription is written to.
///
/// Intervals.icu is the only connected backend with a writable training
/// calendar — every other provider inherits the `push_planned_workout`
/// default, which reports the capability as unsupported.
const PRESCRIBE_PROVIDER: &str = "intervals_icu";

/// Audit status for a prescription that reached the athlete's calendar.
const STATUS_PUSHED: &str = "pushed";

/// Audit status for a prescription the provider refused.
const STATUS_FAILED: &str = "failed";

/// Upper bounds on an inline session. A session is serialized into the audit
/// row and rendered into the calendar event body, so an unbounded payload
/// inflates both; these keep one degenerate call from doing that while staying
/// far above any real coached session.
const MAX_SESSION_STEPS: usize = 50;
const MAX_STEP_REPEAT: u32 = 100;
const MAX_STEP_DURATION_SECONDS: u32 = 6 * 60 * 60;
const MAX_SESSION_DURATION_MINUTES: u64 = 24 * 60;
const MAX_STEP_DISTANCE_METERS: f64 = 1_000_000.0;
const MAX_NAME_LEN: usize = 200;
const MAX_SHORT_TEXT_LEN: usize = 200;
const MAX_NOTE_LEN: usize = 1_000;

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

/// A structured session the coach authored in conversation, rather than one of
/// the compiled-in cornerstones.
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
    /// Ordered steps that make up the session.
    structure: Vec<WorkoutStep>,
}

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

/// Reject a text field that is blank or over-long.
fn required_text(field: &str, value: &str, max: usize) -> AppResult<()> {
    if value.trim().is_empty() {
        return Err(AppError::invalid_input(format!(
            "{field} must not be empty"
        )));
    }
    bounded(field, value, max)
}

/// Validate one step and return the seconds it contributes to the session.
///
/// `index` names the offending step in every rejection, so a model fixing a
/// 12-step payload learns which one to change rather than re-guessing.
fn step_seconds(index: usize, step: &WorkoutStep) -> AppResult<u64> {
    let at = |field: &str| format!("session.structure[{index}].{field}");
    required_text(&at("label"), &step.label, MAX_SHORT_TEXT_LEN)?;
    required_text(&at("target_zone"), &step.target_zone, MAX_SHORT_TEXT_LEN)?;
    if let Some(note) = step.note.as_deref() {
        bounded(&at("note"), note, MAX_NOTE_LEN)?;
    }
    if !(1..=MAX_STEP_DURATION_SECONDS).contains(&step.duration_seconds) {
        return Err(AppError::invalid_input(format!(
            "{} must be between 1 and {MAX_STEP_DURATION_SECONDS}, got {}",
            at("duration_seconds"),
            step.duration_seconds
        )));
    }
    if !(1..=MAX_STEP_REPEAT).contains(&step.repeat) {
        return Err(AppError::invalid_input(format!(
            "{} must be between 1 and {MAX_STEP_REPEAT}, got {}",
            at("repeat"),
            step.repeat
        )));
    }
    if let Some(distance) = step.distance_meters {
        if !distance.is_finite() || distance <= 0.0 || distance > MAX_STEP_DISTANCE_METERS {
            return Err(AppError::invalid_input(format!(
                "{} must be between 0 and {MAX_STEP_DISTANCE_METERS}, got {distance}",
                at("distance_meters")
            )));
        }
    }
    Ok(u64::from(step.duration_seconds) * u64::from(step.repeat))
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
        total_seconds += step_seconds(index, step)?;
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
    let template = WorkoutTemplate {
        id: existing.map_or_else(Uuid::new_v4, |t| t.id),
        tenant_id: Some(tenant_id.as_uuid()),
        user_id: Some(user_id),
        slug,
        name: session.name,
        sport: session.sport,
        duration_minutes,
        intensity_distribution: session.intensity_distribution,
        structure: session.structure,
        // An inline session carries no zone overlay: its steps name the zones
        // directly, and the athlete's own zones apply underneath.
        target_zones: WorkoutTargetZones {
            hr_pct_of_lt2: None,
            power_pct_of_ftp: None,
        },
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
            if let Some(template) = cornerstone_by_slug(slug) {
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
                        "no workout template with slug '{slug}' among the cornerstones or \
                         this athlete's saved sessions"
                    ))
                })
        }
    }
}

/// `list_workout_templates` — read-only catalog of cornerstones.
pub struct ListWorkoutTemplatesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListWorkoutTemplatesTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: Some(Vec::new()),
        };
        tool_definition(
            "list_workout_templates",
            "List the Endurance cornerstone workout templates: long_run_z2, \
             threshold_4x8, vo2_5x3, recovery_30min, tempo_progression, \
             sweet_spot_2x20. Each row carries the structured steps + target \
             zones the prescribe_workout tool will push to the user's \
             Intervals.icu calendar.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let result: AppResult<ToolResult> = async move {
            drop(args);
            let templates = cornerstone_templates();
            let payload = serde_json::to_value(&templates)
                .map_err(|e| AppError::internal(format!("serialize templates: {e}")))?;
            Ok(ToolResult::ok(json!({
                "count": templates.len(),
                "templates": payload,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Schema for one step of an inline structured session.
fn session_step_schema() -> PropertySchema {
    let mut p = HashMap::new();
    p.insert(
        "label".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "What this step is (\"Warm-up\", \"Montées\", \"Interval\", \"Cool-down\")."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    p.insert(
        "duration_seconds".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some("How long ONE repetition of this step lasts, in seconds.".to_owned()),
            ..Default::default()
        },
    );
    p.insert(
        "target_zone".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Intensity RELATIVE to the athlete's thresholds (\"Z2\", \"Threshold\", \
                 \"88-93% FTP\"). Never absolute watts."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    p.insert(
        "repeat".to_owned(),
        PropertySchema {
            property_type: "integer".to_owned(),
            description: Some("Repetitions of this step; omit for a single block.".to_owned()),
            ..Default::default()
        },
    );
    p.insert(
        "distance_meters".to_owned(),
        PropertySchema {
            property_type: "number".to_owned(),
            description: Some("Distance in metres for a distance-based step.".to_owned()),
            ..Default::default()
        },
    );
    p.insert(
        "note".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Coaching cue for this step, in your own voice — it reaches the athlete's \
                 calendar entry."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    PropertySchema {
        property_type: "object".to_owned(),
        description: Some("One step of the session.".to_owned()),
        properties: Some(p),
        required: Some(vec![
            "label".to_owned(),
            "duration_seconds".to_owned(),
            "target_zone".to_owned(),
        ]),
        ..Default::default()
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
        "structure".to_owned(),
        PropertySchema {
            property_type: "array".to_owned(),
            description: Some("The steps in order. Total duration is summed from them.".to_owned()),
            items: Some(Box::new(session_step_schema())),
            ..Default::default()
        },
    );
    PropertySchema {
        property_type: "object".to_owned(),
        description: Some(
            "A structured session you authored, for anything the cornerstones do not express."
                .to_owned(),
        ),
        properties: Some(p),
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
                    "Slug of a stored template: one of the six cornerstones, or a session \
                     you prescribed this athlete before. Use `session` instead for anything \
                     new."
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["date".to_owned()]),
        };
        tool_definition(
            "prescribe_workout",
            "Write one workout onto the athlete's Intervals.icu calendar for a \
             given date, and record it in the prescribed_workouts audit trail. \
             Requires a connected Intervals.icu account. Pass EITHER \
             template_slug — one of the cornerstones (long_run_z2, \
             threshold_4x8, vo2_5x3, recovery_30min, tempo_progression, \
             sweet_spot_2x20) or a session you prescribed this athlete before — \
             OR session, a structured session you authored for anything those do \
             not express. Args: date (YYYY-MM-DD), template_slug or session, \
             optional coach_id. Creates a new calendar entry every call: it \
             cannot edit or replace one already there.",
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

            let template = resolve_template(&context, tenant_id, user_id, &args).await?;
            let payload_json = serde_json::to_string(&template)
                .map_err(|e| AppError::internal(format!("serialize template: {e}")))?;

            let tenant_str = tenant_id.to_string();
            let provider = AuthService::new(context.resources.clone())
                .create_authenticated_provider(
                    PRESCRIBE_PROVIDER,
                    user_id,
                    Some(tenant_str.as_str()),
                )
                .await
                .map_err(|resp| {
                    AppError::invalid_input(resp.error.unwrap_or_else(|| {
                        "Connect an Intervals.icu account before prescribing a workout — \
                         it is the only calendar this platform can write to"
                            .to_owned()
                    }))
                })?;

            // The audit row records the attempt either way. A prescription the
            // provider refused is a fact the coach and the athlete both need,
            // and an audit trail that holds only successes cannot answer the
            // one question it exists for: did this workout reach the athlete?
            let push = provider.push_planned_workout(&template, date).await;
            let (status, provider_event_id) = push.as_ref().map_or((STATUS_FAILED, None), |id| {
                (STATUS_PUSHED, Some(id.clone()))
            });
            let prescription_id = Uuid::new_v4();
            let prescribed = PrescribedWorkout {
                id: prescription_id,
                tenant_id: tenant_id.as_uuid(),
                user_id,
                coach_id,
                template_slug: template.slug.clone(),
                sport: template.sport.clone(),
                prescribed_for_date: date,
                provider: PRESCRIBE_PROVIDER.to_owned(),
                provider_event_id: provider_event_id.clone(),
                payload_json,
                status: status.to_owned(),
                created_at: Utc::now(),
            };
            let audit = context
                .resources
                .repos()
                .prescribed_workouts
                .upsert_prescribed_workout(&prescribed)
                .await;
            if let Err(e) = &audit {
                warn!(error = %e, "prescribe_workout: prescription audit row not saved");
            }

            // A push failure is the root cause and outranks a failed audit
            // write. Only once the push is known to have landed does an audit
            // failure surface — and then it must say the event IS on the
            // calendar, because a coach told "the prescription failed" would
            // retry, and a retry duplicates the entry (there is no edit path).
            let event_id = push?;
            audit.map_err(|e| {
                AppError::internal(format!(
                    "the workout IS on the athlete's Intervals.icu calendar (event {event_id}) \
                     but the prescription audit row failed to save — do not prescribe it again: {e}"
                ))
            })?;

            Ok(ToolResult::ok(json!({
                "prescription_id": prescription_id,
                "provider": PRESCRIBE_PROVIDER,
                "provider_event_id": event_id,
                "template_slug": template.slug,
                "name": template.name,
                "duration_minutes": template.duration_minutes,
                "scheduled_for": date.format("%Y-%m-%d").to_string(),
                "status": STATUS_PUSHED,
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
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(ListWorkoutTemplatesTool => empty);
crate::declare_security!(PrescribeWorkoutTool => empty);
