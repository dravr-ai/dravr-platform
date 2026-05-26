// ABOUTME: Repository trait definitions for the tool selection telemetry persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

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
