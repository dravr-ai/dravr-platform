// ABOUTME: SQLite-backed SignupCodeRepository, emitted from the shared implementation in repositories/signup_codes.rs
// ABOUTME: user_id, created_by and granted_by are TEXT columns here, bound through the TextUuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{SignupCode, SignupCodeRedemption, SignupCodeSource};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::signup_codes::{
    impl_signup_code_repository, redemption_from_row, signup_code_from_row, source_columns,
    CLAIM_SIGNUP_CODE_SQL, FIND_LIVE_SIGNUP_CODE_SQL, GET_REDEMPTION_SQL, INSERT_REDEMPTION_SQL,
    INSERT_SIGNUP_CODE_SQL, LIST_REDEMPTIONS_SQL, LIST_SIGNUP_CODES_SQL,
    MARK_REDEMPTION_ACTIVATED_SQL, RELEASE_SIGNUP_CODE_SQL, REVOKE_SIGNUP_CODE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::SignupCodeRepository;

impl_signup_code_repository!(Database, TextUuid);
