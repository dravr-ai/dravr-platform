// ABOUTME: Endurance Phase 3 MCP tools — export_intervals + export_routes + extract_activity_streams
// ABOUTME: Mirrors the per-activity intervals.json + routes.json + raw stream extraction endpoints
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{Activity, TenantId};
use pierre_fitness_compute::intervals::build_intervals;
use pierre_fitness_compute::routes::{
    build_route_summary_from_streams, route_summary_from_cache, stream_route_identity,
};
use serde_json::{json, Value};

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::environment::default_provider;
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tool_runtime::capabilities::ToolCapabilities;
use pierre_tool_runtime::context::ToolExecutionContext;
use pierre_tool_runtime::conversions::{
    capabilities_to_tronc, tool_definition, tool_result_to_response,
};
use pierre_tool_runtime::protocol::provider_helpers::fetch_activity_from_provider;
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

fn require_tenant(context: &ToolExecutionContext) -> AppResult<TenantId> {
    context.tenant_id.map(TenantId::from_uuid).ok_or_else(|| {
        AppError::new(
            ErrorCode::AuthInvalid,
            "Endurance intervals/routes tools require an active tenant context",
        )
    })
}

fn activity_id_arg(args: &Value) -> AppResult<String> {
    args.get("activity_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| AppError::invalid_input("activity_id is required"))
}

/// Fetch ONE activity by id through the shared single-activity helper.
///
/// A single `get_activity_with_streams` round trip replaces the previous
/// 200-activity list scan: 200× cheaper, reaches activities older than the
/// recent window, and on providers with a real detail endpoint (Strava,
/// Garmin) it carries laps/splits the list rows never had.
async fn fetch_activity(
    resources: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: uuid::Uuid,
    activity_id: &str,
) -> AppResult<Activity> {
    let provider_name = if let Some(p) = default_provider() {
        p
    } else if let Some(conn) = resources
        .repos()
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await?
    {
        conn.provider
    } else {
        return Err(AppError::no_provider_connected());
    };
    let tenant_str = tenant_id.to_string();
    fetch_activity_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        activity_id,
    )
    .await
}

fn activity_id_schema() -> JsonSchema {
    let mut properties = HashMap::new();
    properties.insert(
        "activity_id".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Provider activity id to analyse (e.g. Strava activity id as string).".to_owned(),
            ),
            ..Default::default()
        },
    );
    JsonSchema {
        schema_type: "object".to_owned(),
        properties: Some(properties),
        required: Some(vec!["activity_id".to_owned()]),
        ..Default::default()
    }
}

