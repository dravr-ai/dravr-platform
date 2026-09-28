// ABOUTME: The one AdminConfigRepository implementation, emitted per backend by impl_admin_config_repository!
// ABOUTME: The trait body over the statements in repository.rs; each backend shell supplies its row type and uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin configuration storage, written once.
//!
//! The statements live in [`super::repository`]; this module holds the body
//! that runs them. `AdminConfigManager` (`SQLite`) and
//! `PostgresAdminConfigManager` each invoke [`impl_admin_config_repository`]
//! with their type, their driver's row type and their uuid codec — the one
//! thing the backends cannot share, since `user_id`, `created_by` and
//! `admin_user_id` are `uuid` on Postgres and TEXT on `SQLite`.

use pierre_core::errors::{AppError, AppResult};

/// Read a column, naming it in the error.
pub fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    T: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    row.try_get(name)
        .map_err(|e| AppError::database(format!("admin config column `{name}`: {e}")))
}

/// Emit the whole `AdminConfigRepository` implementation for one backend type.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_admin_config_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// Decode an `admin_config_overrides` row. A `config_value` that is
        /// not JSON reads as the string it holds; an unknown `data_type` as
        /// a string.
        fn row_to_override(row: &$row) -> AppResult<ConfigOverride> {
            let data_type: String = column(row, "data_type")?;
            let config_value: String = column(row, "config_value")?;
            Ok(ConfigOverride {
                id: column(row, "id")?,
                category: column(row, "category")?,
                config_key: column(row, "config_key")?,
                config_value: serde_json::from_str(&config_value)
                    .unwrap_or(serde_json::Value::String(config_value)),
                data_type: ConfigDataType::parse(&data_type).unwrap_or(ConfigDataType::String),
                tenant_id: column(row, "tenant_id")?,
                user_id: $ids::read_text_opt(row, "user_id")?,
                created_by: $ids::read_text(row, "created_by")?,
                created_at: column(row, "created_at")?,
                updated_at: column(row, "updated_at")?,
                reason: column(row, "reason")?,
            })
        }

        #[async_trait]
        impl AdminConfigRepository for $ty {
            async fn get_overrides_at(
                &self,
                scope: ConfigScope<'_>,
            ) -> AppResult<Vec<ConfigOverride>> {
                let query = match scope {
                    ConfigScope::Tenant(tid) => sqlx::query(TENANT_OVERRIDES_SQL).bind(tid),
                    ConfigScope::User(uid) => {
                        sqlx::query(USER_OVERRIDES_SQL).bind($ids::bind_text(uid)?)
                    }
                    ConfigScope::Global => sqlx::query(GLOBAL_OVERRIDES_SQL),
                };
                let rows = query.fetch_all(&self.pool).await.map_err(|e| {
                    AppError::database(format!("Failed to get config overrides: {e}"))
                })?;
                rows.iter().map(row_to_override).collect()
            }

            async fn get_override(
                &self,
                category: &str,
                key: &str,
                scope: ConfigScope<'_>,
            ) -> AppResult<Option<ConfigOverride>> {
                let query = match scope {
                    ConfigScope::Tenant(tid) => sqlx::query(TENANT_OVERRIDE_SQL)
                        .bind(category)
                        .bind(key)
                        .bind(tid),
                    ConfigScope::User(uid) => sqlx::query(USER_OVERRIDE_SQL)
                        .bind(category)
                        .bind(key)
                        .bind($ids::bind_text(uid)?),
                    ConfigScope::Global => {
                        sqlx::query(GLOBAL_OVERRIDE_SQL).bind(category).bind(key)
                    }
                };
                let row = query.fetch_optional(&self.pool).await.map_err(|e| {
                    AppError::database(format!("Failed to get config override: {e}"))
                })?;
                row.as_ref().map(row_to_override).transpose()
            }

            async fn tenants_overriding(
                &self,
                category: &str,
                key: &str,
            ) -> AppResult<Vec<String>> {
                sqlx::query_scalar(TENANTS_OVERRIDING_SQL)
                    .bind(category)
                    .bind(key)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list tenant config overrides: {e}"))
                    })
            }

            async fn set_override(
                &self,
                params: SetOverrideParams<'_>,
            ) -> AppResult<ConfigOverride> {
                let SetOverrideParams {
                    category,
                    key,
                    value,
                    data_type,
                    admin_user_id,
                    scope,
                    reason,
                } = params;
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                let value_str = serde_json::to_string(value)?;
                let created_by = $ids::bind_text(admin_user_id)?;
                let query = match scope {
                    ConfigScope::Tenant(tid) => sqlx::query(SET_TENANT_OVERRIDE_SQL)
                        .bind(&id)
                        .bind(category)
                        .bind(key)
                        .bind(&value_str)
                        .bind(data_type.as_str())
                        .bind(tid)
                        .bind(created_by)
                        .bind(now)
                        .bind(reason),
                    ConfigScope::User(uid) => sqlx::query(SET_USER_OVERRIDE_SQL)
                        .bind(&id)
                        .bind(category)
                        .bind(key)
                        .bind(&value_str)
                        .bind(data_type.as_str())
                        .bind($ids::bind_text(uid)?)
                        .bind(created_by)
                        .bind(now)
                        .bind(reason),
                    ConfigScope::Global => sqlx::query(SET_GLOBAL_OVERRIDE_SQL)
                        .bind(&id)
                        .bind(category)
                        .bind(key)
                        .bind(&value_str)
                        .bind(data_type.as_str())
                        .bind(created_by)
                        .bind(now)
                        .bind(reason),
                };
                query.execute(&self.pool).await.map_err(|e| {
                    AppError::database(format!("Failed to set config override: {e}"))
                })?;
                AdminConfigRepository::get_override(self, category, key, scope)
                    .await?
                    .ok_or_else(|| AppError::internal("Failed to retrieve created override"))
            }

            async fn delete_override(
                &self,
                category: &str,
                key: &str,
                scope: ConfigScope<'_>,
            ) -> AppResult<bool> {
                let query = match scope {
                    ConfigScope::Tenant(tid) => sqlx::query(DELETE_TENANT_OVERRIDE_SQL)
                        .bind(category)
                        .bind(key)
                        .bind(tid),
                    ConfigScope::User(uid) => sqlx::query(DELETE_USER_OVERRIDE_SQL)
                        .bind(category)
                        .bind(key)
                        .bind($ids::bind_text(uid)?),
                    ConfigScope::Global => sqlx::query(DELETE_GLOBAL_OVERRIDE_SQL)
                        .bind(category)
                        .bind(key),
                };
                let result = query.execute(&self.pool).await.map_err(|e| {
                    AppError::database(format!("Failed to delete config override: {e}"))
                })?;
                Ok(result.rows_affected() > 0)
            }

            async fn delete_category_overrides(
                &self,
                category: &str,
                scope: ConfigScope<'_>,
            ) -> AppResult<usize> {
                let query = match scope {
                    ConfigScope::Tenant(tid) => sqlx::query(DELETE_TENANT_CATEGORY_SQL)
                        .bind(category)
                        .bind(tid),
                    ConfigScope::User(uid) => sqlx::query(DELETE_USER_CATEGORY_SQL)
                        .bind(category)
                        .bind($ids::bind_text(uid)?),
                    ConfigScope::Global => sqlx::query(DELETE_GLOBAL_CATEGORY_SQL).bind(category),
                };
                let result = query.execute(&self.pool).await.map_err(|e| {
                    AppError::database(format!("Failed to delete category overrides: {e}"))
                })?;
                Ok(usize::try_from(result.rows_affected()).unwrap_or(usize::MAX))
            }

            async fn log_change(&self, params: LogChangeParams<'_>) -> AppResult<String> {
                let LogChangeParams {
                    admin_user_id,
                    admin_email,
                    category,
                    key,
                    old_value,
                    new_value,
                    data_type,
                    reason,
                    scope,
                    ip_address,
                    user_agent,
                } = params;
                let id = Uuid::new_v4().to_string();
                let old_value_str = old_value.map(|v| serde_json::to_string(v).unwrap_or_default());
                sqlx::query(LOG_CHANGE_SQL)
                    .bind(&id)
                    .bind(Utc::now())
                    .bind($ids::bind_text(admin_user_id)?)
                    .bind(normalize_email(admin_email))
                    .bind(category)
                    .bind(key)
                    .bind(old_value_str)
                    .bind(serde_json::to_string(new_value)?)
                    .bind(data_type.as_str())
                    .bind(reason)
                    .bind(scope.tenant_id())
                    .bind($ids::bind_text_opt(scope.user_id())?)
                    .bind(ip_address)
                    .bind(user_agent)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to log config change: {e}")))?;
                Ok(id)
            }

            async fn get_categories(&self) -> AppResult<Vec<AdminConfigCategory>> {
                let rows = sqlx::query(ACTIVE_CATEGORIES_SQL)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get categories: {e}")))?;
                rows.iter()
                    .map(|row| {
                        let description: Option<String> = column(row, "description")?;
                        Ok(AdminConfigCategory {
                            id: column(row, "id")?,
                            name: column(row, "name")?,
                            display_name: column(row, "display_name")?,
                            description: description.unwrap_or_default(),
                            display_order: column(row, "display_order")?,
                            icon: column(row, "icon")?,
                            is_active: column(row, "is_active")?,
                            // Populated by the service layer.
                            parameters: Vec::new(),
                        })
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_admin_config_repository;
