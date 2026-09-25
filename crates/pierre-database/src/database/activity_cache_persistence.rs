// ABOUTME: SQLite implementation of ActivityCacheRepository, emitted from the shared body in repositories/activity_cache.rs
// ABOUTME: Every identifier column is TEXT here, so the id binds and the freshness join carry no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::activity_cache::{
    activity_from_row, backfill_coverage_from_row, cached_activity_row_from_row,
    capture_freshness_from_row, capture_freshness_snapshot_sql, clamp_backfill_coverage_sql,
    get_backfill_coverage_sql, impl_activity_cache_repository, later, latest_fetch_mark_sql,
    record_activity_fetch_sql, sport_type_string, timestamp_column, upsert_backfill_coverage_sql,
    ActivityCacheRepository, BackfillCoverage, CachedActivityRow, CaptureFreshness,
    DELETE_PROVIDER_ACTIVITIES_SQL, GET_CACHED_ACTIVITIES_SQL, GET_CACHED_ACTIVITY_ROWS_SQL,
    GET_CACHED_ACTIVITY_SQL, LATEST_ANY_SYNC_SQL, LATEST_PROVIDER_SYNC_SQL, PRUNE_ACTIVITIES_SQL,
    UPSERT_CACHED_ACTIVITY_SQL,
};

impl_activity_cache_repository!(Database, "", "");
