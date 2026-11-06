// ABOUTME: Integration tests for the new-user cohort report against a real database
// ABOUTME: A tenant reader never sees another tenant's account or milestone; milestones and first-week turns are read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The report the admin console's "New users" view and the daily digest read
//! (`pierre_services::new_user_cohort`), run against the test database so
//! every milestone subquery executes for real. The tenant boundary is drawn
//! by the repository's SQL: the route only chooses the scope.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, Utc};
use pierre_core::models::{
    ActivityBuilder, AddMessageParams, ConnectionType, SportType, Tenant, TenantId, User,
    UserStatus,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::SyncCursorRow;
use pierre_database::RepositoryRegistry;
use pierre_services::new_user_cohort::{fetch_new_user_cohort, NewUserCohort};
use uuid::Uuid;

use crate::common;

/// An account created `days_ago`, owning a tenant of its own (which makes it
/// a member of that tenant): `(user, tenant)`.
async fn seed_account(repos: &RepositoryRegistry, label: &str, days_ago: i64) -> (Uuid, TenantId) {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email, "hash".to_owned(), Some(label.to_owned()));
    user.user_status = UserStatus::Active;
    user.created_at = Utc::now() - Duration::days(days_ago);
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// A conversation `user` owns in `tenant`, with one message per role given.
async fn seed_messages(repos: &RepositoryRegistry, user: Uuid, tenant: TenantId, roles: &[&str]) {
    let user_id = user.to_string();
    let conversation = repos
        .chat
        .create_conversation(&user_id, tenant, "first chat", "test-model", None, None)
        .await
        .unwrap();
    for role in roles {
        repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: tenant,
                conversation_id: &conversation.id,
                user_id: &user_id,
                role,
                content: "hello",
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
                turn_attempted_write: None,
            })
            .await
            .unwrap();
    }
}

fn listed(cohort: &NewUserCohort, user: Uuid) -> bool {
    let id = user.to_string();
    cohort.users.iter().any(|row| row.user_id == id)
}

#[tokio::test]
async fn a_tenant_reader_never_sees_another_tenants_account_or_milestone() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let (alice, alice_tenant) = seed_account(repos, "alice", 2).await;
    let (bob, bob_tenant) = seed_account(repos, "bob", 2).await;
    // Every one of Alice's milestones lives in Bob's tenant: a provider
    // connection, a cached activity, a completed health sync, a message.
    repos
        .provider_connections
        .register_connection(alice, bob_tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        "alice-ride-1".to_owned(),
        "Morning ride".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        "strava".to_owned(),
    )
    .build();
    repos
        .activity_cache
        .upsert_activities(alice, &bob_tenant, "strava", &[ride])
        .await
        .unwrap();
    repos
        .sync_cursors
        .upsert_sync_cursor(&SyncCursorRow {
            id: format!("{alice}:{bob_tenant}:whoop:recovery"),
            user_id: alice.to_string(),
            tenant_id: bob_tenant.to_string(),
            provider: "whoop".to_owned(),
            data_type: "recovery".to_owned(),
            cursor_value: None,
            last_sync_at: Some(Utc::now() - Duration::hours(12)),
            last_sync_status: "completed".to_owned(),
            records_synced: 1,
            error_message: None,
            retry_count: 0,
            next_retry_at: None,
        })
        .await
        .unwrap();
    seed_messages(repos, alice, bob_tenant, &["user"]).await;
    let now = Utc::now();

    let in_alices = fetch_new_user_cohort(repos, 30, Some(alice_tenant), now)
        .await
        .unwrap();
    assert!(listed(&in_alices, alice));
    assert!(
        !listed(&in_alices, bob),
        "Bob is not a member of Alice's tenant"
    );
    assert_eq!(in_alices.users.len(), 1);
    let alice_in_hers = &in_alices.users[0];
    assert_eq!(
        alice_in_hers.provider_connected_at, None,
        "a connection held in another tenant is not this tenant's milestone"
    );
    assert_eq!(
        alice_in_hers.first_sync_at, None,
        "an activity or a sync cached in another tenant is not this tenant's milestone"
    );
    assert_eq!(
        alice_in_hers.first_turn_at, None,
        "a message in another tenant's conversation is not this tenant's milestone"
    );
    assert_eq!(
        alice_in_hers.turns_first_week, 0,
        "a message in another tenant's conversation is not counted here"
    );

    let in_bobs = fetch_new_user_cohort(repos, 30, Some(bob_tenant), now)
        .await
        .unwrap();
    assert!(listed(&in_bobs, bob));
    assert!(
        !listed(&in_bobs, alice),
        "a connection does not make Alice a member"
    );

    let platform = fetch_new_user_cohort(repos, 30, None, now).await.unwrap();
    assert!(listed(&platform, alice) && listed(&platform, bob));
    let alice_row = platform
        .users
        .iter()
        .find(|row| row.user_id == alice.to_string())
        .unwrap();
    assert!(
        alice_row.provider_connected_at.is_some(),
        "the platform-wide read counts every tenant's connection"
    );
    assert!(
        alice_row.first_sync_at.is_some(),
        "the platform-wide read counts every tenant's synced data"
    );
    assert!(
        alice_row.first_turn_at.is_some(),
        "the platform-wide read counts every tenant's conversations"
    );
    assert_eq!(alice_row.turns_first_week, 1);
}

#[tokio::test]
async fn milestones_and_first_week_turns_are_read_from_the_accounts_rows() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let (athlete, tenant) = seed_account(repos, "athlete", 2).await;
    let (quiet, _) = seed_account(repos, "quiet", 3).await;
    let (old, _) = seed_account(repos, "old", 40).await;
    repos
        .provider_connections
        .register_connection(athlete, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    seed_messages(repos, athlete, tenant, &["user", "assistant", "user"]).await;

    let cohort = fetch_new_user_cohort(repos, 30, Some(tenant), Utc::now())
        .await
        .unwrap();

    assert_eq!(cohort.window_days, 30);
    assert!(!cohort.truncated);
    assert_eq!(cohort.users.len(), 1);
    let row = &cohort.users[0];
    assert_eq!(row.user_id, athlete.to_string());
    assert!(row.provider_connected_at.is_some());
    assert!(row.first_turn_at.is_some());
    assert_eq!(
        row.turns_first_week, 2,
        "the assistant's reply is not a turn"
    );
    assert_eq!(row.status, "active");
    assert_eq!(cohort.funnel.signed_up, 1);
    assert_eq!(cohort.funnel.provider_connected, 1);
    assert_eq!(cohort.funnel.first_turn, 1);
    assert_eq!(cohort.funnel.day_7_eligible, 0, "two days old");

    let platform = fetch_new_user_cohort(repos, 30, None, Utc::now())
        .await
        .unwrap();
    assert!(listed(&platform, quiet));
    assert!(!listed(&platform, old), "created before the window");
    let quiet_row = platform
        .users
        .iter()
        .find(|row| row.user_id == quiet.to_string())
        .unwrap();
    assert_eq!(quiet_row.provider_connected_at, None);
    assert_eq!(quiet_row.first_turn_at, None);
    assert_eq!(quiet_row.turns_first_week, 0);
}
