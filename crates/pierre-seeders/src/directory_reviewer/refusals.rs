// ABOUTME: The directory reviewer seeder's refusals about the accounts it adopts or creates: their workspace and their email
// ABOUTME: A workspace switched off or left with no owner, and an email a deleted account's tombstone still holds, are refused with their remedy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{Tenant, TenantId};
use pierre_database::repositories::import_tombstones::{email_identity_hash, TombstoneKind};
use pierre_database::RepositoryRegistry;
use pierre_middleware::mask_email;

use crate::accounts::Account;

/// The workspace `tenant`, or `None` when it is switched off or has no
/// owner: the tenant read admits only an active workspace with its owner,
/// and answers every other one as not found.
///
/// # Errors
///
/// Returns the repository error of any other failed read.
pub(super) async fn workspace(
    repos: &RepositoryRegistry,
    tenant: TenantId,
) -> AppResult<Option<Tenant>> {
    match repos.tenants.get_by_id(tenant).await {
        Ok(workspace) => Ok(Some(workspace)),
        Err(e) if e.code == ErrorCode::ResourceNotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// [`workspace`] of `account`'s own workspace, refused with its remedy when
/// it is switched off or has no owner: nothing the run writes there could
/// be signed in to or acted in.
///
/// # Errors
///
/// Returns a configuration error naming the email masked and the remedy, or
/// the repository error of the read.
pub(super) async fn live_workspace(
    repos: &RepositoryRegistry,
    account: &Account,
) -> AppResult<Tenant> {
    workspace(repos, account.tenant).await?.ok_or_else(|| {
        AppError::config(format!(
            "The workspace of the account at {} is inactive or has no owner, so no session can \
             act in it; nothing was written. Reactivate the workspace and give it back its \
             owner, or seed another reviewer with --coach-email",
            mask_email(&account.email)
        ))
    })
}

/// Refuse an adopted coach account whose workspace (`users.tenant_id`) is
/// not its own: the group and its agent would be written there, beside the
/// accounts of whoever owns it. An admin's approval of a pending account
/// puts it in the approving admin's workspace, so such an account is never
/// the reviewer; the one this seeder creates always has its own. A
/// workspace switched off or with no owner is refused by [`live_workspace`].
///
/// # Errors
///
/// Returns a configuration error naming the email masked and the remedy.
pub(super) async fn refuse_shared_workspace(
    repos: &RepositoryRegistry,
    coach: &Account,
) -> AppResult<()> {
    if live_workspace(repos, coach).await?.owner_user_id == coach.id {
        return Ok(());
    }
    Err(AppError::config(format!(
        "The account at {} works in a workspace another account owns (an admin's approval \
         puts an account in that admin's workspace); it was not modified. Choose another \
         --coach-email: the seeder creates that account in a workspace of its own",
        mask_email(&coach.email)
    )))
}

/// Refuse to create an account at `email` while a live tombstone of a
/// deleted account names it.
///
/// A sign-up at such an address is held for an admin's review; the seeder
/// creating it active would hand a deleted account's address straight back.
/// A failed read refuses too, so a database fault never creates one.
///
/// # Errors
///
/// Returns a configuration error naming the email masked and the remedy, or
/// the repository error of the read.
pub(super) async fn refuse_deleted_email(repos: &RepositoryRegistry, email: &str) -> AppResult<()> {
    let held = repos
        .import_tombstones
        .find_live(TombstoneKind::Email, None, &email_identity_hash(email))
        .await?;
    if held.is_none() {
        return Ok(());
    }
    Err(AppError::config(format!(
        "{} belonged to a deleted account, and a live tombstone still holds it; no account was \
         created. Once the person it belonged to agrees, an admin clears it with `pierre-cli \
         deletion clear-tombstone`; otherwise seed another reviewer with --coach-email",
        mask_email(email)
    )))
}
