// ABOUTME: SQLite-backed UserRateLimitOverrideRepository, emitted from the shared body in repositories/user_rate_limit_overrides.rs
// ABOUTME: SQLite stores user_id and set_by as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::user_rate_limit_overrides::{
    impl_user_rate_limit_override_repository, limit_from_column, limit_to_column,
    rate_limit_column_error, UserRateLimitOverride, UserRateLimitOverrideRepository,
    DELETE_RATE_LIMIT_OVERRIDE_SQL, GET_RATE_LIMIT_OVERRIDE_SQL, UPSERT_RATE_LIMIT_OVERRIDE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_user_rate_limit_override_repository!(Database, TextUuid);
