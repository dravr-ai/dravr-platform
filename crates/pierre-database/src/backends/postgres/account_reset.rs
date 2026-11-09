// ABOUTME: PostgreSQL-backed AccountResetRepository, emitted from the shared body in repositories/account_reset.rs
// ABOUTME: Every row the account owns wiped in one transaction, read back before it commits; the account stays
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserReference;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::backends::shared::transactions::TransactionGuard;
use crate::repositories::account_reset::{
    impl_account_reset_repository, reset_error, AccountReset, AccountResetCommit,
    AccountResetRepository, POSTGRES_ACCOUNT_RESET, RESET_ACCOUNT_EXISTS_SQL, RESET_BLOCKERS_SQL,
};
use crate::repositories::user_references::user_reference_from_row;
use crate::repositories::uuid_columns::NativeUuid;

impl_account_reset_repository!(PostgresDatabase, NativeUuid, POSTGRES_ACCOUNT_RESET);
