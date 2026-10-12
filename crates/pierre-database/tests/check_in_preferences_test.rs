// ABOUTME: Covers the check-in cadence and the once-a-day claim against whichever backend DATABASE_URL names
// ABOUTME: A choice reads back per tenant, off lists nobody, and a periodic claim is taken once per period (carnet#831)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `user_check_in_preferences` holds how often the coach checks in with each
//! athlete, and `platform_post_claims` the once-a-day claim the check-in
//! worker takes before it posts. Both are written once and emitted for both
//! backends; `create_test_db` opens whichever `DATABASE_URL` names, so the
//! same assertions cover `SQLite` and `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::{CheckInCadence, TenantId, User};
use pierre_database::repositories::CheckInSubscriber;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// A distinct user per call; both tables' foreign keys need the row.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("check-in-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Check-in Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

#[tokio::test]
async fn an_athlete_who_never_chose_reads_no_cadence() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;

    let cadence = repos
        .check_in_preferences
        .check_in_cadence(TenantId::generate(), user_id)
        .await
        .unwrap();
    assert_eq!(cadence, None, "no choice is not a cadence");
}

#[tokio::test]
async fn a_choice_reads_back_in_its_tenant_only_and_a_change_replaces_it() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let prefs = &repos.check_in_preferences;
    let user_id = fresh_user(repos).await;
    let (tenant, other_tenant) = (TenantId::generate(), TenantId::generate());

    for cadence in CheckInCadence::ALL {
        prefs
            .set_check_in_cadence(tenant, user_id, cadence, Utc::now())
            .await
            .unwrap();
        assert_eq!(
            prefs.check_in_cadence(tenant, user_id).await.unwrap(),
            Some(cadence),
            "the latest choice is the one read back"
        );
    }
    assert_eq!(
        prefs.check_in_cadence(other_tenant, user_id).await.unwrap(),
        None,
        "a choice made under one tenant is not read under another"
    );
}

#[tokio::test]
async fn subscribers_are_the_tenants_athletes_whose_cadence_posts() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let prefs = &repos.check_in_preferences;
    let (tenant, other_tenant) = (TenantId::generate(), TenantId::generate());
    let daily = fresh_user(repos).await;
    let weekly = fresh_user(repos).await;
    let off = fresh_user(repos).await;
    let elsewhere = fresh_user(repos).await;
    fresh_user(repos).await; // never chose

    for (user, cadence) in [
        (daily, CheckInCadence::Daily),
        (weekly, CheckInCadence::Weekly),
        (off, CheckInCadence::Off),
    ] {
        prefs
            .set_check_in_cadence(tenant, user, cadence, Utc::now())
            .await
            .unwrap();
    }
    prefs
        .set_check_in_cadence(other_tenant, elsewhere, CheckInCadence::Daily, Utc::now())
        .await
        .unwrap();

    let mut subscribers = prefs.check_in_subscribers(tenant).await.unwrap();
    subscribers.sort_by_key(|subscriber| subscriber.user_id);
    let mut expected = vec![
        CheckInSubscriber {
            user_id: daily,
            cadence: CheckInCadence::Daily,
        },
        CheckInSubscriber {
            user_id: weekly,
            cadence: CheckInCadence::Weekly,
        },
    ];
    expected.sort_by_key(|subscriber| subscriber.user_id);
    assert_eq!(
        subscribers, expected,
        "off, no choice and another tenant's athlete are never listed"
    );
}

#[tokio::test]
async fn a_periodic_claim_is_taken_once_per_period() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let claims = &repos.platform_post_claims;
    let user_id = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let monday = Utc.with_ymd_and_hms(2026, 10, 12, 0, 0, 0).unwrap();
    let tuesday = monday + Duration::days(1);

    let claim =
        |since, at| claims.claim_platform_post_since(tenant, user_id, "check_in", since, at);
    assert!(
        claim(monday, monday + Duration::hours(8)).await.unwrap(),
        "the first claim of Monday is taken"
    );
    assert!(
        !claim(monday, monday + Duration::minutes(510))
            .await
            .unwrap(),
        "a second tick, or a replica, the same Monday finds it taken"
    );
    assert!(
        claim(tuesday, tuesday + Duration::hours(8)).await.unwrap(),
        "Tuesday's period retakes the row Monday's claim left"
    );
    assert!(
        !claim(tuesday, tuesday + Duration::hours(9)).await.unwrap(),
        "and Tuesday's is then taken too"
    );

    // Releasing leaves the period's claim open again.
    assert!(claims
        .release_platform_post(tenant, user_id, "check_in")
        .await
        .unwrap());
    assert!(
        claim(tuesday, tuesday + Duration::hours(10)).await.unwrap(),
        "a released claim is taken by the next tick of the same period"
    );

    // Scoped by tenant and purpose: neither shares the row.
    assert!(claims
        .claim_platform_post_since(
            TenantId::generate(),
            user_id,
            "check_in",
            tuesday,
            tuesday + Duration::hours(10)
        )
        .await
        .unwrap());
    assert!(claims
        .claim_platform_post(tenant, user_id, "first_sync", tuesday)
        .await
        .unwrap());
}
