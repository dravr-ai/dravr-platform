// ABOUTME: Repository trait, statements and shared body for tenant- and user-scoped fitness configurations
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id column needs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Fitness configuration, written once.
//!
//! A configuration is a [`FitnessConfig`] serialised to JSON under a name,
//! at tenant scope (`user_id IS NULL`) or for one user; the user read falls
//! back to the tenant's row of the same name. `tenant_id` is `TEXT` on both
//! backends and binds as the id's hyphenated text. `user_id` is a `uuid`
//! column on Postgres and `TEXT` on `SQLite`, so the shell hands the body its
//! [`uuid_columns`](super::uuid_columns) codec, which also refuses a
//! malformed id on both drivers rather than matching nothing on one. The
//! nullable `user_id` keeps a statement per shape (`IS NULL` against `= $n`),
//! as both backends already had.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres. Timestamps bind as [`DateTime<Utc>`](chrono::DateTime) on both:
//! `TIMESTAMPTZ` on Postgres, RFC 3339 text on `SQLite`.

use async_trait::async_trait;
use pierre_core::config::FitnessConfig;
use pierre_core::errors::{AppError, AppResult};

use pierre_core::models::TenantId;

/// Fitness configuration management repository
#[async_trait]
pub trait FitnessConfigRepository: Send + Sync {
    /// Save tenant-level fitness configuration
    async fn save_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String>;
    /// Save user-specific fitness configuration
    async fn save_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
        config: &FitnessConfig,
    ) -> AppResult<String>;
    /// Get tenant-level fitness configuration
    async fn get_tenant_config(
        &self,
        tenant_id: TenantId,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>>;
    /// Get user-specific fitness configuration
    async fn get_user_config(
        &self,
        tenant_id: TenantId,
        user_id: &str,
        configuration_name: &str,
    ) -> AppResult<Option<FitnessConfig>>;
    /// List all tenant-level fitness configuration names
    async fn list_tenant_configurations(&self, tenant_id: TenantId) -> AppResult<Vec<String>>;
    /// List all user-specific fitness configuration names
    async fn list_user_configurations(
        &self,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<Vec<String>>;
    /// Delete fitness configuration (tenant or user-specific)
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        user_id: Option<&str>,
        configuration_name: &str,
    ) -> AppResult<bool>;
}

/// Save or update the tenant-level row of this name, returning its id — the
/// existing row's id when the upsert updated rather than inserted.
pub(crate) const SAVE_TENANT_FITNESS_CONFIG_SQL: &str = r"
            INSERT INTO fitness_configurations (id, tenant_id, user_id, configuration_name, config_data, created_at, updated_at)
            VALUES ($1, $2, NULL, $3, $4, $5, $5)
            ON CONFLICT (tenant_id, user_id, configuration_name)
            DO UPDATE SET
                config_data = EXCLUDED.config_data,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            ";

/// Save or update one user's row of this name, returning its id as above.
pub(crate) const SAVE_USER_FITNESS_CONFIG_SQL: &str = r"
            INSERT INTO fitness_configurations (id, tenant_id, user_id, configuration_name, config_data, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $6)
            ON CONFLICT (tenant_id, user_id, configuration_name)
            DO UPDATE SET
                config_data = EXCLUDED.config_data,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            ";

/// The tenant-level configuration of this name.
pub(crate) const GET_TENANT_FITNESS_CONFIG_SQL: &str = r"
            SELECT config_data FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id IS NULL AND configuration_name = $2
            ";

/// One user's configuration of this name.
pub(crate) const GET_USER_FITNESS_CONFIG_SQL: &str = r"
            SELECT config_data FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id = $2 AND configuration_name = $3
            ";

/// Every configuration name stored under the tenant, at either scope.
pub(crate) const LIST_TENANT_FITNESS_CONFIGS_SQL: &str = r"
            SELECT DISTINCT configuration_name FROM fitness_configurations
            WHERE tenant_id = $1
            ORDER BY configuration_name
            ";

/// Every configuration name stored for one user.
pub(crate) const LIST_USER_FITNESS_CONFIGS_SQL: &str = r"
            SELECT DISTINCT configuration_name FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY configuration_name
            ";

/// Delete the tenant-level row of this name.
pub(crate) const DELETE_TENANT_FITNESS_CONFIG_SQL: &str = r"
            DELETE FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id IS NULL AND configuration_name = $2
            ";

/// Delete one user's row of this name.
pub(crate) const DELETE_USER_FITNESS_CONFIG_SQL: &str = r"
            DELETE FROM fitness_configurations
            WHERE tenant_id = $1 AND user_id = $2 AND configuration_name = $3
            ";

