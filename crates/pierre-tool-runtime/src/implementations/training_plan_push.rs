// ABOUTME: push_training_plan tool — puts the athlete's active plan on their provider calendar and reconciles the ledger
// ABOUTME: Also renders the calendar block get_training_plan shows and the pending-change preview save_training_plan reports
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek};
use pierre_services::plan_calendar_push::{
    desired_entries, diff_against_ledger, push_active_plan, PushPlanParams, CALENDAR_PROVIDER,
};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use super::calendar::{calendar_provider, destructive_annotations};
use super::training_plan_telemetry::{
    athlete_today, emit_calendar_sync_completed, emit_calendar_sync_failed,
};
use super::training_plans::{load_conversation, resolve_coach_slug};
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use crate::task_cancellation::current_task_cancel_flag;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

/// Tool name, as the notify events name the trigger.
const PUSH_TOOL: &str = "push_training_plan";

/// The calendar block: every entry Dravr has live on the athlete's calendar
/// from `today` on, and what a push would change.
///
/// Each entry carries its `prescription_id` so a later turn can name it to
/// `prescribe_workout`'s `replaces` or to `withdraw_prescribed_workout`, and
/// its `source` so the model knows which entries belong to the plan instead.
///
/// # Errors
///
/// Returns an error when the ledger cannot be read.
pub(super) async fn calendar_block(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    active_weeks: &[PlanWeek],
    today: NaiveDate,
) -> AppResult<Value> {
    let live = repos
        .prescribed_workouts
        .list_live_calendar_events(tenant, user_id, CALENDAR_PROVIDER, Some(today))
        .await?;
    let desired = desired_entries(user_id, active_weeks, today);
    let pending = diff_against_ledger(&desired, &live)?;
    let entries: Vec<Value> = live
        .iter()
        .map(|row| {
            // The pushed payload names the entry whatever its source: a plan
            // day's session and a prescription's template both carry `name`.
            let name = serde_json::from_str::<Value>(&row.payload_json)
                .ok()
                .and_then(|payload| {
                    payload
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .or_else(|| row.template_slug.clone());
            json!({
                "prescription_id": row.id,
                "date": row.prescribed_for_date.format("%Y-%m-%d").to_string(),
                "source": row.source,
                "name": name,
                "provider_event_id": row.provider_event_id,
                "pushed_at": row.updated_at,
            })
        })
        .collect();
    let mut block = json!({
        "provider": CALENDAR_PROVIDER,
        "entries": entries,
        "pending": pending,
        "stale": pending.is_stale(),
    });
    if block["entries"].as_array().is_some_and(Vec::is_empty) {
        // An empty ledger is not an empty calendar, and the difference is the
        // whole answer to "do I have a race today". This block is Dravr's record
        // of what Dravr pushed; nothing here reads the athlete's own calendar,
        // and for an athlete with no calendar provider connected it is empty
        // permanently. Said plainly next to the emptiness, because a model shown
        // `entries: []` under a key called `calendar` will otherwise report it as
        // "nothing on your calendar" — which it did, to an athlete asking whether
        // he had a race (2026-08-28).
        block["scope"] = json!(concat!(
            "Dravr has scheduled nothing. This lists only what Dravr pushed to ",
            "the athlete's calendar provider — it is NOT a view of their calendar. ",
            "An empty list says nothing about races or events they entered ",
            "themselves; say so rather than reporting they have none."
        ));
    }
    Ok(block)
}

/// What a push would change, for the `save_training_plan` reply — only once
/// the calendar already carries this plan, so an athlete who never pushed is
/// not nagged about a calendar they do not use.
///
/// Best-effort like the ramp check: the plan is already committed, so an
/// unreadable ledger degrades to `None` rather than failing the save.
pub(super) async fn calendar_preview_after_save(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    plan_id: &str,
    today: NaiveDate,
) -> Option<Value> {
    let live = best_effort(
        repos
            .prescribed_workouts
            .list_live_calendar_events(tenant, user_id, CALENDAR_PROVIDER, Some(today))
            .await,
        "calendar ledger unreadable",
    )?;
    if !live.iter().any(|row| row.source.is_plan()) {
        return None;
    }
    let weeks = best_effort(
        repos
            .training_plans
            .list_plan_weeks(&tenant.to_string(), &user_id.to_string(), plan_id, false)
            .await,
        "plan weeks unreadable for the calendar preview",
    )?;
    let desired = desired_entries(user_id, &weeks, today);
    let pending = best_effort(
        diff_against_ledger(&desired, &live),
        "calendar preview could not be computed",
    )?;
    Some(json!({
        "provider": CALENDAR_PROVIDER,
        "pending": pending,
        "stale": pending.is_stale(),
    }))
}

/// The preview is best-effort: a step that fails is logged with what it was
/// doing and the preview is dropped, never the save that preceded it.
fn best_effort<T>(result: AppResult<T>, what: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            warn!(error = %e, "save_training_plan: {what}");
            None
        }
    }
}