/// `export_intervals` — Endurance per-lap interval shape for an activity.
pub struct ExportIntervalsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ExportIntervalsTool {
    fn definition(&self) -> Tool {
        let schema = activity_id_schema();
        tool_definition(
            "export_intervals",
            "Export the Endurance 'intervals.json' shape for a single activity — \
             one row per lap with avg HR, normalized power, intensity factor, and \
             decoupling. Use this when a coach needs the per-interval breakdown \
             for tempo/threshold/VO2max workouts. Activities without laps return \
             a single synthetic interval covering the whole session. Mirrors \
             GET /api/v1/endurance/intervals/{activity_id}.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::REQUIRES_PROVIDER
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
            let activity_id = activity_id_arg(&args)?;
            let activity =
                fetch_activity(&context.resources, tenant_id, user_id, &activity_id).await?;
            let physiology = context
                .resources
                .repos()
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?;
            let ftp_watts = physiology.as_ref().and_then(|p| p.ftp_watts);
            let intervals = build_intervals(&activity, ftp_watts);
            let payload = serde_json::to_value(&intervals)
                .map_err(|e| AppError::internal(format!("serialize intervals: {e}")))?;
            Ok(ToolResult::ok(payload))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// `export_routes` — GPX terrain + climbs for an activity.
pub struct ExportRoutesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ExportRoutesTool {
    fn definition(&self) -> Tool {
        let schema = activity_id_schema();
        tool_definition(
            "export_routes",
            "Export the Endurance 'routes.json' shape for a single activity — \
             GPX-derived terrain mix (flat/rolling/climb/steep), elevation gain/loss, \
             and distinct climb segments with Strava-style category. Requires the \
             activity stream to include lat/lon and altitude. Mirrors \
             GET /api/v1/endurance/routes/{activity_id}.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::REQUIRES_PROVIDER
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
            let activity_id = activity_id_arg(&args)?;
            let activity =
                fetch_activity(&context.resources, tenant_id, user_id, &activity_id).await?;
            // LIMITATION(registre#6): real streams arrive from Strava and
            // Intervals.icu via get_activity_with_streams; on every other
            // provider integration (Garmin, Fitbit, COROS, WHOOP, Terra,
            // sciotte) this branch still always fires — no sample source yet.
            let stream = activity.time_series_data().ok_or_else(|| {
                AppError::not_found("activity has no GPS stream — terrain unavailable")
            })?;
            let coords = stream
                .gps_coordinates
                .as_ref()
                .ok_or_else(|| AppError::not_found("activity stream has no gps_coordinates"))?;
            let altitudes = stream
                .altitude
                .as_ref()
                .ok_or_else(|| AppError::not_found("activity stream has no altitude"))?;
            // Warm-cache path: the stream's cache identity (hash + point
            // count) is computable without the terrain/climb analysis, so a
            // matching `route_summaries` row serves the stored summary and
            // skips the analysis and the write. Shared cache with
            // GET /api/v1/endurance/routes/{activity_id}.
            let (gpx_hash, point_count) =
                stream_route_identity(coords, altitudes).ok_or_else(|| {
                    AppError::not_found(
                        "activity stream has fewer than 2 paired GPS+altitude points",
                    )
                })?;
            if let Some((terrain_json, climbs_json)) = context
                .resources
                .repos()
                .route_summaries
                .get_route_summary(tenant_id, user_id, &activity_id, &gpx_hash)
                .await?
            {
                // A stored row that no longer deserializes (the summary shape
                // has evolved since it was written) is a cache miss: recompute
                // below and the upsert overwrites the stale row.
                if let Some(summary) =
                    route_summary_from_cache(&gpx_hash, point_count, &terrain_json, &climbs_json)
                {
                    let payload = serde_json::to_value(&summary)
                        .map_err(|e| AppError::internal(format!("serialize route summary: {e}")))?;
                    return Ok(ToolResult::ok(payload));
                }
                tracing::warn!(
                    %activity_id,
                    "cached route summary no longer deserializes; recomputing"
                );
            }
            let summary = build_route_summary_from_streams(coords, altitudes).ok_or_else(|| {
                AppError::invalid_input(
                    "activity stream produced a route summary with non-finite metrics",
                )
            })?;
            let terrain_json = serde_json::to_string(&summary.terrain)
                .map_err(|e| AppError::internal(format!("serialize terrain: {e}")))?;
            let climbs_json = serde_json::to_string(&summary.climbs)
                .map_err(|e| AppError::internal(format!("serialize climbs: {e}")))?;
            context
                .resources
                .repos()
                .route_summaries
                .upsert_route_summary(
                    tenant_id,
                    user_id,
                    &activity_id,
                    &summary.gpx_hash,
                    &terrain_json,
                    &climbs_json,
                )
                .await?;
            let payload = serde_json::to_value(&summary)
                .map_err(|e| AppError::internal(format!("serialize route summary: {e}")))?;
            Ok(ToolResult::ok(payload))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// `extract_activity_streams` — surface raw HR / power / cadence / GPS / altitude
/// per-second arrays for downstream analysis.
pub struct ExtractActivityStreamsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ExtractActivityStreamsTool {
    fn definition(&self) -> Tool {
        let schema = activity_id_schema();
        tool_definition(
            "extract_activity_streams",
            "Return the raw per-second time-series streams for a single activity — \
             heart_rate (bpm), power (watts), cadence (rpm/spm), speed (m/s), \
             altitude (m), and gps_coordinates (lat/lon pairs). Each stream is \
             omitted when the provider didn't record it. Use this when a coach \
             needs the underlying samples to rerun a custom analysis the higher \
             Endurance tools (export_intervals / export_routes) don't cover.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::REQUIRES_PROVIDER
                | ToolCapabilities::READS_DATA,
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
            let activity_id = activity_id_arg(&args)?;
            let activity =
                fetch_activity(&context.resources, tenant_id, user_id, &activity_id).await?;
            // LIMITATION(registre#6): real streams arrive from Strava and
            // Intervals.icu via get_activity_with_streams; on every other
            // provider integration this branch still always fires — no
            // sample source yet.
            let stream = activity
                .time_series_data()
                .ok_or_else(|| AppError::not_found("activity has no time-series data"))?;
            Ok(ToolResult::ok(json!({
                "activity_id": activity_id,
                "sample_count": stream.timestamps.len(),
                "streams": serde_json::to_value(stream).map_err(|e| {
                    AppError::internal(format!("serialize stream: {e}"))
                })?,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Build the Endurance intervals/routes tool list for registry registration.
#[must_use]
pub fn create_endurance_intervals_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ExportIntervalsTool),
        Box::new(ExportRoutesTool),
        Box::new(ExtractActivityStreamsTool),
    ]
}

// Guardian security classifications (see `pierre_tool_runtime::security`). These
// tools export the caller's own activity intervals/routes — internal, no egress.
pierre_tool_runtime::declare_security!(ExportIntervalsTool => empty);
pierre_tool_runtime::declare_security!(ExportRoutesTool => empty);
pierre_tool_runtime::declare_security!(ExtractActivityStreamsTool => empty);
