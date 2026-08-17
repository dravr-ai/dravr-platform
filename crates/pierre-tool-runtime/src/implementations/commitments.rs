// ABOUTME: Athlete commitment tools — commitment_create, commitment_cancel
// ABOUTME: The only writer of athlete_commitments; a commitment is confirmed explicitly, never inferred
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete Commitment Tools
//!
//! A commitment is the athlete's own promise — "three easy runs this week" —
//! and the sweep in `pierre_services::commitment_sweep` counts it against what
//! they actually recorded.
//!
//! The coach writes it through [`CommitmentCreateTool`] and nothing else does.
//! That is deliberate. Post-hoc extraction from a turn is how the sibling
//! advice-capture path works, and it cannot tell "I'll run three times this
//! week" from a bare "ok" to the coach's suggestion — but the difference is the
//! whole entity, because only the first is something the athlete would recognize
//! as a promise. Requiring the coach to have asked and been answered puts the
//! count and the window in the athlete's own words before anything is recorded,
//! which is also what makes the sweep's later message welcome rather than
//! presumptuous.
//!
//! The observation window closes at the end of the athlete's *local* day.
//! "This week" is a civil-calendar claim; resolving it in UTC would tell an
//! athlete in Auckland they missed a Sunday run at lunchtime on Sunday.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, NaiveDate, TimeZone, Utc};
use pierre_core::models::TenantId;
use pierre_memory::commitments::{
    Commitment, CommitmentStatus, MAX_STATEMENT_LEN, MAX_TARGET_SESSIONS, MAX_WINDOW_DAYS,
    MIN_TARGET_SESSIONS,
};
use pierre_memory::{parse_plan_date, sanitize_sport_slug};
use serde_json::{json, Value};
use tracing::info;
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

/// Annotation set for tools that mutate commitment state.
fn write_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

fn require_string_field(args: &Value, key: &str) -> AppResult<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| AppError::invalid_input(format!("missing required '{key}' field")))
}

/// Read an integer argument, tolerating the `3.0` an LLM emits where the schema
/// says integer — the same leniency `save_training_plan` needed after strict
/// rejection killed seven consecutive live calls.
fn require_whole_number(args: &Value, key: &str) -> AppResult<u32> {
    let raw = args
        .get(key)
        .ok_or_else(|| AppError::invalid_input(format!("missing required '{key}' field")))?;
    let n = raw
        .as_f64()
        .ok_or_else(|| AppError::invalid_input(format!("'{key}' must be a number")))?;
    if n.fract() != 0.0 || n < 0.0 || n > f64::from(u32::MAX) {
        return Err(AppError::invalid_input(format!(
            "'{key}' must be a whole number, got {n}"
        )));
    }
    // Guarded above: non-negative, integral, and within u32.
    Ok(n as u32)
}

/// Resolve the end of `due_date` in the athlete's own timezone, as a UTC instant.
///
/// Falls back to UTC when the athlete has never set one (messaging-only accounts
/// commonly have not). A DST-ambiguous local midnight takes the earliest of the
/// two instants, matching how the prompt's date anchor resolves its boundaries.
///
/// Where a DST jump means local midnight does not exist at all — Chile and
/// Lebanon both spring forward at exactly midnight — the window closes at the
/// first instant that does. Rejecting the date instead would refuse a perfectly
/// ordinary promise twice a year in those countries.
fn window_end_for(due_date: NaiveDate, timezone: Option<&str>) -> AppResult<chrono::DateTime<Utc>> {
    let tz: chrono_tz::Tz = timezone
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::UTC);
    let end_of_day = due_date
        .succ_opt()
        .and_then(|next| next.and_hms_opt(0, 0, 0))
        .ok_or_else(|| AppError::invalid_input("due_date is out of range"))?;

    // Real spring-forward gaps are 30 or 60 minutes; stepping to 120 clears any
    // of them with room to spare.
    (0..=120)
        .step_by(30)
        .find_map(|skip| {
            tz.from_local_datetime(&(end_of_day + Duration::minutes(skip)))
                .earliest()
        })
        .map(|local| local.with_timezone(&Utc))
        .ok_or_else(|| AppError::invalid_input("due_date has no valid local midnight"))
}

/// Look up the athlete's IANA timezone, tolerating an unreadable profile.
async fn athlete_timezone(context: &ToolExecutionContext, user_id: Uuid) -> Option<String> {
    context
        .resources
        .repos()
        .users
        .get_global(user_id)
        .await
        .ok()
        .flatten()
        .and_then(|user| user.timezone)
}

// ============================================================================
// CommitmentCreateTool — record a promise the athlete just made
// ============================================================================

