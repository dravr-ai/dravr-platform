// ABOUTME: Endurance Phase 1 MCP tools — export_latest_snapshot + export_dossier
// ABOUTME: Mirrors GET /api/v1/endurance/{latest,dossier} so coaches can pull the same payloads via MCP
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{Activity, TenantId, UserPhysiologicalProfile};
use pierre_fitness_compute::latest_snapshot::{
    build_latest_snapshot, DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS,
};
use pierre_fitness_compute::threshold_estimation::{ThresholdEstimate, ThresholdInputs};
use serde_json::Value;

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::environment::default_provider;
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tool_runtime::capabilities::ToolCapabilities;
use pierre_tool_runtime::context::ToolExecutionContext;
use pierre_tool_runtime::conversions::{
    capabilities_to_tronc, task_capable, tool_definition, tool_result_to_response,
};
use pierre_tool_runtime::protocol::provider_helpers::fetch_activities_from_provider;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::security::RuntimeTool;
use pierre_tools_core::ToolResult;
use tracing::warn;

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
    resources: &Arc<dyn ToolRuntime>,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Activity>> {
    let provider_name = resolve_provider_for_user(resources, user_id, tenant_id).await?;
    let tenant_str = tenant_id.to_string();
    fetch_activities_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        Some(MAX_ACTIVITIES_PER_CALL),
    )
    .await
}

/// Resolve the user's active provider for non-LLM read paths (REST routes, MCP
/// tools that don't accept a `provider` argument). Mirrors the chat-side
/// `resolve_provider_for_request` priority chain minus the explicit-arg step:
/// env override → user's most-recently-used connection → `AppError::no_provider_connected()`.
async fn resolve_provider_for_user(
    resources: &Arc<dyn ToolRuntime>,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
) -> AppResult<String> {
    if let Some(env_p) = default_provider() {
        return Ok(env_p);
    }
    match resources
        .repos()
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await
    {
        Ok(Some(conn)) => Ok(conn.provider),
        Ok(None) => {
            warn!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                "endurance_export: no provider connected for user — surfacing reconnect signal"
            );
            Err(AppError::no_provider_connected())
        }
        Err(e) => Err(e),
    }
}

/// `export_latest_snapshot` — IF / EF / VI / decoupling / zone-distribution
/// snapshot for the user's most recent training window.
pub struct ExportLatestSnapshotTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ExportLatestSnapshotTool {
    fn definition(&self) -> Tool {
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(Vec::new()),
            ..Default::default()
        };
        task_capable(tool_definition(
            "export_latest_snapshot",
            "Export the Endurance 'latest.json' snapshot for the authenticated user — \
             per-activity intensity factor, efficiency factor, variability index, \
             aerobic decoupling, and time-in-zone distribution aggregated over a \
             sliding window. Default window is 7 days; pass `window` (1..=365) to \
             change it. Use this when a coach needs a structured per-activity \
             training-state snapshot grounded in the Endurance deterministic-output \
             contract. The response shape mirrors `GET /api/v1/endurance/latest`.",
            schema,
            Some(read_only_annotations()),
        ))
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
            let window = args
                .get("window")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(DEFAULT_WINDOW_DAYS)
                .clamp(MIN_WINDOW_DAYS, MAX_WINDOW_DAYS);

            let tenant_id = require_tenant(&context)?;
            let user_id = context.user_id;

            let activities =
                fetch_window_activities(&context.resources, user_id, tenant_id).await?;
            let physiology = context
                .resources
                .repos()
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
        .await;
        tool_result_to_response(result)
    }
}

/// `export_dossier` — composed athlete dossier (physiology, zones, goals,
/// nutrition, equipment slots) for the authenticated user.
pub struct ExportDossierTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ExportDossierTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: Some(Vec::new()),
            ..Default::default()
        };
        task_capable(tool_definition(
            "export_dossier",
            "Export the Endurance 'dossier.json' aggregate for the authenticated \
             user — physiological profile (VO2max, FTP, threshold pace, fitness \
             level), HR + power zones, goals, nutrition, and equipment slots — \
             composed at read time from the underlying tables. Empty slots come \
             back as `null` rather than 404, so coaches can rely on the shape. \
             Mirrors `GET /api/v1/endurance/dossier`.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
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
            // ExportDossierTool takes no arguments — accept any object shape and
            // discard. Forward-compat for future filter/include params.
            drop(args);
            let tenant_id = require_tenant(&context)?;
            let user_id = context.user_id;
            let dossier = context
                .resources
                .repos()
                .dossier
                .compose_dossier(tenant_id, user_id)
                .await?;
            let mut payload = serde_json::to_value(&dossier)
                .map_err(|e| AppError::internal(format!("serialize dossier: {e}")))?;

            // Surface estimated lactate thresholds (LT1/LT2 heart rate, power, and
            // pace) derived from the athlete's physiological profile — Coggan
            // (LT1 power = 0.75 * FTP) and Seiler-style pace anchors. This gives the
            // dossier training-zone anchors even when the athlete hasn't measured
            // them directly. Missing profile fields stay `None`; the estimate block
            // is always present (empty when nothing is known) to keep the shape
            // stable, matching the dossier's "empty slots, not 404" contract.
            let physiology = context
                .resources
                .repos()
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?;
            let estimate =
                ThresholdEstimate::from_inputs(threshold_inputs_from_profile(physiology.as_ref()));
            if let Value::Object(map) = &mut payload {
                map.insert(
                    "threshold_estimate".to_owned(),
                    serde_json::to_value(estimate).map_err(|e| {
                        AppError::internal(format!("serialize threshold estimate: {e}"))
                    })?,
                );
            }

            Ok(ToolResult::ok(payload))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Map a stored [`UserPhysiologicalProfile`] into [`ThresholdInputs`] for LT1/LT2
/// estimation. Unset fields stay `None` so the estimator derives only what the
/// athlete's known physiology supports (e.g. LT2 HR from max HR).
fn threshold_inputs_from_profile(profile: Option<&UserPhysiologicalProfile>) -> ThresholdInputs {
    ThresholdInputs {
        max_hr: profile.and_then(|p| p.max_hr).map(f64::from),
        resting_hr: profile.and_then(|p| p.resting_hr).map(f64::from),
        // The profile stores a lactate-threshold *percentage*, not an absolute
        // LT HR in bpm; the estimator derives LT2 HR from max HR instead.
        lthr: None,
        ftp_watts: profile.and_then(|p| p.ftp_watts).map(f64::from),
        threshold_pace_sec_per_km: profile.and_then(|p| p.threshold_pace_sec_per_km),
    }
}

/// Build the Endurance export tool list for registry registration.
#[must_use]
pub fn create_endurance_export_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ExportLatestSnapshotTool),
        Box::new(ExportDossierTool),
    ]
}

// Guardian security classifications (see `pierre_tool_runtime::security`). These
// tools read the caller's own training data — internal, reversible, no egress.
pierre_tool_runtime::declare_security!(ExportLatestSnapshotTool => empty);
pierre_tool_runtime::declare_security!(ExportDossierTool => empty);
