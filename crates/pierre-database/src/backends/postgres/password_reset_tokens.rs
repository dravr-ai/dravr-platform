// ABOUTME: PostgreSQL-backed PasswordResetRepository, emitted from the shared implementation in repositories/password_reset_tokens.rs
// ABOUTME: user_id is a native uuid column here, so the shared statements bind and read it through the native codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::password_reset_tokens::{
    impl_password_reset_repository, invalid_reset_token, CONSUME_RESET_TOKEN_SQL,
    COUNT_RECENT_RESET_TOKENS_SQL, INVALIDATE_USER_RESET_TOKENS_SQL, LOAD_RESET_TOKEN_SQL,
    LOCK_RESET_TOKEN_SQL, RECORD_RESET_ATTEMPT_SQL, RESET_MAX_VERIFY_ATTEMPTS,
    RESET_TOKEN_TTL_MINUTES, STORE_RESET_TOKEN_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::PasswordResetRepository;

impl_password_reset_repository!(PostgresDatabase, NativeUuid);
