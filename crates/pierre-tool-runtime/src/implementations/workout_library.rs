// ABOUTME: save_workout_to_library tool — keeps one session of a training plan as a template in the caller's own workout library
// ABOUTME: Reads the plan the caller sees (a coach's draft over the live season), writes only the caller's own provider account
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `save_workout_to_library`
//!
//! A coach builds a session in an athlete's plan and wants to reuse it: on
//! their own Intervals.icu account, it goes into the workout library, from
//! where they drop it on any athlete's calendar. Two people are involved,
//! and the tool keeps them apart:
//!
//! - **Whose plan** the session is read from is the plan scope's question
//!   ([`resolve_plan_scope`]): the caller's own plan, or a coached athlete's
//!   behind every coaching gate, read under the athlete's AI consents. A
//!   coach reads what they see, their unpublished draft over the live season.
//! - **Whose library** it lands in is always the caller's: their own
//!   connection to a provider that keeps one
//!   (`ProviderCapabilities::WORKOUT_LIBRARY`), never a connection held
//!   through a coach's link and never the athlete's account.
//!
//! A call acting for an athlete over an external transport is refused first,
//! as the plan tools refuse it. The library is resolved next, so a caller
//! with nowhere to save is told so before anything of an athlete's is read
//! or recorded. The template
//! leaves the athlete's fuelling out: the rates are theirs, not the
//! session's. Saving the same session twice writes nothing the second time
//! (`already_saved`), and nothing in the library is replaced or deleted.

use crate::tool_data::ToolData;
use dravr_barrage::Stream;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::CivilDate;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConnectionType, ConversationRecord, PlannedSession, TenantId};
use pierre_mcp_schema::PropertySchema;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, PlannedDay};
use pierre_providers::ai_scope;
use pierre_services::coach_plan_draft::{draft_over, CoachDraftTarget};
use pierre_services::plan_calendar_push::plan_day_session;
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_tools_core::ToolResult;
use serde::Serialize;
use serde_json::{json, Value};

use super::calendar::{bounded, calendar_provider};
use super::plan_scope::{external_write_refusal, resolve_plan_scope, PlanScope, PlanScopeRequest};
use super::training_plan_authorship::load_conversation;
use super::training_plan_schema::{athlete_prop, string_prop};
use crate::annotations::{annotations, Effect, Reach};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;

/// Tool name, as the plan scope's gates and the log lines name it.
const TOOL: &str = "save_workout_to_library";

/// The folder a save goes to when the call names none.
const DEFAULT_FOLDER: &str = "Dravr";

/// Longest folder name the call accepts.
const MAX_FOLDER_CHARS: usize = 100;

/// Most sessions one plan date is looked through for.
const MAX_SESSION_NUMBER: u64 = 10;

/// What a save did: where the template is, and which plan session it is.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SavedLibraryWorkout {
    /// The provider whose library holds it, as the caller knows it.
    pub provider: String,
    /// The library folder holding it.
    pub folder: String,
    /// The folder did not exist and the save created it.
    pub folder_created: bool,
    /// The folder already held this workout, name and content alike, so
    /// nothing was written.
    pub already_saved: bool,
    /// The provider's id for the library workout.
    pub workout_id: String,
    /// The template's name: the plan session's title.
    pub name: String,
    /// The plan date the session was taken from.
    pub date: String,
    /// The coached athlete whose plan it came from; absent for the caller's
    /// own plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub athlete: Option<String>,
}

/// `save_workout_to_library` — a plan session into the caller's own library.
#[derive(Debug)]
pub struct SaveWorkoutToLibraryTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SaveWorkoutToLibraryTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert("date".to_owned(), string_prop("Session date, YYYY-MM-DD."));
        properties.insert(
            "session".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Nth session that day; default 1.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "folder".to_owned(),
            string_prop("Library folder; default Dravr."),
        );
        properties.insert("athlete".to_owned(), athlete_prop());
        let schema = object_schema(properties, Some(vec!["date".to_owned()]));
        answers_with::<SavedLibraryWorkout>(tool_definition(
            "save_workout_to_library",
            "Save one plan session as a template in the caller's own Intervals.icu workout \
             library: a coach's own, whoever's plan it is. Fuelling is left out; saving it again \
             writes nothing.",
            schema,
            annotations(
                "Save workout to library",
                Effect::IdempotentWrite,
                Reach::AthleteAccount,
            ),
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
        tool_result_to_response(save_to_library(&context, &args).await)
    }
}

/// What the call asked for, read and checked before anything is looked up.
struct SaveRequest {
    date: CivilDate,
    /// Zero-based position among the date's sessions.
    ordinal: usize,
    folder: String,
    athlete: Option<String>,
}

