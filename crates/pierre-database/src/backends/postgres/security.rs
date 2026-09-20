// ABOUTME: PostgreSQL-backed SecurityRepository, emitted from the shared implementation in repositories/security.rs
// ABOUTME: The RSA signing keypair and the system secrets; no bind differs from SQLite on these two tables
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::admin::AdminJwtManager;
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use tracing::warn;

use crate::backends::postgres::PostgresDatabase;
use crate::backends::shared::encryption::{
    decrypt_rsa_private_key, encrypt_rsa_private_key, is_plaintext_private_key_pem, HasEncryption,
};
use crate::repositories::security::{
    impl_security_repository, SecurityRepository, GET_SYSTEM_SECRET_SQL, INSERT_SYSTEM_SECRET_SQL,
    LOAD_RSA_KEYPAIRS_SQL, REWRITE_RSA_PRIVATE_KEY_SQL, SAVE_RSA_KEYPAIR_SQL,
    UPSERT_SYSTEM_SECRET_SQL,
};

impl_security_repository!(PostgresDatabase);
