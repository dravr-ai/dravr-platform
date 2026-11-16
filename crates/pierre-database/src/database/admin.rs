// ABOUTME: SQLite-backed AdminRepository, emitted from the shared body in repositories/admin.rs
// ABOUTME: SQLite stores the caller's address as plain TEXT, so the shell resolves the inet statements with no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::normalize_email;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use super::Database;
use crate::backends::shared::transactions::TransactionGuard;
use crate::repositories::admin::{
    admin_statements_sql, admin_token_columns, admin_token_from_row, admin_token_usage_from_row,
    count_to_column, deleted_account_column, impl_admin_repository, mint_admin_token,
    replacement_request, token_takes_a_caller_sql, AdminRepository, MintedAdminToken,
    TokenRotation, CLAIM_ADMIN_TOKEN_SQL, CREATE_ADMIN_TOKEN_SQL, DEACTIVATE_ADMIN_TOKEN_SQL,
    RECORD_PROVISIONED_KEY_SQL,
};
use crate::repositories::users::admin_removal_lock_sql;
use crate::repositories::uuid_columns::TextUuid;

// This backend's resolved address statements: last_used_ip and ip_address
// are TEXT columns here, so the bind needs no cast and the column reads as
// it is.
admin_statements_sql!("", "");

impl_admin_repository!(Database, SqliteRow, TextUuid, "BEGIN IMMEDIATE", "");
