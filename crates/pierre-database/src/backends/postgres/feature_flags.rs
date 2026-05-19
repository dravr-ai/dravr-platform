// ABOUTME: PostgreSQL implementation of FeatureFlagsRepository
// ABOUTME: Native UUID + BOOLEAN + TIMESTAMPTZ; mirrors the SQLite path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use sqlx::postgres::PgRow;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::{FeatureFlagRow, FeatureFlagsRepository};

fn row_to_flag(row: &PgRow) -> Option<FeatureFlagRow> {
    let key_raw: String = row.get("feature_key");
    let feature_key = FeatureKey::from_str(&key_raw).ok()?;
    Some(FeatureFlagRow {
        feature_key,
        enabled: row.get("enabled"),
        updated_at: row.get("updated_at"),
        updated_by: row.get("updated_by"),
    })
}

#[async_trait]
impl FeatureFlagsRepository for PostgresDatabase {
    async fn list_tenant_defaults(&self, tenant_id: Uuid) -> AppResult<Vec<FeatureFlagRow>> {
        let rows = sqlx::query(
            r"
            SELECT feature_key, enabled, updated_at, updated_by
            FROM tenant_feature_defaults
            WHERE tenant_id = $1
            ORDER BY feature_key
            ",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list tenant feature defaults: {e}")))?;

        Ok(rows.iter().filter_map(row_to_flag).collect())
    }

    async fn set_tenant_default(
        &self,
        tenant_id: Uuid,
        feature_key: FeatureKey,
        enabled: bool,
        updated_by: Option<Uuid>,
    ) -> AppResult<()> {
        let now = Utc::now();
        sqlx::query(
            r"
            INSERT INTO tenant_feature_defaults
                (tenant_id, feature_key, enabled, updated_at, updated_by)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (tenant_id, feature_key) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                updated_at = EXCLUDED.updated_at,
                updated_by = EXCLUDED.updated_by
            ",
        )
        .bind(tenant_id)
        .bind(feature_key.as_str())
        .bind(enabled)
        .bind(now)
        .bind(updated_by)
        .execute(&self.pool)
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
            WHERE tenant_id = $1 AND feature_key = $2
            ",
        )
        .bind(tenant_id)
        .bind(feature_key.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to clear tenant feature default: {e}")))?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_user_overrides(&self, user_id: Uuid) -> AppResult<Vec<FeatureFlagRow>> {
        let rows = sqlx::query(
            r"
            SELECT feature_key, enabled, updated_at, updated_by
            FROM user_feature_overrides
            WHERE user_id = $1
            ORDER BY feature_key
            ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list user feature overrides: {e}")))?;

        Ok(rows.iter().filter_map(row_to_flag).collect())
    }

    async fn set_user_override(
        &self,
        user_id: Uuid,
        feature_key: FeatureKey,
        enabled: bool,
        updated_by: Option<Uuid>,
    ) -> AppResult<()> {
        let now = Utc::now();
        sqlx::query(
            r"
            INSERT INTO user_feature_overrides
                (user_id, feature_key, enabled, updated_at, updated_by)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id, feature_key) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                updated_at = EXCLUDED.updated_at,
                updated_by = EXCLUDED.updated_by
            ",
        )
        .bind(user_id)
        .bind(feature_key.as_str())
        .bind(enabled)
        .bind(now)
        .bind(updated_by)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set user feature override: {e}")))?;
        Ok(())
    }

    async fn clear_user_override(&self, user_id: Uuid, feature_key: FeatureKey) -> AppResult<bool> {
        let res = sqlx::query(
            r"
            DELETE FROM user_feature_overrides
            WHERE user_id = $1 AND feature_key = $2
            ",
        )
        .bind(user_id)
        .bind(feature_key.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to clear user feature override: {e}")))?;
        Ok(res.rows_affected() > 0)
    }
}