impl SaveRequest {
    /// # Errors
    ///
    /// Returns an invalid-input error for a missing or malformed date, a
    /// session number out of range, or an overlong folder name.
    fn read(args: &Value) -> AppResult<Self> {
        let raw = args
            .get("date")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("date is required, as YYYY-MM-DD"))?;
        let date = parse_plan_date(raw).ok_or_else(|| {
            AppError::invalid_input(format!("date must be a canonical YYYY-MM-DD, got '{raw}'"))
        })?;
        let number = match args.get("session") {
            None | Some(Value::Null) => 1,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=MAX_SESSION_NUMBER).contains(n))
                .ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "session must be a whole number from 1 to {MAX_SESSION_NUMBER}: 1 for \
                         the day's first session, 2 for its second"
                    ))
                })?,
        };
        let folder = text_arg(args, "folder").unwrap_or_else(|| DEFAULT_FOLDER.to_owned());
        bounded("folder", &folder, MAX_FOLDER_CHARS)?;
        Ok(Self {
            date,
            ordinal: usize::try_from(number - 1).unwrap_or(usize::MAX),
            folder,
            athlete: text_arg(args, "athlete"),
        })
    }
}

/// The trimmed text of `key`, or `None` when the call left it out or blank.
fn text_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// The tool's body: the transport, then the caller's library, then whose
/// plan, then the session, then the save.
///
/// A call acting for an athlete — one it names, or the athlete of the
/// coach's thread — over an external transport is refused first, with the
/// plan tools' reason, whatever library the caller holds: the plan scope
/// would refuse it for the same reason after the library check.
///
/// # Errors
///
/// Returns an invalid-input error for a malformed call, a repository error
/// from a lookup, the refusal of a plan an external caller may not read, or
/// the provider's error when the save fails.
async fn save_to_library(context: &ToolExecutionContext, args: &Value) -> AppResult<ToolResult> {
    let request = SaveRequest::read(args)?;
    let requester_tenant = TenantId::from_uuid(context.require_tenant()?);
    let repos = context.resources.repos();
    let conversation = load_conversation(
        repos,
        context.conversation_id.as_deref(),
        requester_tenant,
        &context.user_id.to_string(),
    )
    .await?;
    let for_athlete = request.athlete.is_some()
        || conversation
            .as_ref()
            .and_then(ConversationRecord::subject)
            .is_some();
    if for_athlete {
        if let Some(refused) = external_write_refusal("plan") {
            return Ok(refused);
        }
    }
    let backend = match library_backend(context, requester_tenant).await? {
        Ok(backend) => backend,
        Err(refused) => return Ok(refused),
    };
    let scope = match resolve_plan_scope(PlanScopeRequest {
        context,
        requester_tenant,
        conversation: conversation.as_ref(),
        arg_agent: None,
        athlete: request.athlete.as_deref(),
        tool_name: TOOL,
    })
    .await?
    {
        Ok(scope) => scope,
        Err(refused) => return Ok(refused),
    };
    let read = plan_session(context, &scope, request.date, request.ordinal);
    let session = match scope.read_as_athlete(repos, read).await? {
        Ok(session) => session,
        Err(refused) => return Ok(refused),
    };
    let provider = calendar_provider(context, requester_tenant, context.user_id, &backend).await?;
    let saved = provider
        .save_library_workout(&session, &request.folder)
        .await?;
    let registry = context.resources.provider_registry();
    ok_typed(
        TOOL,
        SavedLibraryWorkout {
            provider: registry
                .get_display_name(&backend)
                .unwrap_or(&backend)
                .to_owned(),
            folder: saved.folder,
            folder_created: saved.folder_created,
            already_saved: saved.already_saved,
            workout_id: saved.workout_id,
            name: session.name,
            date: request.date.to_string(),
            athlete: scope.acting_for().map(str::to_owned),
        },
    )
}

