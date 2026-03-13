// ABOUTME: PostgreSQL implementation of the admin configuration repository
// ABOUTME: Uses native PG types (TIMESTAMPTZ, BOOLEAN, $N params) for admin config operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::repository::AdminConfigRepository;
use super::types::{
    AdminConfigCategory, ConfigAuditEntry, ConfigAuditFilter, ConfigDataType, ConfigOverride,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// PostgreSQL-backed admin configuration manager
pub struct PostgresAdminConfigManager {
    pool: PgPool,
}

impl PostgresAdminConfigManager {
    /// Create a new `PostgreSQL` admin config manager
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn row_to_override(row: &PgRow) -> ConfigOverride {
        let created_at: DateTime<Utc> = row.get("created_at");
        let updated_at: DateTime<Utc> = row.get("updated_at");
        let data_type_str: String = row.get("data_type");
        let config_value_str: String = row.get("config_value");

        let data_type = ConfigDataType::parse(&data_type_str).unwrap_or(ConfigDataType::String);
        let config_value: serde_json::Value = serde_json::from_str(&config_value_str)
            .unwrap_or(serde_json::Value::String(config_value_str));

        let created_by: Uuid = row.get("created_by");

        ConfigOverride {
            id: row.get("id"),
            category: row.get("category"),
            config_key: row.get("config_key"),
            config_value,
            data_type,
            tenant_id: row.get("tenant_id"),
            created_by: created_by.to_string(),
            created_at,
            updated_at,
            reason: row.get("reason"),
        }
    }

    fn row_to_audit_entry(row: &PgRow) -> ConfigAuditEntry {
        let timestamp: DateTime<Utc> = row.get("timestamp");
        let data_type_str: String = row.get("data_type");
        let old_value_str: Option<String> = row.get("old_value");
        let new_value_str: String = row.get("new_value");

        let data_type = ConfigDataType::parse(&data_type_str).unwrap_or(ConfigDataType::String);
        let old_value = old_value_str.and_then(|s| serde_json::from_str(&s).ok());
        let new_value: serde_json::Value = serde_json::from_str(&new_value_str)
            .unwrap_or(serde_json::Value::String(new_value_str));

        let admin_user_id: Uuid = row.get("admin_user_id");

        ConfigAuditEntry {
            id: row.get("id"),
            timestamp,
            admin_user_id: admin_user_id.to_string(),
            admin_email: row.get("admin_email"),
            category: row.get("category"),
            config_key: row.get("config_key"),
            old_value,
            new_value,
            data_type,
            reason: row.get("reason"),
            tenant_id: row.get("tenant_id"),
            ip_address: row.get("ip_address"),
            user_agent: row.get("user_agent"),
        }
    }

    /// Helper to execute dynamic queries with bind values
    async fn execute_query_with_binds(
        &self,
        query: &str,
        bind_values: &[String],
    ) -> AppResult<Vec<PgRow>> {
        let mut q = sqlx::query(query);
        for val in bind_values {
            q = q.bind(val);
        }
        q.fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Query execution failed: {e}")))
    }
}

#[async_trait]
impl AdminConfigRepository for PostgresAdminConfigManager {
    async fn get_overrides(&self, tenant_id: Option<&str>) -> AppResult<Vec<ConfigOverride>> {
        let rows = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT id, category, config_key, config_value, data_type, tenant_id,
                       created_by, created_at, updated_at, reason
                FROM admin_config_overrides
                WHERE tenant_id = $1 OR tenant_id IS NULL
                ORDER BY category, config_key
                ",
            )
            .bind(tid)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                SELECT id, category, config_key, config_value, data_type, tenant_id,
                       created_by, created_at, updated_at, reason
                FROM admin_config_overrides
                WHERE tenant_id IS NULL
                ORDER BY category, config_key
                ",
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to get config overrides: {e}")))?;

        let mut overrides = Vec::with_capacity(rows.len());
        for row in rows {
            overrides.push(Self::row_to_override(&row));
        }
        Ok(overrides)
    }

    async fn get_override(
        &self,
        category: &str,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<ConfigOverride>> {
        let row = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                SELECT id, category, config_key, config_value, data_type, tenant_id,
                       created_by, created_at, updated_at, reason
                FROM admin_config_overrides
                WHERE category = $1 AND config_key = $2 AND tenant_id = $3
                ",
            )
            .bind(category)
            .bind(key)
            .bind(tid)
            .fetch_optional(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                SELECT id, category, config_key, config_value, data_type, tenant_id,
                       created_by, created_at, updated_at, reason
                FROM admin_config_overrides
                WHERE category = $1 AND config_key = $2 AND tenant_id IS NULL
                ",
            )
            .bind(category)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to get config override: {e}")))?;

        Ok(row.map(|r| Self::row_to_override(&r)))
    }

    async fn get_effective_override(
        &self,
        category: &str,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<ConfigOverride>> {
        // First try tenant-specific override
        if let Some(tid) = tenant_id {
            if let Some(override_val) = self.get_override(category, key, Some(tid)).await? {
                return Ok(Some(override_val));
            }
        }
        // Fall back to system-wide override
        AdminConfigRepository::get_override(self, category, key, None).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn set_override(
        &self,
        category: &str,
        key: &str,
        value: &serde_json::Value,
        data_type: ConfigDataType,
        admin_user_id: &str,
        tenant_id: Option<&str>,
        reason: Option<&str>,
    ) -> AppResult<ConfigOverride> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let value_str = serde_json::to_string(value)?;

        if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                INSERT INTO admin_config_overrides
                    (id, category, config_key, config_value, data_type, tenant_id, created_by, created_at, updated_at, reason)
                VALUES ($1, $2, $3, $4, $5, $6, $7::uuid, $8, $8, $9)
                ON CONFLICT(category, config_key, tenant_id) DO UPDATE SET
                    config_value = $4,
                    data_type = $5,
                    updated_at = $8,
                    reason = $9
                ",
            )
            .bind(&id)
            .bind(category)
            .bind(key)
            .bind(&value_str)
            .bind(data_type.as_str())
            .bind(tid)
            .bind(admin_user_id)
            .bind(now)
            .bind(reason)
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                INSERT INTO admin_config_overrides
                    (id, category, config_key, config_value, data_type, tenant_id, created_by, created_at, updated_at, reason)
                VALUES ($1, $2, $3, $4, $5, NULL, $6::uuid, $7, $7, $8)
                ON CONFLICT(category, config_key, tenant_id) DO UPDATE SET
                    config_value = $4,
                    data_type = $5,
                    updated_at = $7,
                    reason = $8
                ",
            )
            .bind(&id)
            .bind(category)
            .bind(key)
            .bind(&value_str)
            .bind(data_type.as_str())
            .bind(admin_user_id)
            .bind(now)
            .bind(reason)
            .execute(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to set config override: {e}")))?;

        // Return the created/updated override
        AdminConfigRepository::get_override(self, category, key, tenant_id)
            .await?
            .ok_or_else(|| AppError::internal("Failed to retrieve created override"))
    }

    async fn delete_override(
        &self,
        category: &str,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<bool> {
        let result = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                DELETE FROM admin_config_overrides
                WHERE category = $1 AND config_key = $2 AND tenant_id = $3
                ",
            )
            .bind(category)
            .bind(key)
            .bind(tid)
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                DELETE FROM admin_config_overrides
                WHERE category = $1 AND config_key = $2 AND tenant_id IS NULL
                ",
            )
            .bind(category)
            .bind(key)
            .execute(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to delete config override: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete_category_overrides(
        &self,
        category: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<usize> {
        let result = if let Some(tid) = tenant_id {
            sqlx::query(
                r"
                DELETE FROM admin_config_overrides
                WHERE category = $1 AND tenant_id = $2
                ",
            )
            .bind(category)
            .bind(tid)
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                r"
                DELETE FROM admin_config_overrides
                WHERE category = $1 AND tenant_id IS NULL
                ",
            )
            .bind(category)
            .execute(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to delete category overrides: {e}")))?;

        #[allow(clippy::cast_possible_truncation)]
        Ok(result.rows_affected() as usize)
    }

    #[allow(clippy::too_many_arguments)]
    async fn log_change(
        &self,
        admin_user_id: &str,
        admin_email: &str,
        category: &str,
        key: &str,
        old_value: Option<&serde_json::Value>,
        new_value: &serde_json::Value,
        data_type: ConfigDataType,
        reason: Option<&str>,
        tenant_id: Option<&str>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> AppResult<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let old_value_str = old_value.map(|v| serde_json::to_string(v).unwrap_or_default());
        let new_value_str = serde_json::to_string(new_value)?;

        sqlx::query(
            r"
            INSERT INTO admin_config_audit
                (id, timestamp, admin_user_id, admin_email, category, config_key,
                 old_value, new_value, data_type, reason, tenant_id, ip_address, user_agent)
            VALUES ($1, $2, $3::uuid, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ",
        )
        .bind(&id)
        .bind(now)
        .bind(admin_user_id)
        .bind(admin_email)
        .bind(category)
        .bind(key)
        .bind(old_value_str)
        .bind(&new_value_str)
        .bind(data_type.as_str())
        .bind(reason)
        .bind(tenant_id)
        .bind(ip_address)
        .bind(user_agent)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to log config change: {e}")))?;

        Ok(id)
    }

    async fn get_audit_log(
        &self,
        filter: &ConfigAuditFilter,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<ConfigAuditEntry>, usize)> {
        let mut conditions = vec!["1=1".to_owned()];
        let mut bind_values: Vec<String> = Vec::new();

        if let Some(category) = &filter.category {
            bind_values.push(category.clone());
            conditions.push(format!("category = ${}", bind_values.len()));
        }
        if let Some(key) = &filter.config_key {
            bind_values.push(key.clone());
            conditions.push(format!("config_key = ${}", bind_values.len()));
        }
        if let Some(admin_id) = &filter.admin_user_id {
            bind_values.push(admin_id.clone());
            conditions.push(format!("admin_user_id::text = ${}", bind_values.len()));
        }
        if let Some(tid) = &filter.tenant_id {
            bind_values.push(tid.clone());
            conditions.push(format!("tenant_id = ${}", bind_values.len()));
        }
        if let Some(from_ts) = &filter.from_timestamp {
            bind_values.push(from_ts.to_rfc3339());
            conditions.push(format!("timestamp >= ${}::timestamptz", bind_values.len()));
        }
        if let Some(to_ts) = &filter.to_timestamp {
            bind_values.push(to_ts.to_rfc3339());
            conditions.push(format!("timestamp <= ${}::timestamptz", bind_values.len()));
        }

        let where_clause = conditions.join(" AND ");

        // Get total count
        let count_query =
            format!("SELECT COUNT(*) as cnt FROM admin_config_audit WHERE {where_clause}");
        let count_row = self
            .execute_query_with_binds(&count_query, &bind_values)
            .await?;
        let total_count: i64 = count_row.first().map_or(0, |r| r.get("cnt"));

        // Get paginated results
        let select_query = format!(
            r"
            SELECT id, timestamp, admin_user_id, admin_email, category, config_key,
                   old_value, new_value, data_type, reason, tenant_id, ip_address, user_agent
            FROM admin_config_audit
            WHERE {where_clause}
            ORDER BY timestamp DESC
            LIMIT ${}::bigint OFFSET ${}::bigint
            ",
            bind_values.len() + 1,
            bind_values.len() + 2
        );

        bind_values.push(limit.to_string());
        bind_values.push(offset.to_string());

        let rows = self
            .execute_query_with_binds(&select_query, &bind_values)
            .await?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            entries.push(Self::row_to_audit_entry(&row));
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok((entries, total_count as usize))
    }

    async fn get_categories(&self) -> AppResult<Vec<AdminConfigCategory>> {
        let rows = sqlx::query(
            r"
            SELECT id, name, display_name, description, display_order, icon, is_active
            FROM admin_config_categories
            WHERE is_active = TRUE
            ORDER BY display_order
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get categories: {e}")))?;

        let mut categories = Vec::with_capacity(rows.len());
        for row in rows {
            let is_active: bool = row.get("is_active");
            categories.push(AdminConfigCategory {
                id: row.get("id"),
                name: row.get("name"),
                display_name: row.get("display_name"),
                description: row
                    .get::<Option<String>, _>("description")
                    .unwrap_or_default(),
                display_order: row.get("display_order"),
                icon: row.get("icon"),
                is_active,
                parameters: Vec::new(), // Populated by service layer
            });
        }
        Ok(categories)
    }
}
