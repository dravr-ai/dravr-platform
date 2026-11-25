// ABOUTME: SQLite-backed UploadedActivityFileRepository, emitted from the shared implementation in repositories/uploaded_activity_files.rs
// ABOUTME: Every id column is TEXT here, so the binds carry no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::iter;

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::uploaded_activity_files::{
    claim_upload_name, content_named_file, file_activity_id_pattern,
    impl_uploaded_activity_file_repository, lease_until, release_failed, ContentNamedFile,
    LeaseRelease, UploadHolder, UploadNameLock, UploadedActivityFileRepository, UploadedFileWrite,
    CLAIM_UPLOAD_NAME_SQL, DELETE_UPLOADED_FILE_SQL, FENCE_UPLOAD_NAME_SQL,
    FILE_BACKS_CACHED_ACTIVITY_SQL, FILE_OF_RECORD_BASE_SQL, HOLDS_UPLOADED_FILE_SQL,
    RECORD_BASE_OF_FILE_SQL, RECORD_UPLOADED_FILE_SQL, RELEASE_UPLOAD_NAME_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_uploaded_activity_file_repository!(Database, TextUuid);
