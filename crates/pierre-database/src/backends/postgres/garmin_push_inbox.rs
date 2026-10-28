// ABOUTME: PostgreSQL-backed GarminPushInboxRepository, emitted from the shared implementation in repositories/garmin_push_inbox.rs
// ABOUTME: The claim subquery takes FOR UPDATE SKIP LOCKED so a second sweep at the same instant takes other rows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::garmin_push_inbox::{
    claim_garmin_pushes_sql, impl_garmin_push_inbox_repository, push_from_row,
    GarminPushInboxRepository, GarminPushInboxRow, InboxClaim, DELETE_PROCESSED_PUSHES_SQL,
    FAIL_PUSH_SQL, FINISH_PUSH_SQL, GET_PUSH_SQL, GIVE_UP_EXHAUSTED_PUSHES_SQL, RECORD_PUSH_SQL,
    RETRY_PUSH_SQL,
};
use crate::repositories::memory_extraction_jobs::JobLease;

impl_garmin_push_inbox_repository!(PostgresDatabase, "FOR UPDATE SKIP LOCKED ");
