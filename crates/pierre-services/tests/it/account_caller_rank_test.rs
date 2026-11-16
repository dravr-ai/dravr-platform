// ABOUTME: The rank rule in the service layer: no account write is reached on an admin account without a platform caller
// ABOUTME: Drives every account write through the one door each takes, with each kind of caller, against a real database

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The rank rule used to be a call each admin handler made before its write
//! (carnet#1589), so a new handler, or a surface that is not a handler,
//! reached the write without it. It is now the door to every account write
//! of this crate: a write takes an `ActingOn`, and only
//! `AccountCaller::acting_on` builds one.
//!
//! The first test goes through that door for every write, with every kind of
//! caller that is not a platform one, on an admin account: the door refuses,
//! and the store is as it was. If the door let it through, the write would
//! run and the test names which one did. The second is the control: the
//! same writes land for a platform caller on an admin account, and for a
//! caller without the rank on a plain one.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::{TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::repositories::OnboardingResetScope;
use pierre_database::RepositoryRegistry;
use pierre_services::admin_ops::{self, AccountCaller, ActingOn};
use pierre_services::onboarding_reset::{self, OnboardingResetOutcome};
use pierre_services::pre_approval;
use pierre_services::provider_revocation::{DisconnectReason, RevocationOutcome};
use pierre_services::user_removal::{
    self, HeldProvider, ProviderDisconnection, ProviderDisconnector, UserRemoval,
};
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// What a caller without the rank is answered on an admin account.
const RANK_REFUSAL: &str = "Acting on an admin account requires a super-admin token";

/// A chokepoint that confirms every disconnect and counts them.
#[derive(Default)]
struct CountingDisconnector {
    disconnects: AtomicUsize,
}

#[async_trait]
impl ProviderDisconnector for CountingDisconnector {
    async fn disconnect(
        &self,
        _user_id: Uuid,
        _provider: &str,
        _tenant_id: TenantId,
        _reason: DisconnectReason,
    ) -> AppResult<RevocationOutcome> {
        self.disconnects.fetch_add(1, Ordering::SeqCst);
        Ok(RevocationOutcome::Revoked)
    }

    async fn withdraw_upstream(
        &self,
        _user_id: Uuid,
        _provider: &str,
        _tenant_id: TenantId,
    ) -> RevocationOutcome {
        RevocationOutcome::NoGrant
    }

    fn supports(&self, _provider: &str) -> bool {
        true
    }
}

/// An account write of the service layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Write {
    Suspend,
    Approve,
    ActivateSignUp,
    ResetPassword,
    Disconnect,
    ResetOnboarding,
    Remove,
}

impl Write {
    const ALL: [Self; 7] = [
        Self::Suspend,
        Self::Approve,
        Self::ActivateSignUp,
        Self::ResetPassword,
        Self::Disconnect,
        Self::ResetOnboarding,
        Self::Remove,
    ];

    /// The status an account is in for the write to have something to do.
    const fn starts_as(self) -> UserStatus {
        match self {
            Self::Approve => UserStatus::Suspended,
            Self::ActivateSignUp => UserStatus::Pending,
            Self::Suspend
            | Self::ResetPassword
            | Self::Disconnect
            | Self::ResetOnboarding
            | Self::Remove => UserStatus::Active,
        }
    }

    /// Run the write on the account `acting` holds; whether it did its work.
    async fn run(
        self,
        repos: &RepositoryRegistry,
        chokepoint: &CountingDisconnector,
        acting: &ActingOn<'_>,
    ) -> AppResult<bool> {
        Ok(match self {
            Self::Suspend => {
                admin_ops::transition_user_status(repos, acting, UserStatus::Suspended)
                    .await?
                    .user
                    .user_status
                    == UserStatus::Suspended
            }
            Self::Approve => {
                admin_ops::approve_user(repos, acting, None)
                    .await?
                    .user_status
                    == UserStatus::Active
            }
            Self::ActivateSignUp => {
                admin_ops::activate_pending_sign_up(repos, acting)
                    .await?
                    .activated
            }
            Self::ResetPassword => !admin_ops::issue_password_reset_token(repos, acting)
                .await?
                .is_empty(),
            Self::Disconnect => {
                let held = vec![HeldProvider {
                    tenant_id: TenantId::generate(),
                    provider: "strava".to_owned(),
                }];
                let before = chokepoint.disconnects.load(Ordering::SeqCst);
                let outcome =
                    user_removal::disconnect_user_provider(chokepoint, acting, held).await;
                matches!(outcome, ProviderDisconnection::Disconnected(done) if done.len() == 1)
                    && chokepoint.disconnects.load(Ordering::SeqCst) == before + 1
            }
            Self::ResetOnboarding => matches!(
                onboarding_reset::reset_onboarding(
                    repos,
                    Some(chokepoint),
                    acting,
                    OnboardingResetScope::OnboardingOnly,
                )
                .await?,
                OnboardingResetOutcome::Reset(_)
            ),
            Self::Remove => matches!(
                user_removal::remove_user(repos, Some(chokepoint), None, None, acting).await?,
                UserRemoval::Removed(_)
            ),
        })
    }
}

