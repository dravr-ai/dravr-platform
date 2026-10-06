// ABOUTME: SQLite-backed ProviderDataRepository, emitted from the shared implementation in repositories/provider_data.rs
// ABOUTME: Every id column the purge compares is TEXT here, so the shared statements bind hyphenated ids

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::provider_data::{
    impl_provider_data_repository, insert_purge_record, purge_record_from_row, ProviderCacheExpiry,
    ProviderDataPurge, ProviderDataPurgeRecord, ProviderDataRepository, PurgeReason,
    EXPIRED_SCOPES_SQL, INSERT_PURGE_RECORD_SQL, LIST_PURGE_RECORDS_SQL, PROVIDER_PURGE_SQL,
    USER_EXPIRED_ROWS_SQL, USER_PROVIDER_ROWS_SQL, USER_SYNC_STATE_SQL,
};

impl_provider_data_repository!(Database);
