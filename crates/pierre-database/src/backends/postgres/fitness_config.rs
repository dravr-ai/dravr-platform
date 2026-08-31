// ABOUTME: PostgreSQL fitness-configuration repository — tenant- and user-scoped training settings
// ABOUTME: Split from tenant.rs, which held four unrelated repositories in one file
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persistence for [`FitnessConfig`] at both tenant and user scope.
//!
//! This lived in `tenant.rs` alongside three other repositories that share
//! nothing but a backend. Fitness configuration is not a tenant concern — it is
//! per-athlete training settings that happen to be tenant-scoped for isolation,
//! the same way every other row in the system is.
//!
//! Separate trait impls, unlike methods of one impl, move between modules
//! freely, so this is a plain relocation: no delegation layer and no change to
//! the dispatch surface.

use super::super::FitnessConfigRepository;
use super::PostgresDatabase;
use async_trait::async_trait;
use pierre_core::config::FitnessConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

#[async_trait]
impl FitnessConfigRepository for PostgresDatabase {
    /// Save tenant-level fitness configuration
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String> {
        let config_json = serde_json::to_string(config)?;
        let now = chrono::Utc::now();

        let result = sqlx::query(
            r"
            INSERT INTO fitness_configurations (id, tenant_id, user_id, configuration_name, config_data, created_at, updated_at)
            VALUES ($1, $2, NULL, $3, $4, $5, $5)
            ON CONFLICT (tenant_id, user_id, configuration_name)
            DO UPDATE SET
                config_data = EXCLUDED.config_data,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            ",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(tenant_id.to_string())
        .bind(configuration_name)
        .bind(&config_json)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch record: {e}")))?;

        Ok(result.get("id"))
    }

    /// Save user-specific fitness configuration
    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String> {
        let config_json = serde_json::to_string(config)?;
        let now = chrono::Utc::now();

        let result = sqlx::query(
            r"
            INSERT INTO fitness_configurations (id, tenant_id, user_id, configuration_name, config_data, created_at, updated_at)
            VALUES ($1, $2, $3::uuid, $4, $5, $6, $6)
            ON CONFLICT (tenant_id, user_id, configuration_name)
            DO UPDATE SET
                config_data = EXCLUDED.config_data,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            ",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(configuration_name)
        .bind(&config_json)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch record: {e}")))?;

        Ok(result.get("id"))
    }

    /// Get tenant-level fitness configuration
    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>> {
        let result = sqlx::query(
            r"
            SELECT config_data FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id IS NULL AND configuration_name = $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(configuration_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = result {
            let config_json: String = row.get("config_data");
            let config: FitnessConfig = serde_json::from_str(&config_json)?;
            Ok(Some(config))
        } else {
            Ok(None)
        }
    }

    /// Get user-specific fitness configuration
    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>> {
        // First try to get user-specific configuration
        let result = sqlx::query(
            r"
            SELECT config_data FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id = $2::uuid AND configuration_name = $3
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .bind(configuration_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = result {
            let config_json: String = row.get("config_data");
            let config: FitnessConfig = serde_json::from_str(&config_json)?;
            return Ok(Some(config));
        }

        // Fall back to tenant default configuration
        let result = sqlx::query(
            r"
            SELECT config_data FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id IS NULL AND configuration_name = $2
            ",
        )
        .bind(tenant_id.to_string())
        .bind(configuration_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch optional record: {e}")))?;

        if let Some(row) = result {
            let config_json: String = row.get("config_data");
            let config: FitnessConfig = serde_json::from_str(&config_json)?;
            Ok(Some(config))
        } else {
            Ok(None)
        }
    }

    /// List all tenant-level fitness configuration names
    async fn list_tenant_configurations(&self, tenant_id: TenantId) -> AppResult<Vec<String>> {
        let rows = sqlx::query(
            r"
            SELECT DISTINCT configuration_name FROM fitness_configurations
            WHERE tenant_id = $1
            ORDER BY configuration_name
            ",
        )
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let configurations = rows
            .into_iter()
            .map(|row| row.get::<String, _>("configuration_name"))
            .collect();

        Ok(configurations)
    }

    /// List all user-specific fitness configuration names
    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<String>> {
        let rows = sqlx::query(
            r"
            SELECT DISTINCT configuration_name FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id = $2::uuid
            ORDER BY configuration_name
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch records: {e}")))?;

        let configurations = rows
            .into_iter()
            .map(|row| row.get::<String, _>("configuration_name"))
            .collect();

        Ok(configurations)
    }

    /// Delete fitness configuration (tenant or user-specific)
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> AppResult<bool> {
        let rows_affected = if let Some(uid) = user_id {
            sqlx::query(
                r"
                DELETE FROM fitness_configurations
                WHERE tenant_id = $1 AND user_id = $2::uuid AND configuration_name = $3
                ",
            )
            .bind(tenant_id.to_string())
            .bind(uid)
            .bind(configuration_name)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?
        } else {
            sqlx::query(
                r"
                DELETE FROM fitness_configurations
                WHERE tenant_id = $1 AND user_id IS NULL AND configuration_name = $2
                ",
            )
            .bind(tenant_id.to_string())
            .bind(configuration_name)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?
        };

        Ok(rows_affected.rows_affected() > 0)
    }
}
