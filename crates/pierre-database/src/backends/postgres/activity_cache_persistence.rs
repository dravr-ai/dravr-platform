// ABOUTME: PostgreSQL implementation of ActivityCacheRepository (provider-agnostic activity cache)
// ABOUTME: Mirrors the SQLite impl with PG-native TIMESTAMPTZ binds and $N placeholders
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::PostgresDatabase;
use crate::repositories::ActivityCacheRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use sqlx::Row;
use uuid::Uuid;

/// Best-effort string form of an activity's sport type for the indexed column.
fn sport_type_string(activity: &Activity) -> Option<String> {
    serde_json::to_value(activity.sport_type())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
}

#[async_trait]
impl ActivityCacheRepository for PostgresDatabase {
    async fn upsert_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<u64> {
        let user_id_str = user_id.to_string();
        let tenant_str = tenant_id.to_string();
        let now = Utc::now();
        let mut written = 0u64;

        for activity in activities {
            let id = Uuid::new_v4().to_string();
            let sport = sport_type_string(activity);
            let data_json = serde_json::to_string(activity)
                .map_err(|e| AppError::database(format!("Failed to serialize activity: {e}")))?;

            sqlx::query(
                r"
                INSERT INTO cached_activities (id, user_id, tenant_id, provider, activity_id, sport_type, start_date, synced_at, data_json)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT(user_id, tenant_id, provider, activity_id) DO UPDATE SET
                    sport_type = EXCLUDED.sport_type,
                    start_date = EXCLUDED.start_date,
                    synced_at = EXCLUDED.synced_at,
                    data_json = EXCLUDED.data_json
                ",
            )
            .bind(&id)
            .bind(&user_id_str)
            .bind(&tenant_str)
            .bind(provider)
            .bind(activity.id())
            .bind(&sport)
            .bind(activity.start_date())
            .bind(now)
            .bind(&data_json)
            .execute(&self.pool)
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

        let rows = sqlx::query(
            r"
            SELECT data_json
            FROM cached_activities
            WHERE user_id = $1 AND tenant_id = $2 AND start_date >= $3 AND start_date <= $4
              AND ($5::text IS NULL OR provider = $5)
            ORDER BY start_date DESC
            LIMIT $6
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(start)
        .bind(end)
        .bind(provider)
        .bind(limit)
        .fetch_all(&self.pool)
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
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ORDER BY synced_at DESC
            LIMIT 1
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to read activity sync time: {e}")))?;

        Ok(row.map(|r| r.get::<DateTime<Utc>, _>("synced_at")))
    }

    async fn prune_activities_before(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        cutoff: DateTime<Utc>,
    ) -> AppResult<u64> {
        let user_id_str = user_id.to_string();

        let result = sqlx::query(
            r"
            DELETE FROM cached_activities
            WHERE user_id = $1 AND tenant_id = $2 AND start_date < $3
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(cutoff)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to prune cached activities: {e}")))?;

        Ok(result.rows_affected())
    }
}
