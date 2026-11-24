// ABOUTME: PostgreSQL-backed RoutineRepository, emitted from the shared body in repositories/
// ABOUTME: tenant_id, id and owner_user_id are native UUID columns here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Routine, RoutineKind, TenantId};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::routines::{
    impl_routine_repository, NextFiring, RoutineRepository, ACTIVE_ROUTINES_SQL,
    BUILTIN_ROUTINE_SQL, RECORD_ROUTINE_FIRED_SQL, ROUTINE_SQL, SCHEDULE_ROUTINE_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_routine_repository!(PostgresDatabase, NativeUuid, PgRow);
