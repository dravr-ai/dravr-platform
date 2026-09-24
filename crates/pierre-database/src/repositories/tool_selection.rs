// ABOUTME: Repository trait, statements and shared body for the tool catalog and per-tenant tool overrides
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its id columns need
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool selection, written once.
//!
//! `tool_catalog` is the registry's view of every tool (id is the tool's own
//! text key on both backends); `tenant_tool_overrides` flips a tool on or
//! off for one tenant. The override row's `id` and `enabled_by_user_id` are
//! uuid columns, native on Postgres and `TEXT` on `SQLite`, so the shell
//! hands the body its [`uuid_columns`](super::uuid_columns) codec. A
//! [`TenantId`] binds and reads natively on both. Booleans bind and read as
//! `bool` (`BOOLEAN` on Postgres, `INTEGER` 0/1 on `SQLite`); timestamps
//! bind as [`DateTime<Utc>`](chrono::DateTime) and are stamped by
//! `CURRENT_TIMESTAMP` where the row's own moment is not the caller's, which
//! sqlx decodes on both drivers.
//!
//! The plan hierarchy is fixed, so the tools a plan may use are three
//! statements with the plan names spelled in, rather than an array bind on
//! one driver and a placeholder list on the other.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use pierre_core::models::TenantId;
use pierre_core::models::{TenantPlan, TenantToolOverride, ToolCatalogEntry, ToolCategory};
use uuid::Uuid;

/// Tool selection and per-tenant configuration repository
#[async_trait]
pub trait ToolSelectionRepository: Send + Sync {
    /// Get the complete tool catalog
    async fn get_tool_catalog(&self) -> AppResult<Vec<ToolCatalogEntry>>;
    /// Get a specific tool catalog entry by name
    async fn get_tool_catalog_entry(&self, tool_name: &str) -> AppResult<Option<ToolCatalogEntry>>;
    /// Get tools filtered by category
    async fn get_tools_by_category(
        &self,
        category: ToolCategory,
    ) -> AppResult<Vec<ToolCatalogEntry>>;
    /// Get tools available for a specific plan level
    async fn get_tools_by_min_plan(&self, plan: TenantPlan) -> AppResult<Vec<ToolCatalogEntry>>;
    /// Get all tool overrides for a tenant
    async fn get_overrides(&self, tenant_id: TenantId) -> AppResult<Vec<TenantToolOverride>>;
    /// Get a specific tool override for a tenant
    async fn get_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
    ) -> AppResult<Option<TenantToolOverride>>;
    /// Create or update a tool override for a tenant
    async fn upsert_override(
        &self,
        tenant_id: TenantId,
        tool_name: &str,
        is_enabled: bool,
        enabled_by_user_id: Option<Uuid>,
        reason: Option<String>,
    ) -> AppResult<TenantToolOverride>;
    /// Delete a tool override (revert to catalog default)
    async fn delete_override(&self, tenant_id: TenantId, tool_name: &str) -> AppResult<bool>;
    /// Count enabled tools for a tenant
    async fn count_enabled_tools(&self, tenant_id: TenantId) -> AppResult<usize>;

    /// Insert or update a tool catalog entry (used by startup catalog sync)
    async fn upsert_tool_catalog_entry(&self, entry: &ToolCatalogEntry) -> AppResult<()>;
    /// Delete a tool catalog entry by tool name (removes phantom entries)
    async fn delete_tool_catalog_entry(&self, tool_name: &str) -> AppResult<bool>;
}

/// The columns [`tool_catalog_entry_from_row`] reads.
macro_rules! tool_catalog_columns {
    () => {
        "id, tool_name, display_name, description, category, \
         is_enabled_by_default, requires_provider, min_plan, created_at, updated_at"
    };
}

/// The whole catalog, grouped by category.
pub(crate) const GET_TOOL_CATALOG_SQL: &str = concat!(
    "SELECT ",
    tool_catalog_columns!(),
    " FROM tool_catalog ORDER BY category, tool_name"
);

/// One catalog entry by tool name.
pub(crate) const GET_TOOL_CATALOG_ENTRY_SQL: &str = concat!(
    "SELECT ",
    tool_catalog_columns!(),
    " FROM tool_catalog WHERE tool_name = $1"
);

