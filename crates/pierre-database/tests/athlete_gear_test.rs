// ABOUTME: Covers the athlete's gear ledger storage against whichever backend DATABASE_URL names
// ABOUTME: Distance accrual, reassignment across a workout's copies, retire and delete rules, pinning, the purge and user isolation

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `athlete_gear` and `activity_gear` hold the shoes an athlete declared and
//! the pair each activity is attributed to (carnet#711). The statements are
//! written once and emitted for both backends, differing only in how a uuid
//! binds; `create_test_db` opens whichever `DATABASE_URL` names, so the same
//! assertions cover `SQLite` and `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::models::{
    ActivityGearKey, AthleteGear, AttributionSource, GearAttribution, GearDeclaration, GearKind,
    SportType, TenantId, User, DEFAULT_SHOE_WARN_DISTANCE_METERS, MAX_GEAR_PER_ATHLETE,
};
use pierre_database::repositories::{GearDeletion, PurgeReason};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// A distinct user per call; the tables' foreign keys need the row to exist.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("gear-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Gear Ledger Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

fn shoe(name: &str, sport: SportType) -> GearDeclaration {
    GearDeclaration {
        kind: GearKind::Shoe,
        name: name.to_owned(),
        sport,
        initial_distance_meters: 0.0,
        warn_distance_meters: DEFAULT_SHOE_WARN_DISTANCE_METERS,
        pinned: false,
    }
}

fn day(n: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 1, 7, 0, 0).unwrap() + Duration::days(n)
}

fn key(provider: &str, id: &str) -> ActivityGearKey {
    ActivityGearKey {
        provider: provider.to_owned(),
        activity_id: id.to_owned(),
    }
}

fn run(
    provider: &str,
    id: &str,
    gear: Option<Uuid>,
    source: AttributionSource,
    on: i64,
    meters: f64,
) -> GearAttribution {
    GearAttribution {
        key: key(provider, id),
        gear_id: gear,
        source,
        start_date: day(on),
        distance_meters: meters,
    }
}

async fn pair(repos: &RepositoryRegistry, user: Uuid, id: Uuid) -> AthleteGear {
    repos
        .athlete_gear
        .get_gear(user, id)
        .await
        .unwrap()
        .expect("the athlete's pair")
}

#[tokio::test]
async fn a_declared_pair_reads_back_with_its_declared_distance_and_no_history() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user(repos).await;

    let mut declared = shoe("Pegasus 41", SportType::Run);
    declared.initial_distance_meters = 120_000.0;
    declared.warn_distance_meters = 600_000.0;
    let id = repos
        .athlete_gear
        .create_gear(user, &declared, day(0))
        .await
        .unwrap();

    let stored = pair(repos, user, id).await;
    assert_eq!(stored.declaration, declared);
    assert_eq!(stored.added_at, day(0));
    assert_eq!(stored.retired_at, None);
    assert_eq!(stored.activity_count, 0);
    assert_eq!(stored.last_used_at, None);
    assert!((stored.distance_meters() - 120_000.0).abs() < 1e-6);
    assert!(!stored.warn_reached());
}

