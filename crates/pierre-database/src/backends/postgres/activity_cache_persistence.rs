// ABOUTME: PostgreSQL implementation of ActivityCacheRepository (provider-agnostic activity cache)
// ABOUTME: Mirrors the SQLite impl with PG-native TIMESTAMPTZ binds and $N placeholders
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::PostgresDatabase;
use crate::repositories::{ActivityCacheRepository, BackfillCoverage};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use sqlx::{Pool, Postgres, Row};
use std::collections::HashSet;
use uuid::Uuid;

/// String form of an activity's sport type for the indexed column.
/// `SportType` is an externally-tagged enum: named variants serialize as a JSON
/// string (`"run"`, `"trail_running"`), but the catch-all `Other(String)`
/// serializes as an object (`{"other": "<provider_type>"}`). Unwrap that object
/// to the inner provider string so unmapped sports (e.g. a Strava type cageux
/// has no named variant for) get a real column value instead of `NULL` — the
/// canonical value always remains in `data_json` regardless.
fn sport_type_string(activity: &Activity) -> Option<String> {
    match serde_json::to_value(activity.sport_type()).ok()? {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Object(map) => {
            map.get("other").and_then(|v| v.as_str()).map(str::to_owned)
        }
        _ => None,
    }
}

/// Later of the row-derived sync signal and the fetch-freshness mark.
fn later(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Most recent `activity_fetch_freshness` mark for the user, optionally scoped
/// to one provider. `None` when no successful fetch has been recorded.
async fn latest_fetch_mark(
    pool: &Pool<Postgres>,
    user_id: &str,
    tenant_id: &str,
    provider: Option<&str>,
) -> AppResult<Option<DateTime<Utc>>> {
    let row = sqlx::query(
        r"
        SELECT fetched_at
        FROM activity_fetch_freshness
        WHERE user_id = $1::uuid AND tenant_id = $2::uuid
          AND ($3::text IS NULL OR provider = $3)
        ORDER BY fetched_at DESC
        LIMIT 1
        ",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(provider)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to read activity fetch mark: {e}")))?;

    row.map(|r| {
        r.try_get::<DateTime<Utc>, _>("fetched_at")
            .map_err(|e| AppError::database(format!("fetch mark col fetched_at: {e}")))
    })
    .transpose()
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
        // Count NET DISTINCT rows persisted, not raw input length. A provider feed
        // can return the same `activity_id` twice in one batch; each ON CONFLICT
        // upsert overwrites the prior copy, so the input length overstates the
        // distinct rows actually stored. Dedup by the upsert key (activity_id) to
        // report the honest figure — the backfill completion notice surfaces this
        // count to the user.
        let mut distinct_ids = HashSet::new();

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

            distinct_ids.insert(activity.id().to_owned());
        }

        Ok(u64::try_from(distinct_ids.len()).unwrap_or(u64::MAX))
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

        let from_rows = row.map(|r| r.get::<DateTime<Utc>, _>("synced_at"));
        let mark = latest_fetch_mark(
            &self.pool,
            &user_id_str,
            &tenant_id.to_string(),
            Some(provider),
        )
        .await?;
        Ok(later(from_rows, mark))
    }

    async fn latest_activity_sync_any(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<DateTime<Utc>>> {
        let user_id_str = user_id.to_string();

        let row = sqlx::query(
            r"
            SELECT synced_at
            FROM cached_activities
            WHERE user_id = $1 AND tenant_id = $2
            ORDER BY synced_at DESC
            LIMIT 1
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to read activity sync time: {e}")))?;

        let from_rows = row
            .map(|r| {
                r.try_get::<DateTime<Utc>, _>("synced_at")
                    .map_err(|e| AppError::database(format!("activity col synced_at: {e}")))
            })
            .transpose()?;
        let mark =
            latest_fetch_mark(&self.pool, &user_id_str, &tenant_id.to_string(), None).await?;
        Ok(later(from_rows, mark))
    }

    async fn record_activity_fetch(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        fetched_at: DateTime<Utc>,
    ) -> AppResult<()> {
        // tenant_id/user_id are UUID columns (cf. activity_backfill_coverage):
        // bind the string form and cast.
        sqlx::query(
            r"
            INSERT INTO activity_fetch_freshness (tenant_id, user_id, provider, fetched_at)
            VALUES ($1::uuid, $2::uuid, $3, $4)
            ON CONFLICT (tenant_id, user_id, provider) DO UPDATE SET
                fetched_at = EXCLUDED.fetched_at
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(provider)
        .bind(fetched_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to record activity fetch: {e}")))?;

        Ok(())
    }

    async fn delete_provider_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64> {
        let result = sqlx::query(
            r"
            DELETE FROM cached_activities
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to delete provider cached activities: {e}"))
        })?;

        Ok(result.rows_affected())
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

    async fn upsert_backfill_coverage(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        coverage: BackfillCoverage,
    ) -> AppResult<()> {
        // tenant_id/user_id are UUID columns (cf. backfill_push_log): bind the
        // string form and cast.
        sqlx::query(
            r"
            INSERT INTO activity_backfill_coverage
                (tenant_id, user_id, provider, oldest_reached_ts, hit_feed_end, updated_at)
            VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)
            ON CONFLICT (tenant_id, user_id, provider) DO UPDATE SET
                oldest_reached_ts = EXCLUDED.oldest_reached_ts,
                hit_feed_end = EXCLUDED.hit_feed_end,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(provider)
        .bind(coverage.oldest_reached_ts)
        .bind(coverage.hit_feed_end)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert backfill coverage: {e}")))?;

        Ok(())
    }

    async fn get_backfill_coverage(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Option<BackfillCoverage>> {
        let row = sqlx::query(
            r"
            SELECT oldest_reached_ts, hit_feed_end
            FROM activity_backfill_coverage
            WHERE tenant_id = $1::uuid AND user_id = $2::uuid AND provider = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to read backfill coverage: {e}")))?;

        Ok(row.map(|r| BackfillCoverage {
            oldest_reached_ts: r.get("oldest_reached_ts"),
            hit_feed_end: r.get("hit_feed_end"),
        }))
    }
}
