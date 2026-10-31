// ABOUTME: PostgreSQL-backed UploadedActivityFileRepository, emitted from the shared implementation in repositories/uploaded_activity_files.rs
// ABOUTME: The ids are TEXT here, as on SQLite; the bytes still held are BYTEA
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::uploaded_activity_files::{
    file_activity_id_pattern, impl_uploaded_activity_file_repository, PendingUploadBytes,
    UploadedActivityFileRepository, CLEAR_DRAINED_BYTES_SQL, DELETE_UPLOADED_FILE_SQL,
    FILE_BACKS_CACHED_ACTIVITY_SQL, HOLDS_UPLOADED_FILE_SQL, PENDING_UPLOAD_BYTES_OF_SQL,
    PENDING_UPLOAD_BYTES_SQL, RECORD_UPLOADED_FILE_BYTES_SQL, RECORD_UPLOADED_FILE_SQL,
};

impl_uploaded_activity_file_repository!(PostgresDatabase);
