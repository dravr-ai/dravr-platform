// ABOUTME: set_preferred_calendar tool — records which connected calendar Dravr's plan and workout pushes go to
// ABOUTME: The athlete's answer when a push with several calendars asks which, or a switch; a coach records it through the calendar scope
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Preferred calendar
//!
//! An athlete holding several calendar-writing connections (Garmin,
//! Intervals.icu, Wahoo) chooses the one Dravr writes to, and every later
//! `push_training_plan` / `prescribe_workout` that names no provider goes
//! there (carnet#1393). A push that finds several and no choice asks which
//! instead of guessing; this records the answer. The app's Connections
//! settings record the same row.
//!
//! The athlete records it for themselves. A group's human coach records it
//! for a consenting athlete with `athlete`, through the same scope the
//! calendar-write tools resolve ([`resolve_calendar_scope`]), and only among
//! the calendars a coach writes.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use serde::Serialize;
use serde_json::Value;
use tracing::info;

use super::calendar::{
    brand, calendar_to_record, provider_arg, recorded_calendar, CalendarConnections,
    CalendarToRecord,
};
use super::coach_scope::{calendar_refusal, resolve_calendar_scope};
use crate::annotations::{annotations, Effect, Reach};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

/// Tool name, as the scope resolution and the log name it.
const SET_TOOL: &str = "set_preferred_calendar";

/// What `set_preferred_calendar` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SetPreferredCalendarResult {
    /// The calendar Dravr's pushes now go to, as the push tools' `provider`
    /// names it.
    pub provider: String,
    /// Its display name, for the reply.
    pub name: String,
    /// The calendar recorded before, when there was a different one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// What the choice changes, and what it leaves where it is.
    pub note: String,
}

/// `set_preferred_calendar` — record the calendar Dravr's pushes go to.
#[derive(Debug)]
pub struct SetPreferredCalendarTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SetPreferredCalendarTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("The calendar chosen, e.g. intervals_icu.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "athlete".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Roster name of a consenting athlete, for their human coach.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["provider".to_owned()]));
        answers_with::<SetPreferredCalendarResult>(tool_definition(
            "set_preferred_calendar",
            "Record the connected calendar the athlete chose for Dravr's pushes: their answer \
             to choose_calendar, or a switch. Later pushes naming no provider go there.",
            schema,
            annotations(
                "Set preferred calendar",
                Effect::IdempotentWrite,
                Reach::Internal,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
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
            let Some(name) = provider_arg(&args) else {
                return Err(AppError::invalid_input(
                    "provider is required: the connected calendar the athlete chose",
                ));
            };
            let scope = match resolve_calendar_scope(&context, &args, None, SET_TOOL).await? {
                Ok(scope) => scope,
                Err(refused) => return Ok(refused),
            };
            let which = if scope.acts_for_athlete() {
                CalendarConnections::ForCoach
            } else {
                CalendarConnections::Own
            };
            let backend =
                match calendar_to_record(&context, scope.tenant, scope.user_id, name, which).await?
                {
                    CalendarToRecord::Calendar(backend) => backend,
                    CalendarToRecord::NotConnected { backend, connected } => {
                        return Ok(not_connected(
                            &context,
                            scope.athlete_name(),
                            &backend,
                            &connected,
                        ));
                    }
                };
            let previous = recorded_calendar(&context, scope.tenant, scope.user_id)
                .await?
                .filter(|previous| *previous != backend);
            context
                .resources
                .repos()
                .calendar_preferences
                .set_calendar_preference(
                    scope.tenant,
                    scope.user_id,
                    &backend,
                    scope.writer.user_id,
                    Utc::now(),
                )
                .await?;
            if scope.acts_for_athlete() {
                info!(
                    coach_user_id = %scope.writer.user_id,
                    athlete_user_id = %scope.user_id,
                    provider = %backend,
                    "Coach recorded a coached athlete's preferred calendar"
                );
            }
            let display = brand(&context, &backend);
            let note = previous.as_deref().map_or_else(
                || {
                    format!(
                        "Dravr's plan and workout pushes go to {display} from now on, unless a \
                         push names another calendar."
                    )
                },
                |previous| {
                    format!(
                        "Dravr's plan and workout pushes go to {display} from now on, unless a \
                         push names another calendar. What Dravr already put on {} stays \
                         there; push_training_plan puts the plan on {display}.",
                        brand(&context, previous)
                    )
                },
            );
            ok_typed(
                SET_TOOL,
                SetPreferredCalendarResult {
                    provider: backend,
                    name: display,
                    previous,
                    note,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The refusal of a calendar the athlete does not hold among the ones that
/// may be recorded, naming the ones that may.
fn not_connected(
    context: &ToolExecutionContext,
    athlete: Option<&str>,
    backend: &str,
    connected: &[String],
) -> ToolResult {
    let whose = athlete.map_or_else(|| "the athlete's".to_owned(), |name| format!("{name}'s"));
    let held = if connected.is_empty() {
        format!("None of {whose} connected calendars can be recorded here.")
    } else {
        format!(
            "Calendars that can: {}.",
            connected
                .iter()
                .map(|backend| brand(context, backend))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    calendar_refusal(
        context,
        "calendar_not_connected",
        &format!(
            "{} is not among {whose} connected calendars Dravr writes, so it cannot be recorded. \
             {held}",
            brand(context, backend)
        ),
        connected,
    )
}

/// Factory for the preferred-calendar tool.
#[must_use]
pub fn create_calendar_preference_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(SetPreferredCalendarTool)]
}

// Records a choice in Dravr's own database; nothing leaves, nothing is lost.
crate::declare_security!(SetPreferredCalendarTool => empty);
