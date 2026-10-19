// ABOUTME: SQLite-backed PasswordResetRepository, emitted from the shared implementation in repositories/password_reset_tokens.rs
// ABOUTME: user_id is a TEXT column here, so the shared statements bind the uuid as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::link_token_consume::consume_link_token;
use crate::repositories::password_reset_tokens::{
    impl_password_reset_repository, COUNT_RECENT_RESET_TOKENS_SQL,
    INVALIDATE_USER_RESET_TOKENS_SQL, RESET_LINK_TOKEN, RESET_TOKEN_TTL_MINUTES,
    STORE_RESET_TOKEN_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::PasswordResetRepository;

impl_password_reset_repository!(Database, TextUuid);
