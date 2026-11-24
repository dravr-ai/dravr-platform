// ABOUTME: PostgreSQL-backed AccountDeletionRepository, emitted from the shared body in repositories/account_deletion.rs
// ABOUTME: Every provider's data, the account, its tombstones and its chained receipt, in one transaction
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use dravr_tronc::dates::Timestamp;
use pierre_core::admin::models::{
    DELETED_ACCOUNT_ADMIN, DELETED_ACCOUNT_LABEL, DEVICE_CLI_DELETED_ACCOUNT_SERVICE,
};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::backends::shared::transactions::TransactionGuard;
use crate::repositories::account_deletion::{
    impl_account_deletion_repository, json_list, receipt_from_row, scan_result_json,
    AccountDeletion, AccountDeletionPlan, AccountDeletionRepository, DeletionReceipt,
    ReceiptAnchor, GENESIS_HASH, INSERT_ANCHOR_SQL, INSERT_RECEIPT_SQL, LATEST_ANCHOR_SQL,
    LIST_RECEIPTS_SQL, PROVIDER_IDENTITIES_SQL, RECEIPT_HEAD_SQL, SUBJECT_EMAIL_SQL,
};
use crate::repositories::archive_erasures::QUEUE_ARCHIVE_ERASURE_SQL;
use crate::repositories::firebase_identity_deletions::QUEUE_FIREBASE_DELETION_SQL;
use crate::repositories::import_tombstones::{
    email_identity_hash, ensure_tombstone_in_transaction, identity_lookup, provider_identity_hash,
    subject_hash, tombstone_column_error, TombstoneKind, FIND_LIVE_TOMBSTONE_SQL,
    INSERT_TOMBSTONE_SQL,
};
use crate::repositories::provider_data::{
    USER_ACTIVITY_ROWS_SQL, USER_SYNC_STATE_SQL, USER_WELLNESS_ROWS_SQL,
};
use crate::repositories::user_references::{
    clear_user_in_transaction, delete_user_error, LIVE_DEVICE_LOGINS_SQL,
    POSTGRES_SCRUB_ACTOR_AUDIT_EVENTS_SQL as SCRUB_ACTOR_AUDIT_EVENTS_SQL, POSTGRES_USER_PURGE,
    RETIRE_DEVICE_LOGINS_SQL, SCRUB_DEVICE_LOGIN_REQUESTS_SQL, SCRUB_PROVISIONED_BY_NAME_SQL,
    SCRUB_SUBJECT_LLM_USAGE_SQL, SCRUB_TOKEN_NAME_SQL,
};
use crate::repositories::user_sql::DELETE_USER_SQL;
use crate::repositories::uuid_columns::NativeUuid;

impl_account_deletion_repository!(PostgresDatabase, NativeUuid, POSTGRES_USER_PURGE, "BEGIN");
