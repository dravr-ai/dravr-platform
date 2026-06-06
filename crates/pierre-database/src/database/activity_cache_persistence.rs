// ABOUTME: SQLite implementation of ActivityCacheRepository (provider-agnostic activity cache)
// ABOUTME: Stores queryable columns + full serialized Activity in data_json for stale-while-revalidate reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::ActivityCacheRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use sqlx::Row;
use uuid::Uuid;

/// Best-effort string form of an activity's sport type for the indexed column.
/// `SportType` serializes as a JSON string; on the off chance it doesn't, the
/// column stays `NULL` and the canonical value remains in `data_json`.
fn sport_type_string(activity: &Activity) -> Option<String> {
    serde_json::to_value(activity.sport_type())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
}

#[async_trait]
impl ActivityCacheRepository for Database {
    async fn upsert_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<u64> {
        let user_id_str = user_id.to_string();
        let tenant_str = tenant_id.to_string();
        let now = Utc::now().to_rfc3339();
        let mut written = 0u64;

        for activity in activities {
            let id = Uuid::new_v4().to_string();
            let start_date = activity.start_date().to_rfc3339();
            let sport = sport_type_string(activity);
            let data_json = serde_json::to_string(activity)
                .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))?;

            sqlx::query(
                r"
                INSERT INTO cached_activities (id, user_id, tenant_id, provider, activity_id, sport_type, start_date, synced_at, data_json)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(user_id, tenant_id, provider, activity_id) DO UPDATE SET
                    sport_type = excluded.sport_type,
                    start_date = excluded.start_date,
                    synced_at = excluded.synced_at,
                    data_json = excluded.data_json
                ",
            )
            .bind(&id)
            .bind(&user_id_str)
            .bind(&tenant_str)
            .bind(provider)
            .bind(activity.id())
            .bind(&sport)
            .bind(&start_date)
            .bind(&now)
            .bind(&data_json)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to upsert activity: {e}")))?;

            written += 1;
        }

        Ok(written)
    }

    async fn get_cached_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: Option<&str>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<Activity>> {
        let user_id_str = user_id.to_string();
        let start_str = start.to_rfc3339();
        let end_str = end.to_rfc3339();

        let rows = sqlx::query(
            r"
            SELECT data_json
            FROM cached_activities
            WHERE user_id = ? AND tenant_id = ? AND start_date >= ? AND start_date <= ?
              AND (? IS NULL OR provider = ?)
            ORDER BY start_date DESC
            LIMIT ?
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(&start_str)
        .bind(&end_str)
        .bind(provider)
        .bind(provider)
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get cached activities: {e}")))?;

        rows.into_iter()
            .map(|r| {
                let data_json: String = r.get("data_json");
                serde_json::from_str::<Activity>(&data_json).map_err(|e| {
                    AppError::database(format!("Failed to deserialize cached activity: {e}"))
                })
            })
            .collect()
    }

    async fn latest_activity_sync(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>> {
        let user_id_str = user_id.to_string();

        let row = sqlx::query(
            r"
            SELECT synced_at
            FROM cached_activities
            WHERE user_id = ? AND tenant_id = ? AND provider = ?
            ORDER BY synced_at DESC
            LIMIT 1
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(provider)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to read activity sync time: {e}")))?;

        row.map(|r| {
            let synced_at: String = r.get("synced_at");
            DateTime::parse_from_rfc3339(&synced_at)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| AppError::database(format!("Invalid synced_at timestamp: {e}")))
        })
        .transpose()
    }

    async fn prune_activities_before(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        cutoff: DateTime<Utc>,
    ) -> AppResult<u64> {
        let user_id_str = user_id.to_string();
        let cutoff_str = cutoff.to_rfc3339();

        let result = sqlx::query(
            r"
            DELETE FROM cached_activities
            WHERE user_id = ? AND tenant_id = ? AND start_date < ?
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(&cutoff_str)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to prune cached activities: {e}")))?;

        Ok(result.rows_affected())
    }
}
