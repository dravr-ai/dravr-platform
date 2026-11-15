// ABOUTME: The activity store's projection into cached_activities: the cache's one writer, and its live-source sink
// ABOUTME: A live read's rows land at their sync stamp; a retraction releases gear, drops rows and samples, settles expiry

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Serving what the activity store keeps
//!
//! dravr-barrage keeps every provider's activities; the platform serves them
//! from `cached_activities`, which every reader reads. This module is the one
//! place that table is written
//! ([`project_cached_activities`](crate::activity_cache_sink::project_cached_activities)),
//! and the sink that projects a live source's records there
//! ([`LiveCacheSink`](crate::activity_cache_sink::LiveCacheSink)): the activity
//! cache only, each read's rows at the sync stamp the read gave them.
//!
//! The retraction of named records
//! ([`retract_activities`](crate::activity_cache_sink::retract_activities)) is
//! shared by every source: each one's gear attribution is released, its cached
//! row and samples deleted, and an expiry settled: routes, sync state, attestation. An imported archive's
//! projection — samples, best efforts and gear beside the rows — needs the
//! server, and lives there
//! (`pierre_mcp_server::services::archive_import::sink`); it writes its rows
//! and retracts its records through this module.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dravr_barrage::cageux::models::TimeSeriesData;
use dravr_barrage::{
    ActivityId, ActivityRecord, BarrageError, LiveSource, ProjectionSink, RetractReason, Scope,
    SourceKey, WellnessRecords,
};
use pierre_core::errors::AppError;
use pierre_core::models::{Activity, TenantId};
use pierre_database::RepositoryRegistry;
use tracing::{info, warn};
use uuid::Uuid;

use crate::gear_release::release_activity_gear;

/// The platform's ids for `scope`.
#[must_use]
pub fn scope_ids(scope: &Scope) -> (TenantId, Uuid) {
    (TenantId::from_uuid(scope.tenant_id.0), scope.user_id.0)
}

/// A platform error as the projection error barrage retries the step on.
#[must_use]
pub fn projection_error(error: &AppError) -> BarrageError {
    BarrageError::Projection(error.to_string())
}

/// Write `activities` of `source` into the activity cache, each row stamped
/// `synced_at`: the one write of `cached_activities` the activity store's
/// projections make, live or archive.
///
/// # Errors
///
/// A projection error when the cache write fails.
pub async fn project_cached_activities(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    source: SourceKey,
    activities: &[Activity],
    synced_at: DateTime<Utc>,
) -> Result<(), BarrageError> {
    repos
        .activity_cache
        .upsert_activities_synced_at(user_id, &tenant_id, source.as_str(), activities, synced_at)
        .await
        .map_err(|e| projection_error(&e))?;
    Ok(())
}

/// Stop serving the records `activity_ids` of `source`: gear released, rows
/// and samples deleted.
///
/// When retention expired them, their routes go too, the provider's activity
/// sync state is reset and the purge attested.
///
/// An athlete retries the deletion of an archive or of a file they uploaded
/// when it failed, so its gear release stops it (`athlete_retries`). A
/// provider's deletion arrives once, from a webhook: its row goes even when
/// the release fails, since a row kept past its deletion costs every refresh
/// of its window, where a kept attribution costs only the pair's total.
///
/// # Errors
///
/// A projection error when a row or its samples cannot be deleted, the
/// expiry cannot be attested, or an archive record's gear cannot be released.
pub async fn retract_activities(
    repos: &RepositoryRegistry,
    scope: &Scope,
    source: SourceKey,
    activity_ids: &[ActivityId],
    reason: RetractReason,
) -> Result<(), BarrageError> {
    let (tenant_id, user_id) = scope_ids(scope);
    let key = source.as_str();
    for id in activity_ids {
        let released = release_activity_gear(repos, user_id, &tenant_id, key, id.as_str()).await;
        match released {
            Ok(()) => {}
            Err(e) if athlete_retries(source) => return Err(projection_error(&e)),
            Err(e) => {
                warn!(%user_id, source = key, activity_id = id.as_str(), error = %e, "gear attribution not released; the activity goes anyway");
            }
        }
        repos
            .activity_cache
            .delete_cached_activity(user_id, &tenant_id, key, id.as_str())
            .await
            .map_err(|e| projection_error(&e))?;
        repos
            .activity_streams
            .delete_activity_streams(&tenant_id, user_id, key, id.as_str())
            .await
            .map_err(|e| projection_error(&e))?;
    }
    // Retention expired these: their routes go, the sync state that called
    // their window fresh and backfilled is reset so the next read fetches it
    // again, and the attestation says so, as the provider cache sweep's did.
    if reason == RetractReason::Expired {
        let ids: Vec<&str> = activity_ids.iter().map(ActivityId::as_str).collect();
        repos
            .provider_data
            .expire_provider_activities(user_id, &tenant_id, key, &ids)
            .await
            .map_err(|e| projection_error(&e))?;
    }
    Ok(())
}

