// ABOUTME: SQLite-backed FirebaseIdentityDeletionRepository, emitted from the shared body in repositories/
// ABOUTME: The outbox of Firebase identities still to delete at Google after their account went
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;

use crate::database::Database;
use crate::repositories::firebase_identity_deletions::{
    firebase_deletion_column_error, impl_firebase_identity_deletion_repository,
    FirebaseIdentityDeletionRepository, PendingFirebaseDeletion, COMPLETE_FIREBASE_DELETION_SQL,
    DUE_FIREBASE_DELETIONS_SQL, RECORD_FIREBASE_DELETION_FAILURE_SQL,
};

impl_firebase_identity_deletion_repository!(Database);
