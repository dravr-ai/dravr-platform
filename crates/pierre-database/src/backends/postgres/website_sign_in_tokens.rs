// ABOUTME: PostgreSQL-backed WebsiteSignInTokenRepository, emitted from the shared implementation in repositories/website_sign_in_tokens.rs
// ABOUTME: user_id is a native uuid column here, so the shared statements bind the uuid as itself

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::link_token_consume::consume_link_token;
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::website_sign_in_tokens::{
    impl_website_sign_in_token_repository, COUNT_RECENT_SIGN_IN_TOKENS_SQL, SIGN_IN_LINK_TOKEN,
    STORE_SIGN_IN_TOKEN_SQL,
};
use crate::repositories::WebsiteSignInTokenRepository;

impl_website_sign_in_token_repository!(PostgresDatabase, NativeUuid);
