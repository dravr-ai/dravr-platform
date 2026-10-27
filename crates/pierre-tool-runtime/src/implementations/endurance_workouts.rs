// ABOUTME: Endurance MCP tools — list_workout_templates, prescribe_workout, withdraw_prescribed_workout
// ABOUTME: Lists the training catalogue's workout bank by purpose, phase and sport, and writes, replaces, or removes one prescription on the athlete's calendar
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::annotations::{annotations, Effect, Reach};
use pierre_providers::ai_scope;
use std::collections::HashMap;
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
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use super::calendar::{
    entry_provider_property, provider_arg, provider_property, required_text, step_schema,
    validate_step, TargetRule, MAX_SESSION_STEPS,
};
use super::coach_scope::{athlete_property, resolve_calendar_scope};
use super::plan_scope::PlanWriter;
use super::training_plan_authorship::{load_conversation, resolve_turn_agent};
use super::training_plan_telemetry::{emit_calendar_sync_completed, emit_calendar_sync_failed};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::implementations::endurance_workouts_args::{
    optional_prescription_id, sport_arg, summary_row, vocab_arg, vocabulary, ListDetail,
};
use crate::implementations::endurance_workouts_output::{
    PrescribeWorkoutResult, WithdrawWorkoutResult, WorkoutTemplateFilters, WorkoutTemplateRow,
    WorkoutTemplatesResult,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_mcp_schema::PropertySchema;
use pierre_services::agent_package::{load_agent_package, PackagedCatalogue};
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

fn require_tenant(context: &ToolExecutionContext) -> AppResult<TenantId> {
    context.tenant_id.map(TenantId::from_uuid).ok_or_else(|| {
        AppError::new(
            ErrorCode::AuthInvalid,
            "Endurance workout tools require an active tenant context",
        )
    })
}

/// A structured session the agent authored in conversation, rather than one
/// the catalogue's workout bank carries.
///
/// `structure` deserializes straight into [`WorkoutStep`] so the argument shape
/// and the stored shape cannot drift: the step defaults (`repeat` of 1, absent
/// distance and note) are the model's own.
#[derive(Deserialize)]
struct SessionPayload {
    /// Session name as the agent states it to the athlete.
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

/// An inline session as the user-authored template [`keep_session`] stores.
///
/// The slug is unique per (`tenant_id`, `user_id`), so an existing row's id is
/// reused when one matches — minting a fresh id would collide with that unique
/// index on the second prescription of the same session.
async fn authored_template(
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
    // distribution, and so does its purpose unless the agent named one. It is
    // the agent's own judgement with no citation behind it.
    let (default_purpose, readiness_min) =
        WorkoutTemplate::inline_defaults(session.intensity_distribution);
    let purpose = session.purpose.unwrap_or(default_purpose);
    Ok(WorkoutTemplate {
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
    })
}

/// Keep an authored session's `template` as one of the athlete's saved sessions.
async fn keep_session(context: &ToolExecutionContext, template: &WorkoutTemplate) -> AppResult<()> {
    context
        .resources
        .repos()
        .workout_templates
        .upsert_workout_template(template)
        .await
}

/// The agent this call runs under: the turn's, else the conversation's, else
/// the `agent_id` argument on a direct MCP call. What decides which package
/// lays over the catalogue for the athlete.
async fn turn_agent(
    context: &ToolExecutionContext,
    tenant_id: TenantId,
    arg_agent: Option<String>,
) -> AppResult<Option<String>> {
    let conversation = load_conversation(
        context.resources.repos(),
        context.conversation_id.as_deref(),
        tenant_id,
        &context.user_id.to_string(),
    )
    .await?;
    Ok(resolve_turn_agent(
        context,
        conversation.as_ref(),
        arg_agent,
    ))
}

/// The catalogue as this athlete's agent lays it out: the agent's package
/// over the registry.
async fn packaged_catalogue<'a>(
    context: &'a ToolExecutionContext,
    tenant_id: TenantId,
    user_id: Uuid,
    agent: Option<&str>,
) -> AppResult<PackagedCatalogue<'a>> {
    let package = load_agent_package(context.resources.repos(), tenant_id, user_id, agent).await?;
    Ok(PackagedCatalogue::new(
        context.resources.training_catalogue(),
        package,
    ))
}

