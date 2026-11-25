// ABOUTME: The connect-stall scan finds the athletes who answered profile_type and never connected
// ABOUTME: Pins its exclusions and its tenant on the engine under test, an answer saved with no tenant included

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! On 2026-11-24 three athletes in six days were stuck at the onboarding
//! connect step and nothing said so. The sweep reports each one as
//! `onboarding.stuck_at_connect`, once, 30 minutes after the `profile_type`
//! answer; its once-only watermark is pinned beside it, in
//! `pierre_services::connect_stall_sweeper`. The scan is cross-tenant, so
//! every assertion here reads only the athletes this test made: a shared test
//! database may hold other suites'.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use pierre_core::models::{ConnectionType, TenantId, User};
use pierre_database::backends::factory::Database;
use uuid::Uuid;

/// An athlete with a tenant of their own, who answered `profile_type`.
async fn answered(database: &Database, label: &str) -> (Uuid, TenantId) {
    let email = format!("{label}-{}@example.com", Uuid::new_v4().simple());
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(database, &email, "starter")
            .await
            .expect("test user");
    database
        .repositories()
        .user_onboarding
        .set_onboarding_step(
            &user_id.to_string(),
            "profile_type",
            "complete",
            None,
            Some(&tenant_id.to_string()),
        )
        .await
        .expect("answer profile_type");
    (user_id, tenant_id)
}

async fn connect(database: &Database, user_id: Uuid, tenant_id: TenantId, kind: &ConnectionType) {
    database
        .repositories()
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", kind, None)
        .await
        .expect("register connection");
}

#[tokio::test]
async fn the_scan_finds_only_athletes_with_no_way_past_connect() {
    common::init_server_config();
    let database = common::create_test_database().await.expect("test db");
    let repos = database.repositories();

    let (stuck, stuck_tenant) = answered(&database, "stuck").await;
    let (demo, demo_tenant) = answered(&database, "demo").await;
    connect(&database, demo, demo_tenant, &ConnectionType::Synthetic).await;
    let (connected, connected_tenant) = answered(&database, "connected").await;
    connect(
        &database,
        connected,
        connected_tenant,
        &ConnectionType::OAuth,
    )
    .await;
    let (elsewhere, _) = answered(&database, "elsewhere").await;
    connect(&database, elsewhere, stuck_tenant, &ConnectionType::OAuth).await;
    let (skipped, skipped_tenant) = answered(&database, "skipped").await;
    repos
        .user_onboarding
        .set_onboarding_step(
            &skipped.to_string(),
            "connect_provider",
            "skipped",
            None,
            Some(&skipped_tenant.to_string()),
        )
        .await
        .expect("skip connect");
    let found = repos
        .user_onboarding
        .list_stalled_at_connect(500)
        .await
        .expect("scan");
    let ours = |user: Uuid| found.iter().find(|s| s.user_id == user.to_string());

    let row = ours(stuck).expect("bug: an athlete with nothing past connect was not found");
    assert_eq!(
        row.tenant_id,
        stuck_tenant.to_string(),
        "bug: the stall names another tenant than the answer's"
    );
    assert!(
        row.profile_type_at.is_some(),
        "the answer time must read as an instant on this engine"
    );
    assert!(
        ours(demo).is_some(),
        "bug: a synthetic connection is seed data, not a provider the athlete connected"
    );
    assert!(
        ours(elsewhere).is_some(),
        "bug: a connection in another tenant must not hide a stall in this one"
    );
    for (user, why) in [
        (connected, "a real connection in the tenant"),
        (skipped, "a connect_provider step"),
    ] {
        assert!(ours(user).is_none(), "bug: {why} still read as stuck");
    }
}

/// Record `user`'s `profile_type` answer with no tenant, as the onboarding
/// route does for an athlete who belonged to none when they answered.
async fn answer_without_tenant(database: &Database, user: Uuid) {
    database
        .repositories()
        .user_onboarding
        .set_onboarding_step(&user.to_string(), "profile_type", "complete", None, None)
        .await
        .expect("answer with no tenant");
}

#[tokio::test]
async fn an_answer_saved_without_a_tenant_is_checked_in_the_default_tenant() {
    common::init_server_config();
    let database = common::create_test_database().await.expect("test db");
    let repos = database.repositories();

    let (unplaced, unplaced_tenant) = answered(&database, "unplaced").await;
    answer_without_tenant(&database, unplaced).await;

    let (connected, connected_tenant) = answered(&database, "connected-home").await;
    answer_without_tenant(&database, connected).await;
    connect(
        &database,
        connected,
        connected_tenant,
        &ConnectionType::OAuth,
    )
    .await;

    // Connected in a tenant they are not a member of: the stall stays in
    // their own.
    let (away, away_tenant) = answered(&database, "connected-away").await;
    answer_without_tenant(&database, away).await;
    connect(&database, away, unplaced_tenant, &ConnectionType::OAuth).await;

    // A member of two tenants, joined to the second later, and connected
    // only there: the stall is in the first, and never in the second.
    let (both, first) = answered(&database, "two-tenants").await;
    let (_, second) = answered(&database, "second-owner").await;
    repos
        .users
        .update_tenant_id(both, second)
        .await
        .expect("join a second tenant");
    answer_without_tenant(&database, both).await;
    connect(&database, both, second, &ConnectionType::OAuth).await;

    // No tenant on the answer and no membership: nowhere to look.
    let nowhere = User::new(
        format!("nowhere-{}@example.com", Uuid::new_v4().simple()),
        "unused-hash".to_owned(),
        None,
    );
    repos.users.create(&nowhere).await.expect("tenantless user");
    answer_without_tenant(&database, nowhere.id).await;

    let found = repos
        .user_onboarding
        .list_stalled_at_connect(500)
        .await
        .expect("scan");
    let tenants_of = |user: Uuid| -> Vec<&str> {
        found
            .iter()
            .filter(|s| s.user_id == user.to_string())
            .map(|s| s.tenant_id.as_str())
            .collect()
    };

    let unplaced_tenant = unplaced_tenant.to_string();
    assert_eq!(
        tenants_of(unplaced),
        [unplaced_tenant.as_str()],
        "bug: an answer saved without a tenant was not reported in the athlete's tenant"
    );
    assert!(
        tenants_of(connected).is_empty(),
        "bug: a real connection in the athlete's tenant still read as stuck"
    );
    let away_tenant = away_tenant.to_string();
    assert_eq!(
        tenants_of(away),
        [away_tenant.as_str()],
        "bug: a connection in another tenant hid the stall, or the stall moved there"
    );
    let default = repos
        .tenants
        .first_for_user(both)
        .await
        .expect("default tenant")
        .expect("a member of two tenants has a default");
    assert_eq!(
        default, first,
        "the tenant joined first is the account's default"
    );
    let first = first.to_string();
    assert_eq!(
        tenants_of(both),
        [first.as_str()],
        "bug: the stall was reported against the second tenant, or not in the default one"
    );
    assert!(
        tenants_of(nowhere.id).is_empty(),
        "bug: an athlete in no tenant was reported"
    );
}
