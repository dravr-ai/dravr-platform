// ABOUTME: SQLite-backed SecurityRepository, emitted from the shared implementation in repositories/security.rs
// ABOUTME: The RSA signing keypair and the system secrets; no bind differs from Postgres on these two tables
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::admin::AdminJwtManager;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use sqlx::Row;

use crate::backends::shared::encryption::{
    decrypt_rsa_private_key, encrypt_rsa_private_key, is_plaintext_private_key_pem, HasEncryption,
};
use crate::backends::shared::key_material::StoredSecretKind;
use crate::backends::shared::keyring::SealDomain;
use crate::database::Database;
use crate::repositories::security::{
    data_tag_pattern, impl_security_repository, index_tag_pattern, sweep_sql, KeyLifecycleEvent,
    SealedRow, SecurityRepository, COMPARE_AND_SWAP_SYSTEM_SECRET_SQL, COUNT_ENCRYPTED_ROWS_SQL,
    COUNT_LIVE_INDEX_HASHES_SQL, DELETE_SYSTEM_SECRET_SQL, GET_SYSTEM_SECRET_SQL,
    INSERT_SYSTEM_SECRET_IF_ABSENT_SQL, LIST_KEY_LIFECYCLE_SQL, LOAD_RSA_KEYPAIRS_SQL,
    RECORD_KEY_LIFECYCLE_SQL, SAVE_RSA_KEYPAIR_SQL, UPSERT_SYSTEM_SECRET_SQL,
};

impl_security_repository!(Database);
