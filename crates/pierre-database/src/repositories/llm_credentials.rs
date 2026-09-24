// ABOUTME: Shared statements and body for tenant and user LLM provider credentials and the admin config overrides they read
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id and created_by columns need
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! LLM credentials, written once.
//!
//! A `user_llm_credentials` row holds one provider's API key, enveloped, at
//! tenant scope (`user_id IS NULL`) or for one user. `tenant_id` is `TEXT` on
//! both backends and binds as the id's hyphenated text; `user_id` and
//! `created_by` are `uuid` columns on Postgres and `TEXT` on `SQLite`, so the
//! shell hands the body its [`uuid_columns`](super::uuid_columns) codec. The
//! model carries its timestamps as RFC 3339 text; they are parsed once and
//! bound as [`DateTime<Utc>`] on both drivers (`TIMESTAMPTZ` on Postgres,
//! RFC 3339 text on `SQLite`) and rendered back the same way on the read.
//! `is_active` binds and reads as `bool`.
//!
//! The admin config override reads are here because the LLM manager and the
//! pricing loader reach them through the same repository. `tenant_id` on
//! that table is `TEXT` on both backends and is read as text on both.

use std::fmt::Display;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{LlmCredentialRecord, LlmCredentialSummary, TenantId};
use uuid::Uuid;

/// Store or replace one provider's credentials at the record's scope.
pub(crate) const STORE_LLM_CREDENTIALS_SQL: &str = r"
            INSERT INTO user_llm_credentials (
                id, tenant_id, user_id, provider, api_key_encrypted,
                base_url, default_model, is_active, created_at, updated_at, created_by
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT(tenant_id, user_id, provider) DO UPDATE SET
                api_key_encrypted = EXCLUDED.api_key_encrypted,
                base_url = EXCLUDED.base_url,
                default_model = EXCLUDED.default_model,
                is_active = EXCLUDED.is_active,
                updated_at = EXCLUDED.updated_at
            ";

/// The columns [`llm_credential_record_from_row`] reads.
macro_rules! llm_credential_columns {
    () => {
        "id, tenant_id, user_id, provider, api_key_encrypted, \
         base_url, default_model, is_active, created_at, updated_at, created_by"
    };
}

/// One user's live credentials for a provider.
pub(crate) const GET_USER_LLM_CREDENTIALS_SQL: &str = concat!(
    "SELECT ",
    llm_credential_columns!(),
    " FROM user_llm_credentials
                WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND is_active = true"
);

/// The tenant's live credentials for a provider.
pub(crate) const GET_TENANT_LLM_CREDENTIALS_SQL: &str = concat!(
    "SELECT ",
    llm_credential_columns!(),
    " FROM user_llm_credentials
                WHERE tenant_id = $1 AND user_id IS NULL AND provider = $2 AND is_active = true"
);

/// Every credential row of a tenant, at both scopes, without the key.
pub(crate) const LIST_LLM_CREDENTIALS_SQL: &str = r"
            SELECT id, user_id, provider, base_url, default_model, is_active, created_at, updated_at
            FROM user_llm_credentials
            WHERE tenant_id = $1
            ORDER BY provider, user_id
            ";

/// Delete one user's credentials for a provider.
pub(crate) const DELETE_USER_LLM_CREDENTIALS_SQL: &str = r"
                DELETE FROM user_llm_credentials
                WHERE tenant_id = $1 AND user_id = $2 AND provider = $3
                ";

/// Delete the tenant's credentials for a provider.
pub(crate) const DELETE_TENANT_LLM_CREDENTIALS_SQL: &str = r"
                DELETE FROM user_llm_credentials
                WHERE tenant_id = $1 AND user_id IS NULL AND provider = $2
                ";

/// A tenant-scoped admin config override.
pub(crate) const GET_TENANT_ADMIN_CONFIG_OVERRIDE_SQL: &str = r"
                SELECT config_value
                FROM admin_config_overrides
                WHERE category = $1 AND config_key = $2 AND tenant_id = $3
                ";

/// A global admin config override.
pub(crate) const GET_GLOBAL_ADMIN_CONFIG_OVERRIDE_SQL: &str = r"
                SELECT config_value
                FROM admin_config_overrides
                WHERE category = $1 AND config_key = $2 AND tenant_id IS NULL
                ";

/// Every admin config override of one category, at every scope.
pub(crate) const LIST_ADMIN_CONFIG_OVERRIDES_BY_CATEGORY_SQL: &str = r"
            SELECT config_key, tenant_id, config_value
            FROM admin_config_overrides
            WHERE category = $1
            ";

/// The category an override key files under: every `llm.*` key is the LLM
/// provider category, anything else is its first dotted segment.
pub(crate) fn admin_config_category(config_key: &str) -> &str {
    if config_key.starts_with("llm.") {
        "llm_provider"
    } else {
        config_key.split('.').next().unwrap_or("unknown")
    }
}

/// Parse a timestamp the model carries as RFC 3339 text, for binding.
///
/// # Errors
/// Returns an invalid-input error naming the column when the text is not
/// RFC 3339.
pub(crate) fn credential_timestamp(col: &str, value: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            AppError::invalid_input(format!("user_llm_credentials.{col} is not RFC 3339: {e}"))
        })
}

