// ABOUTME: Covers the place a cached row carries: list syncs keep it, reads annotate rows, a gone place reads as none
// ABOUTME: And the athlete_places repository: a clustering never undoes an athlete's name, and due_users finds whom to recompute

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Slice 6 (carnet#1484) persistence. `create_test_db` opens whichever
//! `DATABASE_URL` names, isolated per test, so the same assertions cover
//! `SQLite` and `PostgreSQL`, and `due_users` sees this test's rows only.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::models::{
    Activity, ActivityBuilder, CachedRowKey, PlaceRef, SportType, TenantId, User,
};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{
    AthletePlace, CachedActivityRow, CachedRowsQuery, PlaceNameSource, PlaceRun,
};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// The clustering version these tests record places at.
const VERSION: u32 = 1;

struct World {
    db: Database,
    user: Uuid,
    tenant: TenantId,
}

impl World {
    async fn new() -> Self {
        let db = create_test_db().await.unwrap();
        let user = new_user(db.repositories()).await;
        Self {
            db,
            user,
            tenant: TenantId::generate(),
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        self.db.repositories()
    }

    async fn sync(&self, provider: &str, activities: &[Activity], at: DateTime<Utc>) {
        self.repos()
            .activity_cache
            .upsert_activities_synced_at(self.user, &self.tenant, provider, activities, at)
            .await
            .unwrap();
    }

    async fn rows(&self) -> Vec<CachedActivityRow> {
        let query = CachedRowsQuery::window(Utc::now() - Duration::days(4_000), Utc::now(), 100);
        self.repos()
            .activity_cache
            .get_cached_activity_rows(self.user, &self.tenant, &query)
            .await
            .unwrap()
    }

    async fn row(&self, key: &CachedRowKey) -> CachedActivityRow {
        self.rows()
            .await
            .into_iter()
            .find(|row| row.provider == key.provider && row.activity.id() == key.activity_id)
            .unwrap_or_else(|| panic!("no cached row {key:?}"))
    }

    async fn places(&self) -> Vec<AthletePlace> {
        self.repos()
            .athlete_places
            .list(&self.tenant, self.user)
            .await
            .unwrap()
    }

    async fn run_now(&self) -> PlaceRun {
        let stamp = self
            .repos()
            .athlete_places
            .cached_rows_stamp(&self.tenant, self.user)
            .await
            .unwrap();
        PlaceRun {
            rows_seen: stamp.rows,
            newest_synced_at: stamp.newest_synced_at,
            version: VERSION,
            computed_at: Utc::now(),
            home_declared_at_seen: None,
            newest_track_at_seen: stamp.newest_track_at,
        }
    }

    async fn cluster(
        &self,
        places: &[AthletePlace],
        assignments: &[(CachedRowKey, Option<String>)],
    ) {
        let run = self.run_now().await;
        self.repos()
            .athlete_places
            .replace_clustering(&self.tenant, self.user, places, assignments, &run)
            .await
            .unwrap();
    }

    async fn due(&self, version: u32, now: DateTime<Utc>) -> bool {
        self.repos()
            .athlete_places
            .due_users(100, version, now)
            .await
            .unwrap()
            .contains(&(self.tenant, self.user))
    }
}

async fn new_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("place-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Place Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

fn ride(id: &str, name: &str, start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(id, name, SportType::Ride, start, 3_600, "strava")
        .distance_meters(30_000.0)
        .build()
}

fn day(year: i32, month: u32, d: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, d, 8, 0, 0).unwrap()
}

fn key(provider: &str, activity_id: &str) -> CachedRowKey {
    CachedRowKey {
        provider: provider.to_owned(),
        activity_id: activity_id.to_owned(),
    }
}

fn place(id: &str, name: Option<(&str, PlaceNameSource)>, count: u32) -> AthletePlace {
    AthletePlace {
        id: id.to_owned(),
        centroid_lat: 45.5,
        centroid_lon: -73.6,
        radius_m: 300.0,
        activity_count: count,
        name: name.map(|(name, _)| name.to_owned()),
        name_source: name.map(|(_, source)| source),
        version: VERSION,
        computed_at: Utc::now(),
        created_at: Utc::now(),
    }
}

/// Run one statement binding text values, on whichever engine the test opened.
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

/// The `(key, id)` of every row a read of `ids` (and `keys`) around `start` returns, sorted.
async fn read_ids(
    w: &World,
    start: DateTime<Utc>,
    ids: &[&str],
    keys: Option<&[&str]>,
) -> Vec<(String, String)> {
    let mut query =
        CachedRowsQuery::window(start - Duration::days(1), start + Duration::days(1), 50)
            .with_ids(ids);
    if let Some(keys) = keys {
        query = query.under_keys(keys);
    }
    let mut rows: Vec<(String, String)> = w
        .repos()
        .activity_cache
        .get_cached_activity_rows(w.user, &w.tenant, &query)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.provider, row.activity.id().to_owned()))
        .collect();
    rows.sort();
    rows
}

