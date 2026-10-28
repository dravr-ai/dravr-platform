// ABOUTME: PostgreSQL-backed ArchiveErasureRepository, emitted from the shared body in repositories/
// ABOUTME: The outbox of deleted accounts whose imported history dravr-barrage still holds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;

use super::PostgresDatabase;
use crate::repositories::archive_erasures::{
    archive_erasure_column_error, impl_archive_erasure_repository, ArchiveErasureRepository,
    PendingArchiveErasure, COMPLETE_ARCHIVE_ERASURE_SQL, DUE_ARCHIVE_ERASURES_SQL,
    RECORD_ARCHIVE_ERASURE_FAILURE_SQL,
};

impl_archive_erasure_repository!(PostgresDatabase);
