// ABOUTME: Trait abstraction for admin configuration database backends
// ABOUTME: Enables SQLite and PostgreSQL implementations behind a common async interface
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::types::{
    AdminConfigCategory, ConfigAuditEntry, ConfigAuditFilter, ConfigDataType, ConfigOverride,
};
use async_trait::async_trait;
use pierre_core::errors::AppResult;

/// Database-agnostic repository for admin configuration operations
///
/// Implemented by `AdminConfigManager` (`SQLite`) and `PostgresAdminConfigManager` (`PostgreSQL`).
#[async_trait]
pub trait AdminConfigRepository: Send + Sync {
    /// Get all configuration overrides, optionally filtered by tenant
    async fn get_overrides(&self, tenant_id: Option<&str>) -> AppResult<Vec<ConfigOverride>>;

    /// Get a specific configuration override
    async fn get_override(
        &self,
        category: &str,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<ConfigOverride>>;

    /// Get effective value for a config key (tenant override > system override > default)
    async fn get_effective_override(
        &self,
        category: &str,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<ConfigOverride>>;

    /// Set a configuration override
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
    ) -> AppResult<ConfigOverride>;

    /// Delete a configuration override (reset to default)
    async fn delete_override(
        &self,
        category: &str,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<bool>;

    /// Delete all overrides for a category
    async fn delete_category_overrides(
        &self,
        category: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<usize>;

    /// Record a configuration change in the audit log
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
    ) -> AppResult<String>;

    /// Get audit log entries with filtering and pagination
    async fn get_audit_log(
        &self,
        filter: &ConfigAuditFilter,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<ConfigAuditEntry>, usize)>;

    /// Get all configuration categories
    async fn get_categories(&self) -> AppResult<Vec<AdminConfigCategory>>;
}
