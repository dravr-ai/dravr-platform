// ABOUTME: Covers the order and robustness of the places worker's due scan: a bad id is skipped, a failure goes last
// ABOUTME: A row whose ids are not uuids takes no slot, a failed user waits behind the rest, a home declaration stays due till seen

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Slice 6 (carnet#1484) due scan. `create_test_db` opens whichever
//! `DATABASE_URL` names, isolated per test, so the fleet-wide scan sees this
//! test's rows only, on `SQLite` and `PostgreSQL` alike.

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::models::{ActivityBuilder, HomeDeclaration, SportType, TenantId, User};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::PlaceRun;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// The clustering version these tests run at.
const VERSION: u32 = 1;

/// Tenants whose order the scan's tie-break follows.
const FIRST_TENANT: &str = "00000000-0000-4000-8000-000000000001";
const SECOND_TENANT: &str = "00000000-0000-4000-8000-000000000002";

/// One user of the shared database, in one tenant.
#[derive(Clone, Copy)]
struct Athlete {
    user: Uuid,
    tenant: TenantId,
}

async fn athlete(repos: &RepositoryRegistry, tenant: &str) -> Athlete {
    let user = User::new(
        format!("places-due-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Places Due Tester".to_owned()),
    );
    Athlete {
        user: repos.users.create(&user).await.unwrap(),
        tenant: TenantId::parse_str(tenant).unwrap(),
    }
}

/// Store one ride for `who`, synced at `at`.
async fn ride(repos: &RepositoryRegistry, who: Athlete, id: &str, at: DateTime<Utc>) {
    let start = Utc.with_ymd_and_hms(2026, 1, 1, 8, 0, 0).unwrap();
    let activity = ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3_600, "strava")
        .distance_meters(30_000.0)
        .build();
    repos
        .activity_cache
        .upsert_activities_synced_at(who.user, &who.tenant, "strava", &[activity], at)
        .await
        .unwrap();
}

/// Record a clustering of `who` at `at` that saw every row they hold.
async fn cluster(repos: &RepositoryRegistry, who: Athlete, at: DateTime<Utc>) {
    cluster_seeing(repos, who, at, None).await;
}

/// Record a clustering of `who` at `at` that saw every row they hold and the
/// home declaration made at `seen`.
async fn cluster_seeing(
    repos: &RepositoryRegistry,
    who: Athlete,
    at: DateTime<Utc>,
    seen: Option<DateTime<Utc>>,
) {
    let places = &repos.athlete_places;
    let stamp = places
        .cached_rows_stamp(&who.tenant, who.user)
        .await
        .unwrap();
    let run = PlaceRun {
        rows_seen: stamp.rows,
        newest_synced_at: stamp.newest_synced_at,
        version: VERSION,
        computed_at: at,
        home_declared_at_seen: seen,
    };
    places
        .replace_clustering(&who.tenant, who.user, &[], &[], &run)
        .await
        .unwrap();
}

async fn due(repos: &RepositoryRegistry, limit: u32, now: DateTime<Utc>) -> Vec<(TenantId, Uuid)> {
    repos
        .athlete_places
        .due_users(limit, VERSION, now)
        .await
        .unwrap()
}

async fn execute(db: &Database, sql: &str, binds: &[&str]) {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            let mut query = sqlx::query(sql);
            for value in binds {
                query = query.bind(*value);
            }
            query.execute(sqlite.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            let mut query = sqlx::query(sql);
            for value in binds {
                query = query.bind(*value);
            }
            query.execute(pg.pool()).await.unwrap();
        }
    }
}

#[tokio::test]
async fn a_row_whose_ids_are_not_uuids_is_skipped_and_takes_no_slot() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let now = Utc::now();
    let valid = athlete(repos, FIRST_TENANT).await;
    ride(repos, valid, "a", now - Duration::hours(2)).await;
    cluster(repos, valid, now - Duration::hours(1)).await;
    ride(repos, valid, "b", now).await;

    // A run left under a tenant id that is not a uuid, tried before the
    // valid user, and due since it saw rows that are gone: it heads the scan.
    let user = valid.user.to_string();
    execute(
        &db,
        "INSERT INTO athlete_place_runs (tenant_id, user_id, rows_seen, newest_synced_at, version, computed_at) \
         VALUES ('legacy-tenant', $1, 3, NULL, 1, '2020-01-01T00:00:00+00:00')",
        &[user.as_str()],
    )
    .await;

    assert_eq!(
        due(repos, 1, now).await,
        vec![(valid.tenant, valid.user)],
        "bug: a row with a non-uuid id failed the fleet's scan or took the only slot"
    );
    assert_eq!(
        due(repos, 100, now).await,
        vec![(valid.tenant, valid.user)],
        "bug: the scan answered more than the one valid due user"
    );
}

#[tokio::test]
async fn a_failed_recompute_goes_behind_every_user_tried_before_and_stays_due() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let now = Utc::now();
    let first = athlete(repos, FIRST_TENANT).await;
    let second = athlete(repos, SECOND_TENANT).await;
    ride(repos, first, "a", now - Duration::hours(1)).await;
    ride(repos, second, "b", now - Duration::hours(1)).await;
    assert_eq!(
        due(repos, 1, now).await,
        vec![(first.tenant, first.user)],
        "bug: of two users never tried, the tie-break did not pick the first tenant"
    );

    repos
        .athlete_places
        .record_failed_recompute(&first.tenant, first.user, VERSION, now)
        .await
        .unwrap();
    assert_eq!(
        due(repos, 1, now).await,
        vec![(second.tenant, second.user)],
        "bug: a user whose recompute just failed still holds the head of the scan"
    );
    assert_eq!(
        due(repos, 100, now).await,
        vec![(second.tenant, second.user), (first.tenant, first.user)],
        "bug: a user with rows whose first recompute failed is no longer due"
    );

    repos
        .athlete_places
        .record_failed_recompute(
            &second.tenant,
            second.user,
            VERSION,
            now + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(
        due(repos, 1, now).await,
        vec![(first.tenant, first.user)],
        "bug: the user tried longest ago is not first"
    );
}

