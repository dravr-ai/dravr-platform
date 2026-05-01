// ABOUTME: Endurance Phase 1 read-side endpoints — GET /api/v1/endurance/{latest,dossier}
// ABOUTME: Auth: JWT-bearer, scoped by tenant_id from the active session; multi-tenant isolation enforced
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, DailyTrainingState, Dossier, PrescribedWorkout, TenantId, WorkoutTemplate,
};
use pierre_intelligence::intervals::{build_intervals, IntervalsExport};
use pierre_intelligence::latest_snapshot::{
    build_latest_snapshot, LatestSnapshot, DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS,
};
use pierre_intelligence::routes::{build_route_summary_from_streams, RouteSummary};
use pierre_intelligence::training_history_compute::MAX_BACKFILL_DAYS;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::environment::default_provider;
use crate::mcp::resources::ServerResources;
use crate::middleware::extractors::AuthenticatedUser;
use crate::routes::social::SocialRoutes;
use crate::services::training_history_compute::{
    compute_and_persist_history, fetch_history_rows, DEFAULT_BACKFILL_DAYS,
};
use crate::services::workout_library::{cornerstone_templates, require_cornerstone};

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
pub fn endurance_routes() -> Router<Arc<ServerResources>> {
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
        .route("/api/v1/endurance/prescribe", post(post_prescribe))
}

/// Request body for `POST /api/v1/endurance/prescribe`.
#[derive(Debug, Deserialize)]
pub struct PrescribeRequest {
    /// Slug of the cornerstone template to prescribe (one of the six).
    pub template_slug: String,
    /// ISO date the prescription is scheduled for (`YYYY-MM-DD`).
    pub date: NaiveDate,
    /// Optional coach id stamped onto the audit row.
    #[serde(default)]
    pub coach_id: Option<String>,
}

/// Response body for `POST /api/v1/endurance/prescribe`.
#[derive(Debug, Serialize)]
pub struct PrescribeResponse {
    /// Audit row id stored in `prescribed_workouts`.
    pub prescription_id: Uuid,
    /// The provider the workout was pushed to (currently `intervals_icu`).
    pub provider: String,
    /// Provider-side event id when the push call returned one (may be `None`
    /// when the provider call is stubbed).
    pub provider_event_id: Option<String>,
    /// The compiled-in template that was scheduled.
    pub template: WorkoutTemplate,
}

async fn get_workout_templates(
    State(resources): State<Arc<ServerResources>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<Vec<WorkoutTemplate>>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    // Compiled-in cornerstones first (read-only TOML library), then any
    // user-authored overrides for (tenant_id, user_id) ordered newest-first.
    let mut templates = cornerstone_templates();
    let user_authored = resources
        .repos
        .workout_templates
        .list_user_workout_templates(tenant_id, user_id)
        .await?;
    templates.extend(user_authored);
    Ok(Json(templates))
}

async fn post_prescribe(
    State(resources): State<Arc<ServerResources>>,
    auth: AuthenticatedUser,
    Json(body): Json<PrescribeRequest>,
) -> AppResult<Json<PrescribeResponse>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let template = require_cornerstone(&body.template_slug)?;

    let payload_json = serde_json::to_string(&template)
        .map_err(|e| AppError::internal(format!("serialize template payload: {e}")))?;

    // Phase 5 push surface: the Intervals.icu provider is wired but the
    // calendar-event POST is added in a follow-on commit. For now we
    // record the audit row and surface `provider_event_id = None` so the
    // coach knows the prescription is queued but not yet pushed live.
    let provider = "intervals_icu".to_owned();
    let prescription_id = Uuid::new_v4();
    let prescribed = PrescribedWorkout {
        id: prescription_id,
        tenant_id: tenant_id.as_uuid(),
        user_id,
        coach_id: body.coach_id,
        template_slug: template.slug.clone(),
        sport: template.sport.clone(),
        prescribed_for_date: body.date,
        provider: provider.clone(),
        provider_event_id: None,
        payload_json,
        status: "queued".to_owned(),
        created_at: Utc::now(),
    };
    resources
        .repos
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await?;

    Ok(Json(PrescribeResponse {
        prescription_id,
        provider,
        provider_event_id: None,
        template,
    }))
}

async fn get_intervals(
    State(resources): State<Arc<ServerResources>>,
    auth: AuthenticatedUser,
    Path(activity_id): Path<String>,
) -> AppResult<Json<IntervalsExport>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let activity = fetch_activity_by_id(&resources, user_id, tenant_id, &activity_id).await?;
    let physiology = resources
        .repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await?;
    let ftp_watts = physiology.as_ref().and_then(|p| p.ftp_watts);
    Ok(Json(build_intervals(&activity, ftp_watts)))
}

