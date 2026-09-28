// ABOUTME: The SQLite admin configuration repository — the shared body over the shared statements
// ABOUTME: Supplies the SqliteRow row type and the text uuid codec; no SQL of its own
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use pierre_config::admin_types::{
    AdminConfigCategory, ConfigDataType, ConfigOverride, ConfigScope,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::normalize_email;
use uuid::Uuid;

use super::backend::{column, impl_admin_config_repository};
use super::repository::{
    AdminConfigRepository, LogChangeParams, SetOverrideParams, ACTIVE_CATEGORIES_SQL,
    DELETE_GLOBAL_CATEGORY_SQL, DELETE_GLOBAL_OVERRIDE_SQL, DELETE_TENANT_CATEGORY_SQL,
    DELETE_TENANT_OVERRIDE_SQL, DELETE_USER_CATEGORY_SQL, DELETE_USER_OVERRIDE_SQL,
    GLOBAL_OVERRIDES_SQL, GLOBAL_OVERRIDE_SQL, LOG_CHANGE_SQL, SET_GLOBAL_OVERRIDE_SQL,
    SET_TENANT_OVERRIDE_SQL, SET_USER_OVERRIDE_SQL, TENANTS_OVERRIDING_SQL, TENANT_OVERRIDES_SQL,
    TENANT_OVERRIDE_SQL, USER_OVERRIDES_SQL, USER_OVERRIDE_SQL,
};
use pierre_database::repositories::uuid_columns::TextUuid;
use sqlx::sqlite::SqliteRow;
use sqlx::SqlitePool;

/// `SQLite`-backed admin configuration manager
pub struct AdminConfigManager {
    pool: SqlitePool,
}

impl AdminConfigManager {
    /// Create a new admin config manager
    #[must_use]
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl_admin_config_repository!(AdminConfigManager, SqliteRow, TextUuid);