/// The provider whose library the caller saves to: their own connection to
/// one that keeps a workout library, never one held through a coach's link —
/// or the refusal. A caller who reaches such a provider only through a human
/// coach's link is refused in that coach's terms: connecting the platform
/// themselves would end the link, so that refusal never asks them to. Anyone
/// else is told which providers keep a library.
///
/// # Errors
///
/// Returns the repository error when the connections cannot be read.
async fn library_backend(
    context: &ToolExecutionContext,
    tenant: TenantId,
) -> AppResult<Result<String, ToolResult>> {
    let registry = context.resources.provider_registry();
    let keeps_library = |backend: &str| {
        registry
            .get_capabilities(backend)
            .is_some_and(|caps| caps.supports_workout_library())
    };
    let rows = context
        .resources
        .repos()
        .provider_connections
        .get_for_user(context.user_id, Some(tenant))
        .await?;
    if let Some(own) = rows.iter().find(|connection| {
        connection.connection_type != ConnectionType::Delegated
            && keeps_library(&connection.provider)
    }) {
        return Ok(Ok(own.provider.clone()));
    }
    if let Some(linked) = rows.iter().find(|connection| {
        connection.connection_type == ConnectionType::Delegated
            && keeps_library(&connection.provider)
    }) {
        let brand = registry
            .get_display_name(&linked.provider)
            .unwrap_or(&linked.provider);
        return Ok(Err(ToolResult::error(json!({
            "reason": "library_through_coach_link",
            "error": format!(
                "Your {brand} account is reached through your human coach's link, which saves \
                 nothing to a workout library of yours. Ask your human coach to save the session \
                 to theirs. Connecting {brand} yourself would end that link."
            ),
        }))));
    }
    let mut offered: Vec<&str> = registry
        .supported_providers()
        .into_iter()
        .filter(|backend| keeps_library(backend))
        .map(|backend| registry.get_display_name(backend).unwrap_or(backend))
        .collect();
    offered.sort_unstable();
    Ok(Err(ToolResult::error(json!({
        "reason": "no_workout_library",
        "error": format!(
            "Connect your own {} account first: its workout library is where the template \
             goes. The library is always the account of whoever asks, so a coach saves to their \
             own.",
            offered.join(" or ")
        ),
    }))))
}

/// The `ordinal`-th session on `date` of the plan `scope` names, as the
/// caller sees it — a coach's draft over the live season when they hold one —
/// rendered as a template; or the refusal naming what the date holds.
///
/// # Errors
///
/// Returns a repository error, or the refusal of a plan derived from
/// first-party-only data on an external transport.
async fn plan_session(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    date: CivilDate,
    ordinal: usize,
) -> AppResult<Result<PlannedSession, ToolResult>> {
    let repos = context.resources.repos();
    let (tenant_id, user_id) = (scope.tenant.to_string(), scope.user_id.to_string());
    let active = repos
        .training_plans
        .get_active_plan(&tenant_id, &user_id)
        .await?;
    // A plan an external caller may not read is refused as such, never
    // reported as absent (carnet#769).
    if active
        .as_ref()
        .is_some_and(|plan| !ai_scope::admit_agent_written(plan.transport_policy))
    {
        return Err(AppError::unavailable_over_transport(
            "this athlete's training plan is not available over this interface",
        ));
    }
    let live = match &active {
        Some(plan) => {
            repos
                .training_plans
                .list_plan_weeks(&tenant_id, &user_id, &plan.id, false)
                .await?
        }
        None => Vec::new(),
    };
    let weeks = if scope.acting_for().is_some() {
        let target = CoachDraftTarget::new(scope.tenant, scope.user_id, scope.writer.user_id);
        match draft_over(repos, &target, &live).await? {
            Some(view) => view.weeks,
            None => live,
        }
    } else {
        live
    };
    Ok(session_on(scope, &weeks, date, ordinal))
}

/// The template of the `ordinal`-th session on `date` among `weeks`, in the
/// order the week lists them — the order the calendar push numbers them in —
/// or the refusal saying what the date holds.
fn session_on(
    scope: &PlanScope,
    weeks: &[PlanWeek],
    date: CivilDate,
    ordinal: usize,
) -> Result<PlannedSession, ToolResult> {
    let sessions: Vec<&PlannedDay> = weeks
        .iter()
        .flat_map(|week| &week.days)
        .filter(|day| !day.is_rest() && parse_plan_date(&day.date) == Some(date))
        .collect();
    let Some(day) = sessions.get(ordinal) else {
        let held = match sessions.len() {
            0 => format!("The plan holds no session on {date}."),
            n => format!(
                "The plan holds {n} session(s) on {date}; session {} is not one of them.",
                ordinal.saturating_add(1)
            ),
        };
        return Err(ToolResult::error(json!({
            "reason": "no_session",
            "error": held,
        })));
    };
    // The athlete's fuelling rates are theirs, not the session's: the
    // template carries the workout alone.
    let mut template = (*day).clone();
    template.fueling = None;
    let session = plan_day_session(
        scope.user_id,
        &template,
        ordinal,
        &FuelingDisclosure::from_gate(None),
    )
    .ok_or_else(|| {
        ToolResult::error(json!({
            "reason": "no_session",
            "error": format!("The session on {date} has nothing to save."),
        }))
    })?;
    // The result names a session an agent wrote into the plan.
    ai_scope::note_agent_written();
    Ok(session)
}

/// Factory for the workout-library tool set.
#[must_use]
pub fn create_workout_library_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(SaveWorkoutToLibraryTool)]
}

// A save adds a workout to the caller's library and replaces or deletes
// nothing there, so it is undone in the library by deleting it.
crate::declare_security!(
    SaveWorkoutToLibraryTool => empty,
    reads: ToolData::own(&[Stream::Workout]),
);
