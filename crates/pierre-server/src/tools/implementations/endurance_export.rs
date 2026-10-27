// ABOUTME: Endurance Phase 1 MCP tools — export_latest_snapshot + export_dossier
// ABOUTME: Serves the Endurance latest.json and dossier.json payloads to agents via MCP
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_providers::activity_source::resolve_activity_source;
use pierre_providers::ai_scope;
use pierre_tool_runtime::annotations::{annotations, Effect, Reach};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use dravr_cageux::zones::{AerobicThresholdPace, AerobicThresholdPower, HrZones};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::zones::athlete_hr_zones;
use pierre_core::models::{Activity, PhysiologicalSex, TenantId, UserPhysiologicalProfile};
use pierre_fitness_compute::latest_snapshot::{
    build_latest_snapshot, DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_config::environment::default_provider;
use pierre_core::models::Dossier;
use pierre_fitness_compute::latest_snapshot::LatestSnapshot;
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tool_runtime::context::ToolExecutionContext;
use pierre_tool_runtime::conversions::{
    answers_with, ok_typed, task_capable, tool_definition, tool_result_to_response,
};
use pierre_tool_runtime::protocol::provider_helpers::fetch_activities_from_provider;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::security::RuntimeTool;
use pierre_tools_core::ToolResult;
use tracing::warn;

/// What `export_dossier` answers with.
///
/// The composed dossier with the estimated lactate thresholds alongside it.
/// The dossier is flattened in rather than nested, which is what this tool
/// has always sent.
///
/// `threshold_estimate` is always present, empty when nothing is known.
/// That is the dossier's own "empty slots, not 404" contract: an agent reading
/// the payload can tell "no FTP on file" from "this tool does not report
/// thresholds", and a key that appears only sometimes cannot.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DossierExport {
    /// Physiology, zones, goals and the durable facts, as composed.
    #[serde(flatten)]
    pub dossier: Dossier,
    /// LT1/LT2 heart rate, power and pace read off the profile — heart rate
    /// at 75 % / 85 % of the athlete's reserve unless an LTHR is saved, FTP
    /// and two published LT1 power estimates for power, the saved threshold
    /// pace and its LT1 estimate for pace. Gives the dossier training zone
    /// anchors even when the athlete has measured none directly.
    pub threshold_estimate: ThresholdEstimate,
}

/// Lower bound on the analysis window. Mirrors the constant in
/// `routes/endurance.rs` so HTTP and MCP clamp the same way.
const MIN_WINDOW_DAYS: u32 = 1;

