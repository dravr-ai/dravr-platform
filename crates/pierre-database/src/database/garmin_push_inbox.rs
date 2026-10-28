// ABOUTME: SQLite-backed GarminPushInboxRepository, emitted from the shared implementation in repositories/garmin_push_inbox.rs
// ABOUTME: Due rows are claimed in one UPDATE … RETURNING over a LIMITed subquery; SQLite needs no lock clause

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::garmin_push_inbox::{
    claim_garmin_pushes_sql, impl_garmin_push_inbox_repository, push_from_row,
    GarminPushInboxRepository, GarminPushInboxRow, InboxClaim, DELETE_PROCESSED_PUSHES_SQL,
    FAIL_PUSH_SQL, FINISH_PUSH_SQL, GET_PUSH_SQL, GIVE_UP_EXHAUSTED_PUSHES_SQL, RECORD_PUSH_SQL,
    RETRY_PUSH_SQL,
};
use crate::repositories::memory_extraction_jobs::JobLease;

impl_garmin_push_inbox_repository!(Database, "");
