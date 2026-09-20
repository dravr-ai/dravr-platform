// ABOUTME: PostgreSQL-backed EmailVerificationRepository, emitted from the shared implementation in repositories/email_verification_tokens.rs
// ABOUTME: user_id is a native uuid column here, so the shared statements bind the uuid as itself
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::email_verification_tokens::{
    impl_email_verification_repository, invalid_verification_token, CONSUME_VERIFICATION_TOKEN_SQL,
    COUNT_RECENT_VERIFICATION_TOKENS_SQL, IS_EMAIL_VERIFIED_SQL, LOAD_VERIFICATION_TOKEN_SQL,
    LOCK_VERIFICATION_TOKEN_SQL, MARK_EMAIL_VERIFIED_SQL, RECORD_VERIFICATION_ATTEMPT_SQL,
    STORE_VERIFICATION_TOKEN_SQL, VERIFY_MAX_ATTEMPTS,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::EmailVerificationRepository;

impl_email_verification_repository!(PostgresDatabase, NativeUuid);