#[tokio::test]
async fn a_list_sync_keeps_the_rows_place_whether_or_not_its_data_changed() {
    let w = World::new().await;
    let first_sync = Utc::now() - Duration::hours(2);
    let (kept, changed) = (key("strava", "kept"), key("strava", "changed"));
    w.sync(
        "strava",
        &[
            ride("kept", "Gai-Luron loop", day(2026, 9, 1)),
            ride("changed", "Commute", day(2026, 9, 2)),
        ],
        first_sync,
    )
    .await;
    w.cluster(
        &[place("p-gai-luron", None, 2)],
        &[
            (kept.clone(), Some("p-gai-luron".to_owned())),
            (changed.clone(), Some("p-gai-luron".to_owned())),
        ],
    )
    .await;

    let second_sync = Utc::now() - Duration::minutes(1);
    w.sync(
        "strava",
        &[
            ride("kept", "Gai-Luron loop", day(2026, 9, 1)),
            ride("changed", "Commute, renamed", day(2026, 9, 2)),
        ],
        second_sync,
    )
    .await;

    for (row, what) in [(&kept, "an unchanged row"), (&changed, "a changed row")] {
        assert_eq!(
            w.row(row).await.place.map(|place| place.id),
            Some("p-gai-luron".to_owned()),
            "bug: a list sync dropped the place of {what}"
        );
    }
}

#[tokio::test]
async fn a_window_read_annotates_each_row_and_a_gone_place_reads_as_none() {
    let w = World::new().await;
    let (named, city, bare) = (
        key("strava", "named"),
        key("strava", "city"),
        key("strava", "bare"),
    );
    w.sync(
        "strava",
        &[
            ride("named", "Gai-Luron", day(2026, 8, 1)),
            ride("city", "Montréal loop", day(2026, 8, 2)),
            ride("bare", "Trainer", day(2026, 8, 3)),
        ],
        Utc::now(),
    )
    .await;
    w.cluster(
        &[place(
            "p-city",
            Some(("Montréal", PlaceNameSource::City)),
            1,
        )],
        &[(city.clone(), Some("p-city".to_owned()))],
    )
    .await;
    w.repos()
        .athlete_places
        .create_athlete_place(
            &w.tenant,
            w.user,
            &place("p-named", Some(("Gai-Luron", PlaceNameSource::Athlete)), 1),
            &named,
        )
        .await
        .unwrap();

    assert_eq!(
        w.row(&named).await.place,
        Some(PlaceRef {
            id: "p-named".to_owned(),
            name: Some("Gai-Luron".to_owned()),
            athlete_named: true,
        }),
        "bug: an athlete-named place is not read as athlete-named"
    );
    assert_eq!(
        w.row(&city).await.place,
        Some(PlaceRef {
            id: "p-city".to_owned(),
            name: Some("Montréal".to_owned()),
            athlete_named: false,
        }),
        "bug: a city-named place is read as athlete-named or without its name"
    );
    assert_eq!(
        w.row(&bare).await.place,
        None,
        "bug: a row no clustering assigned carries a place"
    );

    execute(
        &w.db,
        "DELETE FROM athlete_places WHERE id = $1",
        &["p-city"],
    )
    .await;
    assert_eq!(
        w.row(&city).await.place,
        None,
        "bug: a place_id whose place is gone must read as no place, not an error"
    );
}

