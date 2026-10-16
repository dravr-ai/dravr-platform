// ABOUTME: SQLite-backed EmailVerificationRepository, emitted from the shared implementation in repositories/email_verification_tokens.rs
// ABOUTME: user_id is a TEXT column here, so the shared statements bind the uuid as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::email_verification_tokens::{
    impl_email_verification_repository, COUNT_RECENT_VERIFICATION_TOKENS_SQL,
    IS_EMAIL_VERIFIED_SQL, MARK_EMAIL_VERIFIED_SQL, STORE_VERIFICATION_TOKEN_SQL,
    VERIFICATION_LINK_TOKEN,
};
use crate::repositories::link_token_consume::consume_link_token;
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::EmailVerificationRepository;

impl_email_verification_repository!(Database, TextUuid);
