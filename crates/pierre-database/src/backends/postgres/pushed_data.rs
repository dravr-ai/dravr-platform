// ABOUTME: PostgreSQL-backed PushedDataRepository, emitted from the shared implementation in repositories/pushed_data.rs
// ABOUTME: The file bytes bind as BYTEA and the instants as TIMESTAMPTZ; every id column is TEXT, as on SQLite
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession, TenantId,
};
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::activity_cache::sport_type_string;
use crate::repositories::pushed_data::{
    impl_pushed_data_repository, PushedDataRepository, DROP_PUSHED_FILE_SQL,
    DROP_STALE_PUSHED_FILES_SQL, KEEP_PUSHED_FILE_SQL, KEPT_PUSHED_FILE_SQL, MERGE_PUSHED_BODY_SQL,
    MERGE_PUSHED_RECOVERY_SQL, UPSERT_PUSHED_ACTIVITY_SQL, UPSERT_PUSHED_SLEEP_SQL,
};

impl_pushed_data_repository!(PostgresDatabase);
