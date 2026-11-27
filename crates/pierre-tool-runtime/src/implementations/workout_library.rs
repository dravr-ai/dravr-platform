// ABOUTME: save_workout_to_library tool — keeps one session as a template in the caller's own workout library
// ABOUTME: A plan session, a calendar prescription, or a session designed in the call; writes only the caller's own provider account
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `save_workout_to_library`
//!
//! A coach keeps the sessions they reuse in their own Intervals.icu workout
//! library, from where they drop one on any athlete's calendar. A call names
//! exactly one session, from one of three places:
//!
//! - **a plan session** (`date`, `nth`): read from the plan the caller sees,
//!   a coach's unpublished draft over the live season included;
//! - **a prescription** (`prescription_id`): a single workout
//!   `prescribe_workout` prescribed, as its ledger row recorded it,
//!   whatever became of its calendar entry since;
//! - **a designed session** (`session`): steps the caller designs in the
//!   call, in `prescribe_workout`'s inline shape. It needs no plan and no
//!   calendar, and is kept nowhere but the library.
//!
//! Two people can be involved, and the tool keeps them apart:
//!
//! - **Whose plan or calendar** a plan session or a prescription is read from
//!   is the plan scope's question ([`resolve_plan_scope`]): the caller's own,
//!   or a coached athlete's behind every coaching gate, read under the
//!   athlete's AI consents. A designed session is the caller's own and reads
//!   nobody's.
//! - **Whose library** it lands in is always the caller's: their own
//!   connection to a provider that keeps one
//!   (`ProviderCapabilities::WORKOUT_LIBRARY`), never a connection held
//!   through a coach's link and never the athlete's account.
//!
//! A call acting for an athlete over an external transport is refused first,
//! as the plan tools refuse it. The library is resolved next, so a caller
//! with nowhere to save is told so before anything of an athlete's is read
//! or recorded. The template leaves the athlete's fuelling out: the rates
//! are theirs, not the session's. Saving the same session twice writes
//! nothing the second time (`already_saved`), and nothing in the library is
//! replaced or deleted.

use crate::tool_data::{ToolData, WriteKind};
use dravr_barrage::Stream;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::{CivilDate, Timestamp};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    CalendarEventSource, ConnectionType, ConversationRecord, PlannedSession, TenantId,
    WorkoutTemplate,
};
use pierre_mcp_schema::PropertySchema;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, PlannedDay};
use pierre_providers::ai_scope;
use pierre_services::coach_plan_draft::{draft_over, CoachDraftTarget};
use pierre_services::plan_calendar_push::plan_day_session;
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_tools_core::ToolResult;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::calendar::{bounded, calendar_provider};
use super::plan_scope::{external_write_refusal, resolve_plan_scope, PlanScope, PlanScopeRequest};
use super::training_plan_authorship::load_conversation;
use super::training_plan_schema::{athlete_prop, string_prop};
use super::{endurance_workouts, endurance_workouts_args};
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

/// What a save did: where the template is, and which session it is.
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
    /// The template's name: the session's title.
    pub name: String,
    /// The date of the plan session or prescription it was taken from;
    /// absent for a session designed in the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// The coached athlete whose plan or calendar it came from; absent for
    /// the caller's own, and for a session designed in the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub athlete: Option<String>,
}

