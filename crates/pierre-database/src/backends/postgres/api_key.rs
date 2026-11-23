// ABOUTME: PostgreSQL-backed ApiKeyRepository, emitted from the shared implementation in repositories/api_keys.rs
// ABOUTME: user_id is a native uuid column here, and a usage row's UTC day is its TIMESTAMPTZ rendered at UTC
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{normalize_email, ApiKey, ApiKeyRequestDay};
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::api_keys::{
    agent_turn_endpoint, api_key_from_row, api_key_request_days_sql, filtered_api_keys_sql,
    impl_api_key_repository, rate_limit_column, request_day_from_row, ApiKeyRepository,
    CLEANUP_EXPIRED_API_KEYS_SQL, CREATE_API_KEY_SQL, DEACTIVATE_API_KEY_SQL,
    GET_ANY_API_KEY_BY_ID_SQL, GET_API_KEY_BY_PREFIX_SQL, GET_USER_API_KEYS_SQL,
    GET_USER_API_KEY_BY_ID_SQL, MCP_ENDPOINT_PATTERNS, REST_ENDPOINT_PATTERN, SUCCESS_STATUS_BAND,
    UPDATE_API_KEY_LAST_USED_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_api_key_repository!(
    PostgresDatabase,
    NativeUuid,
    day = "TO_CHAR(u.timestamp AT TIME ZONE 'UTC', 'YYYY-MM-DD')"
);
