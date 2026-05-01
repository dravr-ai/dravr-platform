// ABOUTME: Endurance Phase 2 MCP tools — compute_training_history + get_training_history
// ABOUTME: Mirrors GET /api/v1/endurance/history; on-demand backfill + read of daily CTL/ATL/TSB rollup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{Duration, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_intelligence::training_history_compute::MAX_BACKFILL_DAYS;
use serde_json::{json, Value};

use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::services::training_history_compute::{
    compute_and_persist_history, fetch_history_rows, DEFAULT_BACKFILL_DAYS,
};
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

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
        idempotent_hint: Some(true),
        open_world_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

fn require_tenant(context: &ToolExecutionContext) -> AppResult<TenantId> {
    context.tenant_id.map(TenantId::from_uuid).ok_or_else(|| {
        AppError::new(
            ErrorCode::AuthInvalid,
            "Endurance history tools require an active tenant context",
        )
    })
}

fn parse_date(args: &Value, key: &str) -> AppResult<Option<NaiveDate>> {
    let Some(raw) = args.get(key).and_then(Value::as_str) else {
        return Ok(None);
    };
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(Some)
        .map_err(|e| AppError::invalid_input(format!("{key} must be YYYY-MM-DD: {e}")))
}

fn resolve_window(args: &Value) -> AppResult<(NaiveDate, NaiveDate)> {
    let to = parse_date(args, "to")?.unwrap_or_else(|| Utc::now().date_naive());
    let from =
        parse_date(args, "from")?.unwrap_or_else(|| to - Duration::days(DEFAULT_BACKFILL_DAYS));
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    if (to - from).num_days() > MAX_BACKFILL_DAYS {
        return Err(AppError::invalid_input(format!(
            "window exceeds the maximum of {MAX_BACKFILL_DAYS} days"
        )));
    }
    Ok((from, to))
}

fn date_range_schema() -> JsonSchema {
    let mut properties = HashMap::new();
    properties.insert(
        "from".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Inclusive start of the window (ISO date YYYY-MM-DD). Defaults to \
                 90 days before `to` when omitted."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    properties.insert(
        "to".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Inclusive end of the window (ISO date YYYY-MM-DD). Defaults to today (UTC)."
                    .to_owned(),
            ),
            ..Default::default()
        },
    );
    JsonSchema {
        schema_type: "object".to_owned(),
        properties: Some(properties),
        required: Some(Vec::new()),
    }
}

/// `compute_training_history` — on-demand backfill of the daily rollup.
pub struct ComputeTrainingHistoryTool;

#[async_trait]
impl McpTool for ComputeTrainingHistoryTool {
    fn name(&self) -> &'static str {
        "compute_training_history"
    }

    fn description(&self) -> &'static str {
        "Compute and persist Endurance daily training-state rollups for the \
         authenticated user across the requested window — CTL, ATL, TSB \
         (Coggan), ACWR (Gabbett), monotony + strain (Foster), ramp rate, \
         and daily TSS load. Use this to seed the `training_history` table \
         on first use, after a long gap in syncs, or when the user asks for \
         a specific historical window. Default window is the last 90 days. \
         Mirrors the work the post-sync hook does automatically."
    }

    fn input_schema(&self) -> JsonSchema {
        date_range_schema()
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::WRITES_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_safe_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = require_tenant(context)?;
        let user_id = context.user_id;
        let (from, to) = resolve_window(&args)?;
        let count =
            compute_and_persist_history(&context.resources, tenant_id, user_id, from, to).await?;
        Ok(ToolResult::ok(json!({
            "from": from.format("%Y-%m-%d").to_string(),
            "to": to.format("%Y-%m-%d").to_string(),
            "rows_upserted": count,
        })))
    }
}

/// `get_training_history` — read the persisted daily rollup.
pub struct GetTrainingHistoryTool;

#[async_trait]
impl McpTool for GetTrainingHistoryTool {
    fn name(&self) -> &'static str {
        "get_training_history"
    }

    fn description(&self) -> &'static str {
        "Fetch persisted Endurance daily training-state rollups for the \
         authenticated user across the requested window. Returns \
         chronological CTL/ATL/TSB/ACWR/monotony/strain/ramp_rate/daily_load \
         rows. Use this BEFORE prescribing new load so coaching advice can \
         cite the framework values (Coggan / Gabbett / Foster) on every \
         numeric claim per the Endurance deterministic-output rule. \
         Default window is the last 90 days."
    }

    fn input_schema(&self) -> JsonSchema {
        date_range_schema()
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = require_tenant(context)?;
        let user_id = context.user_id;
        let (from, to) = resolve_window(&args)?;
        let rows = fetch_history_rows(&context.resources, tenant_id, user_id, from, to).await?;
        Ok(ToolResult::ok(json!({
            "from": from.format("%Y-%m-%d").to_string(),
            "to": to.format("%Y-%m-%d").to_string(),
            "days": rows,
        })))
    }
}

/// Build the Endurance training-history tool list for registry registration.
#[must_use]
pub fn create_endurance_history_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(ComputeTrainingHistoryTool),
        Box::new(GetTrainingHistoryTool),
    ]
}
