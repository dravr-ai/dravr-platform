// ABOUTME: PostgreSQL-backed AccountDeletionRepository, emitted from the shared body in repositories/account_deletion.rs
// ABOUTME: Every provider's data, the account, its tombstones and its chained receipt, in one transaction
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::backends::shared::transactions::TransactionGuard;
use crate::repositories::account_deletion::{
    impl_account_deletion_repository, json_list, receipt_from_row, scan_result_json,
    AccountDeletion, AccountDeletionPlan, AccountDeletionRepository, DeletionReceipt, GENESIS_HASH,
    INSERT_RECEIPT_SQL, LIST_RECEIPTS_SQL, PROVIDER_IDENTITIES_SQL, RECEIPT_HEAD_SQL,
    SUBJECT_EMAIL_SQL,
};
use crate::repositories::archive_erasures::QUEUE_ARCHIVE_ERASURE_SQL;
use crate::repositories::firebase_identity_deletions::QUEUE_FIREBASE_DELETION_SQL;
use crate::repositories::import_tombstones::{
    email_identity_hash, ensure_tombstone_in_transaction, provider_identity_hash, subject_hash,
    tombstone_column_error, TombstoneKind, FIND_LIVE_TOMBSTONE_SQL, INSERT_TOMBSTONE_SQL,
};
use crate::repositories::provider_data::{
    USER_ACTIVITY_ROWS_SQL, USER_SYNC_STATE_SQL, USER_WELLNESS_ROWS_SQL,
};
use crate::repositories::user_references::{
    clear_user_in_transaction, delete_user_error, POSTGRES_USER_PURGE,
};
use crate::repositories::users::DELETE_USER_SQL;
use crate::repositories::uuid_columns::NativeUuid;

impl_account_deletion_repository!(PostgresDatabase, NativeUuid, POSTGRES_USER_PURGE);
