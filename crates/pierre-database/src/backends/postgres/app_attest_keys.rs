// ABOUTME: PostgreSQL-backed AppAttestKeyRepository, emitted from the shared implementation in repositories/app_attest_keys.rs
// ABOUTME: No backend-specific casts: every column binds and decodes the same way on both drivers

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::AppAttestKey;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::app_attest_keys::{
    impl_app_attest_key_repository, key_from_row, ADVANCE_COUNTER_SQL, FIND_KEY_SQL,
    REGISTER_KEY_SQL,
};
use crate::repositories::AppAttestKeyRepository;

impl_app_attest_key_repository!(PostgresDatabase);
