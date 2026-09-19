// ABOUTME: SQLite-backed UserToolOverrideRepository, emitted from the shared body in repositories/user_tool_overrides.rs
// ABOUTME: SQLite stores user_id and set_by as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::user_tool_overrides::{
    impl_user_tool_override_repository, tool_override_column_error, UserToolOverride,
    UserToolOverrideRepository, DELETE_TOOL_OVERRIDE_SQL, GET_TOOL_OVERRIDE_SQL,
    LIST_TOOL_OVERRIDES_SQL, UPSERT_TOOL_OVERRIDE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_user_tool_override_repository!(Database, SqliteRow, TextUuid);