#[tokio::test]
async fn with_ids_reads_every_copy_of_each_named_activity_and_no_other_row() {
    let w = World::new().await;
    let start = day(2026, 6, 1);
    w.sync(
        "strava",
        &[
            ride("a", "A", start),
            ride("b", "B", start + Duration::hours(1)),
            ride("c", "C", start + Duration::hours(2)),
        ],
        Utc::now(),
    )
    .await;
    w.sync(
        "sciotte_strava",
        &[ride("a", "A mirror", start)],
        Utc::now(),
    )
    .await;

    let pair = |p: &str, id: &str| (p.to_owned(), id.to_owned());
    assert_eq!(
        read_ids(&w, start, &["a", "b"], None).await,
        vec![
            pair("sciotte_strava", "a"),
            pair("strava", "a"),
            pair("strava", "b")
        ],
        "bug: with_ids must read every copy of each named id and no other row"
    );
    assert_eq!(
        read_ids(&w, start, &["a"], Some(&["sciotte_strava"])).await,
        vec![pair("sciotte_strava", "a")],
        "bug: with_ids does not combine with under_keys"
    );
    assert!(
        read_ids(&w, start, &[], None).await.is_empty(),
        "bug: an empty id set must read no row, not every row"
    );
}

#[tokio::test]
async fn a_clustering_never_writes_or_drops_an_athletes_name() {
    let w = World::new().await;
    let (a, b) = (key("strava", "a"), key("strava", "b"));
    w.sync(
        "strava",
        &[
            ride("a", "A", day(2026, 4, 1)),
            ride("b", "B", day(2026, 4, 2)),
        ],
        Utc::now(),
    )
    .await;
    w.repos()
        .athlete_places
        .create_athlete_place(
            &w.tenant,
            w.user,
            &place("p-named", Some(("Gai-Luron", PlaceNameSource::Athlete)), 1),
            &a,
        )
        .await
        .unwrap();

    let mut moved = place("p-named", Some(("Mont-Royal", PlaceNameSource::City)), 5);
    moved.centroid_lat = 45.6;
    w.cluster(
        &[
            moved,
            place("p-city", Some(("Montréal", PlaceNameSource::City)), 3),
            place("p-claimed", Some(("Stale", PlaceNameSource::Athlete)), 3),
        ],
        &[(b.clone(), Some("p-city".to_owned()))],
    )
    .await;
    let places = w.places().await;
    let by_id = |id: &str| places.iter().find(|p| p.id == id).cloned();
    let named = by_id("p-named").unwrap();
    assert_eq!(
        (named.name.as_deref(), named.name_source),
        (Some("Gai-Luron"), Some(PlaceNameSource::Athlete)),
        "bug: a clustering overwrote the athlete's name"
    );
    assert_eq!(
        (named.activity_count, named.centroid_lat),
        (5, 45.6),
        "bug: a clustering did not update an athlete-named place's members"
    );
    assert_eq!(
        by_id("p-claimed").map(|p| (p.name, p.name_source)),
        Some((None, None)),
        "bug: a clustering wrote an athlete name only set_name may write"
    );
    assert_eq!(
        w.row(&b).await.place.map(|p| p.id),
        Some("p-city".to_owned()),
        "bug: the clustering's assignment was not written"
    );

    w.cluster(&[], &[]).await;
    let ids: Vec<String> = w.places().await.into_iter().map(|p| p.id).collect();
    assert_eq!(
        ids,
        vec!["p-named".to_owned()],
        "bug: a clustering kept an unnamed place it left out, or dropped the athlete's"
    );
    assert_eq!(
        w.row(&b).await.place,
        None,
        "bug: a row kept the place_id of a place the clustering dropped"
    );
    assert_eq!(
        w.row(&a).await.place.map(|p| p.id),
        Some("p-named".to_owned()),
        "bug: a row lost the athlete place it belongs to"
    );
}

#[tokio::test]
async fn set_name_names_and_clears_only_the_athletes_name_in_its_own_scope() {
    let w = World::new().await;
    let a = key("strava", "a");
    w.sync("strava", &[ride("a", "A", day(2026, 3, 1))], Utc::now())
        .await;
    w.cluster(
        &[place("p", Some(("Montréal", PlaceNameSource::City)), 3)],
        &[(a, Some("p".to_owned()))],
    )
    .await;
    let places = &w.repos().athlete_places;

    let cleared = places.set_name(&w.tenant, w.user, "p", None).await.unwrap();
    assert_eq!(
        cleared.map(|p| (p.name, p.name_source)),
        Some((Some("Montréal".to_owned()), Some(PlaceNameSource::City))),
        "bug: clearing a place the athlete never named dropped its automatic name"
    );
    let named = places
        .set_name(&w.tenant, w.user, "p", Some("Forêt Héritage"))
        .await
        .unwrap();
    assert_eq!(
        named.map(|p| (p.name, p.name_source)),
        Some((
            Some("Forêt Héritage".to_owned()),
            Some(PlaceNameSource::Athlete)
        )),
        "bug: set_name did not store the athlete's name"
    );
    let cleared = places.set_name(&w.tenant, w.user, "p", None).await.unwrap();
    assert_eq!(
        cleared.map(|p| (p.name, p.name_source)),
        Some((None, None)),
        "bug: clearing the athlete's name kept it"
    );

    let stranger = new_user(w.repos()).await;
    for (tenant, user, what) in [
        (w.tenant, stranger, "another user"),
        (TenantId::generate(), w.user, "another tenant"),
    ] {
        assert!(
            places
                .set_name(&tenant, user, "p", Some("Hijack"))
                .await
                .unwrap()
                .is_none(),
            "bug: {what} could name the user's place"
        );
        assert!(
            places.list(&tenant, user).await.unwrap().is_empty(),
            "bug: {what} lists the user's place"
        );
    }
    assert_eq!(
        w.places().await[0].name,
        None,
        "bug: a name set from another scope reached the user's place"
    );
}

