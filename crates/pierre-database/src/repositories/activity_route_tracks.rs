// ABOUTME: ActivityRouteTrackRepository trait plus the one shared implementation both backends emit
// ABOUTME: One activity's stored route — a drawn track's JSON or the reason there is none — per (tenant, user, provider, activity)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stored route tracks.
//!
//! A completed activity's route does not change, so it is read from the
//! provider once and kept in `activity_route_tracks`. The track itself is
//! opaque JSON here: its shape is `pierre_fitness_compute`'s `RouteTrack`,
//! which this crate cannot name (that crate depends on this one), the same
//! split `route_summaries` makes for its terrain blobs. What this crate does
//! own is the row's two outcomes, which the table's CHECK constraint mirrors:
//! a track, or the reason there is none — never both, never neither.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres. Every id column is `TEXT` on both engines, like
//! `cached_activities`, so the ids bind as hyphenated text and the
//! provider-disconnect purge reaches the rows with the same statement it runs
//! over that table. `created_at` binds as `DateTime<Utc>`, which sqlx stores
//! as RFC 3339 text on `SQLite` and as `TIMESTAMPTZ` on Postgres.

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// One activity's stored route read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredRouteTrack {
    /// A drawable track.
    Drawn {
        /// Where the geometry came from (`summary_polyline` or `streams`).
        source: String,
        /// The trimmed, simplified track as JSON.
        track_json: String,
    },
    /// No drawable track, and why.
    Unavailable {
        /// The read that established it (`summary_polyline` or `streams`).
        source: String,
        /// Why there is no track (`no_gps` or `too_short`).
        reason: String,
    },
}

/// Persistence for the one route read each activity costs.
#[async_trait]
pub trait ActivityRouteTrackRepository: Send + Sync {
    /// The stored route read for one activity, or `None` when it has not been
    /// read yet.
    ///
    /// # Errors
    /// Returns a database error when the read fails or the row holds neither
    /// outcome (a row the table's constraint should have refused).
    async fn get_route_track(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<Option<StoredRouteTrack>>;

    /// Store the route read for one activity, replacing any earlier read.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn upsert_route_track(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
        track: &StoredRouteTrack,
    ) -> AppResult<()>;
}

/// Write or replace one activity's route read.
pub(crate) const UPSERT_ROUTE_TRACK_SQL: &str = r"
    INSERT INTO activity_route_tracks (
        tenant_id, user_id, provider, activity_id, source, track_json, unavailable_reason, created_at
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
    ON CONFLICT (tenant_id, user_id, provider, activity_id) DO UPDATE SET
        source = EXCLUDED.source,
        track_json = EXCLUDED.track_json,
        unavailable_reason = EXCLUDED.unavailable_reason,
        created_at = EXCLUDED.created_at";

/// Read one activity's route, scoped by tenant, user and provider.
pub(crate) const GET_ROUTE_TRACK_SQL: &str = r"
    SELECT source, track_json, unavailable_reason
    FROM activity_route_tracks
    WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND activity_id = $4";

/// The stored outcome in one row of either backend.
///
/// `try_get` rather than `Row::get` so a corrupt row surfaces as a
/// recoverable error rather than a panic.
///
/// # Errors
/// Returns a database error when a column cannot be decoded or the row holds
/// neither a track nor a reason.
pub(crate) fn route_track_from_row<R>(row: &R) -> AppResult<StoredRouteTrack>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let source: String = row
        .try_get("source")
        .map_err(|e| AppError::database(format!("read route track source: {e}")))?;
    let track_json: Option<String> = row
        .try_get("track_json")
        .map_err(|e| AppError::database(format!("read route track json: {e}")))?;
    let reason: Option<String> = row
        .try_get("unavailable_reason")
        .map_err(|e| AppError::database(format!("read route track reason: {e}")))?;
    match (track_json, reason) {
        (Some(track_json), None) => Ok(StoredRouteTrack::Drawn { source, track_json }),
        (None, Some(reason)) => Ok(StoredRouteTrack::Unavailable { source, reason }),
        _ => Err(AppError::database(
            "stored route track holds neither exactly one track nor one reason",
        )),
    }
}

/// The `(source, track_json, unavailable_reason)` columns one read writes.
pub(crate) fn route_track_columns(track: &StoredRouteTrack) -> (&str, Option<&str>, Option<&str>) {
    match track {
        StoredRouteTrack::Drawn { source, track_json } => {
            (source.as_str(), Some(track_json.as_str()), None)
        }
        StoredRouteTrack::Unavailable { source, reason } => {
            (source.as_str(), None, Some(reason.as_str()))
        }
    }
}

/// Emit the whole [`ActivityRouteTrackRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion. The body names its consts, helpers and types unqualified, so
/// the invoking shell must `use` every one of them.
macro_rules! impl_activity_route_track_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl ActivityRouteTrackRepository for $ty {
            async fn get_route_track(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<Option<StoredRouteTrack>> {
                let row = sqlx::query(GET_ROUTE_TRACK_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_route_track: {e}")))?;
                row.as_ref().map(route_track_from_row).transpose()
            }

            async fn upsert_route_track(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
                track: &StoredRouteTrack,
            ) -> AppResult<()> {
                let (source, track_json, reason) = route_track_columns(track);
                sqlx::query(UPSERT_ROUTE_TRACK_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .bind(source)
                    .bind(track_json)
                    .bind(reason)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert_route_track: {e}")))?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_activity_route_track_repository;
