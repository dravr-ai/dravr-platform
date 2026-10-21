// ABOUTME: SQLite-backed FirstRunProgressRepository, emitted from the shared body in repositories/
// ABOUTME: user_id and tenant_id are TEXT columns here, so the shared statements bind them as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::first_run_progress::{
    first_run_day_from_row, impl_first_run_progress_repository, local_date_text, FirstRunDay,
    FirstRunDayRow, FirstRunProgressRepository, FIRST_RUN_DAYS_SQL, RECORD_FIRST_RUN_DAY_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_first_run_progress_repository!(Database, TextUuid);
