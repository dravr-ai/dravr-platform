// ABOUTME: SQLite-backed OAuthTokenRepository, emitted from the shared body in repositories/user_oauth_tokens.rs
// ABOUTME: user_id is a TEXT column here, so the shared statements bind and read the uuid as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    StravaPoolApp, StravaSeatHolder, StravaTokenApp, TenantId, UserOAuthApp, UserOAuthToken,
};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::shared::encryption::{encrypt_oauth_token, HasEncryption};
use crate::database::Database;
use crate::repositories::user_oauth_tokens::{
    impl_oauth_token_repository, strava_pool_app_aad, strava_pool_app_from_row,
    strava_seat_holder_from_row, user_oauth_app_from_row, user_oauth_token_from_row,
    COUNT_STRAVA_SEAT_USAGE_BY_APP_SQL, DELETE_STRAVA_POOL_APP_SQL, DELETE_TOKENS_SQL,
    DELETE_TOKEN_SQL, FIND_USER_BY_PROVIDER_USER_ID_SQL, GET_PROVIDER_LAST_SYNC_SQL,
    GET_STRAVA_POOL_APP_SECRET_SQL, GET_TENANT_PROVIDER_TOKENS_SQL, GET_TOKENS_IN_TENANT_SQL,
    GET_TOKENS_SQL, GET_TOKEN_SQL, GET_USER_OAUTH_APP_SQL, INSERT_TOKEN_IF_ABSENT_SQL,
    LIST_ENABLED_STRAVA_POOL_APPS_SQL, LIST_STRAVA_POOL_APPS_SQL, LIST_STRAVA_SEAT_HOLDERS_SQL,
    LIST_STRAVA_TOKEN_APPS_SQL, LIST_TOKEN_PROVIDERS_SQL, LIST_USER_OAUTH_APPS_SQL,
    REFRESH_TOKEN_SQL, REMOVE_USER_OAUTH_APP_SQL, REPLACE_TOKEN_IF_CURRENT_SQL,
    SET_STRAVA_POOL_APP_ENABLED_SQL, STORE_USER_OAUTH_APP_SQL, UPDATE_PROVIDER_LAST_SYNC_SQL,
    UPSERT_STRAVA_POOL_APP_SQL, UPSERT_TOKEN_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::OAuthTokenRepository;

impl_oauth_token_repository!(Database, SqliteRow, TextUuid);
