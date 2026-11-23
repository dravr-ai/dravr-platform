// ABOUTME: /api/device-health — the athlete's phone syncs its own health store (HealthKit, Health Connect) and deletes what it synced
// ABOUTME: Session only, never an admin acting as the athlete; barrage reads the samples, the live store and the wellness projection keep them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Device health routes (owl phase 9, carnet#1548)
//!
//! | Route | Answers |
//! |---|---|
//! | `POST /api/device-health/{source}/sync` | one page of the phone's raw samples (barrage's `DeviceSamples`, at most 4 MiB), read and stored |
//! | `DELETE /api/device-health/{source}/data` | everything synced from the store deleted |
//!
//! `source` is `apple_health` (the iPhone's `HealthKit`) or `health_connect`
//! (Android's Health Connect); any other is `404`. Both routes take the
//! athlete's own session in their active tenant: an API key, a personal MCP
//! token or a delegated grant is `403`, and so is an admin's support session,
//! since the store's data is the athlete's to bring in and to delete.
//!
//! ## The sync
//!
//! The phone maps nothing: dravr-barrage's `device::read` turns the samples
//! into activities and wellness (`400` when the request is not one it reads,
//! as past its sample cap). Then, in this order, every write scoped to the
//! athlete, their tenant and the store's key:
//!
//! 1. the workouts land through the live activity store, a live write, so a
//!    session newer than anything held is announced as for every provider;
//! 2. the workouts the store deleted are erased: no later sync brings them
//!    back;
//! 3. every night overlapping a sleep window barrage read is deleted, and
//!    the series readings (SDNN, glucose) and the recovery and body days a
//!    quantity window covers are deleted, so what the phone re-read replaces
//!    them and what it no longer holds is gone;
//! 4. the wellness of every recorder is merged and filed under one data
//!    source of the key (`source` `"device"`): recovery and body days as
//!    partial summaries, a day's activity totals as the whole day. Two
//!    recorders' readings of one date land on the key's one row for that
//!    date, each a partial summary of it, the later recorder by name
//!    writing last;
//! 5. the self-reports the store deleted are deleted.
//!
//! It answers `200` with the counts:
//!
//! ```json
//! { "activities": 1, "refused": 0, "erased": 1, "nights": 2, "days": 3, "self_reports": 1 }
//! ```
//!
//! - `activities`: workouts held after the write (inserted, updated or
//!   unchanged);
//! - `refused`: samples barrage refused (unreadable, out of range, of the
//!   other store, not happened yet) plus workouts the live store refused
//!   (erased earlier, or starting past a clock's skew);
//! - `erased`: workouts erased and self-reports deleted, of those the store
//!   deleted that were held;
//! - `nights`: nights written;
//! - `days`: recovery, body and activity-total day rows written;
//! - `self_reports`: self-reports written (mood, cycle tracking,
//!   mindfulness, basal body temperature).
//!
//! ## The delete
//!
//! The provider purge removes every row under the key, the activity store's
//! records with them. It answers `200` with what it removed, in the sync's
//! field names where they apply:
//!
//! ```json
//! { "activities": 12, "nights": 30, "days": 61, "self_reports": 4 }
//! ```
//!
//! `days` counts recovery, body and activity-total rows. The phone then turns
//! the lane off and forgets its anchors; turning it on again reads the store
//! from scratch.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::routing::{delete, post};
use axum::{Json, Router};
use dravr_barrage::device::{self, DeviceRead, DeviceSamples};
use dravr_barrage::{BarrageError, LiveSource, Scope, WellnessRecords};
use dravr_tronc::dates::Timestamp;
use pierre_auth::auth::AuthMethod;
use pierre_core::constants::oauth_providers::{APPLE_HEALTH, HEALTH_CONNECT};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{DataSource, DeviceType, TenantId};
use pierre_database::repositories::{ProviderDataPurge, PurgeReason};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::live_activities::WriteOrigin;
use pierre_services::wellness_projection::{self, WellnessFiling, WellnessModes};
use pierre_services::wellness_store::{WellnessStore, WellnessWriteMode};
use pierre_tool_runtime::runtime::SessionLandedSink;
use serde::Serialize;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The largest sync body: a page of at most barrage's `MAX_SAMPLES`
/// samples, with room for long recorder names.
pub const MAX_SYNC_BODY_BYTES: usize = 4 * 1024 * 1024;

