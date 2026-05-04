// ABOUTME: Endurance Phase 2 service — fetch activities + physiology, run intelligence compute, persist + invalidate cache
// ABOUTME: Wraps pierre_intelligence::training_history_compute with the ServerContext / repos / cache plumbing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use chrono::{Duration, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{DailyTrainingState, TenantId};
use pierre_intelligence::training_history_compute::{
    compute_training_history, AthleteInputs, MAX_BACKFILL_DAYS,
};
use uuid::Uuid;

use crate::config::environment::default_provider;
use crate::mcp::resources::ServerContext;
use crate::routes::social::SocialRoutes;

/// Default backfill window when the caller does not specify one.
pub const DEFAULT_BACKFILL_DAYS: i64 = 90;

/// Activities to pull when running a backfill — bounded to keep the
/// provider hit predictable.
pub const MAX_ACTIVITIES_FOR_BACKFILL: usize = 500;

/// Compute and persist daily training-history rows for `[from, to]`.
///
/// Pulls the user's activity stream + physiology, hands them to
/// [`pierre_intelligence::training_history_compute`], then upserts the
/// resulting rows via `RepositoryRegistry::training_history`.
///
/// Returns the upserted row count for telemetry / tests.
///
/// # Errors
///
/// Returns [`AppError`] when the provider fetch fails or the repository
/// upsert fails. The compute step is infallible.
pub async fn compute_and_persist_history(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<usize> {
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    if (to - from).num_days() > MAX_BACKFILL_DAYS {
        return Err(AppError::invalid_input(format!(
            "backfill window exceeds the maximum of {MAX_BACKFILL_DAYS} days"
        )));
    }

    let provider_name = default_provider();
    let tenant_str = tenant_id.to_string();
    let activities = SocialRoutes::fetch_activities_from_provider(
        resources,
        user_id,
        &provider_name,
        Some(tenant_str.as_str()),
        Some(MAX_ACTIVITIES_FOR_BACKFILL),
    )
    .await?;

    let physiology = resources
        .repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await?;
    let inputs = physiology
        .as_ref()
        .map_or_else(AthleteInputs::default, |p| AthleteInputs {
            ftp_watts: p.ftp_watts.map(f64::from),
            lthr: p
                .lactate_threshold_percentage
                .and_then(|pct| p.max_hr.map(|mhr| f64::from(mhr) * pct)),
            max_hr: p.max_hr.map(f64::from),
            resting_hr: p.resting_hr.map(f64::from),
            weight_kg: p.weight,
        });

    let states = compute_training_history(&activities, inputs, from, to);
    if states.is_empty() {
        return Ok(0);
    }
    resources
        .repos
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, &states)
        .await?;
    Ok(states.len())
}

/// Backfill the default 90-day window relative to today.
///
/// Convenience for the on-demand MCP tool and the post-sync hook.
///
/// # Errors
///
/// Same conditions as [`compute_and_persist_history`].
pub async fn compute_default_window(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<usize> {
    let to = Utc::now().date_naive();
    let from = to - Duration::days(DEFAULT_BACKFILL_DAYS);
    compute_and_persist_history(resources, tenant_id, user_id, from, to).await
}

/// Read-only fetch of persisted rows in `[from, to]`.
///
/// # Errors
///
/// Returns [`AppError`] from the repository when the read fails.
pub async fn fetch_history_rows(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<DailyTrainingState>> {
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    resources
        .repos
        .training_history
        .get_training_history(tenant_id, user_id, from, to)
        .await
}
