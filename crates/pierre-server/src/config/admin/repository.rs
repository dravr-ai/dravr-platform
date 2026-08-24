// ABOUTME: Trait abstraction for admin configuration database backends
// ABOUTME: Enables SQLite and PostgreSQL implementations behind a common async interface
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_config::admin_types::{
    AdminConfigCategory, ConfigAuditEntry, ConfigAuditFilter, ConfigDataType, ConfigOverride,
    ConfigScope,
};
use pierre_core::errors::AppResult;

/// Parameters for [`AdminConfigRepository::set_override`].
///
/// Bundled so the trait method doesn't carry a seven-arg positional
/// signature; callers thread the same set of identifiers
/// (category/key/value/`data_type`/admin/tenant/reason) for both `SQLite`
/// and `PostgreSQL` backends.
pub struct SetOverrideParams<'a> {
    /// Top-level config category (e.g. `"usage_quotas"`).
    pub category: &'a str,
    /// Config key within `category`.
    pub key: &'a str,
    /// New value to persist.
    pub value: &'a serde_json::Value,
    /// Declared data type used for validation + downstream coercion.
    pub data_type: ConfigDataType,
    /// Admin user performing the change (audit attribution).
    pub admin_user_id: &'a str,
    /// Which single row this write targets: system-wide, one tenant, or
    /// one user.
    pub scope: ConfigScope<'a>,
    /// Free-form reason captured in the audit log.
    pub reason: Option<&'a str>,
}

/// Parameters for [`AdminConfigRepository::log_change`].
///
/// Mirrors the audit-log row fields so the trait method body can decompose
/// with `let LogChangeParams { ... } = params;` instead of an eleven-arg
/// list.
pub struct LogChangeParams<'a> {
    /// Admin user performing the change.
    pub admin_user_id: &'a str,
    /// Admin email (operator-visible attribution).
    pub admin_email: &'a str,
    /// Top-level config category.
    pub category: &'a str,
    /// Config key within `category`.
    pub key: &'a str,
    /// Previous value, when one existed.
    pub old_value: Option<&'a serde_json::Value>,
    /// New value being recorded.
    pub new_value: &'a serde_json::Value,
    /// Declared data type used for validation + downstream coercion.
    pub data_type: ConfigDataType,
    /// Free-form reason captured in the audit log.
    pub reason: Option<&'a str>,
    /// Which row the change targeted, recorded so a per-user edit stays
    /// distinguishable from a system-wide one after the fact.
    pub scope: ConfigScope<'a>,
    /// Client IP captured at request time for forensic tracing.
    pub ip_address: Option<&'a str>,
    /// Client user-agent captured at request time.
    pub user_agent: Option<&'a str>,
}

/// Database-agnostic repository for admin configuration operations
///
/// Implemented by `AdminConfigManager` (`SQLite`) and `PostgresAdminConfigManager` (`PostgreSQL`).
#[async_trait]
pub trait AdminConfigRepository: Send + Sync {
    /// Get the rows stored at exactly `scope`. Composing the scopes a lookup
    /// spans is the service's job — it also owns the environment rung, which
    /// no repository can see.
    async fn get_overrides_at(&self, scope: ConfigScope<'_>) -> AppResult<Vec<ConfigOverride>>;

    /// Get the override stored at exactly `scope`, without walking to a
    /// broader one.
    async fn get_override(
        &self,
        category: &str,
        key: &str,
        scope: ConfigScope<'_>,
    ) -> AppResult<Option<ConfigOverride>>;

    /// Set a configuration override at the scope named in `params`
    async fn set_override(&self, params: SetOverrideParams<'_>) -> AppResult<ConfigOverride>;

    /// Delete a configuration override (reset to the next-broadest scope)
    async fn delete_override(
        &self,
        category: &str,
        key: &str,
        scope: ConfigScope<'_>,
    ) -> AppResult<bool>;

    /// Delete all overrides for a category at `scope`
    async fn delete_category_overrides(
        &self,
        category: &str,
        scope: ConfigScope<'_>,
    ) -> AppResult<usize>;

    /// Record a configuration change in the audit log
    async fn log_change(&self, params: LogChangeParams<'_>) -> AppResult<String>;

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
