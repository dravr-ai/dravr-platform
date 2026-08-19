// ABOUTME: Endurance Phase 2 MCP tools — compute_training_history + get_training_history
// ABOUTME: Mirrors GET /api/v1/endurance/history; on-demand backfill + read of daily CTL/ATL/TSB rollup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_fitness_compute::training_history_compute::MAX_BACKFILL_DAYS;
use serde_json::{json, Value};

use crate::services::training_history_compute::{
    compute_and_persist_history, fetch_history_rows, DEFAULT_BACKFILL_DAYS,
};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tool_runtime::capabilities::ToolCapabilities;
use pierre_tool_runtime::context::ToolExecutionContext;
use pierre_tool_runtime::conversions::{
    capabilities_to_tronc, tool_definition, tool_result_to_response,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::security::RuntimeTool;
use pierre_tools_core::ToolResult;

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
impl McpTool<dyn ToolRuntime> for ComputeTrainingHistoryTool {
    fn definition(&self) -> Tool {
        let schema = date_range_schema();
        tool_definition(
            "compute_training_history",
            "Compute and persist Endurance daily training-state rollups for the \
             authenticated user across the requested window — CTL, ATL, TSB \
             (Coggan), acute:chronic load balance, monotony + strain (Foster), \
             ramp rate, and daily TSS load. Use this to seed the \
             `training_history` table on first use, after a long gap in syncs, \
             or when the user asks for a specific historical window. Default \
             window is the last 90 days.",
            schema,
            Some(write_safe_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::REQUIRES_PROVIDER
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::ANALYTICS,
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
            let (from, to) = resolve_window(&args)?;
            let count =
                compute_and_persist_history(&context.resources, tenant_id, user_id, from, to)
                    .await?;
            Ok(ToolResult::ok(json!({
                "from": from.format("%Y-%m-%d").to_string(),
                "to": to.format("%Y-%m-%d").to_string(),
                "rows_upserted": count,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// `get_training_history` — read the persisted daily rollup.
pub struct GetTrainingHistoryTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetTrainingHistoryTool {
    fn definition(&self) -> Tool {
        let schema = date_range_schema();
        tool_definition(
            "get_training_history",
            "Fetch persisted Endurance daily training-state rollups for the \
             authenticated user across the requested window. Returns \
             chronological CTL/ATL/TSB/ACWR/monotony/strain/ramp_rate/daily_load \
             rows. Use this BEFORE prescribing new load so coaching advice can \
             cite the framework values (Coggan / Foster) on every numeric claim \
             per the Endurance deterministic-output rule. Interpret TSB relative \
             to CTL (form as % of fitness), and report `acwr` strictly as the \
             magnitude of 7-day load against the 28-day baseline — a \
             descriptive ratio, not a predictor of harm. Default window is the \
             last 90 days.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::ANALYTICS,
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
            let (from, to) = resolve_window(&args)?;
            let rows =
                fetch_history_rows(&context.resources.data(), tenant_id, user_id, from, to).await?;
            Ok(ToolResult::ok(json!({
                "from": from.format("%Y-%m-%d").to_string(),
                "to": to.format("%Y-%m-%d").to_string(),
                "days": rows,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Build the Endurance training-history tool list for registry registration.
#[must_use]
pub fn create_endurance_history_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ComputeTrainingHistoryTool),
        Box::new(GetTrainingHistoryTool),
    ]
}

// Guardian security classifications (see `pierre_tool_runtime::security`). These
// tools read/compute the caller's own training history — internal, no egress.
pierre_tool_runtime::declare_security!(ComputeTrainingHistoryTool => empty);
pierre_tool_runtime::declare_security!(GetTrainingHistoryTool => empty);
