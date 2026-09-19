// ABOUTME: PostgreSQL-backed OAuthClientStateRepository, emitted from the shared implementation in repositories/oauth_client_state.rs
// ABOUTME: The CSRF `state` and PKCE verifier minted at authorization start, consumed once at callback, reaped once expired

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthClientState;
use sqlx::Row;
use std::collections::BTreeMap;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::oauth_client_state::{
    client_state_from_row, impl_oauth_client_state_repository, CONSUME_CLIENT_STATE_SQL,
    REAP_EXPIRED_CLIENT_STATES_SQL, STORE_CLIENT_STATE_SQL,
};
use crate::repositories::OAuthClientStateRepository;

impl_oauth_client_state_repository!(PostgresDatabase);
