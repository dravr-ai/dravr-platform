// ABOUTME: PostgreSQL implementation of ActivityCacheRepository, emitted from the shared body in repositories/activity_cache.rs
// ABOUTME: activity_fetch_freshness and activity_backfill_coverage type their ids as UUID: binds cast up, the join casts down to text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::activity_cache::sql::{
    capture_freshness_snapshot_sql, clamp_backfill_coverage_sql, get_backfill_coverage_sql,
    latest_fetch_failure_sql, latest_fetch_mark_sql, record_activity_fetch_failure_sql,
    record_activity_fetch_sql, upsert_backfill_coverage_sql, DELETE_CACHED_ACTIVITIES_BETWEEN_SQL,
    DELETE_CACHED_ACTIVITY_SQL, DELETE_PROVIDER_ACTIVITIES_SQL, GET_CACHED_ACTIVITIES_SQL,
    GET_CACHED_ACTIVITY_ROWS_SQL, GET_CACHED_ACTIVITY_SQL, LATEST_ANY_SYNC_SQL,
    LATEST_PROVIDER_SYNC_SQL, PRUNE_ACTIVITIES_SQL, UPSERT_CACHED_ACTIVITY_SQL,
};
use crate::repositories::activity_cache::{
    activity_from_row, backfill_coverage_from_row, cached_activity_row_from_row,
    capture_freshness_from_row, fetch_failure_from_row, impl_activity_cache_repository, later,
    sport_type_string, timestamp_column, ActivityCacheRepository, ActivityFetchFailureRecord,
    BackfillCoverage, CachedActivityRow, CaptureFreshness,
};

impl_activity_cache_repository!(PostgresDatabase, "::uuid", "::text");