/// Maximum activities pulled per call — same cap as the HTTP endpoint.
const MAX_ACTIVITIES_PER_CALL: usize = 200;

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
/// env override → the elected activity source → `AppError::no_provider_connected()`.
async fn resolve_provider_for_user(
    resources: &Arc<dyn ToolRuntime>,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
) -> AppResult<String> {
    if let Some(env_p) = default_provider() {
        return Ok(env_p);
    }
    match resolve_activity_source(
        resources.repos().provider_connections.as_ref(),
        resources.provider_registry(),
        user_id,
        Some(tenant_id),
    )
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
#[derive(Debug)]
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
            properties: Some(properties.into_iter().collect()),
            required: Some(Vec::new()),
            ..Default::default()
        };
        answers_with::<LatestSnapshot>(task_capable(tool_definition(
            "export_latest_snapshot",
            "Export the Endurance 'latest.json' snapshot for the authenticated user — \
             per-activity intensity factor, efficiency factor, variability index, \
             aerobic decoupling, and time-in-zone distribution aggregated over a \
             sliding window. Default window is 7 days; pass `window` (1..=365) to \
             change it. Use this when a coach needs a structured per-activity \
             training-state snapshot grounded in the Endurance deterministic-output \
             contract.",
            schema,
            annotations(Effect::ReadOnly, Reach::OpenWorld),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::READS_DATA
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
                .await?
                // A profile written from first-party-only data is withheld from an
                // external caller (carnet#769).
                .filter(|profile| ai_scope::admit_derived(profile.transport_policy));
            let ftp_watts = physiology.as_ref().and_then(|p| p.ftp_watts);
            let hr_zones = athlete_hr_zones(physiology.as_ref());
            let snapshot = build_latest_snapshot(&activities, window, ftp_watts, hr_zones.as_ref());

            let payload = serde_json::to_value(&snapshot)
                .map_err(|e| AppError::internal(format!("serialize latest snapshot: {e}")))?;
            Ok(ToolResult::ok(payload))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// `export_dossier` — composed athlete dossier (physiology, zones, goals,
/// nutrition slots) for the authenticated user.
#[derive(Debug)]
pub struct ExportDossierTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ExportDossierTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: Some(Vec::new()),
            ..Default::default()
        };
        answers_with::<DossierExport>(tool_definition(
            "export_dossier",
            "Export the Endurance 'dossier.json' aggregate for the authenticated \
             user — physiological profile (VO2max, FTP, threshold pace, fitness \
             level, critical power, W′, critical speed, D′), HR + power zones, \
             goals and nutrition slots — composed at read time from \
             the underlying tables. Critical power, W′, critical speed and D′ each \
             carry `kind` (measured or estimated) and `origin`: quote an estimated \
             value as an estimate attributed to its origin ('Vekta estimates your \
             CP at 312 W'), never as a measurement. An FTP, threshold or max heart \
             rate, or threshold pace a provider supplied is named in \
             `threshold_provenance` with its origin: quote it as that provider's \
             estimate ('intervals.icu has your FTP at 285 W'); one not named there \
             is the athlete's own. Empty slots come back as `null` rather than \
             404, so agents can rely on the shape.",
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
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
            // Facts and a profile derived from first-party-only data are left
            // out for an external caller (carnet#769).
            let dossier = context
                .resources
                .repos()
                .dossier
                .compose_dossier(
                    tenant_id,
                    user_id,
                    ai_scope::readable_policy(),
                    &ai_scope::admit_derived,
                )
                .await?;
            let dossier = dossier;

            // Surface the lactate threshold anchors (LT1/LT2 heart rate, power and
            // pace) read off the athlete's physiological profile through cageux's
            // zone model. This gives the dossier training-zone anchors even when
            // the athlete hasn't measured them directly. Missing profile fields
            // stay `None`; the estimate block is always present (empty when
            // nothing is known) to keep the shape stable, matching the dossier's
            // "empty slots, not 404" contract.
            let physiology = context
                .resources
                .repos()
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?
                .filter(|profile| ai_scope::admit_derived(profile.transport_policy));
            let estimate = ThresholdEstimate::from_profile(physiology.as_ref());

            ok_typed(
                "export_dossier",
                DossierExport {
                    dossier,
                    threshold_estimate: estimate,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The dossier's LT1 / LT2 anchors, each read from where it lives rather
/// than re-derived here.
///
/// - **LT1** (aerobic threshold) heart rate is cageux's
///   [`HrZones::aerobic_threshold_bpm`], 75 % of the athlete's heart-rate
///   reserve; LT1 power is cageux's [`AerobicThresholdPower::estimates`]
///   from the FTP, one per published VT1/VT2 power ratio, each with its note.
/// - **LT2** (lactate threshold) heart rate is the athlete's LTHR (measured,
///   else estimated from their lactate threshold), else cageux's
///   [`HrZones::lactate_threshold_bpm`], 85 % of reserve; LT2 power is the
///   FTP and LT2 pace the saved threshold pace.
/// - **LT1 pace** is cageux's [`AerobicThresholdPace`] from the saved
///   (running) threshold pace: the VT1/VT2 speed ratio of Esteve-Lanao et al.
///   (2026), with its one-SD bounds and the cohort the ratio came from: the
///   men's or women's ratio when the athlete's sex is saved, the pooled one
///   otherwise.
///
/// An anchor whose inputs are missing is absent: the heart-rate anchors need
/// the athlete's zones (a saved resting and maximum heart rate), the power
/// anchors an FTP, the pace anchor a threshold pace.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ThresholdEstimate {
    /// Aerobic threshold heart rate in bpm: 75 % of the athlete's heart-rate
    /// reserve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt1_hr: Option<f64>,
    /// Lactate threshold heart rate in bpm: the athlete's LTHR, else 85 % of
    /// their heart-rate reserve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt2_hr: Option<f64>,
    /// LT1 power estimated from the FTP, one entry per published VT1/VT2
    /// power ratio (Pallarés et al. 2016, then Benítez-Muñoz et al. 2025).
    /// Each carries its watts, the ratio, the study's range when it reports
    /// one, and a `note` on what it rests on: quote LT1 power as these
    /// estimates with their notes, never as one measured number. Absent
    /// without a saved FTP.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lt1_power: Vec<AerobicThresholdPower>,
    /// LT2 power (watts): the FTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt2_power_watts: Option<f64>,
    /// LT2 pace (sec/km): the saved threshold pace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt2_pace_sec_per_km: Option<f64>,
    /// LT1 pace estimated from that threshold pace (Esteve-Lanao et al.
    /// 2026), with the faster and slower bound one SD of the ratio either
    /// side and the cohort. Absent without a saved threshold pace, or when it
    /// lies outside the 120-600 s/km running range the estimate covers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt1_pace: Option<AerobicThresholdPace>,
}

impl ThresholdEstimate {
    /// Read the anchors off a stored [`UserPhysiologicalProfile`].
    fn from_profile(profile: Option<&UserPhysiologicalProfile>) -> Self {
        let zones: Option<HrZones> = athlete_hr_zones(profile).ok();
        let ftp_watts = profile.and_then(|p| p.ftp_watts).map(f64::from);
        let threshold_pace = profile.and_then(|p| p.threshold_pace_sec_per_km);
        // LT2 heart rate is the LTHR training load scores against — the
        // measured one, else the estimate from the lactate threshold and max
        // HR — so the dossier and the load cannot name two thresholds.
        let lt2_hr = profile
            .and_then(UserPhysiologicalProfile::lactate_threshold_hr)
            .or_else(|| zones.as_ref().map(|z| f64::from(z.lactate_threshold_bpm)));
        let lt1_power = ftp_watts
            .and_then(|ftp| AerobicThresholdPower::estimates(ftp).ok())
            .unwrap_or_default();
        Self {
            lt1_hr: zones.as_ref().map(|z| f64::from(z.aerobic_threshold_bpm)),
            lt2_hr,
            lt1_power,
            lt2_power_watts: ftp_watts,
            lt2_pace_sec_per_km: threshold_pace,
            lt1_pace: threshold_pace.and_then(|pace| {
                let sex = profile
                    .and_then(|p| p.sex)
                    .map(PhysiologicalSex::for_equations);
                AerobicThresholdPace::from_threshold_pace(pace, sex).ok()
            }),
        }
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