/// The data source every record of a store is filed under names this as its
/// `source`: the store itself, whichever app or device recorded the sample.
const DEVICE_SOURCE_MARKER: &str = "device";

/// The athlete's routes.
pub fn device_health_routes() -> Router<Arc<ServerContext>> {
    Router::new()
        .route(
            "/api/device-health/{source}/sync",
            post(sync).layer(DefaultBodyLimit::max(MAX_SYNC_BODY_BYTES)),
        )
        .route("/api/device-health/{source}/data", delete(delete_data))
}

/// The body of `POST /api/device-health/{source}/sync`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
struct SyncCounts {
    /// Workouts held after the write.
    activities: u64,
    /// Samples barrage refused, and workouts the live store refused.
    refused: u64,
    /// Deleted workouts erased and deleted self-reports removed.
    erased: u64,
    /// Nights written.
    nights: u64,
    /// Recovery, body and activity-total day rows written.
    days: u64,
    /// Self-reports written.
    self_reports: u64,
}

/// The body of `DELETE /api/device-health/{source}/data`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
struct DeletedCounts {
    /// Cached activities removed.
    activities: u64,
    /// Nights removed.
    nights: u64,
    /// Recovery, body and activity-total day rows removed.
    days: u64,
    /// Self-reports removed.
    self_reports: u64,
}

impl DeletedCounts {
    fn of(purge: &ProviderDataPurge) -> Self {
        Self {
            activities: purge.removed_from("cached_activities"),
            nights: purge.removed_from("sleep_sessions"),
            days: purge.removed_from("recovery_metrics")
                + purge.removed_from("health_snapshots")
                + purge.removed_from("daily_activity"),
            self_reports: purge.removed_from("self_reports"),
        }
    }
}

/// The athlete a request acts for, and their tenant: their own session, never
/// an API key, an MCP token, a delegated grant or an admin acting as them.
async fn athlete(
    resources: &ServerContext,
    auth: &AuthenticatedUser,
) -> AppResult<(Uuid, TenantId)> {
    if !matches!(auth.auth_method, AuthMethod::JwtToken { .. }) {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "A phone's health data is synced and deleted only from Dravr's own app",
        ));
    }
    auth.refuse_impersonation(
        resources.common.repos.admin_audit.as_ref(),
        "sync or delete the athlete's phone health data",
    )
    .await?;
    let tenant = auth
        .active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| {
            AppError::auth_invalid("Device health requires an active tenant in the JWT")
        })?;
    Ok((auth.user_id, tenant))
}

/// The on-device store `source` names, with its key.
fn store_of(source: &str) -> AppResult<(LiveSource, &'static str)> {
    match source {
        APPLE_HEALTH => Ok((LiveSource::AppleHealth, APPLE_HEALTH)),
        HEALTH_CONNECT => Ok((LiveSource::HealthConnect, HEALTH_CONNECT)),
        _ => Err(AppError::not_found(format!(
            "No on-device health store is named {source}"
        ))),
    }
}

/// Barrage's refusal of a request as the platform's: what it cannot read is
/// the request's fault.
fn barrage_error(error: BarrageError) -> AppError {
    match error {
        BarrageError::Invalid { what, why } => AppError::invalid_input(format!("{what}: {why}")),
        other => AppError::internal(format!("device read: {other}")),
    }
}

/// The one data source every record of `key` is filed under, for the athlete.
fn key_data_source(key: &str, user_id: Uuid) -> DataSource {
    DataSource {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        provider: key.to_owned(),
        device_model: None,
        software_version: None,
        source: Some(DEVICE_SOURCE_MARKER.to_owned()),
        device_type: DeviceType::Phone,
        original_source_name: None,
    }
}