/// Records a countable, time-boxed commitment the athlete agreed to.
pub struct CommitmentCreateTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CommitmentCreateTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "statement".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(format!(
                    "The promise in the athlete's own terms (≤ {MAX_STATEMENT_LEN} characters), e.g. 'three easy runs this week'."
                )),
                ..Default::default()
            },
        );
        properties.insert(
            "sessions".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(format!(
                    "How many sessions the athlete committed to ({MIN_TARGET_SESSIONS}-{MAX_TARGET_SESSIONS})."
                )),
                ..Default::default()
            },
        );
        properties.insert(
            "due_date".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Last day that counts, as YYYY-MM-DD in the athlete's local calendar. For 'this week' use the date of the coming Sunday from the current-date anchor.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "sport".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional lowercase sport slug the sessions must match ('run', 'ride', 'swim'). Omit to count any activity.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Coach the athlete made the promise to.".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec![
                "statement".to_owned(),
                "sessions".to_owned(),
                "due_date".to_owned(),
                "coach_id".to_owned(),
            ]),
        };
        tool_definition(
            "commitment_create",
            "Record a commitment the athlete just made, so it can be checked against their real activity data when the window closes. Call this ONLY after the athlete has agreed to a specific number of sessions by a specific day — if they only said 'ok' to your suggestion, or gave no count or no deadline, ask them to confirm both first and call this on their answer. Do not use it for your own plans or reminders.",
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
            let tenant_uuid = context.require_tenant()?;
            let tenant_id = TenantId::from_uuid(tenant_uuid);
            let user_id = context.user_id;

            let statement = require_string_field(&args, "statement")?;
            let statement = statement.trim();
            if statement.is_empty() {
                return Err(AppError::invalid_input(
                    "commitment statement must not be empty",
                ));
            }
            if statement.len() > MAX_STATEMENT_LEN {
                return Err(AppError::invalid_input(format!(
                    "commitment statement exceeds {MAX_STATEMENT_LEN} character limit"
                )));
            }

            let sessions = require_whole_number(&args, "sessions")?;
            if !(MIN_TARGET_SESSIONS..=MAX_TARGET_SESSIONS).contains(&sessions) {
                return Err(AppError::invalid_input(format!(
                    "sessions must be between {MIN_TARGET_SESSIONS} and {MAX_TARGET_SESSIONS}"
                )));
            }

            let due_raw = require_string_field(&args, "due_date")?;
            let due_date = parse_plan_date(due_raw.trim()).ok_or_else(|| {
                AppError::invalid_input("due_date must be a zero-padded YYYY-MM-DD date")
            })?;
            let timezone = athlete_timezone(&context, user_id).await;
            let window_end = window_end_for(due_date, timezone.as_deref())?;

            let now = Utc::now();
            if window_end <= now {
                return Err(AppError::invalid_input(
                    "due_date has already passed in the athlete's timezone",
                ));
            }
            if window_end > now + Duration::days(MAX_WINDOW_DAYS) {
                return Err(AppError::invalid_input(format!(
                    "due_date is more than {MAX_WINDOW_DAYS} days out; that is a training plan, not a commitment"
                )));
            }

            let coach_id = require_string_field(&args, "coach_id")?;
            let sport = sanitize_sport_slug(args.get("sport").and_then(Value::as_str));

            let commitment = Commitment {
                id: Uuid::new_v4().to_string(),
                tenant_id: tenant_id.to_string(),
                user_id: user_id.to_string(),
                coach_id: Some(coach_id),
                conversation_id: context.conversation_id.clone(),
                statement: statement.to_owned(),
                sport,
                target_sessions: sessions,
                window_start: now,
                window_end,
                status: CommitmentStatus::Open,
                outcome: None,
                completed_sessions: None,
                swept_at: None,
                reported_at: None,
                created_at: now,
                updated_at: now,
            };

            let recorded = context
                .resources
                .repos()
                .commitments
                .insert_commitment(&commitment)
                .await?;

            if recorded {
                info!(
                    target: "notify",
                    event = "commitment.created",
                    tenant_id = %commitment.tenant_id,
                    user_id = %commitment.user_id,
                    target_sessions = commitment.target_sessions,
                    window_days = commitment.window_days(),
                    "recorded an athlete commitment"
                );
            }

            Ok(ToolResult::ok(json!({
                "commitment_id": commitment.id,
                "recorded": recorded,
                "sessions": commitment.target_sessions,
                "sport": commitment.sport,
                "window_end": commitment.window_end.to_rfc3339(),
                // A duplicate is dropped rather than stacked, so the coach can
                // say "already noted" instead of promising a second check.
                "duplicate_of_open_commitment": !recorded,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// CommitmentCancelTool — retract a commitment at the athlete's request
// ============================================================================

/// Retracts a commitment the athlete no longer wants held to.
pub struct CommitmentCancelTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CommitmentCancelTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "commitment_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Id of the commitment to retract, as listed in your open-commitments block."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["commitment_id".to_owned()]),
        };
        tool_definition(
            "commitment_cancel",
            "Retract a commitment the athlete no longer wants to be held to, so it is never counted or reported. Use when they say they are dropping it or the circumstances changed — injury, travel, a plan revision. Retracting is not failing; do not use this to record that they missed it.",
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
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let commitment_id = require_string_field(&args, "commitment_id")?;

            let cancelled = context
                .resources
                .repos()
                .commitments
                .cancel_commitment(&tenant_id.to_string(), commitment_id.trim())
                .await?;

            Ok(ToolResult::ok(json!({
                "commitment_id": commitment_id,
                "cancelled": cancelled,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Factory for the athlete-commitment tool set.
#[must_use]
pub fn create_commitment_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(CommitmentCreateTool),
        Box::new(CommitmentCancelTool),
    ]
}

// Neither tool returns provider-derived data, so neither taints the turn.
// Cancelling is a reversible status flip — the athlete can make the promise
// again — so it is not IRREVERSIBLE, which would spend the turn's single
// destructive budget slot.
crate::declare_security!(CommitmentCreateTool => empty);
crate::declare_security!(CommitmentCancelTool => empty);
