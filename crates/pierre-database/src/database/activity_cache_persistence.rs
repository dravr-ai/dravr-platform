// ABOUTME: SQLite implementation of ActivityCacheRepository (provider-agnostic activity cache)
// ABOUTME: Stores queryable columns + full serialized Activity in data_json for stale-while-revalidate reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::{ActivityCacheRepository, BackfillCoverage};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use sqlx::{Pool, Row, Sqlite};
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
    pool: &Pool<Sqlite>,
    user_id: &str,
    tenant_id: &str,
    provider: Option<&str>,
) -> AppResult<Option<DateTime<Utc>>> {
    // `fetched_at` is RFC3339 UTC text, so the lexical DESC order is also the
    // chronological one — the same assumption the `synced_at` reads make.
    let row = sqlx::query(
        r"
        SELECT fetched_at
        FROM activity_fetch_freshness
        WHERE user_id = ? AND tenant_id = ?
          AND (? IS NULL OR provider = ?)
        ORDER BY fetched_at DESC
        LIMIT 1
        ",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(provider)
    .bind(provider)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to read activity fetch mark: {e}")))?;

    row.map(|r| {
        let fetched_at: String = r
            .try_get("fetched_at")
            .map_err(|e| AppError::database(format!("fetch mark col fetched_at: {e}")))?;
        DateTime::parse_from_rfc3339(&fetched_at)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| AppError::database(format!("Invalid fetched_at timestamp: {e}")))
    })
    .transpose()
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
        // Count NET DISTINCT rows persisted, not raw input length. A provider feed
        // can return the same `activity_id` twice in one batch; each ON CONFLICT
        // upsert overwrites the prior copy, so the input length overstates the
        // distinct rows actually stored. Dedup by the upsert key (activity_id) to
        // report the honest figure — the backfill completion notice surfaces this
        // count to the user.
        let mut distinct_ids = HashSet::new();

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

        let from_rows = row
            .map(|r| {
                let synced_at: String = r.get("synced_at");
                DateTime::parse_from_rfc3339(&synced_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| AppError::database(format!("Invalid synced_at timestamp: {e}")))
            })
            .transpose()?;
        let mark = latest_fetch_mark(
            self.pool(),
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

        // `synced_at` is RFC3339 UTC text, so the lexical DESC order is also the
        // chronological one — the same assumption every other read here makes.
        let row = sqlx::query(
            r"
            SELECT synced_at
            FROM cached_activities
            WHERE user_id = ? AND tenant_id = ?
            ORDER BY synced_at DESC
            LIMIT 1
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to read activity sync time: {e}")))?;

        let from_rows = row
            .map(|r| {
                let synced_at: String = r
                    .try_get("synced_at")
                    .map_err(|e| AppError::database(format!("activity col synced_at: {e}")))?;
                DateTime::parse_from_rfc3339(&synced_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| AppError::database(format!("Invalid synced_at timestamp: {e}")))
            })
            .transpose()?;
        let mark =
            latest_fetch_mark(self.pool(), &user_id_str, &tenant_id.to_string(), None).await?;
        Ok(later(from_rows, mark))
    }

    async fn record_activity_fetch(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        fetched_at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO activity_fetch_freshness (tenant_id, user_id, provider, fetched_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(tenant_id, user_id, provider) DO UPDATE SET
                fetched_at = excluded.fetched_at
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(provider)
        .bind(fetched_at.to_rfc3339())
        .execute(self.pool())
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
            WHERE user_id = ? AND tenant_id = ? AND provider = ?
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(self.pool())
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

    async fn upsert_backfill_coverage(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        coverage: BackfillCoverage,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r"
            INSERT INTO activity_backfill_coverage
                (tenant_id, user_id, provider, oldest_reached_ts, hit_feed_end, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(tenant_id, user_id, provider) DO UPDATE SET
                oldest_reached_ts = excluded.oldest_reached_ts,
                hit_feed_end = excluded.hit_feed_end,
                updated_at = excluded.updated_at
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(provider)
        .bind(coverage.oldest_reached_ts)
        .bind(i64::from(coverage.hit_feed_end))
        .bind(&now)
        .execute(self.pool())
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
            WHERE tenant_id = ? AND user_id = ? AND provider = ?
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(provider)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to read backfill coverage: {e}")))?;

        Ok(row.map(|r| {
            let oldest_reached_ts: i64 = r.get("oldest_reached_ts");
            let hit_feed_end: i64 = r.get("hit_feed_end");
            BackfillCoverage {
                oldest_reached_ts,
                hit_feed_end: hit_feed_end != 0,
            }
        }))
    }
}