async fn get_routes(
    State(resources): State<Arc<ServerResources>>,
    auth: AuthenticatedUser,
    Path(activity_id): Path<String>,
) -> AppResult<Json<RouteSummary>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let activity = fetch_activity_by_id(&resources, user_id, tenant_id, &activity_id).await?;
    let stream = activity
        .time_series_data()
        .ok_or_else(|| AppError::not_found("activity has no GPS stream — terrain unavailable"))?;
    let coords = stream.gps_coordinates.as_ref().ok_or_else(|| {
        AppError::not_found("activity stream has no gps_coordinates — terrain unavailable")
    })?;
    let altitudes = stream.altitude.as_ref().ok_or_else(|| {
        AppError::not_found("activity stream has no altitude — terrain unavailable")
    })?;
    // `build_route_summary_from_streams` returns `None` for any of:
    // fewer than 2 paired points, every point filtered out by the lat/lon/
    // finite gate, or a produced summary that contains a non-finite metric.
    // All three are bad-input conditions for the user — surface as 400, not
    // 404 — and crucially we do NOT write the cache row when this happens
    // so a malformed stream can't poison subsequent reads on the same
    // activity (Phase 3 risk #3).
    let summary = build_route_summary_from_streams(coords, altitudes).ok_or_else(|| {
        AppError::invalid_input(
            "activity stream produced no valid GPS+altitude points (fewer than 2 paired, \
             out-of-range lat/lon, or non-finite values)",
        )
    })?;

    // Cache write: terrain + climbs JSON keyed by gpx_hash.
    let terrain_json = serde_json::to_string(&summary.terrain)
        .map_err(|e| AppError::internal(format!("serialize terrain_summary: {e}")))?;
    let climbs_json = serde_json::to_string(&summary.climbs)
        .map_err(|e| AppError::internal(format!("serialize climbs: {e}")))?;
    resources
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
    Ok(Json(summary))
}

async fn fetch_activity_by_id(
    resources: &Arc<ServerResources>,
    user_id: Uuid,
    tenant_id: TenantId,
    activity_id: &str,
) -> AppResult<Activity> {
    // The activity-fetch helper is per-list in the social routes; for a
    // single id we pull a wide window and filter. Acceptable cost given
    // Endurance endpoints are coach-driven, low-cardinality reads.
    let provider_name = default_provider();
    let tenant_str = tenant_id.to_string();
    let activities = SocialRoutes::fetch_activities_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        Some(MAX_ACTIVITIES_PER_LATEST_CALL),
    )
    .await?;
    activities
        .into_iter()
        .find(|a| a.id() == activity_id)
        .ok_or_else(|| AppError::not_found(format!("activity {activity_id} not in recent window")))
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
    State(resources): State<Arc<ServerResources>>,
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

    let mut rows = fetch_history_rows(&resources, tenant_id, user_id, from, to).await?;
    if rows.is_empty() {
        // Cold start: no rows persisted yet for this user. Trigger an
        // on-demand compute for the same window so the next call hits
        // the persisted rollup.
        let _ = compute_and_persist_history(&resources, tenant_id, user_id, from, to).await?;
        rows = fetch_history_rows(&resources, tenant_id, user_id, from, to).await?;
    }
    Ok(Json(HistoryResponse {
        from,
        to,
        days: rows,
    }))
}

async fn get_latest_snapshot(
    State(resources): State<Arc<ServerResources>>,
    auth: AuthenticatedUser,
    Query(query): Query<LatestQuery>,
) -> AppResult<Json<LatestSnapshot>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let window = query
        .window
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(MIN_WINDOW_DAYS, MAX_WINDOW_DAYS);

    let activities = fetch_window_activities(&resources, user_id, tenant_id).await?;
    let physiology = resources
        .repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await?;
    let ftp_watts = physiology.as_ref().and_then(|p| p.ftp_watts);
    let hr_zones = physiology.as_ref().and_then(|p| p.hr_zones);
    let snapshot = build_latest_snapshot(&activities, window, ftp_watts, hr_zones);
    Ok(Json(snapshot))
}

async fn get_dossier(
    State(resources): State<Arc<ServerResources>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<Dossier>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let dossier = resources
        .repos
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
    resources: &Arc<ServerResources>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Activity>> {
    let provider_name = default_provider();
    let tenant_str = tenant_id.to_string();
    SocialRoutes::fetch_activities_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        Some(MAX_ACTIVITIES_PER_LATEST_CALL),
    )
    .await
}