/// The error for a `user_llm_credentials` column that cannot be read, naming
/// the column so the bad row can be found.
pub(crate) fn credential_column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("user_llm_credentials.{col}: {e}"))
}

/// Decode one full credentials row. The two uuid columns are read by the
/// caller through its backend's codec. A malformed identifier column is a
/// data-integrity fault, not a value: decoding it to a nil id would produce
/// something that looks valid and then quietly matches no rows downstream,
/// so the read fails, naming the column.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn llm_credential_record_from_row<R>(
    row: &R,
    user_id: Option<Uuid>,
    created_by: Uuid,
) -> AppResult<LlmCredentialRecord>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let id: String = row
        .try_get("id")
        .map_err(|e| credential_column_error("id", e))?;
    let tenant_id: String = row
        .try_get("tenant_id")
        .map_err(|e| credential_column_error("tenant_id", e))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| credential_column_error("created_at", e))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| credential_column_error("updated_at", e))?;
    Ok(LlmCredentialRecord {
        id: Uuid::parse_str(&id).map_err(|e| {
            AppError::database(format!("user_llm_credentials.id is not a valid UUID: {e}"))
        })?,
        tenant_id: TenantId::parse_str(&tenant_id).map_err(|e| {
            AppError::database(format!(
                "user_llm_credentials.tenant_id is not a valid UUID: {e}"
            ))
        })?,
        user_id,
        provider: row
            .try_get("provider")
            .map_err(|e| credential_column_error("provider", e))?,
        api_key_encrypted: row
            .try_get("api_key_encrypted")
            .map_err(|e| credential_column_error("api_key_encrypted", e))?,
        base_url: row
            .try_get("base_url")
            .map_err(|e| credential_column_error("base_url", e))?,
        default_model: row
            .try_get("default_model")
            .map_err(|e| credential_column_error("default_model", e))?,
        is_active: row
            .try_get("is_active")
            .map_err(|e| credential_column_error("is_active", e))?,
        created_at: created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
        created_by,
    })
}

/// Decode one listing row. `user_id` is read by the caller through its
/// backend's codec; the scope follows from whether it is set.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn llm_credential_summary_from_row<R>(
    row: &R,
    user_id: Option<Uuid>,
) -> AppResult<LlmCredentialSummary>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let id: String = row
        .try_get("id")
        .map_err(|e| credential_column_error("id", e))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| credential_column_error("created_at", e))?;
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| credential_column_error("updated_at", e))?;
    Ok(LlmCredentialSummary {
        id: Uuid::parse_str(&id).map_err(|e| {
            AppError::database(format!("user_llm_credentials.id is not a valid UUID: {e}"))
        })?,
        user_id,
        provider: row
            .try_get("provider")
            .map_err(|e| credential_column_error("provider", e))?,
        scope: if user_id.is_some() {
            "user".to_owned()
        } else {
            "tenant".to_owned()
        },
        base_url: row
            .try_get("base_url")
            .map_err(|e| credential_column_error("base_url", e))?,
        default_model: row
            .try_get("default_model")
            .map_err(|e| credential_column_error("default_model", e))?,
        is_active: row
            .try_get("is_active")
            .map_err(|e| credential_column_error("is_active", e))?,
        created_at: created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
    })
}

