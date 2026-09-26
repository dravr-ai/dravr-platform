// ABOUTME: PostgreSQL-backed UsageRepository, emitted from the shared implementation in repositories/analytics.rs
// ABOUTME: User ids are native uuids here, a usage row's SERIAL id is left to the table, and an IP address is cast to inet
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::constants::http_status::{BAD_REQUEST, SUCCESS_MAX, SUCCESS_MIN};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    ApiKeyUsage, ApiKeyUsageStats, ApiKeyWindowUsage, JwtMonthlyUsage, JwtUsage, RequestLog,
    ToolUsage,
};
use sqlx::Row;
use tracing::warn;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::analytics::{
    api_key_status_column, current_utc_month_start, impl_usage_repository,
    record_api_key_usage_sql, record_jwt_usage_sql, request_log_from_row, request_logs_sql,
    saturating_i32, success_ratio, u32_from_count, u64_from_sum, usage_column_error,
    RequestLogFilters, API_KEY_STATS_SQL, API_KEY_TOOL_USAGE_SQL, API_KEY_WINDOW_USAGE_SQL,
    JWT_CURRENT_USAGE_SQL, TOP_TOOLS_SQL,
};
use crate::repositories::user_rate_limit_overrides::monthly_override_from_columns;
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::UsageRepository;

impl_usage_repository!(
    PostgresDatabase,
    NativeUuid,
    usage_id = "DEFAULT",
    inet = "::inet"
);