/// Whether a deletion of `source`'s records is the athlete's own, which they
/// retry when it fails: an imported archive's, or a file they uploaded.
fn athlete_retries(source: SourceKey) -> bool {
    source.is_archive() || source == SourceKey::Live(LiveSource::Upload)
}

/// The refusal a live sink answers an archive's chunk with.
fn archive_refused() -> BarrageError {
    BarrageError::Projection("an archive projects through the server's sink".to_owned())
}

/// Projects a live source's records into the activity cache.
///
/// It holds the repositories alone: every live read, push and upload is kept
/// through it, and the retention sweep retracts expired records through it.
pub struct LiveCacheSink {
    repos: Arc<RepositoryRegistry>,
}

impl fmt::Debug for LiveCacheSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveCacheSink").finish_non_exhaustive()
    }
}

impl LiveCacheSink {
    /// A live sink over `repos`' activity cache.
    #[must_use]
    pub const fn new(repos: Arc<RepositoryRegistry>) -> Self {
        Self { repos }
    }
}

#[async_trait]
impl ProjectionSink for LiveCacheSink {
    /// The activity cache only, each read's rows at the sync stamp the read
    /// gave them (`provenance.ingested_at`).
    async fn project(
        &self,
        scope: &Scope,
        source: SourceKey,
        rows: &[ActivityRecord],
        _series: &[Option<TimeSeriesData>],
    ) -> Result<(), BarrageError> {
        if rows.is_empty() {
            return Ok(());
        }
        if source.is_archive() {
            return Err(archive_refused());
        }
        let (tenant_id, user_id) = scope_ids(scope);
        let synced_at = rows
            .iter()
            .map(|row| row.provenance.ingested_at)
            .max()
            .unwrap_or_else(Utc::now);
        let activities: Vec<Activity> = rows.iter().map(|row| row.activity.clone()).collect();
        project_cached_activities(
            &self.repos,
            tenant_id,
            user_id,
            source,
            &activities,
            synced_at,
        )
        .await
    }

    /// A live read carries no wellness: only an archive's does.
    async fn project_wellness(
        &self,
        _scope: &Scope,
        _source: SourceKey,
        wellness: &WellnessRecords,
    ) -> Result<(), BarrageError> {
        if wellness.is_empty() {
            return Ok(());
        }
        Err(archive_refused())
    }

    async fn retract(
        &self,
        scope: &Scope,
        source: SourceKey,
        ids_to_retract: Option<&[ActivityId]>,
        reason: RetractReason,
    ) -> Result<(), BarrageError> {
        match ids_to_retract {
            Some(activity_ids) => {
                retract_activities(&self.repos, scope, source, activity_ids, reason).await
            }
            // A live source is deleted by a provider purge
            // (`crate::provider_purge`), which deletes and attests every row
            // the provider contributed, for the reason it knows, before the
            // store deletes its records. The projection stops serving the
            // source's activities, whatever that purge left; it attests
            // nothing, so the purge's row is the only one.
            None if !source.is_archive() => {
                let (tenant_id, user_id) = scope_ids(scope);
                let key = source.as_str();
                let removed = self
                    .repos
                    .activity_cache
                    .delete_provider_activities(user_id, &tenant_id, key)
                    .await
                    .map_err(|e| projection_error(&e))?;
                info!(%user_id, source = key, removed, "live source retracted");
                Ok(())
            }
            None => Err(archive_refused()),
        }
    }
}
