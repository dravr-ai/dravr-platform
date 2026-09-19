// ABOUTME: PostgreSQL-backed AdminRepository, emitted from the shared body in repositories/admin.rs
// ABOUTME: Postgres stores the caller's address in INET columns, so the shell resolves the inet statements with ::inet
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Duration, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminPermissions, AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::admin::{AdminJwtManager, TokenScope};
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;
use tracing::debug;

use super::PostgresDatabase;
use crate::repositories::admin::{
    admin_statements_sql, admin_token_columns, admin_token_from_row, admin_token_usage_from_row,
    count_to_column, impl_admin_repository, new_admin_token_id, provisioned_key_from_row,
    AdminRepository, CREATE_ADMIN_TOKEN_SQL, DEACTIVATE_ADMIN_TOKEN_SQL,
    GET_ALL_PROVISIONED_KEYS_SQL, GET_PROVISIONED_KEYS_FOR_TOKEN_SQL, RECORD_PROVISIONED_KEY_SQL,
};

// This backend's resolved address statements: last_used_ip and ip_address
// are INET columns here, so the text bind is cast on the way in and the
// column is rendered with host() on the way out.
admin_statements_sql!("::inet", "host");

impl_admin_repository!(PostgresDatabase);
