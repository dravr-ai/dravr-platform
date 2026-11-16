// ABOUTME: Decides a pending sign-up for a surface that is no admin's decision: a sign-up rule, or the Slack sign-up room
// ABOUTME: The write lands only while the account is still pending; the room's approval gives the account a tenant or is put back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppError;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_database::RepositoryRegistry;
use tracing::{error, info, warn};

use super::caller::ActingOn;
use super::credentials::AdminStandingLost;
use super::StatusTransition;
use crate::tenant_admin::provision_tenant_for_approval;

/// What a sign-up surface decides of a pending account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignUpDecision {
    /// Let the sign-up in: the account becomes active.
    Approve,
    /// Turn the sign-up away: the account is suspended.
    Reject,
}

impl SignUpDecision {
    /// The status the decision gives the account.
    #[must_use]
    pub const fn status(self) -> UserStatus {
        match self {
            Self::Approve => UserStatus::Active,
            Self::Reject => UserStatus::Suspended,
        }
    }
}

/// Write `decision` on the sign-up `acting` names, only while it is still
/// pending: the write, or `None` when it matched no row.
///
/// The one status write of every surface that decides sign-ups and nothing
/// else: a sign-up rule ([`activate_pending_sign_up`]) and the Slack
/// sign-up room, on either button. Such a surface read the account pending
/// some time before it gets here, and an admin, or another tap, can decide
/// the same sign-up in between. So the status is checked by the statement
/// that changes it (`UserRepository::update_status_from`): of two
/// decisions on one sign-up the first is written and the second changes
/// nothing, and an admin's suspension is never undone.
///
/// `None` is all a lost write says: the other decision won, the same one
/// did, or the account left the queue another way (approved on the
/// console, deleted). The caller that needs to know which reads the
/// account again. A rejection withdraws what the account still held from a
/// time as an admin, in the same write; the caller records it
/// ([`super::AccountCaller::record_withdrawn`]).
///
/// The decision is written against
/// [`super::AccountCaller::recorded_as`], as
/// [`super::transition_user_status`] records an admin's.
///
/// # Errors
///
/// Returns the repository's error when the write fails.
pub async fn decide_sign_up(
    repos: &RepositoryRegistry,
    acting: &ActingOn<'_>,
    decision: SignUpDecision,
) -> Result<Option<StatusTransition>, AppError> {
    let write = repos
        .users
        .update_status_from(
            acting.id(),
            UserStatus::Pending,
            decision.status(),
            acting.caller().recorded_as(),
        )
        .await?;
    Ok(write.map(|write| StatusTransition {
        user: write.user,
        withdrawn: write.withdrawn,
    }))
}

/// What became of a sign-up rule's activation of an account
/// ([`activate_pending_sign_up`]).
#[derive(Debug)]
pub struct PendingActivation {
    /// The account as the store holds it after the attempt.
    pub user: User,
    /// Whether this call activated it. `false` when the account was decided
    /// before the write, which then changed nothing: `user` says how.
    pub activated: bool,
}

/// Activate the sign-up `acting` names for a rule that lets pending accounts in:
/// a standing allow, a signup code, a proven address, an auto-approved
/// domain.
///
/// Such a rule admits a sign-up nobody has decided, and every caller read
/// the account pending some time before it gets here: a login verifies a
/// password in between, a redemption claims a code. An admin who suspends
/// the account in that time has decided it, so the status is checked by the
/// statement that changes it (`UserRepository::update_status_from`): the
/// activation lands only while the account is still pending, and a
/// suspension is never undone by a rule. When it does not land the account
/// is read again, and the caller acts on what it is now; nothing an
/// activation brings (a default MCP token, an announcement, a redemption
/// marked) belongs to an account this call did not activate.
///
/// `acting` is the account and the caller activating it
/// ([`super::AccountCaller::acting_on`]): a sign-up rule, or an admin surface
/// recording a standing allow. The write is [`decide_sign_up`]'s.
///
/// # Errors
///
/// Returns `NotFound` for an account deleted since the caller read it, and
/// the repository's error if a call fails.
pub async fn activate_pending_sign_up(
    repos: &RepositoryRegistry,
    acting: &ActingOn<'_>,
) -> Result<PendingActivation, AppError> {
    let user_id = acting.id();
    if let Some(write) = decide_sign_up(repos, acting, SignUpDecision::Approve).await? {
        return Ok(PendingActivation {
            user: write.user,
            activated: true,
        });
    }
    let user = repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::not_found("User not found"))?;
    info!(
        user_id = %user_id,
        status = %user.user_status,
        "A sign-up rule found the account already decided; it was not activated"
    );
    Ok(PendingActivation {
        user,
        activated: false,
    })
}

/// A sign-up the room approved.
#[derive(Debug)]
pub struct ApprovedSignUp {
    /// The account as stored after the approval.
    pub user: User,
    /// The tenant the approval is attributed to: the one provisioned for
    /// the account, or the one it already owned or belonged to.
    pub tenant_id: Option<TenantId>,
}