/// The catalog entries of one category.
pub(crate) const GET_TOOLS_BY_CATEGORY_SQL: &str = concat!(
    "SELECT ",
    tool_catalog_columns!(),
    " FROM tool_catalog WHERE category = $1 ORDER BY tool_name"
);

/// The catalog entries a starter plan may use.
pub(crate) const GET_STARTER_TOOLS_SQL: &str = concat!(
    "SELECT ",
    tool_catalog_columns!(),
    " FROM tool_catalog WHERE min_plan IN ('starter') ORDER BY category, tool_name"
);

/// The catalog entries a professional plan may use.
pub(crate) const GET_PROFESSIONAL_TOOLS_SQL: &str = concat!(
    "SELECT ",
    tool_catalog_columns!(),
    " FROM tool_catalog WHERE min_plan IN ('starter', 'professional') ORDER BY category, tool_name"
);

/// The catalog entries an enterprise plan may use.
pub(crate) const GET_ENTERPRISE_TOOLS_SQL: &str = concat!(
    "SELECT ",
    tool_catalog_columns!(),
    " FROM tool_catalog WHERE min_plan IN ('starter', 'professional', 'enterprise') ORDER BY category, tool_name"
);

/// The statement listing what a plan may use: every tool whose `min_plan`
/// sits at or below it in the hierarchy.
pub(crate) const fn tools_by_min_plan_sql(plan: TenantPlan) -> &'static str {
    match plan {
        TenantPlan::Starter => GET_STARTER_TOOLS_SQL,
        TenantPlan::Professional => GET_PROFESSIONAL_TOOLS_SQL,
        TenantPlan::Enterprise => GET_ENTERPRISE_TOOLS_SQL,
    }
}

/// The columns [`tenant_tool_override_from_row`] reads.
macro_rules! tool_override_columns {
    () => {
        "id, tenant_id, tool_name, is_enabled, enabled_by_user_id, reason, created_at, updated_at"
    };
}

/// Every override of one tenant.
pub(crate) const GET_TENANT_TOOL_OVERRIDES_SQL: &str = concat!(
    "SELECT ",
    tool_override_columns!(),
    " FROM tenant_tool_overrides WHERE tenant_id = $1 ORDER BY tool_name"
);

/// One tenant's override of one tool.
pub(crate) const GET_TENANT_TOOL_OVERRIDE_SQL: &str = concat!(
    "SELECT ",
    tool_override_columns!(),
    " FROM tenant_tool_overrides WHERE tenant_id = $1 AND tool_name = $2"
);

