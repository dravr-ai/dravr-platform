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

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{
    CheckInCadence, PostPurpose, RoutineKind, RoutineState, Schedule, TenantId, User,
};
use pierre_database::repositories::NextFiring;
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
            .set_check_in_cadence(tenant, user_id, cadence, Timestamp::now())
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

/// The routines the engine reconciles are the tenant's athletes whose
/// cadence posts: `off`, no choice and another tenant's athlete are never
/// read.
#[tokio::test]
async fn active_routines_are_the_tenants_athletes_whose_cadence_posts() {
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
            .set_check_in_cadence(tenant, user, cadence, Timestamp::now())
            .await
            .unwrap();
    }
    prefs
        .set_check_in_cadence(
            other_tenant,
            elsewhere,
            CheckInCadence::Daily,
            Timestamp::now(),
        )
        .await
        .unwrap();

    let mut active: Vec<(Uuid, Option<Schedule>)> = repos
        .routines
        .active_routines(tenant)
        .await
        .unwrap()
        .into_iter()
        .map(|routine| (routine.owner_user_id, routine.schedule()))
        .collect();
    active.sort_by_key(|(user_id, _)| *user_id);
    let mut expected = vec![
        (daily, Schedule::of_check_in(CheckInCadence::Daily)),
        (weekly, Schedule::of_check_in(CheckInCadence::Weekly)),
    ];
    expected.sort_by_key(|(user_id, _)| *user_id);
    assert_eq!(
        active, expected,
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
    let monday = Timestamp::parse_from_rfc3339("2026-10-12T00:00:00Z").unwrap();
    let tuesday = monday + Elapsed::days(1);

    let claim = |since, at| {
        claims.claim_platform_post_since(
            tenant,
            user_id,
            "check_in",
            PostPurpose::NO_SUBJECT,
            since,
            at,
        )
    };
    assert!(
        claim(monday, monday + Elapsed::hours(8))
            .await
            .unwrap()
            .is_some(),
        "the first claim of Monday is taken"
    );
    assert!(
        claim(monday, monday + Elapsed::minutes(510))
            .await
            .unwrap()
            .is_none(),
        "a second tick, or a replica, the same Monday finds it taken"
    );
    let tuesday_token = claim(tuesday, tuesday + Elapsed::hours(8))
        .await
        .unwrap()
        .expect("Tuesday's period retakes the row Monday's claim left");
    assert!(
        claim(tuesday, tuesday + Elapsed::hours(9))
            .await
            .unwrap()
            .is_none(),
        "and Tuesday's is then taken too"
    );

    // Releasing leaves the period's claim open again.
    assert!(claims
        .release_platform_post(
            tenant,
            user_id,
            "check_in",
            PostPurpose::NO_SUBJECT,
            &tuesday_token
        )
        .await
        .unwrap());
    assert!(
        claim(tuesday, tuesday + Elapsed::hours(10))
            .await
            .unwrap()
            .is_some(),
        "a released claim is taken by the next tick of the same period"
    );

    // Scoped by tenant and purpose: neither shares the row.
    assert!(claims
        .claim_platform_post_since(
            TenantId::generate(),
            user_id,
            "check_in",
            PostPurpose::NO_SUBJECT,
            tuesday,
            tuesday + Elapsed::hours(10)
        )
        .await
        .unwrap()
        .is_some());
    assert!(claims
        .claim_platform_post_once(
            tenant,
            user_id,
            "first_sync",
            PostPurpose::NO_SUBJECT,
            tuesday,
            tuesday
        )
        .await
        .unwrap()
        .is_some());
}

