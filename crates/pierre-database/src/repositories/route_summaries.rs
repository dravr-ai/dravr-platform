// ABOUTME: RouteSummaryRepository trait plus the one shared implementation both backends emit (Endurance Phase 3)
// ABOUTME: Caches parsed-GPX terrain + climbs JSON keyed by (tenant_id, user_id, activity_id), guarded by the GPX hash
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde_json::Value;
use uuid::Uuid;

/// CRUD for the `route_summaries` cache table.
///
/// Stores parsed-GPX terrain + climbs JSON keyed by `(tenant_id, user_id,
/// activity_id)` so the route endpoint can skip re-parsing when the
/// underlying GPX hash matches. Cache freshness check is the
/// caller's responsibility — `get_route_summary` returns `None` when the
/// row is missing OR when the supplied `expected_hash` does not match.
///
/// The two JSON blobs are opaque: the repository hands back a compact
/// rendering of what was stored, not the caller's bytes. The column is
/// `jsonb` on Postgres (which re-orders keys) and `TEXT` on `SQLite`, and
/// both engines are written to and read through one [`Value`] bind, so the
/// only reader, `pierre_fitness_compute::routes::route_summary_from_cache`,
/// sees the same content from either.
#[async_trait]
pub trait RouteSummaryRepository: Send + Sync {
    /// Insert or update the cached terrain + climbs JSON for an activity.
    async fn upsert_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        gpx_hash: &str,
        terrain_summary_json: &str,
        climbs_json: &str,
    ) -> AppResult<()>;

    /// Fetch the cached entry. Returns `None` when the row is missing or
    /// when `expected_hash` does not match the stored `gpx_hash`.
    async fn get_route_summary(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activity_id: &str,
        expected_hash: &str,
    ) -> AppResult<Option<(String, String)>>;
}

/// Write or refresh the cached row for one activity.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres. `tenant_id` binds as [`TenantId`] and `user_id` as
/// [`pierre_core::models::UserId`], each of which encodes as hyphenated text
/// on `SQLite` and as a native `uuid` on Postgres; the two JSON columns bind
/// as [`Value`], which sqlx stores as `TEXT` on `SQLite` and `jsonb` on
/// Postgres. `CURRENT_TIMESTAMP` is the spelling both engines accept.
pub(crate) const UPSERT_ROUTE_SUMMARY_SQL: &str = r"
            INSERT INTO route_summaries (
                tenant_id, user_id, activity_id, gpx_hash,
                terrain_summary_json, climbs_json, computed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, CURRENT_TIMESTAMP)
            ON CONFLICT (tenant_id, user_id, activity_id) DO UPDATE SET
                gpx_hash = EXCLUDED.gpx_hash,
                terrain_summary_json = EXCLUDED.terrain_summary_json,
                climbs_json = EXCLUDED.climbs_json,
                computed_at = CURRENT_TIMESTAMP
            ";

/// Read the stored hash and both blobs for one activity.
pub(crate) const GET_ROUTE_SUMMARY_SQL: &str = r"
            SELECT gpx_hash, terrain_summary_json, climbs_json
            FROM route_summaries
            WHERE tenant_id = $1 AND user_id = $2 AND activity_id = $3
            LIMIT 1
            ";

/// The stored hash and both blobs, re-rendered compactly, in column order.
///
/// `try_get` rather than `Row::get` so a corrupt row surfaces as a
/// recoverable error rather than a panic.
///
/// # Errors
/// Returns a database error when a column cannot be decoded or a blob
/// cannot be re-rendered.
pub(crate) fn route_summary_from_row<R>(row: &R) -> AppResult<(String, String, String)>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Value: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let gpx_hash: String = row
        .try_get("gpx_hash")
        .map_err(|e| AppError::database(format!("read gpx_hash: {e}")))?;
    let terrain: Value = row
        .try_get("terrain_summary_json")
        .map_err(|e| AppError::database(format!("read terrain_summary_json: {e}")))?;
    let climbs: Value = row
        .try_get("climbs_json")
        .map_err(|e| AppError::database(format!("read climbs_json: {e}")))?;
    let terrain_text = serde_json::to_string(&terrain)
        .map_err(|e| AppError::database(format!("serialize terrain: {e}")))?;
    let climbs_text = serde_json::to_string(&climbs)
        .map_err(|e| AppError::database(format!("serialize climbs: {e}")))?;
    Ok((gpx_hash, terrain_text, climbs_text))
}

/// Parse a caller's JSON text into the value the column bind takes.
///
/// # Errors
/// Returns a database error naming `column` when the text is not JSON.
pub(crate) fn json_column_value(column: &str, text: &str) -> AppResult<Value> {
    serde_json::from_str(text).map_err(|e| AppError::database(format!("parse {column}: {e}")))
}

/// Emit the whole [`RouteSummaryRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion.
macro_rules! impl_route_summary_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl RouteSummaryRepository for $ty {
            async fn upsert_route_summary(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                activity_id: &str,
                gpx_hash: &str,
                terrain_summary_json: &str,
                climbs_json: &str,
            ) -> AppResult<()> {
                let terrain_value =
                    json_column_value("terrain_summary_json", terrain_summary_json)?;
                let climbs_value = json_column_value("climbs_json", climbs_json)?;
                sqlx::query(UPSERT_ROUTE_SUMMARY_SQL)
                    .bind(tenant_id)
                    .bind(UserId::from_uuid(user_id))
                    .bind(activity_id)
                    .bind(gpx_hash)
                    .bind(terrain_value)
                    .bind(climbs_value)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert_route_summary: {e}")))?;
                Ok(())
            }

            async fn get_route_summary(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                activity_id: &str,
                expected_hash: &str,
            ) -> AppResult<Option<(String, String)>> {
                let row = sqlx::query(GET_ROUTE_SUMMARY_SQL)
                    .bind(tenant_id)
                    .bind(UserId::from_uuid(user_id))
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_route_summary: {e}")))?;
                let Some(row) = row else { return Ok(None) };
                let (stored_hash, terrain, climbs) = route_summary_from_row(&row)?;
                if stored_hash != expected_hash {
                    return Ok(None);
                }
                Ok(Some((terrain, climbs)))
            }
        }
    };
}
pub(crate) use impl_route_summary_repository;
