// ABOUTME: Endurance Phase 1 read-side endpoints — GET /api/v1/endurance/{latest,dossier}
// ABOUTME: Auth: JWT-bearer, scoped by tenant_id from the active session; multi-tenant isolation enforced
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, DailyTrainingState, Dossier, TenantId, WorkoutTemplate};
use pierre_fitness_compute::intervals::{build_intervals, IntervalsExport};
use pierre_fitness_compute::latest_snapshot::{
    build_latest_snapshot, LatestSnapshot, DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS,
};
use pierre_fitness_compute::routes::{
    build_route_summary_from_streams, route_summary_from_cache, stream_route_identity, RouteSummary,
};
use pierre_fitness_compute::training_history_compute::MAX_BACKFILL_DAYS;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::training_history_compute::{
    compute_and_persist_history, fetch_history_rows, DEFAULT_BACKFILL_DAYS,
};
use crate::tools::runtime_adapter::into_runtime;
use pierre_config::environment::default_provider;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::workout_library::cornerstone_templates;
use pierre_tool_runtime::protocol::provider_helpers::{
    fetch_activities_from_provider, fetch_activity_from_provider,
};
use pierre_tool_runtime::runtime::ToolRuntime;

/// Lower bound on the analysis window — defended in addition to the
/// `clamp` inside [`build_latest_snapshot`] so the API surface and the
/// intelligence module agree.
const MIN_WINDOW_DAYS: u32 = 1;

/// Maximum number of activities the latest endpoint will pull from
/// providers per call. Keeps the cost of the read predictable when an
/// athlete's window contains many short sessions.
const MAX_ACTIVITIES_PER_LATEST_CALL: usize = 200;

/// Query parameters for `GET /api/v1/endurance/latest`.
#[derive(Debug, Deserialize)]
pub struct LatestQuery {
    /// Window length in days. Clamped to `[MIN_WINDOW_DAYS, MAX_WINDOW_DAYS]`
    /// before use to satisfy the input-domain-validation rule in CLAUDE.md.
    #[serde(default)]
    pub window: Option<u32>,
}

/// Build the Endurance router.
///
/// Endpoints:
/// - `GET /api/v1/endurance/latest?window=N` — per-window IF/EF/VI/decoupling
///   + zone distribution snapshot (auth: JWT, tenant scope from session)
/// - `GET /api/v1/endurance/dossier` — composed athlete dossier
///   (physiology, zones, goals, nutrition, equipment slots)
/// - `GET /api/v1/endurance/history?from=&to=` — daily CTL/ATL/TSB/ACWR/
///   monotony/strain/`ramp_rate`/`daily_load` rollup over the requested
///   window. Auto-recomputes when persisted rows are stale or missing.
pub fn endurance_routes() -> Router<Arc<ServerContext>> {
    Router::new()
        .route("/api/v1/endurance/latest", get(get_latest_snapshot))
        .route("/api/v1/endurance/dossier", get(get_dossier))
        .route("/api/v1/endurance/history", get(get_history))
        .route(
            "/api/v1/endurance/intervals/{activity_id}",
            get(get_intervals),
        )
        .route("/api/v1/endurance/routes/{activity_id}", get(get_routes))
        .route(
            "/api/v1/endurance/workout-templates",
            get(get_workout_templates),
        )
}

async fn get_workout_templates(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<Vec<WorkoutTemplate>>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    // Compiled-in cornerstones first (read-only TOML library), then any
    // user-authored overrides for (tenant_id, user_id) ordered newest-first.
    let mut templates = cornerstone_templates();
    let user_authored = resources
        .repos()
        .workout_templates
        .list_user_workout_templates(tenant_id, user_id)
        .await?;
    templates.extend(user_authored);
    Ok(Json(templates))
}

async fn get_intervals(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(activity_id): Path<String>,
) -> AppResult<Json<IntervalsExport>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let activity =
        fetch_activity_by_id(&into_runtime(&resources), user_id, tenant_id, &activity_id).await?;
    let physiology = resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await?;
    let ftp_watts = physiology.as_ref().and_then(|p| p.ftp_watts);
    Ok(Json(build_intervals(&activity, ftp_watts)))
}

