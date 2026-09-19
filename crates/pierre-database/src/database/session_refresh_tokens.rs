// ABOUTME: SQLite-backed SessionRefreshTokenRepository, emitted from the shared implementation in repositories/session_refresh_tokens.rs
// ABOUTME: user_id is a TEXT column here, so the shared statements take no uuid cast on either side of the seam

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::SessionRefreshToken;
use uuid::Uuid;

use crate::backends::shared::encryption::HasEncryption;
use crate::database::Database;
use crate::repositories::session_refresh_tokens::{
    consume_token_sql, impl_session_refresh_token_repository, revoke_user_tokens_sql,
    store_token_sql, token_from_row, REVOKE_TOKEN_FAMILY_SQL,
};
use crate::repositories::SessionRefreshTokenRepository;

impl_session_refresh_token_repository!(Database, "", "");
