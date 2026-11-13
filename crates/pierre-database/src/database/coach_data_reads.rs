// ABOUTME: SQLite-backed CoachDataReadRepository, emitted from the shared body in repositories/coach_data_reads.rs
// ABOUTME: SQLite stores user ids as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::coach_data_reads::{
    coach_read_error, impl_coach_data_read_repository, read_day, CoachDataRead,
    CoachDataReadRepository, CoachReadClass, NewCoachDataRead, LIST_COACH_READS_SQL,
    PURGE_COACH_READS_SQL, RECORD_COACH_READ_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_coach_data_read_repository!(Database, TextUuid);
