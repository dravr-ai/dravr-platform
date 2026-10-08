// ABOUTME: PostgreSQL-backed CoachAccessRequestRepository, emitted from the shared implementation in repositories/coach_access_requests.rs
// ABOUTME: Supplies the Postgres row type and the native uuid codec; everything else is shared

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoachAccessRequest, CoachAccessStatus};
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::coach_access_requests::{
    impl_coach_access_request_repository, request_from_row, DECIDE_REQUEST_SQL, GET_REQUEST_SQL,
    LATEST_FOR_USER_SQL, LIST_BY_STATUS_SQL, OPEN_REQUEST_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::CoachAccessRequestRepository;

impl_coach_access_request_repository!(PostgresDatabase, PgRow, NativeUuid);
