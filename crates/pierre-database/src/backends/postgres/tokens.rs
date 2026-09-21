// ABOUTME: PostgreSQL-backed OAuth2ServerRepository, emitted from the shared implementation in repositories/tokens.rs
// ABOUTME: Every id column is TEXT and every timestamp TIMESTAMPTZ here, so the shared statements need no engine argument
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    DeviceAuthorization, OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State,
    OAuthClientGrant,
};

use crate::backends::postgres::PostgresDatabase;
use crate::backends::shared::encryption::HasEncryption;
use crate::repositories::tokens::{
    client_grant_from_row, device_authorization_from_row, impl_oauth2_server_repository,
    oauth2_auth_code_from_row, oauth2_client_from_row, oauth2_refresh_token_from_row,
    oauth2_state_from_row, APPROVE_DEVICE_AUTHORIZATION_SQL, CONSUME_OAUTH2_AUTH_CODE_SQL,
    CONSUME_OAUTH2_REFRESH_TOKEN_SQL, CONSUME_OAUTH2_STATE_SQL, CREATE_DEVICE_AUTHORIZATION_SQL,
    DELETE_DEVICE_AUTHORIZATION_SQL, DENY_DEVICE_AUTHORIZATION_SQL, FIND_ACTIVE_CLIENT_GRANT_SQL,
    GET_DEVICE_AUTHORIZATION_BY_CODE_HASH_SQL, GET_DEVICE_AUTHORIZATION_BY_USER_CODE_SQL,
    GET_OAUTH2_CLIENT_SQL, GET_OAUTH2_REFRESH_TOKEN_SQL, LIST_CLIENT_GRANTS_SQL,
    REVOKE_CLIENT_GRANT_SQL, STORE_CLIENT_GRANT_SQL, STORE_OAUTH2_AUTH_CODE_SQL,
    STORE_OAUTH2_CLIENT_SQL, STORE_OAUTH2_REFRESH_TOKEN_SQL, STORE_OAUTH2_STATE_SQL,
};
use crate::repositories::OAuth2ServerRepository;

impl_oauth2_server_repository!(PostgresDatabase);