#[tokio::test]
async fn attributed_activities_add_to_a_pair_and_a_default_never_overwrites() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user(repos).await;
    let id = repos
        .athlete_gear
        .create_gear(user, &shoe("Pegasus 41", SportType::Run), day(0))
        .await
        .unwrap();

    let recorded = repos
        .athlete_gear
        .record_default_attributions(
            user,
            &[
                run(
                    "strava",
                    "a1",
                    Some(id),
                    AttributionSource::Default,
                    1,
                    10_000.0,
                ),
                run(
                    "strava",
                    "a2",
                    Some(id),
                    AttributionSource::Default,
                    3,
                    21_100.0,
                ),
            ],
        )
        .await
        .unwrap();
    assert_eq!(recorded, 2);
    let stored = pair(repos, user, id).await;
    assert_eq!(stored.activity_count, 2);
    assert!((stored.attributed_distance_meters - 31_100.0).abs() < 1e-6);
    assert_eq!(stored.last_used_at, Some(day(3)));

    // A copy the athlete chose for is never taken over by a later default.
    repos
        .athlete_gear
        .declare_gear_attribution(
            user,
            &run(
                "strava",
                "a3",
                None,
                AttributionSource::Declared,
                4,
                8_000.0,
            ),
            &[],
        )
        .await
        .unwrap();
    let again = repos
        .athlete_gear
        .record_default_attributions(
            user,
            &[
                run(
                    "strava",
                    "a1",
                    Some(id),
                    AttributionSource::Default,
                    1,
                    99_000.0,
                ),
                run(
                    "strava",
                    "a3",
                    Some(id),
                    AttributionSource::Default,
                    4,
                    8_000.0,
                ),
            ],
        )
        .await
        .unwrap();
    assert_eq!(again, 0);
    let stored = pair(repos, user, id).await;
    assert_eq!(stored.activity_count, 2, "the declared none stays none");
    assert!((stored.attributed_distance_meters - 31_100.0).abs() < 1e-6);

    let window = repos
        .athlete_gear
        .list_gear_attributions(user, day(2), day(5))
        .await
        .unwrap();
    let mut seen: Vec<(String, Option<Uuid>, AttributionSource)> = window
        .into_iter()
        .map(|a| (a.key.activity_id, a.gear_id, a.source))
        .collect();
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        seen,
        vec![
            ("a2".to_owned(), Some(id), AttributionSource::Default),
            ("a3".to_owned(), None, AttributionSource::Declared),
        ]
    );
}

#[tokio::test]
async fn reassigning_a_workout_moves_its_distance_and_clears_its_other_copies() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user(repos).await;
    let road = repos
        .athlete_gear
        .create_gear(user, &shoe("Road", SportType::Run), day(0))
        .await
        .unwrap();
    let trail = repos
        .athlete_gear
        .create_gear(user, &shoe("Trail", SportType::TrailRunning), day(0))
        .await
        .unwrap();
    repos
        .athlete_gear
        .record_default_attributions(
            user,
            &[run(
                "strava",
                "w1",
                Some(road),
                AttributionSource::Default,
                2,
                15_000.0,
            )],
        )
        .await
        .unwrap();

    // The athlete picks the trail pair on the Garmin copy of the same workout.
    repos
        .athlete_gear
        .declare_gear_attribution(
            user,
            &run(
                "garmin",
                "g1",
                Some(trail),
                AttributionSource::Declared,
                2,
                15_200.0,
            ),
            &[key("strava", "w1"), key("garmin", "g1")],
        )
        .await
        .unwrap();

    let road_now = pair(repos, user, road).await;
    let trail_now = pair(repos, user, trail).await;
    assert_eq!(road_now.activity_count, 0);
    assert!(road_now.attributed_distance_meters.abs() < 1e-6);
    assert_eq!(trail_now.activity_count, 1);
    assert!((trail_now.attributed_distance_meters - 15_200.0).abs() < 1e-6);

    // Choosing again on the same copy replaces it rather than adding a row.
    repos
        .athlete_gear
        .declare_gear_attribution(
            user,
            &run(
                "garmin",
                "g1",
                Some(road),
                AttributionSource::Declared,
                2,
                15_200.0,
            ),
            &[key("strava", "w1"), key("garmin", "g1")],
        )
        .await
        .unwrap();
    assert_eq!(pair(repos, user, trail).await.activity_count, 0);
    assert_eq!(pair(repos, user, road).await.activity_count, 1);
}

