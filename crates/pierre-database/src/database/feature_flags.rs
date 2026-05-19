// ABOUTME: SQLite implementation of FeatureFlagsRepository — tenant defaults + per-user overrides
// ABOUTME: Resolution lives in the default trait method; this file is raw CRUD only
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::database::Database;
use crate::repositories::{FeatureFlagRow, FeatureFlagsRepository};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

fn parse_ts(row: &SqliteRow, col: &str) -> AppResult<DateTime<Utc>> {
    let raw: String = row.get(col);
    DateTime::parse_from_rfc3339(&raw)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| AppError::database(format!("Invalid {col} timestamp '{raw}': {e}")))
}

fn parse_opt_uuid(row: &SqliteRow, col: &str) -> AppResult<Option<Uuid>> {
    let raw: Option<String> = row.get(col);
    raw.map(|s| {
        Uuid::parse_str(&s)
            .map_err(|e| AppError::database(format!("Invalid {col} uuid '{s}': {e}")))
    })
    .transpose()
}

fn row_to_flag(row: &SqliteRow) -> AppResult<Option<FeatureFlagRow>> {
    let key_raw: String = row.get("feature_key");
    // Rows for unknown keys are dropped at the read site (forward-compat: a
    // future version may have deleted a key; the row still exists). Caller
    // sees an empty result for that key and falls back to the default.
    let Ok(feature_key) = FeatureKey::from_str(&key_raw) else {
        return Ok(None);
    };
    let enabled_int: i64 = row.get("enabled");
    Ok(Some(FeatureFlagRow {
        feature_key,
        enabled: enabled_int != 0,
        updated_at: parse_ts(row, "updated_at")?,
        updated_by: parse_opt_uuid(row, "updated_by")?,
    }))
}

#[async_trait]
impl FeatureFlagsRepository for Database {
    async fn list_tenant_defaults(&self, tenant_id: Uuid) -> AppResult<Vec<FeatureFlagRow>> {
        let rows = sqlx::query(
            r"
            SELECT feature_key, enabled, updated_at, updated_by
            FROM tenant_feature_defaults
            WHERE tenant_id = ?1
            ORDER BY feature_key
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list tenant feature defaults: {e}")))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            if let Some(flag) = row_to_flag(row)? {
                out.push(flag);
            }
        }
        Ok(out)
    }

    async fn set_tenant_default(
        &self,
        tenant_id: Uuid,
        feature_key: FeatureKey,
        enabled: bool,
        updated_by: Option<Uuid>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r"
            INSERT INTO tenant_feature_defaults
                (tenant_id, feature_key, enabled, updated_at, updated_by)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(tenant_id, feature_key) DO UPDATE SET
                enabled = excluded.enabled,
                updated_at = excluded.updated_at,
                updated_by = excluded.updated_by
            ",
        )
        .bind(tenant_id.to_string())
        .bind(feature_key.as_str())
        .bind(i64::from(enabled))
        .bind(&now)
        .bind(updated_by.map(|u| u.to_string()))
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to set tenant feature default: {e}")))?;
        Ok(())
    }

    async fn clear_tenant_default(
        &self,
        tenant_id: Uuid,
        feature_key: FeatureKey,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            r"
            DELETE FROM tenant_feature_defaults
            WHERE tenant_id = ?1 AND feature_key = ?2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(feature_key.as_str())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to clear tenant feature default: {e}")))?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_user_overrides(&self, user_id: Uuid) -> AppResult<Vec<FeatureFlagRow>> {
        let rows = sqlx::query(
            r"
            SELECT feature_key, enabled, updated_at, updated_by
            FROM user_feature_overrides
            WHERE user_id = ?1
            ORDER BY feature_key
            ",
        )
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to list user feature overrides: {e}")))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            if let Some(flag) = row_to_flag(row)? {
                out.push(flag);
            }
        }
        Ok(out)
    }

    async fn set_user_override(
        &self,
        user_id: Uuid,
        feature_key: FeatureKey,
        enabled: bool,
        updated_by: Option<Uuid>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r"
            INSERT INTO user_feature_overrides
                (user_id, feature_key, enabled, updated_at, updated_by)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(user_id, feature_key) DO UPDATE SET
                enabled = excluded.enabled,
                updated_at = excluded.updated_at,
                updated_by = excluded.updated_by
            ",
        )
        .bind(user_id.to_string())
        .bind(feature_key.as_str())
        .bind(i64::from(enabled))
        .bind(&now)
        .bind(updated_by.map(|u| u.to_string()))
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to set user feature override: {e}")))?;
        Ok(())
    }

    async fn clear_user_override(&self, user_id: Uuid, feature_key: FeatureKey) -> AppResult<bool> {
        let res = sqlx::query(
            r"
            DELETE FROM user_feature_overrides
            WHERE user_id = ?1 AND feature_key = ?2
            ",
        )
        .bind(user_id.to_string())
        .bind(feature_key.as_str())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to clear user feature override: {e}")))?;
        Ok(res.rows_affected() > 0)
    }
}