/// Deserialise the `config_data` column of a fetched row, when there is one.
///
/// # Errors
/// Returns a database error when the column cannot be read, or a
/// serialisation error when the stored JSON no longer parses as a config.
pub(crate) fn fitness_config_from_row<R>(row: Option<R>) -> AppResult<Option<FitnessConfig>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    row.map(|row| {
        let config_json: String = row
            .try_get("config_data")
            .map_err(|e| AppError::database(format!("Failed to get config_data: {e}")))?;
        let config: FitnessConfig = serde_json::from_str(&config_json)?;
        Ok(config)
    })
    .transpose()
}

/// Read the `configuration_name` column of every fetched row.
///
/// # Errors
/// Returns a database error naming the row whose column cannot be decoded.
pub(crate) fn configuration_names_from_rows<R>(rows: &[R]) -> AppResult<Vec<String>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    rows.iter()
        .map(|row| {
            row.try_get::<String, _>("configuration_name")
                .map_err(|e| AppError::database(format!("Failed to get configuration_name: {e}")))
        })
        .collect()
}

/// Emit the whole [`FitnessConfigRepository`] implementation for one backend
/// type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec, which
/// spells how a text user id binds against the `user_id` column. The body is
/// written once here; each backend's shell invokes it with its own type, and
/// sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_fitness_config_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl FitnessConfigRepository for $ty {
            async fn save_tenant_config(
                &self,
                tenant_id: TenantId,
                configuration_name: &str,
                config: &FitnessConfig,
            ) -> AppResult<String> {
                let config_json = serde_json::to_string(config)?;

                let row = sqlx::query(SAVE_TENANT_FITNESS_CONFIG_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(tenant_id.to_string())
                    .bind(configuration_name)
                    .bind(&config_json)
                    .bind(Utc::now())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to save tenant fitness config: {e}"))
                    })?;

                row.try_get("id")
                    .map_err(|e| AppError::database(format!("Failed to get id: {e}")))
            }

            async fn save_user_config(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                configuration_name: &str,
                config: &FitnessConfig,
            ) -> AppResult<String> {
                let config_json = serde_json::to_string(config)?;

                let row = sqlx::query(SAVE_USER_FITNESS_CONFIG_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(tenant_id.to_string())
                    .bind($ids::bind_text(user_id)?)
                    .bind(configuration_name)
                    .bind(&config_json)
                    .bind(Utc::now())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to save user fitness config: {e}"))
                    })?;

                row.try_get("id")
                    .map_err(|e| AppError::database(format!("Failed to get id: {e}")))
            }

            async fn get_tenant_config(
                &self,
                tenant_id: TenantId,
                configuration_name: &str,
            ) -> AppResult<Option<FitnessConfig>> {
                let row = sqlx::query(GET_TENANT_FITNESS_CONFIG_SQL)
                    .bind(tenant_id.to_string())
                    .bind(configuration_name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query tenant fitness config: {e}"))
                    })?;
                fitness_config_from_row(row)
            }

            async fn get_user_config(
                &self,
                tenant_id: TenantId,
                user_id: &str,
                configuration_name: &str,
            ) -> AppResult<Option<FitnessConfig>> {
                let row = sqlx::query(GET_USER_FITNESS_CONFIG_SQL)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind_text(user_id)?)
                    .bind(configuration_name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query user fitness config: {e}"))
                    })?;
                if let Some(config) = fitness_config_from_row(row)? {
                    return Ok(Some(config));
                }

                // The tenant default of the same name stands in for a user who has none.
                self.get_tenant_config(tenant_id, configuration_name).await
            }

            async fn list_tenant_configurations(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<Vec<String>> {
                let rows = sqlx::query(LIST_TENANT_FITNESS_CONFIGS_SQL)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list tenant fitness configurations: {e}"
                        ))
                    })?;
                configuration_names_from_rows(&rows)
            }

            async fn list_user_configurations(
                &self,
                tenant_id: TenantId,
                user_id: &str,
            ) -> AppResult<Vec<String>> {
                let rows = sqlx::query(LIST_USER_FITNESS_CONFIGS_SQL)
                    .bind(tenant_id.to_string())
                    .bind($ids::bind_text(user_id)?)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list user fitness configurations: {e}"
                        ))
                    })?;
                configuration_names_from_rows(&rows)
            }

            async fn delete_config(
                &self,
                tenant_id: TenantId,
                user_id: Option<&str>,
                configuration_name: &str,
            ) -> AppResult<bool> {
                let result = if let Some(uid) = user_id {
                    sqlx::query(DELETE_USER_FITNESS_CONFIG_SQL)
                        .bind(tenant_id.to_string())
                        .bind($ids::bind_text(uid)?)
                        .bind(configuration_name)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to delete user fitness config: {e}"))
                        })?
                } else {
                    sqlx::query(DELETE_TENANT_FITNESS_CONFIG_SQL)
                        .bind(tenant_id.to_string())
                        .bind(configuration_name)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!(
                                "Failed to delete tenant fitness config: {e}"
                            ))
                        })?
                };

                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_fitness_config_repository;