/// Emit the whole [`LlmCredentialRepository`](super::LlmCredentialRepository)
/// implementation for one backend type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec,
/// which spells how `user_id` and `created_by` bind and read. The body is
/// written once here; each backend's shell invokes it with its own type, and
/// sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_llm_credential_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl LlmCredentialRepository for $ty {
            async fn store_credentials(&self, record: &LlmCredentialRecord) -> AppResult<()> {
                sqlx::query(STORE_LLM_CREDENTIALS_SQL)
                    .bind(record.id.to_string())
                    .bind(record.tenant_id.to_string())
                    .bind($ids::bind_opt(record.user_id))
                    .bind(&record.provider)
                    .bind(&record.api_key_encrypted)
                    .bind(&record.base_url)
                    .bind(&record.default_model)
                    .bind(record.is_active)
                    .bind(credential_timestamp("created_at", &record.created_at)?)
                    .bind(credential_timestamp("updated_at", &record.updated_at)?)
                    .bind($ids::bind(record.created_by))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store LLM credentials: {e}"))
                    })?;

                Ok(())
            }

            async fn get_credentials(
                &self,
                tenant_id: TenantId,
                user_id: Option<Uuid>,
                provider: &str,
            ) -> AppResult<Option<LlmCredentialRecord>> {
                let row = if let Some(uid) = user_id {
                    sqlx::query(GET_USER_LLM_CREDENTIALS_SQL)
                        .bind(tenant_id.to_string())
                        .bind($ids::bind(uid))
                        .bind(provider)
                        .fetch_optional(self.pool())
                        .await
                } else {
                    sqlx::query(GET_TENANT_LLM_CREDENTIALS_SQL)
                        .bind(tenant_id.to_string())
                        .bind(provider)
                        .fetch_optional(self.pool())
                        .await
                }
                .map_err(|e| AppError::database(format!("Failed to get LLM credentials: {e}")))?;

                row.map(|row| {
                    llm_credential_record_from_row(
                        &row,
                        $ids::read_opt(&row, "user_id").map_err(|e| {
                            credential_column_error("user_id is not a valid UUID", e)
                        })?,
                        $ids::read(&row, "created_by").map_err(|e| {
                            credential_column_error("created_by is not a valid UUID", e)
                        })?,
                    )
                })
                .transpose()
            }

            async fn list_credentials(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<Vec<LlmCredentialSummary>> {
                let rows = sqlx::query(LIST_LLM_CREDENTIALS_SQL)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list LLM credentials: {e}"))
                    })?;

                rows.iter()
                    .map(|row| {
                        llm_credential_summary_from_row(row, $ids::read_opt(row, "user_id")?)
                    })
                    .collect()
            }

            async fn delete_credentials(
                &self,
                tenant_id: TenantId,
                user_id: Option<Uuid>,
                provider: &str,
            ) -> AppResult<bool> {
                let result = if let Some(uid) = user_id {
                    sqlx::query(DELETE_USER_LLM_CREDENTIALS_SQL)
                        .bind(tenant_id.to_string())
                        .bind($ids::bind(uid))
                        .bind(provider)
                        .execute(self.pool())
                        .await
                } else {
                    sqlx::query(DELETE_TENANT_LLM_CREDENTIALS_SQL)
                        .bind(tenant_id.to_string())
                        .bind(provider)
                        .execute(self.pool())
                        .await
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to delete LLM credentials: {e}"))
                })?;

                Ok(result.rows_affected() > 0)
            }

            async fn get_admin_config_override(
                &self,
                config_key: &str,
                tenant_id: Option<TenantId>,
            ) -> AppResult<Option<String>> {
                let category = admin_config_category(config_key);

                let row = if let Some(tid) = tenant_id {
                    sqlx::query(GET_TENANT_ADMIN_CONFIG_OVERRIDE_SQL)
                        .bind(category)
                        .bind(config_key)
                        .bind(tid.to_string())
                        .fetch_optional(self.pool())
                        .await
                } else {
                    sqlx::query(GET_GLOBAL_ADMIN_CONFIG_OVERRIDE_SQL)
                        .bind(category)
                        .bind(config_key)
                        .fetch_optional(self.pool())
                        .await
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to get admin config override: {e}"))
                })?;

                row.map(|r| {
                    r.try_get::<String, _>("config_value")
                        .map_err(|e| AppError::database(format!("Failed to get config_value: {e}")))
                })
                .transpose()
            }

            async fn list_admin_config_overrides_by_category(
                &self,
                category: &str,
            ) -> AppResult<Vec<AdminConfigOverrideRow>> {
                let rows = sqlx::query(LIST_ADMIN_CONFIG_OVERRIDES_BY_CATEGORY_SQL)
                    .bind(category)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list admin config overrides for category: {e}"
                        ))
                    })?;

                rows.iter()
                    .map(|r| {
                        let column = |col: &str, e: sqlx::Error| {
                            AppError::database(format!(
                                "Failed to get admin_config_overrides.{col}: {e}"
                            ))
                        };
                        Ok(AdminConfigOverrideRow {
                            config_key: r
                                .try_get("config_key")
                                .map_err(|e| column("config_key", e))?,
                            tenant_id: r
                                .try_get("tenant_id")
                                .map_err(|e| column("tenant_id", e))?,
                            config_value: r
                                .try_get("config_value")
                                .map_err(|e| column("config_value", e))?,
                        })
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_llm_credential_repository;
