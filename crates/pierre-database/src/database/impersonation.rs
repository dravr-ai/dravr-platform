// ABOUTME: SQLite-backed ImpersonationRepository, emitted from the shared body in repositories/impersonation.rs
// ABOUTME: SQLite stores the two user ids as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::impersonation::{BoundCredential, ImpersonationSession};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use super::Database;
use crate::repositories::impersonation::{
    impl_impersonation_repository, session_column_error, ImpersonationRepository, SessionCursor,
    CREATE_SESSION_SQL, END_ALL_SESSIONS_SQL, END_SESSION_SQL, GET_ACTIVE_API_KEY_SESSION_SQL,
    GET_ACTIVE_GRANT_SESSION_SQL, GET_ACTIVE_SESSION_SQL, GET_SESSION_SQL, GRANT_LIVE_SQL,
    GRANT_UNEXPIRED_SQL, LIST_SESSIONS_PAGE_SQL,
};
use crate::repositories::support_binding::impl_support_binding_ends;
use crate::repositories::uuid_columns::TextUuid;

impl_impersonation_repository!(Database, SqliteRow, TextUuid);
impl_support_binding_ends!(Database, SqliteRow, TextUuid);
