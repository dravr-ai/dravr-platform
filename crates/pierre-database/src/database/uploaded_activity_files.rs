// ABOUTME: SQLite-backed UploadedActivityFileRepository, emitted from the shared implementation in repositories/uploaded_activity_files.rs
// ABOUTME: Every id column is TEXT and the file is a BLOB here, so the binds carry no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::uploaded_activity_files::{
    impl_uploaded_activity_file_repository, UploadedActivityFileRepository,
    DELETE_UPLOADED_FILE_SQL, GET_UPLOADED_FILE_SQL, HAS_UPLOADED_FILES_SQL,
    STORE_UPLOADED_FILE_SQL,
};

impl_uploaded_activity_file_repository!(Database);