/// An account with `role` in `status`.
async fn account(repos: &RepositoryRegistry, role: UserRole, status: UserStatus) -> User {
    let email = format!("{}@example.com", Uuid::new_v4());
    let mut user = User::new(email, "hash".to_owned(), None);
    user.role = role;
    user.user_status = status;
    repos
        .users
        .create(&user)
        .await
        .expect("the account is stored");
    user
}

/// A service token's caller: `ManageUsers` alone, or the super-admin flag.
fn service_token(is_super_admin: bool) -> AccountCaller {
    AccountCaller::of_token(&ValidatedAdminToken {
        token_id: format!("test:{}", Uuid::new_v4()),
        service_name: "support-desk".to_owned(),
        permissions: AdminPermissions::new(vec![AdminPermission::ManageUsers]),
        is_super_admin,
        tenant_id: None,
        operator_user_id: None,
        user_info: None,
    })
}

/// Every kind of caller that is not a platform one.
fn callers_without_the_rank() -> Vec<AccountCaller> {
    vec![
        service_token(false),
        AccountCaller::slack_room("@reviewer"),
        AccountCaller::sign_up_rule(None),
        AccountCaller::account_holder(),
        AccountCaller::account_holder_on_messaging(),
    ]
}

/// No account write is reached on an admin account by a caller without the
/// rank: the door every write takes refuses, in the rank rule's words, and
/// the account stands as it was. A door that let the caller through would
/// run the write, and the failure names it.
#[tokio::test]
async fn no_account_write_is_reached_on_an_admin_account_without_the_rank() {
    let db = create_test_db().await.expect("a test database");
    let repos = db.repositories();
    let chokepoint = CountingDisconnector::default();
    // A second active admin, so the last-admin rule is never what answers.
    account(repos, UserRole::Admin, UserStatus::Active).await;

    for caller in callers_without_the_rank() {
        for write in Write::ALL {
            let admin = account(repos, UserRole::Admin, write.starts_as()).await;
            let cell = format!("{write:?} as {}", caller.name());

            match caller.acting_on(admin.clone()) {
                Err(refused) => {
                    assert!(
                        refused.code == ErrorCode::PermissionDenied,
                        "{cell}: {:?}",
                        refused.code
                    );
                    assert_eq!(refused.message, RANK_REFUSAL, "{cell}");
                }
                Ok(acting) => {
                    let ran = write.run(repos, &chokepoint, &acting).await;
                    panic!("{cell}: the write was reached on an admin account: {ran:?}");
                }
            }

            let stored = repos
                .users
                .get_global(admin.id)
                .await
                .expect("a read")
                .expect("the admin account is still there");
            assert_eq!(stored.user_status, write.starts_as(), "{cell}");
        }

        // The allow takes its caller itself, and approves a pending account
        // on the spot: a pending admin's address is refused the same way,
        // and neither the account nor the allow-list changes.
        let pending_admin = account(repos, UserRole::Admin, UserStatus::Pending).await;
        let refused = pre_approval::allow(repos, &caller, &pending_admin.email, None)
            .await
            .expect_err("an allow approves no pending admin without the rank");
        assert_eq!(refused.message, RANK_REFUSAL, "allow as {}", caller.name());
        let stored = repos.users.get_global(pending_admin.id).await;
        assert_eq!(
            stored.expect("a read").map(|user| user.user_status),
            Some(UserStatus::Pending),
            "allow as {}",
            caller.name()
        );
        assert!(
            repos
                .pre_approved_emails
                .list()
                .await
                .expect("a read")
                .is_empty(),
            "allow as {}: no standing allow was recorded",
            caller.name()
        );
    }
    assert_eq!(
        chokepoint.disconnects.load(Ordering::SeqCst),
        0,
        "nothing reached the chokepoint"
    );
}

/// The control: the same writes land for a platform caller on an admin
/// account (`pierre-cli` on the host, a super-admin token), and for a
/// caller without the rank on a plain account.
#[tokio::test]
async fn every_account_write_lands_where_the_rank_rule_lets_it() {
    let db = create_test_db().await.expect("a test database");
    let repos = db.repositories();
    let chokepoint = CountingDisconnector::default();
    account(repos, UserRole::Admin, UserStatus::Active).await;

    let cells = [
        (AccountCaller::host_cli(None), UserRole::Admin),
        (service_token(true), UserRole::Admin),
        (service_token(false), UserRole::User),
    ];
    for (caller, role) in &cells {
        for write in Write::ALL {
            let target = account(repos, *role, write.starts_as()).await;
            let cell = format!("{write:?} as {}, {role:?} account", caller.name());
            let acting = caller
                .acting_on(target)
                .unwrap_or_else(|refused| panic!("{cell}: {refused}"));
            let landed = write
                .run(repos, &chokepoint, &acting)
                .await
                .unwrap_or_else(|failed| panic!("{cell}: {failed}"));
            assert!(landed, "{cell}: the write did its work");
        }

        let pending = account(repos, *role, UserStatus::Pending).await;
        let allowed = pre_approval::allow(repos, caller, &pending.email, None)
            .await
            .unwrap_or_else(|failed| panic!("allow as {}: {failed}", caller.name()));
        assert_eq!(
            allowed.outcome,
            pre_approval::AllowOutcome::PendingApproved,
            "allow as {}, {role:?} account",
            caller.name()
        );
    }
}