#[tokio::test]
async fn a_released_copy_hands_its_choice_over_or_lets_the_pair_go() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user(repos).await;
    let other = fresh_user(repos).await;
    let road = repos
        .athlete_gear
        .create_gear(user, &shoe("Road", SportType::Run), day(0))
        .await
        .unwrap();
    let spare = repos
        .athlete_gear
        .create_gear(user, &shoe("Spare", SportType::Run), day(0))
        .await
        .unwrap();
    let theirs = repos
        .athlete_gear
        .create_gear(other, &shoe("Theirs", SportType::Run), day(0))
        .await
        .unwrap();
    repos
        .athlete_gear
        .declare_gear_attribution(
            user,
            &run(
                "upload",
                "u1",
                Some(spare),
                AttributionSource::Declared,
                2,
                12_000.0,
            ),
            &[],
        )
        .await
        .unwrap();
    repos
        .athlete_gear
        .record_default_attributions(
            user,
            &[
                run(
                    "strava",
                    "lone",
                    Some(road),
                    AttributionSource::Default,
                    3,
                    8_000.0,
                ),
                run(
                    "strava",
                    "held",
                    Some(road),
                    AttributionSource::Default,
                    4,
                    5_000.0,
                ),
                run(
                    "upload",
                    "u2",
                    Some(spare),
                    AttributionSource::Default,
                    4,
                    5_100.0,
                ),
            ],
        )
        .await
        .unwrap();
    repos
        .athlete_gear
        .record_default_attributions(
            other,
            &[run(
                "strava",
                "lone",
                Some(theirs),
                AttributionSource::Default,
                3,
                8_000.0,
            )],
        )
        .await
        .unwrap();

    // The deleted upload's choice moves to the Strava copy of its workout.
    repos
        .athlete_gear
        .release_gear_attribution(user, &key("upload", "u1"), Some(&key("strava", "s1")))
        .await
        .unwrap();
    let moved = repos
        .athlete_gear
        .list_gear_attributions(user, day(1), day(3))
        .await
        .unwrap();
    let on_strava = moved
        .iter()
        .find(|attribution| attribution.key == key("strava", "s1"))
        .expect("the choice moved to the remaining copy");
    assert_eq!(on_strava.gear_id, Some(spare));
    assert_eq!(on_strava.source, AttributionSource::Declared);
    assert!((on_strava.distance_meters - 12_000.0).abs() < 1e-6);
    assert!(moved
        .iter()
        .all(|attribution| attribution.key != key("upload", "u1")));

    // A workout's only copy, deleted: the pair stops counting it.
    repos
        .athlete_gear
        .release_gear_attribution(user, &key("strava", "lone"), None)
        .await
        .unwrap();
    let road_now = pair(repos, user, road).await;
    assert_eq!(road_now.activity_count, 1, "{road_now:?}");
    assert!((road_now.attributed_distance_meters - 5_000.0).abs() < 1e-6);

    // A successor holding its own attribution keeps it; the workout counts once.
    repos
        .athlete_gear
        .release_gear_attribution(user, &key("upload", "u2"), Some(&key("strava", "held")))
        .await
        .unwrap();
    let spare_now = pair(repos, user, spare).await;
    assert_eq!(spare_now.activity_count, 1, "{spare_now:?}");
    assert!((spare_now.attributed_distance_meters - 12_000.0).abs() < 1e-6);
    assert_eq!(pair(repos, user, road).await.activity_count, 1);

    // Releasing a copy that holds nothing changes nothing, and another
    // athlete's attribution under the same key stays theirs.
    repos
        .athlete_gear
        .release_gear_attribution(user, &key("strava", "never"), Some(&key("strava", "held")))
        .await
        .unwrap();
    assert_eq!(pair(repos, user, road).await.activity_count, 1);
    assert_eq!(pair(repos, other, theirs).await.activity_count, 1);

    // With nothing left on it, the pair can be deleted.
    repos
        .athlete_gear
        .release_gear_attribution(user, &key("strava", "held"), None)
        .await
        .unwrap();
    assert_eq!(
        repos.athlete_gear.delete_gear(user, road).await.unwrap(),
        GearDeletion::Deleted
    );
}