/// What became of the room's approval of a sign-up ([`approve_sign_up`]).
#[derive(Debug)]
pub enum SignUpApproval {
    /// The sign-up is approved and the account has a tenant.
    Approved(Box<ApprovedSignUp>),
    /// The write found the sign-up decided: nothing was written
    /// ([`decide_sign_up`]).
    LostAtTheWrite,
    /// The approval landed, the account could not be given its tenant, and
    /// the sign-up was put back: it is pending again, for the next approval
    /// to finish. The cause is what failed.
    PutBack(AppError),
}

/// Approve the sign-up `acting` names for the Slack sign-up room: active
/// while it is still pending ([`decide_sign_up`]), with a tenant of its own
/// when it belongs to none.
///
/// The tenant comes after the write, since only the write says which of two
/// taps decides. An approval that cannot give the account its tenant (the
/// tenants cannot be read, or one cannot be made) must not leave an active
/// account with none and nobody told: the room's card is spent by then, and
/// the console finds nothing to approve. So the sign-up is put back to
/// pending, where the next tap or the console approves it. The write that
/// puts it back lands only while the account is still active: one an admin
/// suspended in between stays as the admin left it, and the failure is
/// returned as it is.
///
/// # Errors
///
/// Returns the repository's error when the approving write fails, and what
/// failed after it when the sign-up could not be put back.
pub async fn approve_sign_up(
    repos: &RepositoryRegistry,
    acting: &ActingOn<'_>,
) -> Result<SignUpApproval, AppError> {
    let Some(approved) = decide_sign_up(repos, acting, SignUpDecision::Approve).await? else {
        return Ok(SignUpApproval::LostAtTheWrite);
    };
    let user = approved.user;
    match own_tenant(repos, &user).await {
        Ok(tenant_id) => Ok(SignUpApproval::Approved(Box::new(ApprovedSignUp {
            user,
            tenant_id,
        }))),
        Err(cause) => put_back(repos, acting, cause).await,
    }
}

/// The tenant an approved sign-up's events are attributed to, provisioning
/// one of its own when the account belongs to none.
async fn own_tenant(repos: &RepositoryRegistry, user: &User) -> Result<Option<TenantId>, AppError> {
    let tenants = repos.tenants.list_for_user(user.id).await?;
    if !tenants.is_empty() {
        return Ok(Tenant::attribution_tenant(&tenants, user.id));
    }
    let provisioned = provision_tenant_for_approval(
        repos,
        user.id,
        &user.email,
        user.display_name.as_deref(),
        None,
        None,
    )
    .await?;
    Ok(Some(provisioned.id))
}

/// Set the sign-up `acting` names back to pending after `cause` kept its
/// approval from completing, only while it is still active.
async fn put_back(
    repos: &RepositoryRegistry,
    acting: &ActingOn<'_>,
    cause: AppError,
) -> Result<SignUpApproval, AppError> {
    let user_id = acting.id();
    let undone = repos
        .users
        .update_status_from(user_id, UserStatus::Active, UserStatus::Pending, None)
        .await;
    match undone {
        Ok(Some(write)) => {
            warn!(
                user_id = %user_id,
                error = %cause,
                "A sign-up's approval could not give the account a tenant; the sign-up is pending again"
            );
            // Leaving the active status withdraws what the account held
            // from a time as an admin, as any suspension does.
            acting
                .caller()
                .record_withdrawn(
                    repos,
                    &write.withdrawn,
                    AdminStandingLost::Deactivated,
                    Some(user_id),
                )
                .await;
            Ok(SignUpApproval::PutBack(cause))
        }
        Ok(None) => {
            warn!(
                user_id = %user_id,
                error = %cause,
                "A sign-up's approval could not give the account a tenant, and the account was decided again meanwhile; it is left as it stands"
            );
            Err(cause)
        }
        Err(undo) => {
            error!(
                user_id = %user_id,
                error = %cause,
                undo_error = %undo,
                "A sign-up's approval could not give the account a tenant, and the sign-up could not be put back: the account is active with no tenant"
            );
            Err(cause)
        }
    }
}

#[cfg(test)]
mod tests {
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;

    use super::super::AccountCaller;
    use super::*;

    /// The put-back undoes this approval and nothing else: its write lands
    /// only while the account is still active. An admin who suspended the
    /// account between the approval and the failed tenant has decided it,
    /// and an unguarded write to pending would hand the sign-up back to the
    /// queue (carnet#1589, test review). The account stays suspended and
    /// the cause comes back as the error.
    #[tokio::test]
    async fn a_put_back_leaves_an_account_an_admin_suspended_meanwhile() {
        let database = create_test_db().await.unwrap();
        let repos = database.repositories();
        let mut account = create_test_user("put-back@example.com", None);
        account.user_status = UserStatus::Suspended;
        repos.users.create(&account).await.unwrap();
        let room = AccountCaller::slack_room("@approver");
        let acting = room.acting_on(account.clone()).unwrap();

        let outcome = put_back(
            repos,
            &acting,
            AppError::database("the tenant could not be made"),
        )
        .await;

        let cause = outcome.expect_err("an account decided again is not put back");
        assert!(
            cause.to_string().contains("the tenant could not be made"),
            "the cause that kept the approval from completing comes back: {cause}"
        );
        let stored = repos.users.get_global(account.id).await.unwrap().unwrap();
        assert_eq!(
            stored.user_status,
            UserStatus::Suspended,
            "the suspension stands"
        );
    }
}
