// ABOUTME: SQLite-backed A2ARepository, emitted from the shared implementation in repositories/a2a.rs
// ABOUTME: Ids are hyphenated TEXT, lists are JSON or comma text, and a usage row mints its own hex id here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::a2a::{
    A2AClient, A2APushNotificationConfig, A2ASession, A2ATask, A2AUsage, A2AUsageStats, TaskStatus,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use tracing::{debug, warn};
use uuid::Uuid;

use super::Database;
use crate::backends::shared::enums::{str_to_task_status, task_status_to_str};
use crate::backends::shared::transactions::TransactionGuard;
use crate::repositories::a2a::{
    record_usage_sql, task_columns, usage_history_sql, A2ARepository, ACTIVE_SESSIONS_SQL,
    CLIENT_CREDENTIALS_SQL, CLIENT_USAGE_SINCE_SQL, CREATE_SESSION_SQL, CREATE_TASK_SQL,
    DEACTIVATE_CLIENT_API_KEYS_SQL, DEACTIVATE_CLIENT_SQL, DELETE_PUSH_CONFIG_SQL,
    GET_CLIENT_BY_API_KEY_SQL, GET_CLIENT_BY_NAME_SQL, GET_CLIENT_SQL, GET_PUSH_CONFIG_SQL,
    GET_SESSION_SQL, GET_TASK_SQL, INSERT_CLIENT_API_KEY_SQL, INSERT_CLIENT_SQL,
    INVALIDATE_CLIENT_SESSIONS_SQL, LIST_ALL_CLIENTS_SQL, LIST_PUSH_CONFIGS_SQL,
    LIST_USER_CLIENTS_SQL, TOUCH_SESSION_SQL, UPDATE_TASK_STATUS_SQL, UPDATE_TASK_WIRE_STATE_SQL,
    UPSERT_PUSH_CONFIG_SQL, USAGE_STATS_SQL,
};
use crate::repositories::a2a_backend::{
    a2a_column_error, average_millis, i32_from_u32, impl_a2a_repository, list_tasks_sql,
    u32_from_count, u64_from_sum, TaskListing,
};
use crate::repositories::list_columns::TextList;
use crate::repositories::uuid_columns::TextUuid;

impl_a2a_repository!(
    Database,
    SqliteRow,
    TextUuid,
    TextList,
    day = "date(timestamp)",
    usage_id = "lower(hex(randomblob(16)))",
    inet = ""
);
