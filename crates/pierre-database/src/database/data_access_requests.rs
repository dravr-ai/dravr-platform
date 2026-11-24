// ABOUTME: SQLite-backed DataAccessRequestRepository, emitted from the shared implementation in repositories/data_access_requests.rs
// ABOUTME: Supplies the SQLite row type and the TEXT uuid codec; everything else is shared

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::data_access_requests::{
    due_soon_horizon, impl_data_access_request_repository, request_from_row, CLAIM_DUE_SOON_SQL,
    CLAIM_OVERDUE_SQL, DUE_SOON_SQL, GET_SQL, INSERT_SQL, LIST_SQL, OVERDUE_SQL, UPDATE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::{
    DataAccessRequest, DataAccessRequestRepository, DataAccessStatus, ReminderStage,
};

impl_data_access_request_repository!(Database, SqliteRow, TextUuid);