async fn get_routes(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(activity_id): Path<String>,
) -> AppResult<Json<RouteSummary>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let activity =
        fetch_activity_by_id(&into_runtime(&resources), user_id, tenant_id, &activity_id).await?;
    let stream = activity
        .time_series_data()
        .ok_or_else(|| AppError::not_found("activity has no GPS stream — terrain unavailable"))?;
    let coords = stream.gps_coordinates.as_ref().ok_or_else(|| {
        AppError::not_found("activity stream has no gps_coordinates — terrain unavailable")
    })?;
    let altitudes = stream.altitude.as_ref().ok_or_else(|| {
        AppError::not_found("activity stream has no altitude — terrain unavailable")
    })?;
    // The cache identity (hash of the validated points + their count) is
    // computable without the terrain/climb analysis, so a warm cache serves
    // the stored summary and skips the analysis and the write. A `None`
    // identity means fewer than 2 paired points or every point filtered out
    // by the lat/lon/finite gate — bad-input conditions for the user, so
    // surface as 400, not 404, and write no cache row (a malformed stream
    // must not poison subsequent reads on the same activity — Phase 3
    // risk #3).
    let (gpx_hash, point_count) = stream_route_identity(coords, altitudes).ok_or_else(|| {
        AppError::invalid_input(
            "activity stream produced no valid GPS+altitude points (fewer than 2 paired, \
             out-of-range lat/lon, or non-finite values)",
        )
    })?;
    if let Some((terrain_json, climbs_json)) = resources
        .repos()
        .route_summaries
        .get_route_summary(tenant_id, user_id, &activity_id, &gpx_hash)
        .await?
    {
        // A stored row that no longer deserializes (the summary shape has
        // evolved since it was written) is a cache miss: recompute below and
        // the upsert overwrites the stale row.
        if let Some(summary) =
            route_summary_from_cache(&gpx_hash, point_count, &terrain_json, &climbs_json)
        {
            return Ok(Json(summary));
        }
        tracing::warn!(%activity_id, "cached route summary no longer deserializes; recomputing");
    }
    // The remaining `None` from the full builder: the produced summary
    // carries a non-finite metric — same bad-input stance, same no-write
    // guarantee.
    let summary = build_route_summary_from_streams(coords, altitudes).ok_or_else(|| {
        AppError::invalid_input("activity stream produced a route summary with non-finite metrics")
    })?;

    // Cache write: terrain + climbs JSON keyed by gpx_hash.
    let terrain_json = serde_json::to_string(&summary.terrain)
        .map_err(|e| AppError::internal(format!("serialize terrain_summary: {e}")))?;
    let climbs_json = serde_json::to_string(&summary.climbs)
        .map_err(|e| AppError::internal(format!("serialize climbs: {e}")))?;
    resources
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
    Ok(Json(summary))
}

async fn fetch_activity_by_id(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
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
    // One detail round trip; the MCP export tools share this helper, so the
    // HTTP and tool paths cannot drift back into the 200-row window scan.
    fetch_activity_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        activity_id,
    )
    .await
}

/// Query parameters for `GET /api/v1/endurance/history`.
#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    /// Inclusive start of the window (ISO date `YYYY-MM-DD`). When omitted,
    /// defaults to `to - DEFAULT_BACKFILL_DAYS`.
    #[serde(default)]
    pub from: Option<NaiveDate>,
    /// Inclusive end of the window (ISO date `YYYY-MM-DD`). When omitted,
    /// defaults to today (UTC).
    #[serde(default)]
    pub to: Option<NaiveDate>,
}

/// Response shape for `GET /api/v1/endurance/history`.
#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    /// Inclusive start of the analyzed window (UTC).
    pub from: NaiveDate,
    /// Inclusive end of the analyzed window (UTC).
    pub to: NaiveDate,
    /// Daily rows in chronological order (oldest first).
    pub days: Vec<DailyTrainingState>,
}

async fn get_history(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<HistoryQuery>,
) -> AppResult<Json<HistoryResponse>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;

    let to = query.to.unwrap_or_else(|| Utc::now().date_naive());
    let from = query
        .from
        .unwrap_or_else(|| to - ChronoDuration::days(DEFAULT_BACKFILL_DAYS));
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    if (to - from).num_days() > MAX_BACKFILL_DAYS {
        return Err(AppError::invalid_input(format!(
            "history window exceeds the maximum of {MAX_BACKFILL_DAYS} days"
        )));
    }

    let mut rows = fetch_history_rows(&resources.data(), tenant_id, user_id, from, to).await?;
    if rows.is_empty() {
        // Cold start: no rows persisted yet for this user. Trigger an
        // on-demand compute for the same window so the next call hits
        // the persisted rollup.
        let _ =
            compute_and_persist_history(&into_runtime(&resources), tenant_id, user_id, from, to)
                .await?;
        rows = fetch_history_rows(&resources.data(), tenant_id, user_id, from, to).await?;
    }
    Ok(Json(HistoryResponse {
        from,
        to,
        days: rows,
    }))
}

async fn get_latest_snapshot(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<LatestQuery>,
) -> AppResult<Json<LatestSnapshot>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let window = query
        .window
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(MIN_WINDOW_DAYS, MAX_WINDOW_DAYS);

    let activities = fetch_window_activities(&into_runtime(&resources), user_id, tenant_id).await?;
    let physiology = resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await?;
    let ftp_watts = physiology.as_ref().and_then(|p| p.ftp_watts);
    let hr_zones = physiology.as_ref().and_then(|p| p.hr_zones);
    let snapshot = build_latest_snapshot(&activities, window, ftp_watts, hr_zones);
    Ok(Json(snapshot))
}

async fn get_dossier(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<Dossier>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let dossier = resources
        .repos()
        .dossier
        .compose_dossier(tenant_id, user_id)
        .await?;
    Ok(Json(dossier))
}

fn active_tenant(auth: &AuthenticatedUser) -> AppResult<TenantId> {
    auth.active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| {
            AppError::auth_invalid(
                "Endurance endpoints require an active tenant in the JWT — switch tenant first",
            )
        })
}

async fn fetch_window_activities(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Activity>> {
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
    fetch_activities_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        Some(MAX_ACTIVITIES_PER_LATEST_CALL),
    )
    .await
}