#[tokio::test]
async fn a_failed_recompute_keeps_new_rows_due_and_backs_the_daily_backstop_off_a_day() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let now = Utc::now();
    let who = athlete(repos, FIRST_TENANT).await;
    ride(repos, who, "a", now - Duration::hours(30)).await;
    cluster(repos, who, now - Duration::hours(25)).await;
    assert_eq!(
        due(repos, 100, now).await,
        vec![(who.tenant, who.user)],
        "bug: a clustering over a day old is not due"
    );

    let places = &repos.athlete_places;
    places
        .record_failed_recompute(&who.tenant, who.user, VERSION, now)
        .await
        .unwrap();
    assert!(
        due(repos, 100, now).await.is_empty(),
        "bug: a user due only for the backstop is retried at once after a failure"
    );
    assert_eq!(
        due(repos, 100, now + Duration::hours(25)).await,
        vec![(who.tenant, who.user)],
        "bug: a failed backstop recompute is never retried"
    );

    ride(repos, who, "b", now).await;
    places
        .record_failed_recompute(&who.tenant, who.user, VERSION, now)
        .await
        .unwrap();
    assert_eq!(
        due(repos, 100, now).await,
        vec![(who.tenant, who.user)],
        "bug: a failure made a user with a new row not due"
    );
}

/// Declare `who`'s home elsewhere at `at`, and return the date as stored.
async fn declare(repos: &RepositoryRegistry, who: Athlete, at: DateTime<Utc>) -> DateTime<Utc> {
    let places = &repos.athlete_places;
    places
        .store_home_declaration(&who.tenant, who.user, HomeDeclaration::Elsewhere, at)
        .await
        .unwrap();
    places
        .home_declaration(&who.tenant, who.user)
        .await
        .unwrap()
        .declared_at
        .expect("bug: a stored declaration has no date")
}

#[tokio::test]
async fn a_home_declaration_is_due_until_a_run_sees_it() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let now = Utc::now();
    let who = athlete(repos, FIRST_TENANT).await;
    ride(repos, who, "a", now - Duration::hours(2)).await;
    cluster(repos, who, now - Duration::hours(1)).await;
    assert!(
        due(repos, 100, now).await.is_empty(),
        "bug: a user whose run saw everything is due"
    );

    let declared_at = declare(repos, who, now - Duration::minutes(30)).await;
    assert_eq!(
        due(repos, 100, now).await,
        vec![(who.tenant, who.user)],
        "bug: a home declaration did not make the user due"
    );

    cluster_seeing(repos, who, now - Duration::minutes(10), Some(declared_at)).await;
    assert!(
        due(repos, 100, now).await.is_empty(),
        "bug: a run that saw the declaration left the user due"
    );
}

#[tokio::test]
async fn a_user_who_declares_with_no_rows_is_due() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let now = Utc::now();
    let who = athlete(repos, FIRST_TENANT).await;
    let declared_at = declare(repos, who, now - Duration::minutes(5)).await;
    assert_eq!(
        due(repos, 100, now).await,
        vec![(who.tenant, who.user)],
        "bug: a declaration by a user with no rows and no run is not due"
    );
    cluster_seeing(repos, who, now, Some(declared_at)).await;
    assert!(
        due(repos, 100, now).await.is_empty(),
        "bug: a run with no rows that saw the declaration left the user due"
    );
}

/// A recompute read the declaration, then the athlete declared again before
/// it wrote: the run records the older date, and the user stays due.
#[tokio::test]
async fn a_declaration_made_while_a_run_was_in_flight_stays_due() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let now = Utc::now();
    let who = athlete(repos, FIRST_TENANT).await;
    ride(repos, who, "a", now - Duration::hours(2)).await;
    let read_by_the_run = declare(repos, who, now - Duration::minutes(30)).await;
    declare(repos, who, now - Duration::minutes(20)).await;
    cluster_seeing(
        repos,
        who,
        now - Duration::minutes(10),
        Some(read_by_the_run),
    )
    .await;
    assert_eq!(
        due(repos, 100, now).await,
        vec![(who.tenant, who.user)],
        "bug: a declaration newer than the one the run saw is not due"
    );
}

#[tokio::test]
async fn a_failed_recompute_after_a_declaration_keeps_it_due() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let now = Utc::now();
    let who = athlete(repos, FIRST_TENANT).await;
    ride(repos, who, "a", now - Duration::hours(2)).await;
    cluster(repos, who, now - Duration::hours(1)).await;
    declare(repos, who, now - Duration::minutes(30)).await;
    repos
        .athlete_places
        .record_failed_recompute(&who.tenant, who.user, VERSION, now)
        .await
        .unwrap();
    assert_eq!(
        due(repos, 100, now).await,
        vec![(who.tenant, who.user)],
        "bug: a failure deferred a declaration no run saw to the daily backstop"
    );
}