/// Resolve the workout this call prescribes the athlete `user_id` in
/// `tenant_id`: a named template or an inline session, never both and never
/// neither. A named template resolves through the package of `writer`'s
/// agent, as the writer sees it; the athlete's own sessions are theirs. An
/// inline session is not stored here: the caller keeps it ([`keep_session`]).
async fn resolve_template(
    context: &ToolExecutionContext,
    tenant_id: TenantId,
    user_id: Uuid,
    writer: &PlanWriter,
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
            authored_template(context, tenant_id, user_id, session).await
        }
        (Some(slug), None) => {
            let agent = writer.agent_id.as_deref();
            let catalogue =
                packaged_catalogue(context, writer.tenant, writer.user_id, agent).await?;
            if let Some((template, _)) = catalogue.workout(slug) {
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
        // One derived from first-party-only data does not exist for an
        // external caller (carnet#769).
        .filter(|row| ai_scope::admit_derived(row.transport_policy))
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

/// `list_workout_templates` — the workout bank, filtered by purpose, phase
/// and sport, with the athlete's own saved sessions after it.
#[derive(Debug)]
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
                    "Only templates fitting this phase (any-phase templates always match). \
                     One of: {}.",
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
                    "Only templates written for this sport (primary or variant), in \
                     snake_case: run, ride, swim, strength_training, …"
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
                    "summary (default): slug, name, purpose, sport, duration, evidence tier, \
                     parameter ranges and phase fit; full: the whole template with its steps \
                     and target zones."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(Vec::new()));
        // The purpose list is the kernel's vocabulary, never a copy of it: a
        // purpose the kernel gains reaches the description the build it lands.
        let description = format!(
            "List the workout template bank by what a session is for. Each \
             template has a purpose ({}), the phases it fits, the readiness level \
             it needs, its sports, evidence tier, and parameter ranges with \
             defaults you fill in for the athlete. The athlete's own saved \
             sessions follow the bank. detail = full returns the steps and target \
             zones prescribe_workout pushes.",
            vocabulary(WorkoutPurpose::ALL, WorkoutPurpose::as_str)
        );
        answers_with::<WorkoutTemplatesResult>(tool_definition(
            "list_workout_templates",
            &description,
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let bank = context.resources.training_catalogue().workouts();
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

            // The agent's package and the athlete's own sessions are both
            // scoped by tenant; a call with no tenant context lists the
            // catalogue alone.
            let templates = match context.tenant_id.map(TenantId::from_uuid) {
                Some(tenant_id) => {
                    let agent = turn_agent(&context, tenant_id, None).await?;
                    let catalogue =
                        packaged_catalogue(&context, tenant_id, context.user_id, agent.as_deref())
                            .await?;
                    let mut templates = catalogue.workouts_matching(&filter);
                    let own = context
                        .resources
                        .repos()
                        .workout_templates
                        .list_user_workout_templates(tenant_id, context.user_id)
                        .await?;
                    templates.extend(own.into_iter().filter(|t| filter.matches(t)));
                    templates
                }
                None => context
                    .resources
                    .training_catalogue()
                    .workouts_matching(&filter),
            };

            let rows: Vec<WorkoutTemplateRow> = match detail {
                ListDetail::Summary => templates
                    .iter()
                    .map(|t| summary_row(t).map(|r| WorkoutTemplateRow::Summary(Box::new(r))))
                    .collect::<AppResult<Vec<_>>>()?,
                ListDetail::Full => templates
                    .iter()
                    .map(|t| WorkoutTemplateRow::Full(Box::new(t.clone())))
                    .collect(),
            };
            ok_typed(
                "list_workout_templates",
                WorkoutTemplatesResult {
                    count: rows.len(),
                    filters: WorkoutTemplateFilters {
                        purpose: filter.purpose.map(|p| format!("{p:?}")),
                        phase: filter.phase.map(|p| format!("{p:?}")),
                        sport: filter.sport.map(|s| format!("{s:?}")),
                        detail: detail.as_str().to_owned(),
                    },
                    templates: rows,
                },
            )
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
                "What the session is for, one of: {}. Omitted, it follows from \
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

/// `prescribe_workout` — push a workout to the athlete's calendar provider
/// and record the prescription.
#[derive(Debug)]
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
                    "A stored template's slug: from the workout bank \
                     (list_workout_templates) or a session you prescribed this athlete \
                     before. Use `session` for anything new."
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
                description: Some("Date to schedule it on, YYYY-MM-DD.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "The prescribing agent, read only on a direct MCP call with no \
                     conversation; a chat turn's own agent always wins. Its package resolves \
                     the template."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "replaces".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "prescription_id of an earlier prescription to change in place; the \
                     entry keeps its slot. Omit to add a new entry."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert("provider".to_owned(), provider_property());
        properties.insert("athlete".to_owned(), athlete_property());
        let schema = object_schema(properties, Some(vec!["date".to_owned()]));
        answers_with::<PrescribeWorkoutResult>(tool_definition(
            "prescribe_workout",
            "Write one workout onto the athlete's calendar provider (Intervals.icu \
             today) for a date and record it; needs a connected provider that writes \
             a training calendar. Pass EITHER template_slug OR session. Every call \
             adds a new calendar entry unless replaces names a prescription_id (from \
             an earlier call or get_training_plan's calendar block), which changes \
             that entry in place on its own provider; withdraw_prescribed_workout \
             removes one. A group's human coach prescribes for a consenting athlete \
             with `athlete`, through the athlete's Intervals.icu link once they allow \
             calendar writes on it.",
            schema,
            annotations(Effect::AdditiveWrite, Reach::OpenWorld),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            require_tenant(&context)?;
            let date_str = args
                .get("date")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("date is required"))?;
            let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .map_err(|e| AppError::invalid_input(format!("date must be YYYY-MM-DD: {e}")))?;
            let arg_agent = args
                .get("agent_id")
                .and_then(Value::as_str)
                .map(str::to_owned);

            let replaces = optional_prescription_id(&args, "replaces")?;

            // Whose calendar, and the agent this prescription runs under: the
            // turn's, else the conversation's, else the argument on a direct
            // call. The agent picks the package the template resolves through,
            // as the writer, and is the agent the ledger row names.
            let mut scope =
                match resolve_calendar_scope(&context, &args, arg_agent, PRESCRIBE_TOOL).await? {
                    Ok(scope) => scope,
                    Err(refused) => return Ok(refused),
                };
            let (tenant_id, user_id) = (scope.tenant, scope.user_id);
            let agent = scope.writer.agent_id.clone();
            let template =
                resolve_template(&context, tenant_id, user_id, &scope.writer, &args).await?;
            // An authored session becomes one of the athlete's saved sessions:
            // their own even when no calendar takes it, a coach's only once the
            // coach's writer is built, so a refused write saves nothing of theirs.
            let authored = args.get("session").is_some_and(|session| !session.is_null());
            if authored && !scope.acts_for_athlete() {
                keep_session(&context, &template).await?;
            }
            // The previous entry is resolved before the provider is built, so a
            // bad id is refused without a credential lookup.
            let previous = match replaces {
                Some(id) => Some(live_prescription(&context, tenant_id, user_id, id).await?),
                None => None,
            };
            // A replaced entry lives on the provider the ledger filed it under;
            // naming another would orphan it.
            let named = provider_arg(&args);
            let target = match &previous {
                Some((prev, _)) => {
                    if let Some(named) = named.filter(|name| *name != prev.provider) {
                        return Err(AppError::invalid_input(format!(
                            "prescription {} is on {}, so it cannot be replaced on {named}",
                            prev.id, prev.provider
                        )));
                    }
                    prev.provider.clone()
                }
                None => scope.target(&context, named).await?,
            };
            let provider = match scope.calendar_writer(&context, &target).await? {
                Ok(provider) => provider,
                Err(refused) => return Ok(refused),
            };
            if authored && scope.acts_for_athlete() {
                keep_session(&context, &template).await?;
            }

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

            // What this turn derived from first-party-only data does not leave
            // through an external transport, onto a third-party calendar or
            // anywhere else: the gate `push_training_plan` applies to a stored
            // plan, applied to what this call derived (carnet#901).
            if !ai_scope::admit_derived(ai_scope::derived_policy()) {
                return Err(AppError::unavailable_over_transport(
                    "this workout is not available over this interface",
                ));
            }

            // With `replaces`, the calendar entry keeps its provider id and gets
            // the new workout; without it, a new entry is created.
            let push: AppResult<String> = match &previous {
                Some((_, event_id)) => provider
                    .update_planned_session(event_id, &session)
                    .await
                    .map(|()| event_id.clone()),
                None => provider.push_planned_session(&session).await,
            };
            let push = scope.reacting(&context, provider.as_ref(), push).await;
            let (status, provider_event_id) = push.as_ref().map_or_else(
                |_| (PrescribedWorkout::STATUS_FAILED, None),
                |id| (PrescribedWorkout::STATUS_PUSHED, Some(id.clone())),
            );
            match &push {
                Ok(_) => emit_calendar_sync_completed(
                    tenant_id,
                    user_id,
                    &target,
                    PRESCRIBE_TOOL,
                    1,
                ),
                Err(e) => emit_calendar_sync_failed(
                    tenant_id,
                    user_id,
                    &target,
                    PRESCRIBE_TOOL,
                    &e.to_string(),
                ),
            }

            // The ledger records the attempt either way. A prescription the
            // provider refused is a fact the agent and the athlete both need,
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
                agent_id: agent,
                template_slug: Some(template.slug.clone()),
                sport: template.sport.clone(),
                prescribed_for_date: date,
                provider: target.clone(),
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
                transport_policy: ai_scope::derived_policy(),
            };
            let audit: AppResult<()> = if let Err(e) = superseded {
                Err(e)
            } else {
                repos
                    .prescribed_workouts
                    .upsert_prescribed_workout(&prescribed)
                    .await
            };
            match &audit {
                Ok(()) => scope.record_coach_write(PRESCRIBE_TOOL, 1),
                Err(e) => warn!(error = %e, "prescribe_workout: ledger row not saved"),
            }

            // A push failure is the root cause and outranks a failed ledger
            // write. Only once the push is known to have landed does a ledger
            // failure surface — and then it must say the event IS on the
            // calendar, because an agent told "the prescription failed" would
            // retry, and a retry adds a second entry.
            let event_id = push?;
            audit.map_err(|e| {
                AppError::internal(format!(
                    "the workout IS on the athlete's {target} calendar (event {event_id}) \
                     but the prescription ledger row failed to save — do not prescribe it again: {e}"
                ))
            })?;

            ok_typed(
                "prescribe_workout",
                PrescribeWorkoutResult {
                    prescription_id,
                    provider: target,
                    provider_event_id: event_id,
                    replaced_prescription_id: previous
                        .as_ref()
                        .map(|(prev, _)| prev.id),
                    template_slug: template.slug.clone(),
                    name: template.name.clone(),
                    duration_minutes: template.duration_minutes,
                    scheduled_for: date.format("%Y-%m-%d").to_string(),
                    status: PrescribedWorkout::STATUS_PUSHED.to_owned(),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// `withdraw_prescribed_workout` — remove one prescription from the athlete's
/// calendar and mark it withdrawn in the ledger.
#[derive(Debug)]
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
        properties.insert("provider".to_owned(), entry_provider_property());
        properties.insert("athlete".to_owned(), athlete_property());
        let schema = object_schema(properties, Some(vec!["prescription_id".to_owned()]));
        answers_with::<WithdrawWorkoutResult>(tool_definition(
            "withdraw_prescribed_workout",
            "Remove a workout prescribe_workout wrote to the athlete's calendar \
             provider and mark the prescription withdrawn. Only for single \
             prescriptions: an entry the training plan put there is removed by \
             adjusting the plan (save_training_plan) and pushing it \
             (push_training_plan). A group's human coach passes `athlete` as for \
             prescribe_workout.",
            schema,
            annotations(Effect::Destructive, Reach::OpenWorld),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            require_tenant(&context)?;
            let prescription_id = optional_prescription_id(&args, "prescription_id")?
                .ok_or_else(|| AppError::invalid_input("prescription_id is required"))?;
            let mut scope =
                match resolve_calendar_scope(&context, &args, None, WITHDRAW_TOOL).await? {
                    Ok(scope) => scope,
                    Err(refused) => return Ok(refused),
                };
            let (tenant_id, user_id) = (scope.tenant, scope.user_id);
            let (row, event_id) =
                live_prescription(&context, tenant_id, user_id, prescription_id).await?;
            if let Some(named) = provider_arg(&args).filter(|name| *name != row.provider) {
                return Err(AppError::invalid_input(format!(
                    "prescription {prescription_id} is on {}, not {named}",
                    row.provider
                )));
            }
            let provider = match scope.calendar_writer(&context, &row.provider).await? {
                Ok(provider) => provider,
                Err(refused) => return Ok(refused),
            };

            let deleted = provider
                .delete_planned_sessions(slice::from_ref(&event_id))
                .await;
            let deleted = scope.reacting(&context, provider.as_ref(), deleted).await;
            match &deleted {
                Ok(_) => emit_calendar_sync_completed(
                    tenant_id,
                    user_id,
                    &row.provider,
                    WITHDRAW_TOOL,
                    1,
                ),
                Err(e) => emit_calendar_sync_failed(
                    tenant_id,
                    user_id,
                    &row.provider,
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
                        "the entry is gone from the athlete's {provider_name} calendar (event \
                         {event_id}) but the ledger could not record the withdrawal — call \
                         again to repair the row: {e}",
                        provider_name = row.provider
                    ))
                })?;
            scope.record_coach_write(WITHDRAW_TOOL, 1);

            ok_typed(
                "withdraw_prescribed_workout",
                WithdrawWorkoutResult {
                    prescription_id,
                    provider: row.provider.clone(),
                    provider_event_id: event_id,
                    name: row.template_slug.clone(),
                    scheduled_for: row.prescribed_for_date.format("%Y-%m-%d").to_string(),
                    status: PrescribedWorkout::STATUS_WITHDRAWN.to_owned(),
                },
            )
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
