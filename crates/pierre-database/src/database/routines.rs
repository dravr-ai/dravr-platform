// ABOUTME: SQLite-backed RoutineRepository, emitted from the shared body in repositories/
// ABOUTME: tenant_id, id and owner_user_id are TEXT columns here, so the shared statements bind them as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Routine, RoutineKind, TenantId};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::routines::{
    impl_routine_repository, NextFiring, RoutineRepository, ACTIVE_ROUTINES_SQL,
    BUILTIN_ROUTINE_SQL, RECORD_ROUTINE_FIRED_SQL, ROUTINE_SQL, SCHEDULE_ROUTINE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_routine_repository!(Database, TextUuid, SqliteRow);
