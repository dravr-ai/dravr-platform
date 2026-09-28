// ABOUTME: Trait abstraction for admin configuration database backends
// ABOUTME: Enables SQLite and PostgreSQL implementations behind a common async interface
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_config::admin_types::{
    AdminConfigCategory, ConfigDataType, ConfigOverride, ConfigScope,
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

    /// Tenants holding their own tenant-wide row for `key` of `category`.
    ///
    /// A system-wide write reaches every tenant except these, so the
    /// service checks a cross-parameter rule against each of them before
    /// the write lands.
    async fn tenants_overriding(&self, category: &str, key: &str) -> AppResult<Vec<String>>;

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

    /// Get all configuration categories
    async fn get_categories(&self) -> AppResult<Vec<AdminConfigCategory>>;
}

// ============================================================================
// Shared statements — one SQL text per operation and scope, both backends
// ============================================================================
//
// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
// Postgres. `tenant_id` is TEXT on both. `user_id`, `created_by` and
// `admin_user_id` reference `users(id)`: `uuid` on Postgres and TEXT on
// `SQLite`, so the shared body binds and reads them through a
// `pierre_database::repositories::uuid_columns` codec. Timestamps bind as
// `DateTime<Utc>` on both; sqlx-sqlite writes the RFC 3339 text the `SQLite`
// TEXT columns always held. A scope is a closed enum, so each one is its own
// literal statement and no SQL is built from a runtime value.

/// The columns every override read decodes.
macro_rules! override_columns {
    () => {
        "id, category, config_key, config_value, data_type, tenant_id, user_id, \
         created_by, created_at, updated_at, reason"
    };
}

/// Every tenant-wide override of one tenant.
pub(crate) const TENANT_OVERRIDES_SQL: &str = concat!(
    "SELECT ",
    override_columns!(),
    " FROM admin_config_overrides WHERE tenant_id = $1 AND user_id IS NULL
      ORDER BY category, config_key"
);

/// Every override of one user.
pub(crate) const USER_OVERRIDES_SQL: &str = concat!(
    "SELECT ",
    override_columns!(),
    " FROM admin_config_overrides WHERE user_id = $1 ORDER BY category, config_key"
);

/// Every system-wide override.
pub(crate) const GLOBAL_OVERRIDES_SQL: &str = concat!(
    "SELECT ",
    override_columns!(),
    " FROM admin_config_overrides WHERE tenant_id IS NULL AND user_id IS NULL
      ORDER BY category, config_key"
);

/// One tenant-wide override.
pub(crate) const TENANT_OVERRIDE_SQL: &str = concat!(
    "SELECT ",
    override_columns!(),
    " FROM admin_config_overrides WHERE category = $1 AND config_key = $2
      AND tenant_id = $3 AND user_id IS NULL"
);

/// One user's override.
pub(crate) const USER_OVERRIDE_SQL: &str = concat!(
    "SELECT ",
    override_columns!(),
    " FROM admin_config_overrides WHERE category = $1 AND config_key = $2 AND user_id = $3"
);

/// One system-wide override.
pub(crate) const GLOBAL_OVERRIDE_SQL: &str = concat!(
    "SELECT ",
    override_columns!(),
    " FROM admin_config_overrides WHERE category = $1 AND config_key = $2
      AND tenant_id IS NULL AND user_id IS NULL"
);

/// Tenants holding their own tenant-wide row for one key.
pub(crate) const TENANTS_OVERRIDING_SQL: &str = r"
    SELECT DISTINCT tenant_id FROM admin_config_overrides
    WHERE category = $1 AND config_key = $2 AND tenant_id IS NOT NULL AND user_id IS NULL
    ORDER BY tenant_id
";

// Each scope upserts against its own arbiter. NULL is distinct from NULL
// under `UNIQUE(category, config_key, tenant_id)`, so that constraint can
// arbitrate only the tenant-scoped row; the system-wide and per-user rows
// name their partial unique indexes, restating each predicate so both
// backends infer the index.

/// Upsert a tenant-wide override.
pub(crate) const SET_TENANT_OVERRIDE_SQL: &str = r"
    INSERT INTO admin_config_overrides
        (id, category, config_key, config_value, data_type, tenant_id, user_id, created_by, created_at, updated_at, reason)
    VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, $8, $8, $9)
    ON CONFLICT (category, config_key, tenant_id) DO UPDATE SET
        config_value = $4, data_type = $5, updated_at = $8, reason = $9
";

/// Upsert a user's override.
pub(crate) const SET_USER_OVERRIDE_SQL: &str = r"
    INSERT INTO admin_config_overrides
        (id, category, config_key, config_value, data_type, tenant_id, user_id, created_by, created_at, updated_at, reason)
    VALUES ($1, $2, $3, $4, $5, NULL, $6, $7, $8, $8, $9)
    ON CONFLICT (category, config_key, user_id) WHERE user_id IS NOT NULL DO UPDATE SET
        config_value = $4, data_type = $5, updated_at = $8, reason = $9
";

/// Upsert a system-wide override.
pub(crate) const SET_GLOBAL_OVERRIDE_SQL: &str = r"
    INSERT INTO admin_config_overrides
        (id, category, config_key, config_value, data_type, tenant_id, user_id, created_by, created_at, updated_at, reason)
    VALUES ($1, $2, $3, $4, $5, NULL, NULL, $6, $7, $7, $8)
    ON CONFLICT (category, config_key) WHERE tenant_id IS NULL AND user_id IS NULL DO UPDATE SET
        config_value = $4, data_type = $5, updated_at = $7, reason = $8
";

/// Delete one tenant-wide override.
pub(crate) const DELETE_TENANT_OVERRIDE_SQL: &str = r"
    DELETE FROM admin_config_overrides
    WHERE category = $1 AND config_key = $2 AND tenant_id = $3 AND user_id IS NULL
";

/// Delete one user's override.
pub(crate) const DELETE_USER_OVERRIDE_SQL: &str =
    "DELETE FROM admin_config_overrides WHERE category = $1 AND config_key = $2 AND user_id = $3";

/// Delete one system-wide override.
pub(crate) const DELETE_GLOBAL_OVERRIDE_SQL: &str = r"
    DELETE FROM admin_config_overrides
    WHERE category = $1 AND config_key = $2 AND tenant_id IS NULL AND user_id IS NULL
";

/// Delete a category's tenant-wide overrides.
pub(crate) const DELETE_TENANT_CATEGORY_SQL: &str = r"
    DELETE FROM admin_config_overrides
    WHERE category = $1 AND tenant_id = $2 AND user_id IS NULL
";

/// Delete a category's overrides of one user.
pub(crate) const DELETE_USER_CATEGORY_SQL: &str =
    "DELETE FROM admin_config_overrides WHERE category = $1 AND user_id = $2";

/// Delete a category's system-wide overrides.
pub(crate) const DELETE_GLOBAL_CATEGORY_SQL: &str = r"
    DELETE FROM admin_config_overrides
    WHERE category = $1 AND tenant_id IS NULL AND user_id IS NULL
";

/// Record one change in the audit log.
pub(crate) const LOG_CHANGE_SQL: &str = r"
    INSERT INTO admin_config_audit
        (id, timestamp, admin_user_id, admin_email, category, config_key,
         old_value, new_value, data_type, reason, tenant_id, user_id, ip_address, user_agent)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
";

/// The active categories, in display order.
pub(crate) const ACTIVE_CATEGORIES_SQL: &str = r"
    SELECT id, name, display_name, description, display_order, icon, is_active
    FROM admin_config_categories WHERE is_active = TRUE ORDER BY display_order
";
