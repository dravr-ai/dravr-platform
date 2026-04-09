// ABOUTME: Database operations for tool selection and per-tenant tool configuration
// ABOUTME: Handles CRUD for tool_catalog and tenant_tool_overrides tables
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::repositories::ToolSelectionRepository;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::models::{TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

use super::Database;

impl Database {
    /// Get the complete tool catalog
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_tool_catalog_impl(&self) -> AppResult<Vec<ToolCatalogEntry>> {
        let rows = sqlx::query(
            r"
            SELECT id, tool_name, display_name, description, category,
                   is_enabled_by_default, requires_provider, min_plan,
                   created_at, updated_at
            FROM tool_catalog
            ORDER BY category, tool_name
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tool catalog: {e}")))?;

        rows.iter().map(map_tool_catalog_row).collect()
    }

    /// Get a specific tool catalog entry by name
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_tool_catalog_entry_impl(
        &self,
        tool_name: &str,
    ) -> AppResult<Option<ToolCatalogEntry>> {
        let row = sqlx::query(
            r"
            SELECT id, tool_name, display_name, description, category,
                   is_enabled_by_default, requires_provider, min_plan,
                   created_at, updated_at
            FROM tool_catalog
            WHERE tool_name = ?
            ",
        )
        .bind(tool_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tool catalog entry: {e}")))?;

        row.as_ref().map(map_tool_catalog_row).transpose()
    }

    /// Get tools filtered by category
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_tools_by_category_impl(
        &self,
        category: ToolCategory,
    ) -> AppResult<Vec<ToolCatalogEntry>> {
        let rows = sqlx::query(
            r"
            SELECT id, tool_name, display_name, description, category,
                   is_enabled_by_default, requires_provider, min_plan,
                   created_at, updated_at
            FROM tool_catalog
            WHERE category = ?
            ORDER BY tool_name
            ",
        )
        .bind(category.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tools by category: {e}")))?;

        rows.iter().map(map_tool_catalog_row).collect()
    }

    /// Get tools available for a specific plan level (tools where `min_plan` <= given plan)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_tools_by_min_plan_impl(
        &self,
        plan: TenantPlan,
    ) -> AppResult<Vec<ToolCatalogEntry>> {
        // Build list of acceptable plans based on hierarchy
        let acceptable_plans = match plan {
            TenantPlan::Starter => vec!["starter"],
            TenantPlan::Professional => vec!["starter", "professional"],
            TenantPlan::Enterprise => vec!["starter", "professional", "enterprise"],
        };

        let placeholders: String = acceptable_plans
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ");
        let query = format!(
            r"
            SELECT id, tool_name, display_name, description, category,
                   is_enabled_by_default, requires_provider, min_plan,
                   created_at, updated_at
            FROM tool_catalog
            WHERE min_plan IN ({placeholders})
            ORDER BY category, tool_name
            "
        );

        let mut query_builder = sqlx::query(&query);
        for plan_str in acceptable_plans {
            query_builder = query_builder.bind(plan_str);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to fetch tools by plan: {e}")))?;

        rows.iter().map(map_tool_catalog_row).collect()
    }

    /// Get all tool overrides for a tenant
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_tenant_tool_overrides_impl(
        &self,
        tenant_id: TenantId,
    ) -> AppResult<Vec<TenantToolOverride>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, tool_name, is_enabled, enabled_by_user_id,
                   reason, created_at, updated_at
            FROM tenant_tool_overrides
            WHERE tenant_id = ?
            ORDER BY tool_name
            ",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tenant tool overrides: {e}")))?;

        rows.iter().map(map_tenant_tool_override_row).collect()
    }

    /// Get a specific tool override for a tenant
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn get_tenant_tool_override_impl(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<Option<TenantToolOverride>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, tool_name, is_enabled, enabled_by_user_id,
                   reason, created_at, updated_at
            FROM tenant_tool_overrides
            WHERE tenant_id = ? AND tool_name = ?
            ",
        )
        .bind(tenant_id)
        .bind(tool_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch tenant tool override: {e}")))?;

        row.as_ref().map(map_tenant_tool_override_row).transpose()
    }

    /// Create or update a tool override for a tenant
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or the upserted row cannot be retrieved
    pub async fn upsert_tenant_tool_override_impl(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> AppResult<TenantToolOverride> {
        let now = Utc::now();
        let id = Uuid::new_v4();

        // Use INSERT OR REPLACE for SQLite upsert
        sqlx::query(
            r"
            INSERT INTO tenant_tool_overrides (id, tenant_id, tool_name, is_enabled, enabled_by_user_id, reason, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(tenant_id, tool_name) DO UPDATE SET
                is_enabled = excluded.is_enabled,
                enabled_by_user_id = excluded.enabled_by_user_id,
                reason = excluded.reason,
                updated_at = excluded.updated_at
            ",
        )
        .bind(id.to_string())
        .bind(tenant_id)
        .bind(tool_name)
        .bind(is_enabled)
        .bind(enabled_by_user_id.map(|u| u.to_string()))
        .bind(&reason)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tenant tool override: {e}")))?;

        // Fetch the resulting row (either inserted or updated)
        self.get_tenant_tool_override_impl(tenant_id, tool_name)
            .await?
            .ok_or_else(|| AppError::internal("Failed to retrieve upserted tenant tool override"))
    }

    /// Delete a tool override (revert to catalog default)
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails
    pub async fn delete_tenant_tool_override_impl(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM tenant_tool_overrides
            WHERE tenant_id = ? AND tool_name = ?
            ",
        )
        .bind(tenant_id)
        .bind(tool_name)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete tenant tool override: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Count enabled tools for a tenant
    ///
    /// This calculates the effective count considering catalog defaults and overrides
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or the tenant plan is invalid
    pub async fn count_enabled_tools_impl(&self, tenant_id: TenantId) -> AppResult<usize> {
        // Get tenant's plan to filter by plan restrictions
        let tenant = self.get_tenant_by_id_impl(tenant_id).await?;
        let plan = TenantPlan::parse_str(&tenant.plan)
            .ok_or_else(|| AppError::internal(format!("Invalid tenant plan: {}", tenant.plan)))?;

        // Get tools available for this plan
        let catalog = self.get_tools_by_min_plan_impl(plan).await?;
        let overrides = self.get_tenant_tool_overrides_impl(tenant_id).await?;

        // Build override map
        let override_map: HashMap<String, bool> = overrides
            .into_iter()
            .map(|o| (o.tool_name, o.is_enabled))
            .collect();

        // Count enabled tools
        let count = catalog
            .iter()
            .filter(|tool| {
                override_map
                    .get(&tool.tool_name)
                    .copied()
                    .unwrap_or(tool.is_enabled_by_default)
            })
            .count();

        Ok(count)
    }
}

/// Map a database row to `ToolCatalogEntry`
fn map_tool_catalog_row(row: &SqliteRow) -> AppResult<ToolCatalogEntry> {
    let id_str: String = row.get("id");
    let category_str: String = row.get("category");
    let min_plan_str: String = row.get("min_plan");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");

    Ok(ToolCatalogEntry {
        id: id_str,
        tool_name: row.get("tool_name"),
        display_name: row.get("display_name"),
        description: row.get("description"),
        category: ToolCategory::parse_str(&category_str)
            .ok_or_else(|| AppError::internal(format!("Invalid category: {category_str}")))?,
        is_enabled_by_default: row.get::<i32, _>("is_enabled_by_default") != 0,
        requires_provider: row.get("requires_provider"),
        min_plan: TenantPlan::parse_str(&min_plan_str)
            .ok_or_else(|| AppError::internal(format!("Invalid min_plan: {min_plan_str}")))?,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
    })
}

/// Map a database row to `TenantToolOverride`
fn map_tenant_tool_override_row(row: &SqliteRow) -> AppResult<TenantToolOverride> {
    let id_str: String = row.get("id");
    let tenant_id_str: String = row.get("tenant_id");
    let enabled_by_user_id_str: Option<String> = row.get("enabled_by_user_id");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");

    Ok(TenantToolOverride {
        id: Uuid::parse_str(&id_str).map_err(|e| {
            AppError::internal(format!("Invalid UUID in tenant_tool_overrides: {e}"))
        })?,
        tenant_id: tenant_id_str
            .parse::<TenantId>()
            .map_err(|e| AppError::internal(format!("Invalid tenant_id UUID: {e}")))?,
        tool_name: row.get("tool_name"),
        is_enabled: row.get::<i32, _>("is_enabled") != 0,
        enabled_by_user_id: enabled_by_user_id_str
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| AppError::internal(format!("Invalid enabled_by_user_id UUID: {e}")))?,
        reason: row.get("reason"),
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_or_else(|_| Utc::now(), |dt| dt.with_timezone(&Utc)),
    })
}

#[async_trait]
impl ToolSelectionRepository for Database {
    async fn get_tool_catalog(&self) -> AppResult<Vec<ToolCatalogEntry>> {
        Self::get_tool_catalog_impl(self).await
    }

    async fn get_tool_catalog_entry(&self, tool_name: &str) -> AppResult<Option<ToolCatalogEntry>> {
        Self::get_tool_catalog_entry_impl(self, tool_name).await
    }

    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> AppResult<Vec<ToolCatalogEntry>> {
        Self::get_tools_by_category_impl(self, category).await
    }

    async fn get_tools_by_min_plan(&self, plan: TenantPlan) -> AppResult<Vec<ToolCatalogEntry>> {
        Self::get_tools_by_min_plan_impl(self, plan).await
    }

    async fn get_overrides(&self, tenant_id: TenantId) -> AppResult<Vec<TenantToolOverride>> {
        Self::get_tenant_tool_overrides_impl(self, tenant_id).await
    }

    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<Option<TenantToolOverride>> {
        Self::get_tenant_tool_override_impl(self, tenant_id, tool_name).await
    }

    async fn upsert_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> AppResult<TenantToolOverride> {
        Self::upsert_tenant_tool_override_impl(
            self,
            tenant_id,
            tool_name,
            is_enabled,
            enabled_by_user_id,
            reason,
        )
        .await
    }

    async fn delete_override(&self, tenant_id: TenantId, tool_name: &str) -> AppResult<bool> {
        Self::delete_tenant_tool_override_impl(self, tenant_id, tool_name).await
    }

    async fn count_enabled_tools(&self, tenant_id: TenantId) -> AppResult<usize> {
        Self::count_enabled_tools_impl(self, tenant_id).await
    }

    async fn upsert_tool_catalog_entry(&self, entry: &ToolCatalogEntry) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO tool_catalog (id, tool_name, display_name, description, category,
                                      is_enabled_by_default, requires_provider, min_plan,
                                      created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            ON CONFLICT(tool_name) DO UPDATE SET
                display_name = excluded.display_name,
                description = excluded.description,
                category = excluded.category,
                is_enabled_by_default = excluded.is_enabled_by_default,
                requires_provider = excluded.requires_provider,
                min_plan = excluded.min_plan,
                updated_at = datetime('now')
            ",
        )
        .bind(&entry.id)
        .bind(&entry.tool_name)
        .bind(&entry.display_name)
        .bind(&entry.description)
        .bind(entry.category.as_str())
        .bind(entry.is_enabled_by_default)
        .bind(&entry.requires_provider)
        .bind(entry.min_plan.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert tool catalog entry: {e}")))?;

        Ok(())
    }

    async fn delete_tool_catalog_entry(&self, tool_name: &str) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM tool_catalog WHERE tool_name = ?")
            .bind(tool_name)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete tool catalog entry: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}