#[tokio::test]
async fn retiring_keeps_distance_and_history_and_delete_waits_for_no_attribution() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user(repos).await;
    let mut pinned = shoe("Old pair", SportType::Run);
    pinned.pinned = true;
    let worn = repos
        .athlete_gear
        .create_gear(user, &pinned, day(0))
        .await
        .unwrap();
    let spare = repos
        .athlete_gear
        .create_gear(user, &shoe("Spare", SportType::Run), day(0))
        .await
        .unwrap();
    repos
        .athlete_gear
        .record_default_attributions(
            user,
            &[run(
                "strava",
                "r1",
                Some(worn),
                AttributionSource::Default,
                1,
                42_195.0,
            )],
        )
        .await
        .unwrap();

    assert!(repos
        .athlete_gear
        .set_gear_retired(user, worn, Some(day(30)))
        .await
        .unwrap());
    let retired = pair(repos, user, worn).await;
    assert_eq!(retired.retired_at, Some(day(30)));
    assert!(!retired.declaration.pinned, "a retired pair is no default");
    assert_eq!(retired.activity_count, 1);
    assert!((retired.distance_meters() - 42_195.0).abs() < 1e-6);

    assert_eq!(
        repos.athlete_gear.delete_gear(user, worn).await.unwrap(),
        GearDeletion::Attributed
    );
    assert!(repos
        .athlete_gear
        .get_gear(user, worn)
        .await
        .unwrap()
        .is_some());

    assert!(repos
        .athlete_gear
        .set_gear_retired(user, worn, None)
        .await
        .unwrap());
    assert_eq!(pair(repos, user, worn).await.retired_at, None);

    assert_eq!(
        repos.athlete_gear.delete_gear(user, spare).await.unwrap(),
        GearDeletion::Deleted
    );
    assert_eq!(
        repos.athlete_gear.delete_gear(user, spare).await.unwrap(),
        GearDeletion::NotFound
    );
    let left: Vec<Uuid> = repos
        .athlete_gear
        .list_gear(user)
        .await
        .unwrap()
        .into_iter()
        .map(|g| g.id)
        .collect();
    assert_eq!(left, vec![worn]);
}

#[tokio::test]
async fn one_pair_per_sport_is_pinned() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user(repos).await;
    let pinned_shoe = |name: &str, sport: SportType| GearDeclaration {
        pinned: true,
        ..shoe(name, sport)
    };
    let first = repos
        .athlete_gear
        .create_gear(user, &pinned_shoe("First", SportType::Run), day(0))
        .await
        .unwrap();
    let trail = repos
        .athlete_gear
        .create_gear(user, &pinned_shoe("Trail", SportType::TrailRunning), day(0))
        .await
        .unwrap();
    let second = repos
        .athlete_gear
        .create_gear(user, &pinned_shoe("Second", SportType::Run), day(1))
        .await
        .unwrap();

    assert!(!pair(repos, user, first).await.declaration.pinned);
    assert!(pair(repos, user, second).await.declaration.pinned);
    assert!(
        pair(repos, user, trail).await.declaration.pinned,
        "another sport's pin stays"
    );

    // Pinning through an update moves the pin back.
    let mut repinned = pair(repos, user, first).await.declaration;
    repinned.pinned = true;
    repinned.name = "First, renamed".to_owned();
    assert!(repos
        .athlete_gear
        .update_gear(user, first, &repinned)
        .await
        .unwrap());
    let first_now = pair(repos, user, first).await;
    assert!(first_now.declaration.pinned);
    assert_eq!(first_now.declaration.name, "First, renamed");
    assert!(!pair(repos, user, second).await.declaration.pinned);
}

