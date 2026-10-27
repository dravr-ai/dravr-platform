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
    impl_email_verification_repository, COUNT_RECENT_VERIFICATION_TOKENS_SQL,
    IS_EMAIL_VERIFIED_SQL, MARK_EMAIL_VERIFIED_SQL, STORE_VERIFICATION_TOKEN_SQL,
    VERIFICATION_LINK_TOKEN, VERIFICATION_TOKEN_OWNER_SQL,
};
use crate::repositories::link_token_consume::consume_link_token;
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::EmailVerificationRepository;

impl_email_verification_repository!(PostgresDatabase, NativeUuid);