/// Delete what the windows barrage read cover, before their re-read records
/// are written: the key's nights overlapping a sleep window, and its
/// readings a quantity window re-read.
async fn clear_windows(
    store: &WellnessStore<'_>,
    (user_id, tenant): (Uuid, &TenantId),
    key: &str,
    read: &DeviceRead,
) -> AppResult<()> {
    for window in &read.sleep_windows {
        store
            .delete_sleep_window(tenant, user_id, key, (window.from, window.to))
            .await?;
    }
    let source = key_data_source(key, user_id);
    for window in &read.quantity_windows {
        store
            .delete_quantity_window(tenant, &source, window)
            .await?;
    }
    Ok(())
}

/// Every recorder's wellness as one batch: the key files them under one data
/// source, as the export of the same store files its own.
fn merged(wellness: Vec<(device::Recorder, WellnessRecords)>) -> WellnessRecords {
    let mut all = WellnessRecords::default();
    for (_, records) in wellness {
        all.extend(records);
    }
    all
}

/// `POST /api/device-health/{source}/sync`.
async fn sync(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(source): Path<String>,
    body: Bytes,
) -> AppResult<Json<SyncCounts>> {
    let (user_id, tenant) = athlete(&resources, &auth).await?;
    let (live, key) = store_of(&source)?;
    let samples: DeviceSamples = serde_json::from_slice(&body)
        .map_err(|e| AppError::invalid_input(format!("device samples: {e}")))?;
    let now = Timestamp::now();
    let read = device::read(&Scope::new(tenant.as_uuid(), user_id), live, &samples, now)
        .map_err(barrage_error)?;
    let mut counts = SyncCounts {
        refused: read.refused,
        ..SyncCounts::default()
    };

    let live_store = &resources.common.live_activities;
    if !read.activities.is_empty() {
        let ingest = live_store
            .ingest(
                user_id,
                tenant,
                key,
                &read.activities,
                now,
                WriteOrigin::Live,
            )
            .await?;
        counts.activities = ingest.stored;
        counts.refused += ingest.refused;
        if let Some(session) = ingest.landed {
            let sink: Arc<dyn SessionLandedSink> = resources.mcp.session_landed.clone();
            tokio::spawn(async move {
                sink.session_landed(user_id, tenant, session).await;
            });
        }
    }
    for workout in &read.deleted.workouts {
        if live_store
            .erase(user_id, tenant, key, workout.as_str())
            .await?
        {
            counts.erased += 1;
        }
    }

    let repos = &resources.common.repos;
    let store = WellnessStore::new(repos);
    clear_windows(&store, (user_id, &tenant), key, &read).await?;
    let projected = wellness_projection::project(
        repos,
        WellnessFiling {
            tenant_id: tenant,
            user_id,
            provider: key,
            data_source: key_data_source(key, user_id),
            modes: WellnessModes {
                recovery: WellnessWriteMode::PartialSummary,
                body: WellnessWriteMode::PartialSummary,
                daily_activity: WellnessWriteMode::FullDay,
            },
            label: "device health sync",
        },
        &merged(read.wellness),
        now,
    )
    .await?;
    counts.nights = projected.nights as u64;
    counts.days = (projected.recovery + projected.body + projected.daily_activity) as u64;
    counts.self_reports = projected.sensitive.self_reports;

    counts.erased += repos
        .health_records
        .delete_self_reports(&tenant, user_id, key, &read.deleted.self_reports)
        .await?;
    Ok(Json(counts))
}

/// `DELETE /api/device-health/{source}/data`.
async fn delete_data(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(source): Path<String>,
) -> AppResult<Json<DeletedCounts>> {
    let (user_id, tenant) = athlete(&resources, &auth).await?;
    let (_, key) = store_of(&source)?;
    let purge = resources
        .common
        .provider_purge
        .purge_user_provider_data(user_id, &tenant, key, PurgeReason::AthleteDisconnect)
        .await?;
    Ok(Json(DeletedCounts::of(&purge)))
}
