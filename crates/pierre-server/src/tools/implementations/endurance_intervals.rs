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
use pierre_intelligence::intervals::build_intervals;
use pierre_intelligence::routes::build_route_summary_from_streams;
use serde_json::{json, Value};

use crate::config::environment::default_provider;
use crate::mcp::resources::ServerResources;
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::routes::social::SocialRoutes;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

const MAX_ACTIVITIES_PER_FETCH: usize = 200;

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

async fn fetch_activity(
    resources: &Arc<ServerResources>,
    tenant_id: TenantId,
    user_id: uuid::Uuid,
    activity_id: &str,
) -> AppResult<Activity> {
    let provider_name = default_provider();
    let tenant_str = tenant_id.to_string();
    let activities = SocialRoutes::fetch_activities_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        Some(MAX_ACTIVITIES_PER_FETCH),
    )
    .await?;
    activities
        .into_iter()
        .find(|a| a.id() == activity_id)
        .ok_or_else(|| AppError::not_found(format!("activity {activity_id} not in recent window")))
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
    }
}

/// `export_intervals` — Endurance per-lap interval shape for an activity.
pub struct ExportIntervalsTool;

#[async_trait]
impl McpTool for ExportIntervalsTool {
    fn name(&self) -> &'static str {
        "export_intervals"
    }

    fn description(&self) -> &'static str {
        "Export the Endurance 'intervals.json' shape for a single activity — \
         one row per lap with avg HR, normalized power, intensity factor, and \
         decoupling. Use this when a coach needs the per-interval breakdown \
         for tempo/threshold/VO2max workouts. Activities without laps return \
         a single synthetic interval covering the whole session. Mirrors \
         GET /api/v1/endurance/intervals/{activity_id}."
    }

    fn input_schema(&self) -> JsonSchema {
        activity_id_schema()
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = require_tenant(context)?;
        let user_id = context.user_id;
        let activity_id = activity_id_arg(&args)?;
        let activity = fetch_activity(&context.resources, tenant_id, user_id, &activity_id).await?;
        let physiology = context
            .resources
            .repos
            .user_physiological_profile
            .get_user_physiological_profile(tenant_id, user_id)
            .await?;
        let ftp_watts = physiology.as_ref().and_then(|p| p.ftp_watts);
        let intervals = build_intervals(&activity, ftp_watts);
        let payload = serde_json::to_value(&intervals)
            .map_err(|e| AppError::internal(format!("serialize intervals: {e}")))?;
        Ok(ToolResult::ok(payload))
    }
}

/// `export_routes` — GPX terrain + climbs for an activity.
pub struct ExportRoutesTool;

#[async_trait]
impl McpTool for ExportRoutesTool {
    fn name(&self) -> &'static str {
        "export_routes"
    }

    fn description(&self) -> &'static str {
        "Export the Endurance 'routes.json' shape for a single activity — \
         GPX-derived terrain mix (flat/rolling/climb/steep), elevation gain/loss, \
         and distinct climb segments with Strava-style category. Requires the \
         activity stream to include lat/lon and altitude. Mirrors \
         GET /api/v1/endurance/routes/{activity_id}."
    }

    fn input_schema(&self) -> JsonSchema {
        activity_id_schema()
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::ANALYTICS
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = require_tenant(context)?;
        let user_id = context.user_id;
        let activity_id = activity_id_arg(&args)?;
        let activity = fetch_activity(&context.resources, tenant_id, user_id, &activity_id).await?;
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
        let summary = build_route_summary_from_streams(coords, altitudes).ok_or_else(|| {
            AppError::not_found("activity stream has fewer than 2 paired GPS+altitude points")
        })?;
        // Cache write so the HTTP endpoint can short-circuit on the next call.
        let terrain_json = serde_json::to_string(&summary.terrain)
            .map_err(|e| AppError::internal(format!("serialize terrain: {e}")))?;
        let climbs_json = serde_json::to_string(&summary.climbs)
            .map_err(|e| AppError::internal(format!("serialize climbs: {e}")))?;
        context
            .resources
            .repos
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
}

/// `extract_activity_streams` — surface raw HR / power / cadence / GPS / altitude
/// per-second arrays for downstream analysis.
pub struct ExtractActivityStreamsTool;

#[async_trait]
impl McpTool for ExtractActivityStreamsTool {
    fn name(&self) -> &'static str {
        "extract_activity_streams"
    }

    fn description(&self) -> &'static str {
        "Return the raw per-second time-series streams for a single activity — \
         heart_rate (bpm), power (watts), cadence (rpm/spm), speed (m/s), \
         altitude (m), and gps_coordinates (lat/lon pairs). Each stream is \
         omitted when the provider didn't record it. Use this when a coach \
         needs the underlying samples to rerun a custom analysis the higher \
         Endurance tools (export_intervals / export_routes) don't cover."
    }

    fn input_schema(&self) -> JsonSchema {
        activity_id_schema()
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = require_tenant(context)?;
        let user_id = context.user_id;
        let activity_id = activity_id_arg(&args)?;
        let activity = fetch_activity(&context.resources, tenant_id, user_id, &activity_id).await?;
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
}

/// Build the Endurance intervals/routes tool list for registry registration.
#[must_use]
pub fn create_endurance_intervals_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(ExportIntervalsTool),
        Box::new(ExportRoutesTool),
        Box::new(ExtractActivityStreamsTool),
    ]
}