#[tokio::test]
async fn one_athlete_never_reads_or_writes_anothers_ledger() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let owner = fresh_user(repos).await;
    let other = fresh_user(repos).await;
    let id = repos
        .athlete_gear
        .create_gear(owner, &shoe("Mine", SportType::Run), day(0))
        .await
        .unwrap();
    repos
        .athlete_gear
        .record_default_attributions(
            owner,
            &[run(
                "strava",
                "o1",
                Some(id),
                AttributionSource::Default,
                1,
                5_000.0,
            )],
        )
        .await
        .unwrap();

    assert!(repos
        .athlete_gear
        .list_gear(other)
        .await
        .unwrap()
        .is_empty());
    assert!(repos
        .athlete_gear
        .get_gear(other, id)
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .athlete_gear
        .list_gear_attributions(other, day(0), day(5))
        .await
        .unwrap()
        .is_empty());
    let mut renamed = shoe("Taken", SportType::Run);
    renamed.pinned = true;
    assert!(!repos
        .athlete_gear
        .update_gear(other, id, &renamed)
        .await
        .unwrap());
    assert!(!repos
        .athlete_gear
        .set_gear_retired(other, id, Some(day(2)))
        .await
        .unwrap());
    assert_eq!(
        repos.athlete_gear.delete_gear(other, id).await.unwrap(),
        GearDeletion::NotFound
    );

    let mine = pair(repos, owner, id).await;
    assert_eq!(mine.declaration.name, "Mine");
    assert!(!mine.declaration.pinned);
    assert_eq!(mine.retired_at, None);
    assert_eq!(mine.activity_count, 1);
}

#[tokio::test]
async fn disconnecting_a_provider_clears_its_attributions_and_keeps_the_pairs() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user(repos).await;
    let other = fresh_user(repos).await;
    let id = repos
        .athlete_gear
        .create_gear(user, &shoe("Pair", SportType::Run), day(0))
        .await
        .unwrap();
    let theirs = repos
        .athlete_gear
        .create_gear(other, &shoe("Theirs", SportType::Run), day(0))
        .await
        .unwrap();
    repos
        .athlete_gear
        .record_default_attributions(
            user,
            &[
                run(
                    "strava",
                    "s1",
                    Some(id),
                    AttributionSource::Default,
                    1,
                    7_000.0,
                ),
                run(
                    "garmin",
                    "g1",
                    Some(id),
                    AttributionSource::Default,
                    2,
                    9_000.0,
                ),
            ],
        )
        .await
        .unwrap();
    repos
        .athlete_gear
        .record_default_attributions(
            other,
            &[run(
                "strava",
                "t1",
                Some(theirs),
                AttributionSource::Default,
                1,
                3_000.0,
            )],
        )
        .await
        .unwrap();

    let purge = repos
        .provider_data
        .purge_user_provider_data(
            user,
            &TenantId::generate(),
            "strava",
            PurgeReason::AthleteDisconnect,
        )
        .await
        .unwrap();
    assert_eq!(purge.removed_from("activity_gear"), 1);

    let kept = pair(repos, user, id).await;
    assert_eq!(kept.activity_count, 1, "the Garmin attribution stays");
    assert!((kept.attributed_distance_meters - 9_000.0).abs() < 1e-6);
    assert_eq!(
        pair(repos, other, theirs).await.activity_count,
        1,
        "another athlete's attributions stay"
    );
}

#[tokio::test]
async fn an_athlete_declares_at_most_the_bounded_number_of_pairs() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user(repos).await;
    for n in 0..MAX_GEAR_PER_ATHLETE {
        repos
            .athlete_gear
            .create_gear(user, &shoe(&format!("Pair {n}"), SportType::Walk), day(0))
            .await
            .unwrap();
    }
    let refused = repos
        .athlete_gear
        .create_gear(user, &shoe("One too many", SportType::Walk), day(0))
        .await;
    assert!(refused.is_err());
    assert_eq!(
        i64::try_from(repos.athlete_gear.list_gear(user).await.unwrap().len()).unwrap(),
        MAX_GEAR_PER_ATHLETE
    );
}