/// `save_workout_to_library` — one session into the caller's own library.
#[derive(Debug)]
pub struct SaveWorkoutToLibraryTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SaveWorkoutToLibraryTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "date".to_owned(),
            string_prop("Plan session date, YYYY-MM-DD."),
        );
        properties.insert(
            "nth".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Nth plan session that day; default 1.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "prescription_id".to_owned(),
            string_prop("Id of a workout prescribe_workout prescribed."),
        );
        // The inline shape is prescribe_workout's, declared there in full; its
        // essentials are named here rather than copied, since every turn
        // sends the catalogue and the copy would cost ~2.7 KB of it.
        properties.insert(
            "session".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some(
                    "A session you design, in prescribe_workout's session shape: name, sport, \
                     intensity_distribution, structure (steps: label, duration_seconds, \
                     target_zone, repeat)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "folder".to_owned(),
            string_prop("Library folder; default Dravr."),
        );
        properties.insert("athlete".to_owned(), athlete_prop());
        let schema = object_schema(properties, None);
        answers_with::<SavedLibraryWorkout>(tool_definition(
            "save_workout_to_library",
            "Save one session as a template in the caller's own Intervals.icu workout library \
             (a coach's own, whoever's plan): a plan session by date, a prescription_id, or a \
             session you design, no plan needed. Fuelling is left out; saving it again writes \
             nothing.",
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

/// Where the session a call saves comes from: exactly one place per call.
enum Source {
    /// Someone's plan or calendar, which the plan scope decides.
    Scoped(ScopedSource),
    /// A session the caller designed in the call, as a template of their own.
    Designed(Box<WorkoutTemplate>),
}

/// A session read from the plan or the calendar of the athlete the plan
/// scope names.
enum ScopedSource {
    /// The session at zero-based position `ordinal` on the plan's `date`.
    Plan { date: CivilDate, ordinal: usize },
    /// A single workout `prescribe_workout` prescribed, by its ledger id:
    /// live, withdrawn, replaced or failed, its content is the template.
    Prescription(Uuid),
}

/// What the call asked for, read and checked before anything is looked up.
struct SaveRequest {
    source: Source,
    folder: String,
    athlete: Option<String>,
}

impl SaveRequest {
    /// Read the call of `user_id` in `tenant`: a designed session becomes
    /// their own template here, so a malformed one is refused before any
    /// lookup.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for a call naming no session or more
    /// than one, a malformed date or prescription id, a session number out
    /// of range or without a date, a designed session that is malformed or
    /// out of bounds or names an athlete, or an overlong folder name.
    fn read(args: &Value, tenant: TenantId, user_id: Uuid) -> AppResult<Self> {
        let date = text_arg(args, "date")
            .map(|raw| {
                parse_plan_date(&raw).ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "date must be a canonical YYYY-MM-DD, got '{raw}'"
                    ))
                })
            })
            .transpose()?;
        let number = match args.get("nth") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .filter(|n| (1..=MAX_SESSION_NUMBER).contains(n))
                    .ok_or_else(|| {
                        AppError::invalid_input(format!(
                            "nth must be a whole number from 1 to {MAX_SESSION_NUMBER}: 1 for \
                             the day's first plan session, 2 for its second"
                        ))
                    })?,
            ),
        };
        let prescription =
            endurance_workouts_args::optional_prescription_id(args, "prescription_id")?;
        let designed = args.get("session").filter(|value| !value.is_null());
        let athlete = text_arg(args, "athlete");
        let source = match (date, prescription, designed) {
            (Some(date), None, None) => Source::Scoped(ScopedSource::Plan {
                date,
                ordinal: usize::try_from(number.unwrap_or(1) - 1).unwrap_or(usize::MAX),
            }),
            (None, Some(id), None) => Source::Scoped(ScopedSource::Prescription(id)),
            (None, None, Some(raw)) => {
                if athlete.is_some() {
                    return Err(AppError::invalid_input(
                        "a session you design is nobody's plan, so it takes no athlete: it \
                         saves to your own library",
                    ));
                }
                Source::Designed(Box::new(endurance_workouts::designed_session_template(
                    tenant, user_id, raw,
                )?))
            }
            (None, None, None) => {
                return Err(AppError::invalid_input(
                    "name the session to save: date (a plan session), prescription_id (a \
                     prescribed workout) or session (one you design)",
                ))
            }
            _ => {
                return Err(AppError::invalid_input(
                    "name one session to save: date, prescription_id or session, never two",
                ))
            }
        };
        if number.is_some() && date.is_none() {
            return Err(AppError::invalid_input(
                "nth picks among a plan date's sessions, so it goes with date",
            ));
        }
        let folder = text_arg(args, "folder").unwrap_or_else(|| DEFAULT_FOLDER.to_owned());
        bounded("folder", &folder, MAX_FOLDER_CHARS)?;
        Ok(Self {
            source,
            folder,
            athlete,
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

/// The session a call saves, with where it came from.
struct Picked {
    session: PlannedSession,
    /// The date of the plan session or prescription; none for a session
    /// designed in the call.
    date: Option<CivilDate>,
    /// The coached athlete whose plan or calendar it came from.
    athlete: Option<String>,
}

/// The tool's body: the transport, then the caller's library, then the
/// session, then the save.
///
/// A call acting for an athlete — one it names, or the athlete of the
/// coach's thread — over an external transport is refused first, with the
/// plan tools' reason, whatever library the caller holds: the plan scope
/// would refuse it for the same reason after the library check. A designed
/// session acts for nobody; what the turn derived from first-party-only data
/// does not leave through an external transport in one, as it does not onto
/// a calendar through `prescribe_workout` (carnet#901).
///
/// # Errors
///
/// Returns an invalid-input error for a malformed call, a repository error
/// from a lookup, the refusal of a plan, prescription or session an external
/// caller may not read or send, or the provider's error when the save fails.
async fn save_to_library(context: &ToolExecutionContext, args: &Value) -> AppResult<ToolResult> {
    let requester_tenant = TenantId::from_uuid(context.require_tenant()?);
    let request = SaveRequest::read(args, requester_tenant, context.user_id)?;
    let repos = context.resources.repos();
    let conversation = match &request.source {
        Source::Scoped(_) => {
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
            conversation
        }
        Source::Designed(_) => {
            if !ai_scope::admit_derived(ai_scope::derived_policy()) {
                return Err(AppError::unavailable_over_transport(
                    "this workout is not available over this interface",
                ));
            }
            None
        }
    };
    let backend = match library_backend(context, requester_tenant).await? {
        Ok(backend) => backend,
        Err(refused) => return Ok(refused),
    };
    let picked = match request.source {
        Source::Designed(template) => Picked {
            session: designed_session(&template),
            date: None,
            athlete: None,
        },
        Source::Scoped(source) => {
            let scope = PlanScopeRequest {
                context,
                requester_tenant,
                conversation: conversation.as_ref(),
                arg_agent: None,
                athlete: request.athlete.as_deref(),
                tool_name: TOOL,
            };
            match scoped_session(scope, source).await? {
                Ok(picked) => picked,
                Err(refused) => return Ok(refused),
            }
        }
    };
    let provider = calendar_provider(context, requester_tenant, context.user_id, &backend).await?;
    let saved = provider
        .save_library_workout(&picked.session, &request.folder)
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
            name: picked.session.name,
            date: picked.date.map(|date| date.to_string()),
            athlete: picked.athlete,
        },
    )
}