/// Create or update a tenant's override of one tool.
pub(crate) const UPSERT_TENANT_TOOL_OVERRIDE_SQL: &str = r"
            INSERT INTO tenant_tool_overrides (id, tenant_id, tool_name, is_enabled, enabled_by_user_id, reason, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
            ON CONFLICT(tenant_id, tool_name) DO UPDATE SET
                is_enabled = EXCLUDED.is_enabled,
                enabled_by_user_id = EXCLUDED.enabled_by_user_id,
                reason = EXCLUDED.reason,
                updated_at = EXCLUDED.updated_at
            ";

/// Drop a tenant's override of one tool, reverting to the catalog default.
pub(crate) const DELETE_TENANT_TOOL_OVERRIDE_SQL: &str =
    "DELETE FROM tenant_tool_overrides WHERE tenant_id = $1 AND tool_name = $2";

/// Create or refresh a catalog entry; the row's own clock stamps it.
pub(crate) const UPSERT_TOOL_CATALOG_ENTRY_SQL: &str = r"
            INSERT INTO tool_catalog (id, tool_name, display_name, description, category,
                                      is_enabled_by_default, requires_provider, min_plan,
                                      created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT(tool_name) DO UPDATE SET
                display_name = EXCLUDED.display_name,
                description = EXCLUDED.description,
                category = EXCLUDED.category,
                is_enabled_by_default = EXCLUDED.is_enabled_by_default,
                requires_provider = EXCLUDED.requires_provider,
                min_plan = EXCLUDED.min_plan,
                updated_at = CURRENT_TIMESTAMP
            ";

/// Drop a catalog entry.
pub(crate) const DELETE_TOOL_CATALOG_ENTRY_SQL: &str =
    "DELETE FROM tool_catalog WHERE tool_name = $1";

/// Decode one catalog row; every column decodes the same way on both drivers.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or an internal error when the stored category or plan is not one the
/// model knows.
pub(crate) fn tool_catalog_entry_from_row<R>(row: &R) -> AppResult<ToolCatalogEntry>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| {
        AppError::database(format!("Failed to get tool_catalog.{col}: {e}"))
    };
    let category_str: String = row.try_get("category").map_err(|e| column("category", e))?;
    let min_plan_str: String = row.try_get("min_plan").map_err(|e| column("min_plan", e))?;
    Ok(ToolCatalogEntry {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        tool_name: row
            .try_get("tool_name")
            .map_err(|e| column("tool_name", e))?,
        display_name: row
            .try_get("display_name")
            .map_err(|e| column("display_name", e))?,
        description: row
            .try_get("description")
            .map_err(|e| column("description", e))?,
        category: ToolCategory::parse_str(&category_str)
            .ok_or_else(|| AppError::internal(format!("Invalid category: {category_str}")))?,
        is_enabled_by_default: row
            .try_get("is_enabled_by_default")
            .map_err(|e| column("is_enabled_by_default", e))?,
        requires_provider: row
            .try_get("requires_provider")
            .map_err(|e| column("requires_provider", e))?,
        min_plan: TenantPlan::parse_str(&min_plan_str)
            .ok_or_else(|| AppError::internal(format!("Invalid min_plan: {min_plan_str}")))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column("updated_at", e))?,
    })
}

/// Decode one override row. The two uuid columns are read by the caller
/// through its backend's codec; every other column decodes the same way on
/// both drivers.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn tenant_tool_override_from_row<R>(
    row: &R,
    id: Uuid,
    enabled_by_user_id: Option<Uuid>,
) -> AppResult<TenantToolOverride>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    TenantId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| {
        AppError::database(format!("Failed to get tenant_tool_overrides.{col}: {e}"))
    };
    Ok(TenantToolOverride {
        id,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column("tenant_id", e))?,
        tool_name: row
            .try_get("tool_name")
            .map_err(|e| column("tool_name", e))?,
        is_enabled: row
            .try_get("is_enabled")
            .map_err(|e| column("is_enabled", e))?,
        enabled_by_user_id,
        reason: row.try_get("reason").map_err(|e| column("reason", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column("updated_at", e))?,
    })
}

