// ABOUTME: Endurance Phase 1 MCP tools — export_latest_snapshot + export_dossier
// ABOUTME: Mirrors GET /api/v1/endurance/{latest,dossier} so coaches can pull the same payloads via MCP
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{Activity, TenantId};
use pierre_intelligence::latest_snapshot::{
    build_latest_snapshot, DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS,
};
use serde_json::Value;

use crate::config::environment::default_provider;
use crate::mcp::resources::ServerContext;
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::routes::social::SocialRoutes;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

/// Lower bound on the analysis window. Mirrors the constant in
/// `routes/endurance.rs` so HTTP and MCP clamp the same way.
const MIN_WINDOW_DAYS: u32 = 1;

/// Maximum activities pulled per call — same cap as the HTTP endpoint.
const MAX_ACTIVITIES_PER_CALL: usize = 200;

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
            "Endurance export tools require an active tenant context",
        )
    })
}

async fn fetch_window_activities(
    resources: &Arc<ServerContext>,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Activity>> {
    let provider_name = default_provider();
    let tenant_str = tenant_id.to_string();
    SocialRoutes::fetch_activities_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        Some(MAX_ACTIVITIES_PER_CALL),
    )
    .await
}

/// `export_latest_snapshot` — IF / EF / VI / decoupling / zone-distribution
/// snapshot for the user's most recent training window.
pub struct ExportLatestSnapshotTool;

#[async_trait]
impl McpTool for ExportLatestSnapshotTool {
    fn name(&self) -> &'static str {
        "export_latest_snapshot"
    }

    fn description(&self) -> &'static str {
        "Export the Endurance 'latest.json' snapshot for the authenticated user — \
         per-activity intensity factor, efficiency factor, variability index, \
         aerobic decoupling, and time-in-zone distribution aggregated over a \
         sliding window. Default window is 7 days; pass `window` (1..=365) to \
         change it. Use this when a coach needs a structured per-activity \
         training-state snapshot grounded in the Endurance deterministic-output \
         contract. The response shape mirrors `GET /api/v1/endurance/latest`."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "window".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Window length in days (1..=365). Defaults to 7 when omitted.".to_owned(),
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
        let window = args
            .get("window")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(MIN_WINDOW_DAYS, MAX_WINDOW_DAYS);

        let tenant_id = require_tenant(context)?;
        let user_id = context.user_id;

        let activities = fetch_window_activities(&context.resources, user_id, tenant_id).await?;
        let physiology = context
            .resources
            .repos
            .user_physiological_profile
            .get_user_physiological_profile(tenant_id, user_id)
            .await?;
        let ftp_watts = physiology.as_ref().and_then(|p| p.ftp_watts);
        let hr_zones = physiology.as_ref().and_then(|p| p.hr_zones);
        let snapshot = build_latest_snapshot(&activities, window, ftp_watts, hr_zones);

        let payload = serde_json::to_value(&snapshot)
            .map_err(|e| AppError::internal(format!("serialize latest snapshot: {e}")))?;
        Ok(ToolResult::ok(payload))
    }
}

/// `export_dossier` — composed athlete dossier (physiology, zones, goals,
/// nutrition, equipment slots) for the authenticated user.
pub struct ExportDossierTool;

#[async_trait]
impl McpTool for ExportDossierTool {
    fn name(&self) -> &'static str {
        "export_dossier"
    }

    fn description(&self) -> &'static str {
        "Export the Endurance 'dossier.json' aggregate for the authenticated \
         user — physiological profile (VO2max, FTP, threshold pace, fitness \
         level), HR + power zones, goals, nutrition, and equipment slots — \
         composed at read time from the underlying tables. Empty slots come \
         back as `null` rather than 404, so coaches can rely on the shape. \
         Mirrors `GET /api/v1/endurance/dossier`."
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: Some(Vec::new()),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        // ExportDossierTool takes no arguments — accept any object shape and
        // discard. Forward-compat for future filter/include params.
        drop(args);
        let tenant_id = require_tenant(context)?;
        let user_id = context.user_id;
        let dossier = context
            .resources
            .repos
            .dossier
            .compose_dossier(tenant_id, user_id)
            .await?;
        let payload = serde_json::to_value(&dossier)
            .map_err(|e| AppError::internal(format!("serialize dossier: {e}")))?;
        Ok(ToolResult::ok(payload))
    }
}

/// Build the Endurance export tool list for registry registration.
#[must_use]
pub fn create_endurance_export_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(ExportLatestSnapshotTool),
        Box::new(ExportDossierTool),
    ]
}