/// `push_training_plan` — the athlete's active plan onto their calendar.
pub struct PushTrainingPlanTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for PushTrainingPlanTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Coach persona slug whose plan to push; falls back to the athlete's \
                     coach-agnostic plan."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "from_date".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "First date to push (YYYY-MM-DD). Defaults to today in the athlete's \
                     calendar, and is never earlier than that: dates already past are not \
                     rewritten."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        tool_definition(
            "push_training_plan",
            "Put the athlete's active training plan on their Intervals.icu calendar, or \
             bring the calendar up to date after the plan changed: creates the days that \
             are missing, updates the ones that changed, removes the ones the plan no \
             longer has, and leaves alone any the athlete edited on Intervals.icu. Never \
             touches dates before today. Call it when the athlete or coach asks to put or \
             update the plan on their calendar — not on your own initiative after a save; \
             save_training_plan's reply says when the calendar is behind. Requires a saved \
             plan and a connected Intervals.icu account. Args: optional coach_id, optional \
             from_date.",
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
            let tenant = TenantId::from_uuid(context.require_tenant()?);
            let user_id = context.user_id;
            let user_str = user_id.to_string();
            let arg_coach = args
                .get("coach_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|s| !s.trim().is_empty());
            let repos = context.resources.repos();
            let today = athlete_today(repos, &user_str).await;
            let from = match args.get("from_date").and_then(Value::as_str) {
                Some(raw) => parse_plan_date(raw)
                    .ok_or_else(|| {
                        AppError::invalid_input(format!(
                            "from_date must be a canonical YYYY-MM-DD, got '{raw}'"
                        ))
                    })?
                    .max(today),
                None => today,
            };
            let conv =
                load_conversation(repos, context.conversation_id.as_deref(), tenant, &user_str)
                    .await?;
            let coach = resolve_coach_slug(conv.as_ref(), arg_coach);
            let provider = calendar_provider(&context, tenant, user_id).await?;

            // When this call runs behind an MCP task handle the dispatcher
            // scoped a cancel flag around it; handing it to the push loop is
            // what lets tasks/cancel stop the calendar writes between
            // entries. An inline call has no flag and pushes to completion.
            let cancel_flag = current_task_cancel_flag();
            let report = push_active_plan(
                repos,
                provider.as_ref(),
                &PushPlanParams {
                    tenant,
                    user_id,
                    coach_slug: coach.as_deref(),
                    provider: CALENDAR_PROVIDER,
                    from,
                    cancel: cancel_flag.as_deref(),
                },
            )
            .await?;

            let landed = report.created + report.updated + report.removed;
            let attempted = landed + report.failed.len();
            if let Some(first) = report.failed.first() {
                emit_calendar_sync_failed(
                    tenant,
                    user_id,
                    CALENDAR_PROVIDER,
                    PUSH_TOOL,
                    &format!(
                        "{} of {attempted} entries failed; first: {}",
                        report.failed.len(),
                        first.error
                    ),
                );
            }
            if report.failed.is_empty() || landed > 0 {
                emit_calendar_sync_completed(tenant, user_id, CALENDAR_PROVIDER, PUSH_TOOL, landed);
            }

            let payload = serde_json::to_value(&report)
                .map_err(|e| AppError::internal(format!("serialize push report: {e}")))?;
            Ok(ToolResult::ok(payload))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Factory for the plan-push tool set.
#[must_use]
pub fn create_training_plan_push_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(PushTrainingPlanTool)]
}

// A push deletes calendar entries the plan no longer wants, and a delete on
// the provider cannot be undone from here — re-pushing recreates the entry,
// but as a new one. Destructive, so at most one per turn under the Guardian's
// default budget, and the confirm path applies to it when armed.
crate::declare_security!(PushTrainingPlanTool => IRREVERSIBLE);