/// Emit the whole [`ToolSelectionRepository`] implementation for one backend
/// type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec,
/// which spells how the override row's uuid columns bind and read. The body
/// is written once here; each backend's shell invokes it with its own type,
/// and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_tool_selection_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl ToolSelectionRepository for $ty {
            async fn get_tool_catalog(&self) -> AppResult<Vec<ToolCatalogEntry>> {
                let rows = sqlx::query(GET_TOOL_CATALOG_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch tool catalog: {e}"))
                    })?;

                rows.iter().map(tool_catalog_entry_from_row).collect()
            }

            async fn get_tool_catalog_entry(
                &self,
                tool_name: &str,
            ) -> AppResult<Option<ToolCatalogEntry>> {
                let row = sqlx::query(GET_TOOL_CATALOG_ENTRY_SQL)
                    .bind(tool_name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch tool catalog entry: {e}"))
                    })?;

                row.as_ref().map(tool_catalog_entry_from_row).transpose()
            }

            async fn get_tools_by_category(
                &self,
                category: ToolCategory,
            ) -> AppResult<Vec<ToolCatalogEntry>> {
                let rows = sqlx::query(GET_TOOLS_BY_CATEGORY_SQL)
                    .bind(category.as_str())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch tools by category: {e}"))
                    })?;

                rows.iter().map(tool_catalog_entry_from_row).collect()
            }

            async fn get_tools_by_min_plan(
                &self,
                plan: TenantPlan,
            ) -> AppResult<Vec<ToolCatalogEntry>> {
                let rows = sqlx::query(tools_by_min_plan_sql(plan))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch tools by plan: {e}"))
                    })?;

                rows.iter().map(tool_catalog_entry_from_row).collect()
            }

            async fn get_overrides(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<Vec<TenantToolOverride>> {
                let rows = sqlx::query(GET_TENANT_TOOL_OVERRIDES_SQL)
                    .bind(tenant_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch tenant tool overrides: {e}"))
                    })?;

                rows.iter()
                    .map(|row| {
                        tenant_tool_override_from_row(
                            row,
                            $ids::read(row, "id")?,
                            $ids::read_opt(row, "enabled_by_user_id")?,
                        )
                    })
                    .collect()
            }

            async fn get_override(
                &self,
                tenant_id: TenantId,
                tool_name: &str,
            ) -> AppResult<Option<TenantToolOverride>> {
                let row = sqlx::query(GET_TENANT_TOOL_OVERRIDE_SQL)
                    .bind(tenant_id)
                    .bind(tool_name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to fetch tenant tool override: {e}"))
                    })?;

                row.map(|row| {
                    tenant_tool_override_from_row(
                        &row,
                        $ids::read(&row, "id")?,
                        $ids::read_opt(&row, "enabled_by_user_id")?,
                    )
                })
                .transpose()
            }

            async fn upsert_override(
                &self,
                tenant_id: TenantId,
                tool_name: &str,
                is_enabled: bool,
                enabled_by_user_id: Option<Uuid>,
                reason: Option<String>,
            ) -> AppResult<TenantToolOverride> {
                sqlx::query(UPSERT_TENANT_TOOL_OVERRIDE_SQL)
                    .bind($ids::bind(Uuid::new_v4()))
                    .bind(tenant_id)
                    .bind(tool_name)
                    .bind(is_enabled)
                    .bind($ids::bind_opt(enabled_by_user_id))
                    .bind(&reason)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert tenant tool override: {e}"))
                    })?;

                self.get_override(tenant_id, tool_name)
                    .await?
                    .ok_or_else(|| {
                        AppError::internal("Failed to retrieve upserted tenant tool override")
                    })
            }

            async fn delete_override(
                &self,
                tenant_id: TenantId,
                tool_name: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_TENANT_TOOL_OVERRIDE_SQL)
                    .bind(tenant_id)
                    .bind(tool_name)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete tenant tool override: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }

            async fn count_enabled_tools(&self, tenant_id: TenantId) -> AppResult<usize> {
                // The plan bounds the catalog; overrides then flip entries either way.
                let tenant = TenantRepository::get_by_id(self, tenant_id).await?;
                let plan = TenantPlan::parse_str(&tenant.plan).ok_or_else(|| {
                    AppError::internal(format!("Invalid tenant plan: {}", tenant.plan))
                })?;

                let catalog = self.get_tools_by_min_plan(plan).await?;
                let overrides = self.get_overrides(tenant_id).await?;

                let override_map: HashMap<String, bool> = overrides
                    .into_iter()
                    .map(|o| (o.tool_name, o.is_enabled))
                    .collect();

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

            async fn upsert_tool_catalog_entry(&self, entry: &ToolCatalogEntry) -> AppResult<()> {
                sqlx::query(UPSERT_TOOL_CATALOG_ENTRY_SQL)
                    .bind(&entry.id)
                    .bind(&entry.tool_name)
                    .bind(&entry.display_name)
                    .bind(&entry.description)
                    .bind(entry.category.as_str())
                    .bind(entry.is_enabled_by_default)
                    .bind(&entry.requires_provider)
                    .bind(entry.min_plan.as_str())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert tool catalog entry: {e}"))
                    })?;

                Ok(())
            }

            async fn delete_tool_catalog_entry(&self, tool_name: &str) -> AppResult<bool> {
                let result = sqlx::query(DELETE_TOOL_CATALOG_ENTRY_SQL)
                    .bind(tool_name)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete tool catalog entry: {e}"))
                    })?;

                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_tool_selection_repository;
