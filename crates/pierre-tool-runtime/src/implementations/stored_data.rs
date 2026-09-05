// ABOUTME: The stored health-data tools — sleep, recovery, snapshots, and connected sources
// ABOUTME: Each runs its query body inline against stored rows rather than a provider API

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Stored Health-Data Tools
//!
//! - `GetSleepSessionsTool` — query stored sleep sessions
//! - `GetRecoveryMetricsTool` — query stored recovery and readiness metrics
//! - `GetHealthSnapshotsTool` — query stored health snapshots (body composition, vitals)
//! - `ListDataSourcesTool` — list connected data sources (devices and providers)
//!
//! These read rows the sync pipeline already persisted, so each executes its
//! query body inline instead of bridging into a provider-API handler. They
//! share one date-range + format argument shape, built by
//! [`date_range_properties`] and parsed by [`parse_date_range`] /
//! [`apply_format`].

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_core::models::TenantId;
use serde_json::{json, Value};

use crate::capabilities::{ToolCapabilities, PROVIDER_READ};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    apply_format, capabilities_to_tronc, object_schema, ok_typed, tool_definition,
    tool_result_to_response,
};
use crate::implementations::data_helpers::{parse_output_format, read_only_annotations};
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

// ============================================================================
// Helper - argument parsing and schema builders for stored health-data tools
// ============================================================================

/// Default lookback window when no explicit date range is supplied.
const DEFAULT_LOOKBACK_DAYS: i64 = 30;

/// Widest span one stored-data read serves. Population is one row per night
/// per user, so a year bounds the result set; the underlying queries carry no
/// SQL LIMIT, which made an unclamped caller range the only thing standing
/// between a read and the whole table.
const MAX_RANGE_DAYS: i64 = 366;

/// Parse `start`/`end` RFC3339 args, defaulting to (now - 30d, now).
///
/// An inverted range is refused; a span wider than [`MAX_RANGE_DAYS`] is
/// clipped to the most recent year (the payload echoes the effective
/// `start`/`end`, so the clip is visible to the caller). `pub` so the clamp
/// decisions are exercisable by the integration test suite.
///
/// # Errors
///
/// Returns `invalid_input` when `start` is after `end`.
pub fn parse_date_range(args: &Value) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    let end = args
        .get("end")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc));
    let mut start = args
        .get("start")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(
            || Utc::now() - Duration::days(DEFAULT_LOOKBACK_DAYS),
            |dt| dt.with_timezone(&Utc),
        );
    if start > end {
        return Err(AppError::invalid_input(format!(
            "start ({start}) is after end ({end})"
        )));
    }
    if end - start > Duration::days(MAX_RANGE_DAYS) {
        start = end - Duration::days(MAX_RANGE_DAYS);
    }
    Ok((start, end))
}

/// Build the standard date-range + format property set used by stored
/// health-data queries (sleep, recovery, snapshots). Inferred from the
/// handler bodies in `handlers/health_data.rs`, which read `start`, `end`,
/// and `format` from `request.parameters`.
fn date_range_properties() -> BTreeMap<String, PropertySchema> {
    let mut properties = BTreeMap::new();
    properties.insert(
        "start".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Start of the date range as RFC3339 timestamp. Defaults to 30 days ago.".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties.insert(
        "end".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "End of the date range as RFC3339 timestamp. Defaults to now.".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties.insert(
        "format".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Output format: 'json' (default) or 'toon' (token-efficient for LLMs).".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties
}

// ============================================================================
// GetSleepSessionsTool - Query stored sleep sessions
// ============================================================================

/// Tool for querying stored sleep sessions from the database.
pub struct GetSleepSessionsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetSleepSessionsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
            ..Default::default()
        };

        tool_definition(
            "get_sleep_sessions",
            "Get stored sleep sessions from the database",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_READ)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let (start, end) = parse_date_range(&args)?;

            match context
                .resources
                .repos()
                .sleep
                .get_sleep_sessions(context.user_id, &tenant_id, start, end)
                .await
            {
                Ok(sessions) => {
                    let payload = json!({
                        "count": sessions.len(),
                        "sessions": sessions,
                        "range": {
                            "start": start.to_rfc3339(),
                            "end": end.to_rfc3339(),
                        },
                    });
                    ok_typed("get_sleep_sessions", apply_format(payload, format))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch sleep sessions: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetRecoveryMetricsTool - Query stored recovery and readiness metrics
// ============================================================================

/// Tool for querying stored recovery and readiness data from the database.
pub struct GetRecoveryMetricsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetRecoveryMetricsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
            ..Default::default()
        };

        tool_definition(
            "get_recovery_metrics",
            "Get stored recovery and readiness metrics",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_READ)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let (start, end) = parse_date_range(&args)?;

            match context
                .resources
                .repos()
                .recovery
                .get_recovery_metrics(context.user_id, &tenant_id, start, end)
                .await
            {
                Ok(metrics) => {
                    let payload = json!({
                        "count": metrics.len(),
                        "metrics": metrics,
                        "range": {
                            "start": start.to_rfc3339(),
                            "end": end.to_rfc3339(),
                        },
                    });
                    ok_typed("get_recovery_metrics", apply_format(payload, format))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch recovery metrics: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetHealthSnapshotsTool - Query stored health snapshots
// ============================================================================

/// Tool for querying stored body composition and vitals snapshots.
pub struct GetHealthSnapshotsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetHealthSnapshotsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
            ..Default::default()
        };

        tool_definition(
            "get_health_snapshots",
            "Get stored health snapshots (body composition, vitals)",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(PROVIDER_READ)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let (start, end) = parse_date_range(&args)?;

            match context
                .resources
                .repos()
                .health_snapshots
                .get_health_snapshots(context.user_id, &tenant_id, start, end)
                .await
            {
                Ok(snapshots) => {
                    let payload = json!({
                        "count": snapshots.len(),
                        "snapshots": snapshots,
                        "range": {
                            "start": start.to_rfc3339(),
                            "end": end.to_rfc3339(),
                        },
                    });
                    ok_typed("get_health_snapshots", apply_format(payload, format))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch health snapshots: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ListDataSourcesTool - List connected devices and providers
// ============================================================================

/// Tool for listing connected data sources (devices and providers) for the user.
pub struct ListDataSourcesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListDataSourcesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);

        tool_definition(
            "list_data_sources",
            "List connected data sources (devices and providers)",
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
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);

            match context
                .resources
                .repos()
                .data_sources
                .list_data_sources(context.user_id, &tenant_id)
                .await
            {
                Ok(sources) => {
                    let payload = json!({
                        "count": sources.len(),
                        "sources": sources,
                    });
                    ok_typed("list_data_sources", apply_format(payload, format))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to list data sources: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
//
// Provider-synced health records: the same third-party-scrape provenance as
// GetActivities/GetAthlete (Whoop/Garmin/Fitbit/Terra), carrying
// provider-controlled free-text fields (device/source names, data_source_id —
// cf. the garmin data_source_id blob leak). A taint SOURCE, so a later
// consequential sink in the same turn is gated.
crate::declare_security!(GetSleepSessionsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetRecoveryMetricsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetHealthSnapshotsTool => UNTRUSTED_OUTPUT);
// Surfaces provider/source names synced from third parties as free text.
crate::declare_security!(ListDataSourcesTool => UNTRUSTED_OUTPUT);
