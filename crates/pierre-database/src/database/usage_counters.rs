// ABOUTME: Database operations for usage counter tracking and quota enforcement
// ABOUTME: Provides atomic upsert counters with time-bucketed periods for rate limiting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::repositories::UsageCounterRepository;
use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UsageCounterRecord;

use super::Database;

impl Database {
    /// Atomically increment a usage counter via upsert (inherent implementation)
    ///
    /// Creates the counter if it doesn't exist, or increments the existing value.
    /// Uses INSERT ON CONFLICT for atomic upsert behavior.
    pub(crate) async fn increment_usage_counter_impl(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> AppResult<UsageCounterRecord> {
        let now = chrono::Utc::now().to_rfc3339();

        // RETURNING, not a follow-up SELECT. The upsert and the read must be one
        // statement or the value a caller sees is not the value its own
        // increment produced: two concurrent turns can both land their
        // increments before either reads, and both then see 2. A caller using
        // `value == 1` to claim a once-per-window slot — the quota notice does
        // exactly that — would have BOTH turns decline it, and the athlete is
        // never told about their budget at all (registre#258).
        let row: (String, String, String, String, i64, String) = sqlx::query_as(
            r"
            INSERT INTO usage_counters (tenant_id, user_id, counter_key, period, value, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (tenant_id, user_id, counter_key, period)
            DO UPDATE SET value = usage_counters.value + excluded.value, updated_at = excluded.updated_at
            RETURNING tenant_id, user_id, counter_key, period, value, updated_at
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(counter_key)
        .bind(period)
        .bind(amount)
        .bind(&now)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to increment usage counter: {e}")))?;

        Ok(UsageCounterRecord {
            tenant_id: row.0,
            user_id: row.1,
            counter_key: row.2,
            period: row.3,
            value: row.4,
            updated_at: row.5,
        })
    }

    /// Get the current value of a usage counter (inherent implementation)
    ///
    /// Returns a record with value=0 if no counter exists for the given key/period.
    pub(crate) async fn get_usage_counter_impl(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> AppResult<UsageCounterRecord> {
        let row: Option<(String, String, String, String, i64, String)> = sqlx::query_as(
            r"
            SELECT tenant_id, user_id, counter_key, period, value, updated_at
            FROM usage_counters
            WHERE tenant_id = $1 AND user_id = $2 AND counter_key = $3 AND period = $4
            ",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(counter_key)
        .bind(period)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get usage counter: {e}")))?;

        match row {
            Some((tid, uid, key, per, val, updated)) => Ok(UsageCounterRecord {
                tenant_id: tid,
                user_id: uid,
                counter_key: key,
                period: per,
                value: val,
                updated_at: updated,
            }),
            None => Ok(UsageCounterRecord {
                tenant_id: tenant_id.to_owned(),
                user_id: user_id.to_owned(),
                counter_key: counter_key.to_owned(),
                period: period.to_owned(),
                value: 0,
                updated_at: String::new(),
            }),
        }
    }

    /// Delete counters older than the given period cutoff (inherent implementation)
    ///
    /// System-level housekeeping: intentionally operates across ALL tenants to prune
    /// expired counter data. Called only from the background pruning task, not from
    /// user-facing endpoints. The comparison is lexicographic on the period string.
    pub(crate) async fn delete_old_usage_counters_impl(
        &self,
        period_before: &str,
    ) -> AppResult<u64> {
        let result = sqlx::query(
            r"
            DELETE FROM usage_counters
            WHERE period < $1
            ",
        )
        .bind(period_before)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete old usage counters: {e}")))?;

        Ok(result.rows_affected())
    }
}

#[async_trait]
impl UsageCounterRepository for Database {
    async fn increment_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
        amount: i64,
    ) -> AppResult<UsageCounterRecord> {
        self.increment_usage_counter_impl(tenant_id, user_id, counter_key, period, amount)
            .await
    }

    async fn get_counter(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_key: &str,
        period: &str,
    ) -> AppResult<UsageCounterRecord> {
        self.get_usage_counter_impl(tenant_id, user_id, counter_key, period)
            .await
    }

    async fn delete_old_counters(&self, period_before: &str) -> AppResult<u64> {
        self.delete_old_usage_counters_impl(period_before).await
    }
}