/// The session `source` names, read from the plan or the calendar of the
/// athlete the plan scope of `request` resolves to, under that athlete's AI
/// consents; or the refusal of the scope or of the read.
///
/// # Errors
///
/// Returns a repository error, or the refusal of a plan or prescription an
/// external caller may not read.
async fn scoped_session(
    request: PlanScopeRequest<'_>,
    source: ScopedSource,
) -> AppResult<Result<Picked, ToolResult>> {
    let context = request.context;
    let scope = match resolve_plan_scope(request).await? {
        Ok(scope) => scope,
        Err(refused) => return Ok(Err(refused)),
    };
    let repos = context.resources.repos();
    let read = match source {
        ScopedSource::Plan { date, ordinal } => scope
            .read_as_athlete(repos, plan_session(context, &scope, date, ordinal))
            .await?
            .map(|session| (session, date)),
        ScopedSource::Prescription(id) => {
            scope
                .read_as_athlete(repos, prescribed_session(context, &scope, id))
                .await?
        }
    };
    Ok(read.map(|(session, date)| Picked {
        session,
        date: Some(date),
        athlete: scope.acting_for().map(str::to_owned),
    }))
}

/// A designed session as the library save takes it. A library workout
/// keeps no date and no calendar key (the provider's library body reads
/// neither), so the session carries today's date and no key.
fn designed_session(template: &WorkoutTemplate) -> PlannedSession {
    PlannedSession::from_template(template, Timestamp::now().utc_date(), String::new())
}

/// The workout prescription `id` prescribed the athlete `scope` names, as
/// its ledger row recorded it, with its date; or the refusal naming what the
/// id is. Its status does not matter: a withdrawn or replaced entry still
/// holds the workout a coach may keep.
///
/// # Errors
///
/// Returns a repository error, the refusal of a prescription derived from
/// first-party-only data on an external transport, or an internal error for
/// a ledger row whose session cannot be read back.
async fn prescribed_session(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    id: Uuid,
) -> AppResult<Result<(PlannedSession, CivilDate), ToolResult>> {
    let Some(row) = context
        .resources
        .repos()
        .prescribed_workouts
        .get_prescribed_workout(scope.tenant, scope.user_id, id)
        .await?
    else {
        return Ok(Err(ToolResult::error(json!({
            "reason": "no_prescription",
            "error": format!(
                "No prescription {id} on this calendar: get_training_plan's calendar block \
                 lists them with their ids."
            ),
        }))));
    };
    // Refused as such over an interface that may not read it, never reported
    // as absent: a plan is refused the same way (carnet#769).
    if !ai_scope::admit_agent_written(row.transport_policy) {
        return Err(AppError::unavailable_over_transport(
            "this prescription is not available over this interface",
        ));
    }
    // A plan's day is saved from the plan, by its date, where its fuelling
    // is left out.
    if row.source != CalendarEventSource::Prescription {
        return Ok(Err(ToolResult::error(json!({
            "reason": "plan_entry",
            "error": format!(
                "{id} is an entry of the training plan: save it by its date, {}.",
                row.prescribed_for_date
            ),
        }))));
    }
    let session = serde_json::from_str::<PlannedSession>(&row.payload_json).map_err(|e| {
        AppError::internal(format!("prescription {id} holds no readable session: {e}"))
    })?;
    Ok(Ok((session, row.prescribed_for_date)))
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
// nothing there, so it is undone in the library by deleting it. It files
// the model's free-text folder name there, so it is a free-text write
// (carnet#1511 call 16).
crate::declare_security!(
    SaveWorkoutToLibraryTool => empty,
    reads: ToolData::own(&[Stream::Workout])
        .writes(WriteKind::FreeText),
);
