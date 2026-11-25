// ABOUTME: Covers the places recompute and name_place: one start per workout, the home exclusion, names, erasure, tenants
// ABOUTME: Drives the places worker's pass and the naming service against a real database, as the worker and the tool do

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Slice 6 (carnet#1484) places. Every athlete here has a home: a door
//! sixteen loops around the block start and end at, four in each direction.
//! A loop's trimmed start and end lie on the ring the privacy trim leaves
//! around the door, and the door's thirty-two endpoints, counted as one spot,
//! outnumber any other spot's, so every other spot is free to form a place.
//! The block loops never leave the door's neighbourhood, so no area takes
//! them (carnet#1723) and they stay excluded; rides from the door into an
//! area are their own tests.

use std::f64::consts::TAU;
use std::sync::atomic::{AtomicI64, Ordering};

use dravr_barrage::parse::polyline::encode_polyline;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{Activity, ActivityBuilder, SegmentEffort, SportType, TenantId, User};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{
    AthletePlace, CachedActivityRow, CachedRowsQuery, PlaceNameSource,
};
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::routes::haversine_meters_between;
use pierre_services::athlete_places::{
    declare_home, name_place, places_pass, HomeChoice, NamedPlace, PlaceRefusal, PlaceTarget,
    PlacesPassReport,
};
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// The door every athlete's home loops start and end at.
pub(crate) const DOOR: (f64, f64) = (45.500, -73.600);
/// Spots well away from the door and from each other.
pub(crate) const SUTTON: (f64, f64) = (45.100, -72.600);
const BROMONT: (f64, f64) = (45.300, -72.650);
pub(crate) const OKA: (f64, f64) = (45.470, -74.080);

/// Degrees of latitude and of longitude (at 45°N) per ~100 m.
const STEP_LAT: f64 = 0.0009;
const STEP_LON: f64 = 0.001_27;

/// A loop of four 1 km legs that starts and ends at `origin`, its first leg
/// heading the way `turn` names (0 north, 1 east, 2 south, 3 west).
pub(crate) fn loop_from(origin: (f64, f64), turn: usize) -> Vec<(f64, f64)> {
    const HEADINGS: [(f64, f64); 4] = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
    let mut points = vec![origin];
    let mut at = origin;
    for leg in 0..4 {
        let (dlat, dlon) = HEADINGS[(turn + leg) % 4];
        for _ in 0..10 {
            at = (dlat.mul_add(STEP_LAT, at.0), dlon.mul_add(STEP_LON, at.1));
            points.push(at);
        }
    }
    points
}

/// A loop around the block: four 300 m legs from `origin`, its first leg
/// heading the way `turn` names (0 north, 1 east, 2 south, 3 west). It never
/// goes farther than about 420 m from `origin`, so a ride of it from home
/// reaches no area and stays excluded.
pub(crate) fn block_from(origin: (f64, f64), turn: usize) -> Vec<(f64, f64)> {
    const HEADINGS: [(f64, f64); 4] = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
    let mut points = vec![origin];
    let mut at = origin;
    for leg in 0..4 {
        let (dlat, dlon) = HEADINGS[(turn + leg) % 4];
        for _ in 0..3 {
            at = (dlat.mul_add(STEP_LAT, at.0), dlon.mul_add(STEP_LON, at.1));
            points.push(at);
        }
    }
    points
}

/// Out 600 m north from `origin` and back on the same road: the trimmed
/// start and end sit on one spot, so the trailhead is one endpoint cluster,
/// and the ride never goes far enough for an area to take it when the
/// trailhead is taken for home.
pub(crate) fn out_and_back(origin: (f64, f64)) -> Vec<(f64, f64)> {
    let out: Vec<(f64, f64)> = (0..=6)
        .map(|i| (f64::from(i).mul_add(STEP_LAT, origin.0), origin.1))
        .collect();
    out.iter()
        .chain(out.iter().rev().skip(1))
        .copied()
        .collect()
}

/// A loop that leaves `origin` north and comes home from the south: out
/// 1 km north, 1 km east, 2 km south, 1 km west, 1 km north. Its trimmed
/// start and end sit on opposite sides of the trim ring, about 400 m apart.
fn north_out_south_back(origin: (f64, f64)) -> Vec<(f64, f64)> {
    const LEGS: [((f64, f64), usize); 5] = [
        ((1.0, 0.0), 10),
        ((0.0, 1.0), 10),
        ((-1.0, 0.0), 20),
        ((0.0, -1.0), 10),
        ((1.0, 0.0), 10),
    ];
    let mut points = vec![origin];
    let mut at = origin;
    for ((dlat, dlon), steps) in LEGS {
        for _ in 0..steps {
            at = (dlat.mul_add(STEP_LAT, at.0), dlon.mul_add(STEP_LON, at.1));
            points.push(at);
        }
    }
    points
}

/// A ride from `origin` to an area centred `north` and `east` metres off
/// and back: out in 100 m steps, 48 points around a 150 m circle there
/// (`phase` turns where it starts), and home the way it went. Most of its
/// points lie in the area, as a ride that spends its hour on the trails
/// does.
pub(crate) fn trip_to(origin: (f64, f64), north: f64, east: f64, phase: f64) -> Vec<(f64, f64)> {
    let reach = north.hypot(east);
    let at = |north_m: f64, east_m: f64| {
        (
            (north_m / 100.0).mul_add(STEP_LAT, origin.0),
            (east_m / 100.0).mul_add(STEP_LON, origin.1),
        )
    };
    let steps = (reach / 100.0).floor() as usize;
    let out: Vec<(f64, f64)> = (0..steps)
        .map(|step| {
            let share = step as f64 * 100.0 / reach;
            at(north * share, east * share)
        })
        .collect();
    let mut path = out.clone();
    path.extend((0..48).map(|i| {
        let angle = (f64::from(i) / 48.0).mul_add(TAU, phase);
        at(
            angle.cos().mul_add(150.0, north),
            angle.sin().mul_add(150.0, east),
        )
    }));
    path.extend(out.iter().rev());
    path
}

pub(crate) fn meters(a: (f64, f64), b: (f64, f64)) -> f64 {
    haversine_meters_between(a.0, a.1, b.0, b.1)
}

pub(crate) struct World {
    db: Database,
    pub(crate) user: Uuid,
    pub(crate) tenant: TenantId,
    day: AtomicI64,
}

impl World {
    pub(crate) async fn new() -> Self {
        Self::in_db(create_test_db().await.unwrap(), TenantId::generate()).await
    }

    /// Another athlete, in `tenant`, sharing this world's database.
    pub(crate) async fn beside(&self, tenant: &str) -> Self {
        Self::in_db(self.db.clone(), TenantId::parse_str(tenant).unwrap()).await
    }

    async fn in_db(db: Database, tenant: TenantId) -> Self {
        let user = User::new(
            format!("places-{}@example.com", Uuid::new_v4()),
            "argon2-hash-placeholder".to_owned(),
            Some("Places Tester".to_owned()),
        );
        let user = db.repositories().users.create(&user).await.unwrap();
        Self {
            db,
            user,
            tenant,
            day: AtomicI64::new(0),
        }
    }

    pub(crate) fn repos(&self) -> &RepositoryRegistry {
        self.db.repositories()
    }

    /// A start on a day no other ride of this world uses, so no two rides
    /// look like copies of one workout.
    pub(crate) fn next_start(&self) -> Timestamp {
        let day = self.day.fetch_add(1, Ordering::Relaxed);
        Timestamp::parse_from_rfc3339("2025-01-01T08:00:00Z").unwrap() + Elapsed::days(day)
    }

    pub(crate) async fn store(&self, tenant: &TenantId, provider: &str, activity: Activity) {
        self.repos()
            .activity_cache
            .upsert_activities_synced_at(self.user, tenant, provider, &[activity], Timestamp::now())
            .await
            .unwrap();
    }

    /// A ride with a route overview looping from `origin`, stored under
    /// Strava; returns its id.
    pub(crate) async fn ride(&self, origin: (f64, f64), turn: usize) -> String {
        self.ride_with(origin, turn, |b| b).await
    }

    pub(crate) async fn ride_with(
        &self,
        origin: (f64, f64),
        turn: usize,
        extra: impl FnOnce(ActivityBuilder) -> ActivityBuilder,
    ) -> String {
        self.ride_along(&loop_from(origin, turn), extra).await
    }

    /// A ride along `path`, stored under Strava; returns its id.
    pub(crate) async fn ride_along(
        &self,
        path: &[(f64, f64)],
        extra: impl FnOnce(ActivityBuilder) -> ActivityBuilder,
    ) -> String {
        let id = Uuid::new_v4().simple().to_string();
        let activity = extra(
            ActivityBuilder::new(
                &id,
                "Ride",
                SportType::Ride,
                self.next_start(),
                3_600,
                "strava",
            )
            .distance_meters(4_000.0)
            .summary_polyline(encode_polyline(path)),
        )
        .build();
        self.store(&self.tenant, "strava", activity).await;
        id
    }

    /// Sixteen loops around the block from the door, four in each direction.
    async fn home(&self) {
        self.home_rides().await;
    }

    /// [`Self::home`], answering the rides' ids.
    pub(crate) async fn home_rides(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for turn in 0..16 {
            ids.push(self.ride_along(&block_from(DOOR, turn % 4), |b| b).await);
        }
        ids
    }

    pub(crate) async fn pass(&self) -> PlacesPassReport {
        self.pass_at(Timestamp::now()).await
    }

    async fn pass_at(&self, now: Timestamp) -> PlacesPassReport {
        let report = places_pass(self.repos(), now, 20).await.unwrap();
        assert_eq!(report.failed, 0, "bug: a recompute failed: {report:?}");
        report
    }

    pub(crate) async fn places(&self) -> Vec<AthletePlace> {
        self.places_in(&self.tenant).await
    }

    async fn places_in(&self, tenant: &TenantId) -> Vec<AthletePlace> {
        self.repos()
            .athlete_places
            .list(tenant, self.user)
            .await
            .unwrap()
    }

    /// The one place whose centroid lies within 500 m of `spot`.
    pub(crate) async fn place_at(&self, spot: (f64, f64)) -> AthletePlace {
        let mut near: Vec<AthletePlace> = self
            .places()
            .await
            .into_iter()
            .filter(|p| meters((p.centroid_lat, p.centroid_lon), spot) < 500.0)
            .collect();
        assert_eq!(
            near.len(),
            1,
            "bug: expected one place near {spot:?}, found {near:?}"
        );
        near.remove(0)
    }

    async fn rows(&self) -> Vec<CachedActivityRow> {
        let query = CachedRowsQuery::window(Timestamp::UNIX_EPOCH, Timestamp::now(), 1_000);
        self.repos()
            .activity_cache
            .get_cached_activity_rows(self.user, &self.tenant, &query)
            .await
            .unwrap()
    }

    /// The place id each row of `id` carries, one per stored copy.
    pub(crate) async fn places_of(&self, id: &str) -> Vec<Option<String>> {
        let copies: Vec<Option<String>> = self
            .rows()
            .await
            .into_iter()
            .filter(|row| row.activity.id() == id)
            .map(|row| row.place.map(|p| p.id))
            .collect();
        assert!(!copies.is_empty(), "bug: no stored row for {id}");
        copies
    }

    pub(crate) async fn name(
        &self,
        target: PlaceTarget,
        name: Option<&str>,
    ) -> Result<NamedPlace, PlaceRefusal> {
        name_place(
            self.repos(),
            &self.tenant,
            self.user,
            target,
            name.map(str::to_owned),
        )
        .await
        .unwrap()
    }

    async fn delete(&self, id: &str) {
        self.repos()
            .activity_cache
            .delete_cached_activity(self.user, &self.tenant, "strava", id)
            .await
            .unwrap();
    }
}

/// Run one statement on the test database, every bind as text.
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

fn effort(name: &str) -> SegmentEffort {
    SegmentEffort {
        id: Uuid::new_v4().to_string(),
        segment_id: None,
        name: name.to_owned(),
        elapsed_time: 300,
        moving_time: None,
        start_date: Timestamp::now(),
        distance: 1_000.0,
        average_heart_rate: None,
        max_heart_rate: None,
        average_cadence: None,
        average_watts: None,
        kom_rank: None,
        pr_rank: None,
        climb_category: None,
        average_grade: None,
        elevation_gain: None,
    }
}

#[tokio::test]
async fn three_workouts_at_a_spot_make_a_place_and_two_make_none() {
    let world = World::new().await;
    world.home().await;
    let mut sutton = Vec::new();
    for _ in 0..3 {
        sutton.push(world.ride(SUTTON, 0).await);
    }
    for _ in 0..2 {
        world.ride(BROMONT, 0).await;
    }
    let report = world.pass().await;
    assert_eq!(report.users, 1, "bug: a user with rows and no run is due");

    let place = world.place_at(SUTTON).await;
    assert_eq!(
        place.activity_count, 3,
        "bug: three workouts started at Sutton"
    );
    for id in &sutton {
        assert_eq!(
            world.places_of(id).await,
            vec![Some(place.id.clone())],
            "bug: every Sutton ride carries the Sutton place"
        );
    }
    assert!(
        world
            .places()
            .await
            .iter()
            .all(|p| meters((p.centroid_lat, p.centroid_lon), BROMONT) > 1_000.0),
        "bug: two workouts are not a place"
    );
}

#[tokio::test]
async fn starts_ringing_the_door_form_no_place_and_no_centroid_sits_near_it() {
    let world = World::new().await;
    let home = world.home_rides().await;
    for _ in 0..3 {
        world.ride(SUTTON, 0).await;
    }
    world.pass().await;

    let places = world.places().await;
    assert_eq!(
        places.len(),
        1,
        "bug: only Sutton is a place; the door is home, got {places:?}"
    );
    for place in &places {
        let from_door = meters((place.centroid_lat, place.centroid_lon), DOOR);
        assert!(
            from_door > 500.0,
            "bug: a stored centroid sits {from_door:.0} m from the door"
        );
    }
    for id in &home {
        assert_eq!(
            world.places_of(id).await,
            vec![None],
            "bug: a ride from the door joined a place"
        );
    }
}

#[tokio::test]
async fn an_always_driven_to_trailhead_is_excluded_and_named_once_home_is_elsewhere() {
    // No home rides: every ride goes out and back from the trailhead, so
    // the trailhead is the densest endpoint cluster and reads as home.
    let world = World::new().await;
    let mut rides = Vec::new();
    for _ in 0..5 {
        rides.push(world.ride_along(&out_and_back(OKA), |b| b).await);
    }
    world.pass().await;
    assert!(
        world.places().await.is_empty(),
        "bug: the only endpoint cluster is taken for home and excluded"
    );

    // What reads as home is never made a place by naming one of its rides
    // (carnet#1723); the athlete says home is elsewhere instead.
    assert_eq!(
        world
            .name(PlaceTarget::Activity(rides[0].clone()), Some("Gai-Luron"))
            .await,
        Err(PlaceRefusal::PlaceNotFound),
        "bug: naming a ride from what reads as home made home a place"
    );
    assert!(
        world.places().await.is_empty(),
        "bug: a refused name stored a place"
    );
    declare_home(
        world.repos(),
        &world.tenant,
        world.user,
        HomeChoice::Elsewhere,
    )
    .await
    .unwrap()
    .unwrap();
    let named = world
        .name(PlaceTarget::Activity(rides[0].clone()), Some("Gai-Luron"))
        .await
        .expect("bug: with home elsewhere the trailhead's ride has a place to name");
    assert_eq!(
        (named.name.as_deref(), named.athlete_named),
        (Some("Gai-Luron"), true),
        "bug: the place carries the athlete's name"
    );
    for id in &rides {
        assert_eq!(
            world.places_of(id).await,
            vec![Some(named.id.clone())],
            "bug: every ride from the named trailhead carries it"
        );
    }

    // A later ingest makes the athlete due; the place survives the pass
    // under the same id and keeps every ride.
    let later = world.ride_along(&out_and_back(OKA), |b| b).await;
    let report = world.pass().await;
    assert_eq!(report.users, 1, "bug: an ingest makes the athlete due");
    let place = world.place_at(OKA).await;
    assert_eq!(place.id, named.id, "bug: the named place kept its id");
    assert_eq!(place.activity_count, 6, "bug: the new ride joined it");
    assert_eq!(
        world.places_of(&rides[0]).await,
        vec![Some(named.id.clone())],
        "bug: the named activity keeps its place after a re-cluster"
    );
    assert_eq!(
        world.places_of(&later).await,
        vec![Some(named.id)],
        "bug: a later ride from the named trailhead joins it"
    );
}

#[tokio::test]
async fn a_trailhead_ridden_in_loops_keeps_its_rides_once_the_athlete_names_it() {
    // Every ride loops from the trailhead, out north and back from the
    // south: the trimmed starts and ends sit about 400 m apart, farther
    // than the clustering distance, on opposite sides of the trailhead.
    let world = World::new().await;
    let mut rides = Vec::new();
    for _ in 0..5 {
        rides.push(world.ride_along(&north_out_south_back(OKA), |b| b).await);
    }
    world.pass().await;
    assert!(
        world.places().await.is_empty(),
        "bug: the only spot's loops are taken for home and excluded"
    );

    // The athlete says home is elsewhere, names the trailhead, then asks
    // for the guess back: the named trailhead is never the guess, and the
    // loops' far ends never exclude its rides.
    declare_home(
        world.repos(),
        &world.tenant,
        world.user,
        HomeChoice::Elsewhere,
    )
    .await
    .unwrap()
    .unwrap();
    let named = world
        .name(PlaceTarget::Activity(rides[0].clone()), Some("Gai-Luron"))
        .await
        .expect("bug: with home elsewhere the trailhead's loop has a place to name");
    declare_home(world.repos(), &world.tenant, world.user, HomeChoice::Forget)
        .await
        .unwrap()
        .unwrap();
    let place = world.place_at(OKA).await;
    assert_eq!(place.id, named.id, "bug: the named place kept its id");
    assert_eq!(
        place.activity_count, 5,
        "bug: the loops' far ends were taken for home and excluded the named trailhead's rides"
    );
    for id in &rides {
        assert_eq!(
            world.places_of(id).await,
            vec![Some(named.id.clone())],
            "bug: every loop from the named trailhead joins it"
        );
    }

    world.ride_along(&north_out_south_back(OKA), |b| b).await;
    assert_eq!(
        world.pass().await.users,
        1,
        "bug: an ingest makes the athlete due"
    );
    assert_eq!(
        world.place_at(OKA).await.activity_count,
        6,
        "bug: a later loop from the named trailhead joins it"
    );
}

/// Three rides from the door into an area 2.6 km east-north-east of it and
/// three into one 2.6 km east-south-east, about 1.3 km apart, as jf rides
/// from his door to Gailuron and to Forêt Héritage (carnet#1723). Every
/// ride carries a city and a segment, as his do, which name neither area.
async fn rides_into_two_areas(world: &World) -> (Vec<String>, Vec<String>) {
    let mut north = Vec::new();
    let mut south = Vec::new();
    for i in 0..3 {
        let phase = f64::from(i);
        let tagged = |b: ActivityBuilder| {
            b.city("Prévost".to_owned())
                .segment_efforts(vec![effort("Warm up")])
        };
        north.push(
            world
                .ride_along(&trip_to(DOOR, 650.0, 2_500.0, phase), tagged)
                .await,
        );
        south.push(
            world
                .ride_along(&trip_to(DOOR, -650.0, 2_500.0, phase), tagged)
                .await,
        );
    }
    (north, south)
}

#[tokio::test]
async fn rides_from_the_door_are_placed_by_the_area_they_ride_through() {
    let world = World::new().await;
    let walks = world.home_rides().await;
    let (north, south) = rides_into_two_areas(&world).await;
    world.pass().await;

    let places = world.places().await;
    assert_eq!(
        places.len(),
        2,
        "bug: the door's rides into two areas are two places: {places:?}"
    );
    for (area, rides) in [("north", &north), ("south", &south)] {
        let first = world.places_of(&rides[0]).await;
        assert!(
            first[0].is_some(),
            "bug: a ride into the {area} area has no place"
        );
        for id in rides {
            assert_eq!(
                world.places_of(id).await,
                first,
                "bug: the rides into the {area} area do not share one place"
            );
        }
    }
    assert_ne!(
        world.places_of(&north[0]).await,
        world.places_of(&south[0]).await,
        "bug: two areas 1.3 km apart are one place"
    );
    for place in &places {
        assert!(
            meters((place.centroid_lat, place.centroid_lon), DOOR) > 700.0,
            "bug: an area's centroid lies near the door: {place:?}"
        );
        assert_eq!(
            (place.name.as_deref(), place.name_source),
            (None, None),
            "bug: an area of rides from home was named for the door's city or segment: {place:?}"
        );
    }
    for id in &walks {
        assert_eq!(
            world.places_of(id).await,
            vec![None],
            "bug: a loop around the block took a place"
        );
    }
}

#[tokio::test]
async fn naming_a_ride_from_the_door_names_its_area_and_the_name_sticks() {
    let world = World::new().await;
    world.home_rides().await;
    let (north, _) = rides_into_two_areas(&world).await;
    world.pass().await;
    let area = world.places_of(&north[0]).await[0]
        .clone()
        .expect("bug: precondition: the ride must carry its area");

    let named = world
        .name(PlaceTarget::Activity(north[1].clone()), Some("Gailuron"))
        .await
        .expect("bug: a ride from the door into an area can be named");
    assert_eq!(
        (
            named.id.as_str(),
            named.name.as_deref(),
            named.athlete_named
        ),
        (area.as_str(), Some("Gailuron"), true),
        "bug: naming a ride from the door did not name its area"
    );

    let later = world
        .ride_along(&trip_to(DOOR, 650.0, 2_500.0, 4.0), |b| b)
        .await;
    world.pass().await;
    assert_eq!(
        world.places_of(&later).await,
        vec![Some(area.clone())],
        "bug: a later ride into the named area does not carry its name"
    );
    let stored = world
        .places()
        .await
        .into_iter()
        .find(|place| place.id == area)
        .expect("bug: the named area is gone");
    assert_eq!(
        (stored.name.as_deref(), stored.activity_count),
        (Some("Gailuron"), 4),
        "bug: the name did not stick to the area's rides"
    );
}

#[tokio::test]
async fn naming_a_ride_from_the_door_before_any_pass_names_its_area() {
    // Right after the clustering version changes, rows still carry no
    // place: naming one places it first, and names its area, never home.
    let world = World::new().await;
    world.home_rides().await;
    let (north, _) = rides_into_two_areas(&world).await;
    let named = world
        .name(PlaceTarget::Activity(north[0].clone()), Some("Gailuron"))
        .await
        .expect("bug: a ride from the door into an area can be named before a pass");
    for id in &north {
        assert_eq!(
            world.places_of(id).await,
            vec![Some(named.id.clone())],
            "bug: the rides into the area do not all carry the named area"
        );
    }
    let place = world
        .places()
        .await
        .into_iter()
        .find(|place| place.id == named.id)
        .expect("bug: the named area is not stored");
    assert!(
        meters((place.centroid_lat, place.centroid_lon), DOOR) > 700.0,
        "bug: the named place sits at the door: {place:?}"
    );
}

#[tokio::test]
async fn a_lone_trailhead_ridden_far_is_placed_by_the_area_and_named_through_a_ride() {
    // Rides 1.5 km out and back from the only spot: it reads as home, and
    // the rides are placed by where they went, which naming a ride names.
    let world = World::new().await;
    let out: Vec<(f64, f64)> = (0..=15)
        .map(|i| (f64::from(i).mul_add(STEP_LAT, OKA.0), OKA.1))
        .collect();
    let path: Vec<(f64, f64)> = out
        .iter()
        .chain(out.iter().rev().skip(1))
        .copied()
        .collect();
    let mut rides = Vec::new();
    for _ in 0..5 {
        rides.push(world.ride_along(&path, |b| b).await);
    }
    world.pass().await;
    let places = world.places().await;
    assert_eq!(
        places.len(),
        1,
        "bug: the far rides make one area: {places:?}"
    );
    let named = world
        .name(PlaceTarget::Activity(rides[0].clone()), Some("Gai-Luron"))
        .await
        .expect("bug: a ride into the area can be named");
    assert_eq!(
        named.id, places[0].id,
        "bug: naming the ride did not name its area"
    );
    for id in &rides {
        assert_eq!(
            world.places_of(id).await,
            vec![Some(named.id.clone())],
            "bug: a ride lost the named area"
        );
    }
}

#[tokio::test]
async fn naming_a_ride_that_never_left_the_door_makes_no_place() {
    let world = World::new().await;
    let walks = world.home_rides().await;
    world.ride(SUTTON, 0).await;
    world.pass().await;
    let before = world.places().await;

    assert_eq!(
        world
            .name(PlaceTarget::Activity(walks[0].clone()), Some("Maison"))
            .await,
        Err(PlaceRefusal::PlaceNotFound),
        "bug: naming a loop around the block made home a place"
    );
    let after = world.places().await;
    assert_eq!(
        after.len(),
        before.len(),
        "bug: a refused name stored a place: {after:?}"
    );
    assert!(
        after
            .iter()
            .all(|place| meters((place.centroid_lat, place.centroid_lon), DOOR) > 1_000.0),
        "bug: a place sits at the door: {after:?}"
    );
    assert_eq!(
        world.places_of(&walks[0]).await,
        vec![None],
        "bug: the walk took a place"
    );
}

#[tokio::test]
async fn a_single_visit_is_placed_by_its_name_and_kept_by_later_passes() {
    let world = World::new().await;
    world.home().await;
    let visit = world.ride(BROMONT, 0).await;
    world.pass().await;
    assert_eq!(
        world.places_of(&visit).await,
        vec![None],
        "bug: one visit is not a place on its own"
    );

    let named = world
        .name(PlaceTarget::Activity(visit.clone()), Some("Bromont"))
        .await
        .unwrap();
    world.ride(SUTTON, 0).await;
    world.pass().await;
    assert_eq!(
        world.places_of(&visit).await,
        vec![Some(named.id.clone())],
        "bug: a single-visit athlete place keeps its row after a re-cluster"
    );
    assert_eq!(
        world.place_at(BROMONT).await.activity_count,
        1,
        "bug: the single-visit place counts its one ride after a re-cluster"
    );
}

#[tokio::test]
async fn names_come_from_the_athlete_then_the_city_then_the_segment() {
    let world = World::new().await;
    world.home().await;
    for city in ["Sutton", "Sutton", "Cowansville"] {
        world
            .ride_with(SUTTON, 0, |b| b.city(city.to_owned()))
            .await;
    }
    for segments in [
        vec!["Montée Chilly"],
        vec!["Montée Chilly", "Other"],
        vec!["Montée Chilly"],
    ] {
        let efforts: Vec<SegmentEffort> = segments.into_iter().map(effort).collect();
        world
            .ride_with(BROMONT, 0, |b| b.segment_efforts(efforts))
            .await;
    }
    for _ in 0..3 {
        world.ride(OKA, 0).await;
    }
    world.pass().await;

    let sutton = world.place_at(SUTTON).await;
    assert_eq!(
        (sutton.name.as_deref(), sutton.name_source),
        (Some("Sutton"), Some(PlaceNameSource::City)),
        "bug: the most frequent city names the place"
    );
    let bromont = world.place_at(BROMONT).await;
    assert_eq!(
        (bromont.name.as_deref(), bromont.name_source),
        (Some("Montée Chilly"), Some(PlaceNameSource::Segment)),
        "bug: with no city, the segment most workouts ride names the place"
    );
    let oka = world.place_at(OKA).await;
    assert_eq!(
        (oka.name, oka.name_source),
        (None, None),
        "bug: a place with no city and no segment has no name"
    );

    let named = world
        .name(PlaceTarget::Place(sutton.id.clone()), Some("  Gai-Luron  "))
        .await
        .unwrap();
    assert_eq!(
        named,
        NamedPlace {
            id: sutton.id.clone(),
            name: Some("Gai-Luron".to_owned()),
            athlete_named: true,
        },
        "bug: the athlete's name is stored trimmed and answered as theirs"
    );
    world.ride(SUTTON, 1).await;
    world.pass().await;
    let sutton = world.place_at(SUTTON).await;
    assert_eq!(
        (sutton.name.as_deref(), sutton.name_source),
        (Some("Gai-Luron"), Some(PlaceNameSource::Athlete)),
        "bug: the athlete's name survives a re-cluster over the city"
    );

    let cleared = world
        .name(PlaceTarget::Place(sutton.id.clone()), None)
        .await
        .unwrap();
    assert_eq!(
        cleared,
        NamedPlace {
            id: sutton.id,
            name: Some("Sutton".to_owned()),
            athlete_named: false,
        },
        "bug: clearing the athlete's name brings the city name back at once"
    );
}

#[tokio::test]
async fn copies_of_one_workout_count_as_one_start_and_all_take_its_place() {
    let world = World::new().await;
    world.home().await;
    let mut pairs = Vec::new();
    for _ in 0..3 {
        let start = world.next_start();
        let strava = Uuid::new_v4().simple().to_string();
        let garmin = Uuid::new_v4().simple().to_string();
        world
            .store(
                &world.tenant,
                "strava",
                ActivityBuilder::new(&strava, "Ride", SportType::Ride, start, 3_600, "strava")
                    .distance_meters(4_000.0)
                    .summary_polyline(encode_polyline(&loop_from(SUTTON, 0)))
                    .build(),
            )
            .await;
        // The Garmin twin carries no route of its own.
        world
            .store(
                &world.tenant,
                "garmin",
                ActivityBuilder::new(&garmin, "Ride", SportType::Ride, start, 3_600, "garmin")
                    .distance_meters(4_000.0)
                    .build(),
            )
            .await;
        pairs.push((strava, garmin));
    }
    world.pass().await;

    let place = world.place_at(SUTTON).await;
    assert_eq!(
        place.activity_count, 3,
        "bug: a Strava ride and its Garmin twin are one workout, one start"
    );
    for (strava, garmin) in &pairs {
        for id in [strava, garmin] {
            assert_eq!(
                world.places_of(id).await,
                vec![Some(place.id.clone())],
                "bug: every copy of a placed workout carries its place"
            );
        }
    }
}

#[tokio::test]
async fn erasure_removes_an_unnamed_place_and_keeps_a_named_one_empty() {
    let world = World::new().await;
    world.home().await;
    let mut sutton = Vec::new();
    let mut bromont = Vec::new();
    for _ in 0..3 {
        sutton.push(world.ride(SUTTON, 0).await);
        bromont.push(world.ride(BROMONT, 0).await);
    }
    world.pass().await;
    let named = world.place_at(BROMONT).await;
    world
        .name(PlaceTarget::Place(named.id.clone()), Some("Bromont"))
        .await
        .unwrap();

    world.delete(&sutton[0]).await;
    for id in &bromont {
        world.delete(id).await;
    }
    assert_eq!(
        world.pass().await.users,
        1,
        "bug: an erasure makes the athlete due"
    );

    let places = world.places().await;
    assert!(
        places
            .iter()
            .all(|p| meters((p.centroid_lat, p.centroid_lon), SUTTON) > 500.0),
        "bug: two workouts left are no place, so Sutton went: {places:?}"
    );
    let kept = world.place_at(BROMONT).await;
    assert_eq!(
        (kept.id, kept.activity_count, kept.name.as_deref()),
        (named.id, 0, Some("Bromont")),
        "bug: a named place outlives its rows at count 0, keeping its name"
    );
    for id in &sutton[1..] {
        assert_eq!(
            world.places_of(id).await,
            vec![None],
            "bug: a row whose place went carries none"
        );
    }
}

#[tokio::test]
async fn another_tenant_never_sees_or_names_the_athletes_places() {
    let world = World::new().await;
    let other = TenantId::generate();
    world.home().await;
    for _ in 0..3 {
        world.ride(SUTTON, 0).await;
    }
    for turn in 0..16 {
        let id = Uuid::new_v4().simple().to_string();
        let ride = ActivityBuilder::new(
            &id,
            "Ride",
            SportType::Ride,
            world.next_start(),
            3_600,
            "strava",
        )
        .distance_meters(4_000.0)
        .summary_polyline(encode_polyline(&block_from(OKA, turn % 4)))
        .build();
        world.store(&other, "strava", ride).await;
    }
    let report = world.pass().await;
    assert_eq!(
        report.users, 2,
        "bug: each tenant's rows are clustered on their own"
    );

    let sutton = world.place_at(SUTTON).await;
    assert!(
        world.places_in(&other).await.is_empty(),
        "bug: the other tenant's only spot is its home; Sutton is not its place"
    );
    let refused = name_place(
        world.repos(),
        &other,
        world.user,
        PlaceTarget::Place(sutton.id.clone()),
        Some("Stolen".to_owned()),
    )
    .await
    .unwrap();
    assert_eq!(
        refused,
        Err(PlaceRefusal::PlaceNotFound),
        "bug: a place is named only in its own tenant"
    );
    assert_eq!(
        world.place_at(SUTTON).await.name.as_deref(),
        sutton.name.as_deref(),
        "bug: another tenant's name reached the place"
    );
}

#[tokio::test]
async fn a_raw_start_never_shapes_a_centroid_and_only_joins_a_place() {
    let world = World::new().await;
    world.home().await;
    let mut raw_only = Vec::new();
    for _ in 0..5 {
        let id = Uuid::new_v4().simple().to_string();
        world
            .store(
                &world.tenant,
                "strava",
                ActivityBuilder::new(
                    &id,
                    "Ride",
                    SportType::Ride,
                    world.next_start(),
                    3_600,
                    "strava",
                )
                .distance_meters(4_000.0)
                .start_latitude(BROMONT.0)
                .start_longitude(BROMONT.1)
                .build(),
            )
            .await;
        raw_only.push(id);
    }
    for _ in 0..3 {
        world.ride(SUTTON, 0).await;
    }
    world.pass().await;
    assert!(
        world
            .places()
            .await
            .iter()
            .all(|p| meters((p.centroid_lat, p.centroid_lon), BROMONT) > 1_000.0),
        "bug: raw starts alone made a place"
    );
    for id in &raw_only {
        assert_eq!(
            world.places_of(id).await,
            vec![None],
            "bug: a raw start outside every place is placed nowhere"
        );
    }
    let before = world.place_at(SUTTON).await;

    // A track-less ride whose raw start lies inside the Sutton place.
    let trimmed_start = loop_from(SUTTON, 0)[2];
    let id = Uuid::new_v4().simple().to_string();
    world
        .store(
            &world.tenant,
            "strava",
            ActivityBuilder::new(
                &id,
                "Ride",
                SportType::Ride,
                world.next_start(),
                3_600,
                "strava",
            )
            .distance_meters(4_000.0)
            .start_latitude(trimmed_start.0 + 0.0004)
            .start_longitude(trimmed_start.1)
            .build(),
        )
        .await;
    world.pass().await;
    let after = world.place_at(SUTTON).await;
    assert_eq!(
        world.places_of(&id).await,
        vec![Some(after.id.clone())],
        "bug: a raw start inside a place joins it"
    );
    assert_eq!(after.id, before.id, "bug: the place kept its id");
    assert!(
        (after.centroid_lat - before.centroid_lat).abs() < 1e-9
            && (after.centroid_lon - before.centroid_lon).abs() < 1e-9,
        "bug: a raw start moved the centroid"
    );
}

#[tokio::test]
async fn an_athlete_is_recomputed_after_an_ingest_an_erasure_or_a_day_and_not_otherwise() {
    let world = World::new().await;
    world.home().await;
    world.ride(SUTTON, 0).await;
    assert_eq!(world.pass().await.users, 1, "bug: never clustered is due");
    assert_eq!(
        world.pass().await.users,
        0,
        "bug: nothing changed, nothing is due"
    );

    world.ride(SUTTON, 0).await;
    assert_eq!(
        world.pass().await.users,
        1,
        "bug: an ingest makes the athlete due"
    );
    assert_eq!(
        world.pass().await.users,
        0,
        "bug: an athlete just clustered is not due again"
    );

    assert_eq!(
        world
            .pass_at(Timestamp::now() + Elapsed::hours(25))
            .await
            .users,
        1,
        "bug: a day-old clustering is due"
    );

    for row in world.rows().await {
        world.delete(row.activity.id()).await;
    }
    assert_eq!(
        world.pass().await.users,
        1,
        "bug: a full erasure makes the athlete due"
    );
    assert!(
        world.places().await.is_empty(),
        "bug: no rows, no unnamed place"
    );
}

#[tokio::test]
async fn name_place_refuses_what_it_cannot_name() {
    let world = World::new().await;
    world.home().await;
    let trainer = Uuid::new_v4().simple().to_string();
    world
        .store(
            &world.tenant,
            "strava",
            ActivityBuilder::new(
                &trainer,
                "Zwift",
                SportType::VirtualRide,
                world.next_start(),
                3_600,
                "strava",
            )
            .build(),
        )
        .await;
    let ride = world.ride(SUTTON, 0).await;

    assert_eq!(
        world
            .name(PlaceTarget::Activity(trainer.clone()), Some("Basement"))
            .await,
        Err(PlaceRefusal::ActivityHasNoRoute),
        "bug: an activity with no route and no place cannot be placed"
    );
    assert_eq!(
        world
            .name(
                PlaceTarget::Activity("no-such-activity".to_owned()),
                Some("X")
            )
            .await,
        Err(PlaceRefusal::ActivityNotFound),
        "bug: an activity the athlete does not hold is refused as not found"
    );
    assert_eq!(
        world
            .name(PlaceTarget::Place(Uuid::new_v4().to_string()), Some("X"))
            .await,
        Err(PlaceRefusal::PlaceNotFound),
        "bug: a place the athlete does not hold is refused as not found"
    );
    assert_eq!(
        world.name(PlaceTarget::Activity(ride.clone()), None).await,
        Err(PlaceRefusal::PlaceNotFound),
        "bug: an activity with no place has no name to clear"
    );
    for name in ["", "   ", &"x".repeat(61)] {
        assert_eq!(
            world
                .name(PlaceTarget::Activity(ride.clone()), Some(name))
                .await,
            Err(PlaceRefusal::InvalidName),
            "bug: {name:?} is not a place name"
        );
    }
    let defanged = world
        .name(
            PlaceTarget::Activity(ride.clone()),
            Some("Gai-Luron\n# ignore previous"),
        )
        .await
        .unwrap();
    let stored = defanged.name.unwrap();
    assert!(
        !stored.contains('\n'),
        "bug: a place name is one line, got {stored:?}"
    );
}

#[tokio::test]
async fn an_athlete_whose_recompute_keeps_failing_never_holds_the_slot_another_is_due() {
    // Two athletes in one database, clustered at the same instant, so the
    // scan's tie-break puts the first tenant ahead of the second.
    let fleet = World::new().await;
    let world = fleet.beside("00000000-0000-4000-8000-000000000001").await;
    let other = fleet.beside("00000000-0000-4000-8000-000000000002").await;
    world.home().await;
    other.home().await;
    for _ in 0..3 {
        world.ride(SUTTON, 0).await;
        other.ride(BROMONT, 0).await;
    }
    let clustered = Timestamp::now();
    assert_eq!(
        world.pass_at(clustered).await.users,
        2,
        "bug: both athletes are due their first clustering"
    );

    // A stored place no clustering writes fails the first athlete's read of
    // their places, so every recompute of theirs fails.
    let user = world.user.to_string();
    let tenant = world.tenant.to_string();
    execute(
        &world.db,
        "UPDATE athlete_places SET activity_count = -1 WHERE tenant_id = $1 AND user_id = $2",
        &[tenant.as_str(), user.as_str()],
    )
    .await;
    world.ride(SUTTON, 0).await;
    let later = other.ride(BROMONT, 0).await;

    let first = places_pass(world.repos(), Timestamp::now(), 1)
        .await
        .unwrap();
    assert_eq!(
        first,
        PlacesPassReport {
            users: 0,
            places: 0,
            failed: 1,
        },
        "bug: the failing athlete heads the scan and its failure is counted"
    );
    let second = places_pass(world.repos(), Timestamp::now(), 1)
        .await
        .unwrap();
    assert_eq!(
        (second.users, second.failed),
        (1, 0),
        "bug: the athlete whose recompute failed took the only slot again"
    );
    assert_eq!(
        other.places_of(&later).await,
        vec![Some(other.place_at(BROMONT).await.id)],
        "bug: the other athlete's new ride was not placed"
    );
    let third = places_pass(world.repos(), Timestamp::now(), 1)
        .await
        .unwrap();
    assert_eq!(
        third.failed, 1,
        "bug: the failing athlete with a new row is no longer retried"
    );
}