#[tokio::test]
async fn a_place_id_another_user_holds_is_refused_and_nothing_is_written() {
    let w = World::new().await;
    w.sync("strava", &[ride("a", "A", day(2026, 2, 1))], Utc::now())
        .await;
    w.cluster(
        &[place(
            "p-shared",
            Some(("Montréal", PlaceNameSource::City)),
            3,
        )],
        &[],
    )
    .await;
    let stranger = new_user(w.repos()).await;
    let run = w.run_now().await;

    let refused = w
        .repos()
        .athlete_places
        .replace_clustering(
            &w.tenant,
            stranger,
            &[place(
                "p-shared",
                Some(("Elsewhere", PlaceNameSource::City)),
                9,
            )],
            &[],
            &run,
        )
        .await;
    assert!(
        refused.is_err(),
        "bug: a place id another user holds was taken over"
    );
    assert!(
        w.repos()
            .athlete_places
            .list(&w.tenant, stranger)
            .await
            .unwrap()
            .is_empty(),
        "bug: a refused clustering wrote places"
    );
    assert_eq!(
        w.places().await[0].name.as_deref(),
        Some("Montréal"),
        "bug: a refused clustering changed the other user's place"
    );
}

#[tokio::test]
async fn creating_an_athlete_place_needs_a_row_the_user_holds() {
    let w = World::new().await;
    let created = w
        .repos()
        .athlete_places
        .create_athlete_place(
            &w.tenant,
            w.user,
            &place("p-lonely", Some(("Nowhere", PlaceNameSource::Athlete)), 1),
            &key("strava", "missing"),
        )
        .await;
    assert!(
        created.is_err(),
        "bug: a place was created for a row the user does not hold"
    );
    assert!(
        w.places().await.is_empty(),
        "bug: a refused single-visit place was still written"
    );
}

#[tokio::test]
async fn due_users_finds_new_rows_age_version_and_erasure_and_nothing_else() {
    let w = World::new().await;
    let now = Utc::now();
    w.sync(
        "strava",
        &[ride("a", "A", day(2026, 1, 1))],
        now - Duration::hours(1),
    )
    .await;
    assert!(
        w.due(VERSION, now).await,
        "bug: a user never clustered is not due"
    );

    w.cluster(&[], &[]).await;
    assert!(
        !w.due(VERSION, Utc::now()).await,
        "bug: a user just clustered is due"
    );
    assert!(
        w.due(VERSION + 1, Utc::now()).await,
        "bug: a user clustered by another version is not due"
    );
    assert!(
        w.due(VERSION, Utc::now() + Duration::hours(25)).await,
        "bug: a user clustered over 24 hours ago is not due"
    );

    w.sync("strava", &[ride("b", "B", day(2026, 1, 2))], Utc::now())
        .await;
    assert!(
        w.due(VERSION, Utc::now()).await,
        "bug: a user with a new row is not due"
    );
    w.cluster(&[], &[]).await;
    assert!(
        !w.due(VERSION, Utc::now()).await,
        "bug: a re-clustered user is still due"
    );

    let cache = &w.repos().activity_cache;
    for id in ["a", "b"] {
        cache
            .delete_cached_activity(w.user, &w.tenant, "strava", id)
            .await
            .unwrap();
    }
    assert!(
        w.due(VERSION, Utc::now()).await,
        "bug: a user whose rows were all erased is not due, so their places outlive them"
    );
    w.cluster(&[], &[]).await;
    assert!(
        !w.due(VERSION, Utc::now()).await,
        "bug: a user with no rows, clustered as such, is due forever"
    );
}
