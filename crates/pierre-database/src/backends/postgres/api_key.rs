// ABOUTME: PostgreSQL-backed ApiKeyRepository, emitted from the shared implementation in repositories/api_keys.rs
// ABOUTME: user_id is a native uuid column here, so the shared statements bind the uuid as itself
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::ApiKey;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::api_keys::{
    api_key_from_row, filtered_api_keys_sql, impl_api_key_repository, rate_limit_column,
    ApiKeyRepository, CLEANUP_EXPIRED_API_KEYS_SQL, CREATE_API_KEY_SQL, DEACTIVATE_API_KEY_SQL,
    GET_ANY_API_KEY_BY_ID_SQL, GET_API_KEY_BY_PREFIX_SQL, GET_USER_API_KEYS_SQL,
    GET_USER_API_KEY_BY_ID_SQL, UPDATE_API_KEY_LAST_USED_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_api_key_repository!(PostgresDatabase, NativeUuid);
