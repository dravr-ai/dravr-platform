// ABOUTME: PostgreSQL-backed ImpersonationRepository, emitted from the shared body in repositories/impersonation.rs
// ABOUTME: Postgres stores the two user ids as native uuid columns, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::impersonation::ImpersonationSession;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::impersonation::{
    impl_impersonation_repository, session_column_error, ImpersonationRepository,
    CREATE_SESSION_SQL, END_ALL_SESSIONS_SQL, END_SESSION_SQL, GET_ACTIVE_SESSION_SQL,
    GET_SESSION_SQL, LIST_SESSIONS_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_impersonation_repository!(PostgresDatabase, PgRow, NativeUuid);
