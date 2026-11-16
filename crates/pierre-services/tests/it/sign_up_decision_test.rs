// ABOUTME: The one write of a surface that decides sign-ups, and the room's approval with its tenant
// ABOUTME: A decided sign-up is never written twice; an approval that cannot provision is put back to pending

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Slack sign-up room wrote a sign-up's status and provisioned its
//! tenant itself, beside the service's own approval (carnet#1589). Both are
//! the service's now: `decide_sign_up` is the write, for either button and
//! for every sign-up rule, and `approve_sign_up` is the room's approval.
//!
//! An approval that landed and then could not give the account a tenant
//! used to leave it active with none, on a card whose buttons were gone.
//! It is put back to pending, where the next approval finishes it.

use chrono::Utc;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::RepositoryRegistry;
use pierre_services::admin_ops::{self, AccountCaller, SignUpApproval, SignUpDecision};
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// An account that is not an admin, stored in `status`.
async fn account(repos: &RepositoryRegistry, status: UserStatus) -> User {
    let mut user = User::new(
        format!("{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        None,
    );
    user.role = UserRole::User;
    user.user_status = status;
    repos
        .users
        .create(&user)
        .await
        .expect("the account is stored");
    user
}

/// The stored status of `user_id`, `None` once the account is gone.
async fn status_of(repos: &RepositoryRegistry, user_id: Uuid) -> Option<UserStatus> {
    repos
        .users
        .get_global(user_id)
        .await
        .expect("a read")
        .map(|user| user.user_status)
}

/// A tenant owned by `owner` that holds the slug an approval would give
/// `sign_up`'s own tenant, so provisioning that tenant fails.
async fn take_the_slug_of(repos: &RepositoryRegistry, sign_up: Uuid, owner: Uuid) {
    repos
        .tenants
        .create(&Tenant {
            id: TenantId::generate(),
            name: "in the way".to_owned(),
            slug: format!("user-{}", sign_up.as_simple()),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("the tenant in the way is stored");
}

/// The write, entered with the sign-up no longer pending: what the tap that
/// loses a race does (carnet#1589). Decided the other way, decided the same
/// way, or gone, the write matches no row, writes nothing, and is not an
/// error: there is no refusal to draw over the card of whatever decided. A
/// sign-up still pending is decided by the same write.
#[tokio::test]
async fn a_write_that_finds_the_sign_up_decided_is_lost_and_writes_nothing() {
    let db = create_test_db().await.expect("a test database");
    let repos = db.repositories();
    let room = AccountCaller::slack_room("@reviewer");

    for (label, decided, tapped) in [
        (
            "the other button won",
            UserStatus::Suspended,
            SignUpDecision::Approve,
        ),
        (
            "the other button won",
            UserStatus::Active,
            SignUpDecision::Reject,
        ),
        (
            "the same button won",
            UserStatus::Active,
            SignUpDecision::Approve,
        ),
    ] {
        let sign_up = account(repos, decided).await;
        let acting = room.acting_on(sign_up.clone()).expect("a plain account");

        let write = admin_ops::decide_sign_up(repos, &acting, tapped).await;

        assert_eq!(
            write
                .map(|write| write.map(|write| write.user.user_status))
                .ok(),
            Some(None),
            "{label}, account {decided}: a lost write is no refusal"
        );
        assert_eq!(
            status_of(repos, sign_up.id).await,
            Some(decided),
            "{label}: nothing was written"
        );
    }

    let never_stored = User::new("gone@example.com".to_owned(), "hash".to_owned(), None);
    let acting = room.acting_on(never_stored).expect("a plain account");
    let gone = admin_ops::decide_sign_up(repos, &acting, SignUpDecision::Approve).await;
    assert_eq!(
        gone.map(|write| write.map(|write| write.user.user_status))
            .ok(),
        Some(None),
        "an account that is gone"
    );

    // The control: a sign-up still pending is decided by the same write.
    let waiting = account(repos, UserStatus::Pending).await;
    let acting = room.acting_on(waiting).expect("a plain account");
    let decided = admin_ops::decide_sign_up(repos, &acting, SignUpDecision::Reject)
        .await
        .expect("the write runs");
    assert_eq!(
        decided.map(|write| write.user.user_status),
        Some(UserStatus::Suspended)
    );
}

/// The room's approval gives a sign-up with no tenant one of its own, and
/// leaves an account that already has one where it is.
#[tokio::test]
async fn the_rooms_approval_gives_the_account_a_tenant_of_its_own() {
    let db = create_test_db().await.expect("a test database");
    let repos = db.repositories();
    let room = AccountCaller::slack_room("@reviewer");
    let sign_up = account(repos, UserStatus::Pending).await;
    let acting = room.acting_on(sign_up.clone()).expect("a plain account");

    let approval = admin_ops::approve_sign_up(repos, &acting)
        .await
        .expect("the approval runs");

    let SignUpApproval::Approved(approved) = approval else {
        panic!("the sign-up is approved: {approval:?}");
    };
    assert_eq!(approved.user.user_status, UserStatus::Active);
    assert_eq!(
        approved.user.approved_by, None,
        "the room is written against no admin account"
    );
    let tenants = repos
        .tenants
        .list_for_user(sign_up.id)
        .await
        .expect("a read");
    assert_eq!(tenants.len(), 1, "one tenant of its own");
    assert_eq!(approved.tenant_id, Some(tenants[0].id));
    assert_eq!(tenants[0].owner_user_id, sign_up.id);

    // A second approval of the same sign-up finds it decided.
    let again = admin_ops::approve_sign_up(repos, &acting)
        .await
        .expect("the approval runs");
    assert!(matches!(again, SignUpApproval::LostAtTheWrite), "{again:?}");
    assert_eq!(
        repos
            .tenants
            .list_for_user(sign_up.id)
            .await
            .expect("a read")
            .len(),
        1,
        "and provisions nothing"
    );
}

/// An approval lands, then the account's tenant cannot be made (its slug is
/// taken). The account must not stay active with no tenant and nobody told:
/// the sign-up is pending again, with no approver recorded, and the
/// approval says so. An admin then approves it from the console's path,
/// which provisions nothing, and the account is active.
#[tokio::test]
async fn an_approval_that_cannot_give_the_account_a_tenant_is_put_back_to_pending() {
    let db = create_test_db().await.expect("a test database");
    let repos = db.repositories();
    let admin = account(repos, UserStatus::Active).await;
    let sign_up = account(repos, UserStatus::Pending).await;
    take_the_slug_of(repos, sign_up.id, admin.id).await;
    let room = AccountCaller::slack_room("@reviewer");
    let acting = room.acting_on(sign_up.clone()).expect("a plain account");

    let approval = admin_ops::approve_sign_up(repos, &acting)
        .await
        .expect("a put-back approval is an outcome, not an error");

    assert!(
        matches!(approval, SignUpApproval::PutBack(_)),
        "the approval was put back: {approval:?}"
    );
    let stored = repos
        .users
        .get_global(sign_up.id)
        .await
        .expect("a read")
        .expect("the account");
    assert_eq!(
        stored.user_status,
        UserStatus::Pending,
        "the sign-up is pending again"
    );
    assert_eq!(
        (stored.approved_by, stored.approved_at),
        (None, None),
        "and carries no approval"
    );
    assert!(
        repos
            .tenants
            .list_for_user(sign_up.id)
            .await
            .expect("a read")
            .is_empty(),
        "it was given no tenant"
    );

    // The console finishes: an admin's approval provisions nothing.
    let console = AccountCaller::host_cli(Some(admin.id));
    let acting = console.acting_on(stored).expect("a plain account");
    let approved = admin_ops::approve_user(repos, &acting, None)
        .await
        .expect("an admin approves the sign-up that was put back");
    assert_eq!(approved.user_status, UserStatus::Active);
    assert_eq!(approved.approved_by, Some(admin.id));
}
