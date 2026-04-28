// ABOUTME: LLM credential management database operations with inline SQL
// ABOUTME: Handles encrypted storage, retrieval, and admin config overrides for LLM provider credentials
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::repositories::LlmCredentialRepository;
use async_trait::async_trait;
use pierre_core::admin::models::AdminConfigOverrideRow;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{LlmCredentialRecord, LlmCredentialSummary, TenantId};
use sqlx::Row;
use uuid::Uuid;

#[async_trait]
impl LlmCredentialRepository for Database {
    async fn store_credentials(&self, record: &LlmCredentialRecord) -> AppResult<()> {
        // Use INSERT OR REPLACE to handle both insert and update
        sqlx::query(
            r"
            INSERT INTO user_llm_credentials (
                id, tenant_id, user_id, provider, api_key_encrypted,
                base_url, default_model, is_active, created_at, updated_at, created_by
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(tenant_id, user_id, provider) DO UPDATE SET
                api_key_encrypted = excluded.api_key_encrypted,
                base_url = excluded.base_url,
                default_model = excluded.default_model,
                is_active = excluded.is_active,
                updated_at = excluded.updated_at
            ",
        )
        .bind(record.id.to_string())
        .bind(record.tenant_id)
        .bind(record.user_id.map(|u| u.to_string()))
        .bind(&record.provider)
        .bind(&record.api_key_encrypted)
        .bind(&record.base_url)
        .bind(&record.default_model)
        .bind(record.is_active)
        .bind(&record.created_at)
        .bind(&record.updated_at)
        .bind(record.created_by.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store LLM credentials: {e}")))?;

        Ok(())
    }

    async fn get_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<Option<LlmCredentialRecord>> {
        let row = if let Some(uid) = user_id {
            sqlx::query(
                r"
                SELECT id, tenant_id, user_id, provider, api_key_encrypted,
                       base_url, default_model, is_active, created_at, updated_at, created_by
                FROM user_llm_credentials
                WHERE tenant_id = ?1 AND user_id = ?2 AND provider = ?3 AND is_active = 1
                ",
            )
            .bind(tenant_id)
            .bind(uid.to_string())
            .bind(provider)
            .fetch_optional(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                SELECT id, tenant_id, user_id, provider, api_key_encrypted,
                       base_url, default_model, is_active, created_at, updated_at, created_by
                FROM user_llm_credentials
                WHERE tenant_id = ?1 AND user_id IS NULL AND provider = ?2 AND is_active = 1
                ",
            )
            .bind(tenant_id)
            .bind(provider)
            .fetch_optional(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to get LLM credentials: {e}")))?;

        Ok(row.map(|r| LlmCredentialRecord {
            id: Uuid::parse_str(r.get::<&str, _>("id")).unwrap_or_default(),
            tenant_id: r
                .get::<&str, _>("tenant_id")
                .parse()
                .unwrap_or_else(|_| TenantId::nil()),
            user_id: r
                .get::<Option<&str>, _>("user_id")
                .and_then(|s| Uuid::parse_str(s).ok()),
            provider: r.get::<String, _>("provider"),
            api_key_encrypted: r.get::<String, _>("api_key_encrypted"),
            base_url: r.get::<Option<String>, _>("base_url"),
            default_model: r.get::<Option<String>, _>("default_model"),
            is_active: r.get::<bool, _>("is_active"),
            created_at: r.get::<String, _>("created_at"),
            updated_at: r.get::<String, _>("updated_at"),
            created_by: Uuid::parse_str(r.get::<&str, _>("created_by")).unwrap_or_default(),
        }))
    }

    async fn list_credentials(&self, tenant_id: TenantId) -> AppResult<Vec<LlmCredentialSummary>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, base_url, default_model, is_active, created_at, updated_at
            FROM user_llm_credentials
            WHERE tenant_id = ?1
            ORDER BY provider, user_id
            ",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list LLM credentials: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let user_id: Option<String> = r.get("user_id");
                LlmCredentialSummary {
                    id: Uuid::parse_str(r.get::<&str, _>("id")).unwrap_or_default(),
                    user_id: user_id.and_then(|s| Uuid::parse_str(&s).ok()),
                    provider: r.get::<String, _>("provider"),
                    scope: if r.get::<Option<&str>, _>("user_id").is_some() {
                        "user".to_owned()
                    } else {
                        "tenant".to_owned()
                    },
                    base_url: r.get::<Option<String>, _>("base_url"),
                    default_model: r.get::<Option<String>, _>("default_model"),
                    is_active: r.get::<bool, _>("is_active"),
                    created_at: r.get::<String, _>("created_at"),
                    updated_at: r.get::<String, _>("updated_at"),
                }
            })
            .collect())
    }

    async fn delete_credentials(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
        provider: &str,
    ) -> AppResult<bool> {
        let result = if let Some(uid) = user_id {
            sqlx::query(
                r"
                DELETE FROM user_llm_credentials
                WHERE tenant_id = ?1 AND user_id = ?2 AND provider = ?3
                ",
            )
            .bind(tenant_id)
            .bind(uid.to_string())
            .bind(provider)
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                DELETE FROM user_llm_credentials
                WHERE tenant_id = ?1 AND user_id IS NULL AND provider = ?2
                ",
            )
            .bind(tenant_id)
            .bind(provider)
            .execute(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to delete LLM credentials: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_admin_config_override(
        &self,
        config_key: &str,
        tenant_id: Option<TenantId>,
    ) -> AppResult<Option<String>> {
        // Extract category from key (e.g., "llm.gemini_api_key" -> "llm_provider")
        let category = if config_key.starts_with("llm.") {
            "llm_provider"
        } else {
            config_key.split('.').next().unwrap_or("unknown")
        };

        let row = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT config_value
                FROM admin_config_overrides
                WHERE category = ?1 AND config_key = ?2 AND tenant_id = ?3
                ",
            )
            .bind(category)
            .bind(config_key)
            .bind(tid)
            .fetch_optional(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                SELECT config_value
                FROM admin_config_overrides
                WHERE category = ?1 AND config_key = ?2 AND tenant_id IS NULL
                ",
            )
            .bind(category)
            .bind(config_key)
            .fetch_optional(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to get admin config override: {e}")))?;

        Ok(row.map(|r| r.get::<String, _>("config_value")))
    }

    async fn list_admin_config_overrides_by_category(
        &self,
        category: &str,
    ) -> AppResult<Vec<AdminConfigOverrideRow>> {
        let rows = sqlx::query(
            r"
            SELECT config_key, tenant_id, config_value
            FROM admin_config_overrides
            WHERE category = ?1
            ",
        )
        .bind(category)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to list admin config overrides for category: {e}"
            ))
        })?;

        Ok(rows
            .iter()
            .map(|r| AdminConfigOverrideRow {
                config_key: r.get::<String, _>("config_key"),
                tenant_id: r.get::<Option<String>, _>("tenant_id"),
                config_value: r.get::<String, _>("config_value"),
            })
            .collect())
    }
}