/// A post owed once is claimed once; a claim whose post landed is never
/// retaken, and one abandoned mid-post is retaken only once its lease is past
/// (carnet#911).
#[tokio::test]
async fn a_once_claim_is_retaken_only_when_abandoned_past_its_lease() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let claims = &repos.platform_post_claims;
    let tenant = TenantId::generate();
    let noon = Timestamp::parse_from_rfc3339("2026-10-14T12:00:00Z").unwrap();
    let lease = Elapsed::minutes(15);
    let claim = |user_id, at| {
        claims.claim_platform_post_once(
            tenant,
            user_id,
            "first_sync",
            PostPurpose::NO_SUBJECT,
            at - lease,
            at,
        )
    };

    let abandoned = fresh_user(repos).await;
    assert!(
        claim(abandoned, noon).await.unwrap().is_some(),
        "the first claim is taken"
    );
    assert!(
        claim(abandoned, noon + Elapsed::minutes(14))
            .await
            .unwrap()
            .is_none(),
        "inside its lease an unposted claim is someone's post under way"
    );
    assert!(
        claim(abandoned, noon + Elapsed::minutes(16))
            .await
            .unwrap()
            .is_some(),
        "past its lease an unposted claim was abandoned and is retaken"
    );

    let posted = fresh_user(repos).await;
    let posted_token = claim(posted, noon)
        .await
        .unwrap()
        .expect("the first claim is taken");
    assert!(claims
        .mark_platform_post_posted(
            tenant,
            posted,
            "first_sync",
            PostPurpose::NO_SUBJECT,
            &posted_token,
            noon + Elapsed::minutes(1)
        )
        .await
        .unwrap());
    assert!(
        claim(posted, noon + Elapsed::days(30))
            .await
            .unwrap()
            .is_none(),
        "a post that landed is never claimed again"
    );
    assert!(
        !claims
            .mark_platform_post_posted(
                TenantId::generate(),
                posted,
                "first_sync",
                PostPurpose::NO_SUBJECT,
                &posted_token,
                noon + Elapsed::minutes(1)
            )
            .await
            .unwrap(),
        "marking is scoped by tenant"
    );
}

/// The cadence is also the athlete's check-in routine (ADR-036): written in
/// the same transaction, active on the cadence's days at 08:00, paused for
/// `off`, and unscheduled by every change so a wake queued for the old
/// schedule finds nothing to fire.
#[tokio::test]
async fn a_cadence_writes_the_check_in_routine_it_schedules() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let routine = || async {
        repos
            .routines
            .builtin_routine(tenant, user_id, RoutineKind::CheckIn)
            .await
            .unwrap()
    };

    repos
        .check_in_preferences
        .set_check_in_cadence(
            tenant,
            user_id,
            CheckInCadence::ThreePerWeek,
            Timestamp::now(),
        )
        .await
        .unwrap();
    let written = routine().await.expect("the cadence wrote its routine");
    assert_eq!(
        written.state,
        RoutineState::Active,
        "bug: a cadence that posts wrote a paused routine"
    );
    assert_eq!(
        written.schedule(),
        Schedule::of_check_in(CheckInCadence::ThreePerWeek),
        "bug: the routine's schedule is not the cadence's days at the check-in hour"
    );
    assert_eq!(
        (written.next_slot, written.next_fire),
        (None, None),
        "bug: a freshly written routine is already scheduled"
    );

    // The engine schedules it; only while the next firing is still the one
    // it read.
    // A slot is on the hour; the column keeps it to the millisecond.
    let slot = Timestamp::parse_from_rfc3339("2026-11-25T13:00:00Z").unwrap();
    let next = NextFiring {
        slot: Some(slot),
        fire: Some(slot),
        deferrals: 0,
    };
    let moved = repos
        .routines
        .schedule_routine(tenant, written.id, None, next, Timestamp::now())
        .await
        .unwrap();
    assert!(
        moved,
        "bug: an unscheduled routine refused its first schedule"
    );
    let stale = repos
        .routines
        .schedule_routine(tenant, written.id, None, next, Timestamp::now())
        .await
        .unwrap();
    assert!(!stale, "a guard on the old firing no longer matches");
    assert_eq!(
        routine().await.unwrap().next_fire,
        Some(slot),
        "bug: the schedule the guard let through was not stored"
    );

    repos
        .check_in_preferences
        .set_check_in_cadence(tenant, user_id, CheckInCadence::Off, Timestamp::now())
        .await
        .unwrap();
    let paused = routine().await.unwrap();
    assert_eq!(paused.id, written.id, "one check-in routine per athlete");
    assert_eq!(
        paused.state,
        RoutineState::Paused,
        "bug: `off` left the routine active"
    );
    assert_eq!(
        paused.schedule(),
        None,
        "bug: a paused routine still reads a schedule"
    );
    assert_eq!(paused.next_fire, None, "the change unscheduled it");
    assert!(
        repos
            .routines
            .active_routines(tenant)
            .await
            .unwrap()
            .is_empty(),
        "bug: a paused routine is still listed for the engine to reconcile"
    );
}
