// ABOUTME: PostgreSQL-backed CoachDataReadRepository, emitted from the shared body in repositories/coach_data_reads.rs
// ABOUTME: Postgres stores user ids as native uuid columns, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::coach_data_reads::{
    coach_read_error, impl_coach_data_read_repository, parse_transport, read_day, CoachDataRead,
    CoachDataReadRepository, CoachReadClass, NewCoachDataRead, LIST_COACH_READS_SINCE_SQL,
    LIST_COACH_READS_SQL, PURGE_COACH_READS_SQL, RECORD_COACH_READ_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_coach_data_read_repository!(PostgresDatabase, PgRow, NativeUuid);
