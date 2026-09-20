// ABOUTME: SQLite-backed health persistence (data sources, sleep, recovery, snapshots, sync cursors, time series), emitted from the shared repositories modules
// ABOUTME: user_oauth_tokens.tenant_id is hyphenated TEXT here, so the connected-users read takes the column as it is
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use dravr_riviere::aggregation::aggregate_windows;
use dravr_riviere::{
    AggregatedPoint, Aggregation, DataPoint, QueryResult, RiviereError, TimeRange, TimeSeriesStore,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    DataSource, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession, TenantId,
};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::health_persistence::{
    data_source_from_row, device_type_to_str, health_metrics_from_row,
    impl_health_persistence_repositories, recovery_metrics_from_row, sleep_session_from_row,
    tenant_from_row, DELETE_DATA_SOURCE_SQL, DELETE_HEALTH_SNAPSHOT_SQL,
    DELETE_RECOVERY_METRIC_SQL, DELETE_SLEEP_SESSIONS_SQL, DELETE_SLEEP_SESSION_SQL,
    FIND_HEALTH_SNAPSHOT_TENANT_SQL, FIND_RECOVERY_METRIC_TENANT_SQL,
    FIND_SLEEP_SESSION_TENANT_SQL, GET_DATA_SOURCE_SQL, GET_HEALTH_SNAPSHOTS_SQL,
    GET_RECOVERY_METRICS_SQL, GET_SLEEP_SESSIONS_SQL, LATEST_HEALTH_SNAPSHOT_SQL,
    LATEST_RECOVERY_SQL, LATEST_SLEEP_SESSION_SQL, LIST_DATA_SOURCES_BY_PROVIDER_SQL,
    LIST_DATA_SOURCES_SQL, SOFT_DELETE_HEALTH_SNAPSHOT_SQL, SOFT_DELETE_RECOVERY_METRIC_SQL,
    SOFT_DELETE_SLEEP_SESSION_SQL, UPSERT_DATA_SOURCE_SQL, UPSERT_HEALTH_SNAPSHOT_SQL,
    UPSERT_RECOVERY_METRICS_SQL, UPSERT_SLEEP_SESSION_SQL,
};
use crate::repositories::sync_cursors::{
    connected_user_from_row, impl_sync_cursor_repository, list_connected_provider_users_sql,
    sync_cursor_from_row, GET_SYNC_CURSOR_SQL, UPSERT_SYNC_CURSOR_SQL,
};
use crate::repositories::time_series::{
    data_point_from_row, impl_time_series_store, TS_DELETE_RANGE_SQL, TS_FETCH_RANGE_SQL,
    TS_INSERT_POINT_SQL, TS_LATEST_SQL,
};
use crate::repositories::{
    ConnectedUserRow, DataSourceRepository, HealthSnapshotRepository, RecoveryRepository,
    SleepRepository, SyncCursorRepository, SyncCursorRow,
};

impl_health_persistence_repositories!(Database);
impl_sync_cursor_repository!(Database, "tenant_id");
impl_time_series_store!(Database);
