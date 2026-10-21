// ABOUTME: Delete an account the way production does, for tests that only need it gone
// ABOUTME: The one-transaction account deletion with nothing to purge upstream: its receipt and tombstones included
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use pierre_database::repositories::{AccountDeletion, AccountDeletionPlan, DeletionActor};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Delete `user_id` through the account deletion, as an operator.
///
/// No provider backend is purged and no upstream grant reported: what a test
/// whose account holds no provider needs. `firebase_project` queues the
/// account's Firebase identity, as a server with Firebase configured does.
///
/// # Errors
///
/// Returns the deletion's error: not found, a row still referencing the
/// account (`ResourceLocked`), or a database failure.
pub async fn delete_account(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    firebase_project: Option<&str>,
) -> AppResult<AccountDeletion> {
    repos
        .account_deletion
        .delete_account(&AccountDeletionPlan {
            user_id,
            actor: DeletionActor::Operator,
            firebase_project: firebase_project.map(str::to_owned),
            backends: Vec::new(),
            revocations: "[]".to_owned(),
            not_deleted: Vec::new(),
        })
        .await
}
