// ABOUTME: Covers own_loops: the athlete's past loops at a place, ranked for effort, home's through their area, never another's
// ABOUTME: Drives the places pass and the loops search against a real database, on the places test's world

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's own loops (carnet#1665), on the world of
//! `athlete_places_test`: sixteen loops around the block from the door make
//! home, the rides at `SUTTON` make a place, and rides from the door into an
//! area make an area place (carnet#1723). Every search is made the way the
//! `discover_routes` tool makes it.

use dravr_tronc::dates::Timestamp;
use pierre_core::ai_policy::{ProviderTerms, SourcePolicy};
use pierre_core::models::{ActivityBuilder, SportType, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_fitness_compute::encode_polyline;
use pierre_services::athlete_loops::{own_loops, Effort, LoopSearch, OwnLoop};
use pierre_services::athlete_places::{declare_home, HomeChoice, PlaceTarget};
use uuid::Uuid;

use crate::athlete_places_test::{loop_from, meters, trip_to, World, DOOR, SUTTON};

/// Terms that keep nothing from a model: every stored row is served.
struct OpenTerms;

impl ProviderTerms for OpenTerms {
    fn ai_policy(&self, _provider: &str) -> Option<&'static SourcePolicy> {
        None
    }

    fn transport_policy(&self, _provider: &str) -> Option<TransportPolicy> {
        None
    }
}

/// A search around `spot`, 10 km out, for a ride of `effort`.
fn search(spot: (f64, f64), effort: Option<Effort>) -> LoopSearch {
    LoopSearch {
        sport: SportType::Ride,
        latitude: spot.0,
        longitude: spot.1,
        radius_meters: 10_000.0,
        effort,
        target_distance_meters: None,
    }
}

impl World {
    /// The loops `user` in `tenant` is offered by `search`.
    async fn loops_of(&self, tenant: &TenantId, user: Uuid, search: &LoopSearch) -> Vec<OwnLoop> {
        own_loops(
            self.repos(),
            &OpenTerms,
            tenant,
            user,
            search,
            Timestamp::now(),
        )
        .await
        .unwrap()
    }

    /// The loops this athlete is offered by `search`.
    async fn loops(&self, search: &LoopSearch) -> Vec<OwnLoop> {
        self.loops_of(&self.tenant, self.user, search).await
    }

    /// A loop from `SUTTON` of `km` and `gain` metres of climb; its id.
    async fn sutton_loop(&self, km: f64, gain: f64) -> String {
        self.ride_with(SUTTON, 0, |b| {
            b.distance_meters(km * 1_000.0).elevation_gain(gain)
        })
        .await
    }
}

/// Home, and an easy, a moderate and a hard loop from Sutton, placed.
async fn three_sutton_loops(world: &World) -> [String; 3] {
    world.home_rides().await;
    let easy = world.sutton_loop(12.0, 60.0).await;
    let moderate = world.sutton_loop(30.0, 450.0).await;
    let hard = world.sutton_loop(62.0, 1_300.0).await;
    world.pass().await;
    [easy, moderate, hard]
}

fn ids(loops: &[OwnLoop]) -> Vec<&str> {
    loops
        .iter()
        .map(|found| found.activity_id.as_str())
        .collect()
}

#[tokio::test]
async fn an_easy_loop_ranks_before_a_hard_one() {
    let world = World::new().await;
    let [easy, moderate, hard] = three_sutton_loops(&world).await;

    let loops = world.loops(&search(SUTTON, Some(Effort::Easy))).await;
    assert_eq!(
        ids(&loops),
        vec![easy.as_str(), moderate.as_str(), hard.as_str()],
        "bug: asked for easy, the shortest and flattest loop there leads"
    );
    assert_eq!(
        loops
            .iter()
            .map(|found| found.relative_effort)
            .collect::<Vec<_>>(),
        vec![
            Some(Effort::Easy),
            Some(Effort::Moderate),
            Some(Effort::Hard)
        ],
        "bug: each loop's effort is where it falls among the athlete's rides there"
    );

    let hardest_first = world.loops(&search(SUTTON, Some(Effort::Hard))).await;
    assert_eq!(
        ids(&hardest_first),
        vec![hard.as_str(), moderate.as_str(), easy.as_str()],
        "bug: asked for hard, the longest and hilliest loop there leads"
    );
}

#[tokio::test]
async fn a_loop_carries_its_place_date_distance_gain_and_a_trimmed_line() {
    let world = World::new().await;
    let [easy, ..] = three_sutton_loops(&world).await;
    let place = world.place_at(SUTTON).await;

    let loops = world.loops(&search(SUTTON, Some(Effort::Easy))).await;
    let first = loops.first().expect("bug: Sutton's loops are offered");
    assert_eq!(first.activity_id, easy, "bug: the easy loop leads");
    assert_eq!(
        first.place.id, place.id,
        "bug: the loop carries the place it started at"
    );
    assert_eq!(
        (first.distance_meters, first.elevation_gain_meters),
        (12_000.0, Some(60.0)),
        "bug: the loop carries its recorded distance and climb"
    );
    assert_eq!(
        first.name.as_deref(),
        Some("Ride"),
        "bug: the loop carries the workout's name"
    );
    let (lat, lon) = first.line[0];
    assert!(
        first.line.len() > 2 && meters((lat, lon), SUTTON) >= 150.0,
        "bug: the line is the privacy-trimmed track, never starting at the spot itself: {:?}",
        first.line.first()
    );
    assert!(
        (first.distance_from_center_meters - meters((lat, lon), SUTTON)).abs() < 1.0,
        "bug: the distance from the centre is measured to the line's start"
    );
}

#[tokio::test]
async fn no_loop_without_a_stored_track_and_no_ride_that_ends_elsewhere() {
    let world = World::new().await;
    let [easy, ..] = three_sutton_loops(&world).await;

    // The shortest ride there has a recorded start and no track: it joins
    // Sutton's place, and it is no loop anyone can draw.
    let trackless = Uuid::new_v4().simple().to_string();
    world
        .store(
            &world.tenant,
            "strava",
            ActivityBuilder::new(
                &trackless,
                "Ride",
                SportType::Ride,
                world.next_start(),
                1_800,
                "strava",
            )
            .distance_meters(3_000.0)
            .elevation_gain(10.0)
            .start_latitude(SUTTON.0)
            .start_longitude(SUTTON.1)
            .build(),
        )
        .await;
    // A short ride out from Sutton that ends 5 km north of it.
    let one_way: Vec<(f64, f64)> = (0..=55)
        .map(|i| (f64::from(i).mul_add(0.0009, SUTTON.0), SUTTON.1))
        .collect();
    let one_way_id = world
        .ride_along(&one_way, |b| {
            b.distance_meters(5_000.0).elevation_gain(20.0)
        })
        .await;
    world.pass().await;
    let place = world.place_at(SUTTON).await;
    assert_eq!(
        (
            world.places_of(&trackless).await,
            world.places_of(&one_way_id).await
        ),
        (vec![Some(place.id.clone())], vec![Some(place.id.clone())]),
        "bug: precondition: both rides must belong to Sutton's place, or this proves nothing"
    );

    let loops = world.loops(&search(SUTTON, Some(Effort::Easy))).await;
    let offered = ids(&loops);
    assert!(
        !offered.contains(&trackless.as_str()),
        "bug: a ride with no stored track is offered as a loop: {offered:?}"
    );
    assert!(
        !offered.contains(&one_way_id.as_str()),
        "bug: a ride that ends 5 km from its start is offered as a loop: {offered:?}"
    );
    assert_eq!(
        offered.first(),
        Some(&easy.as_str()),
        "bug: the easiest loop with a track leads"
    );
}

/// Home's loops around the block, three rides from the door into an area
/// 2.6 km east-north-east (carnet#1723), and two trail runs into the same
/// area; placed. Returns the rides and the runs.
async fn rides_from_home_into_an_area(world: &World) -> (Vec<String>, Vec<String>) {
    world.home_rides().await;
    let mut rides = Vec::new();
    let distances_km = [12.0, 30.0, 62.0].into_iter().enumerate();
    for (i, km) in distances_km {
        let path = trip_to(DOOR, 650.0, 2_500.0, i as f64);
        rides.push(
            world
                .ride_along(&path, |b| {
                    b.distance_meters(km * 1_000.0).elevation_gain(km * 10.0)
                })
                .await,
        );
    }
    let mut runs = Vec::new();
    for i in 0..2 {
        let id = Uuid::new_v4().simple().to_string();
        let path = trip_to(DOOR, 650.0, 2_500.0, f64::from(i) + 0.5);
        world
            .store(
                &world.tenant,
                "strava",
                ActivityBuilder::new(
                    &id,
                    "Trail",
                    SportType::TrailRunning,
                    world.next_start(),
                    3_600,
                    "strava",
                )
                .distance_meters(8_000.0)
                .summary_polyline(encode_polyline(&path))
                .build(),
            )
            .await;
        runs.push(id);
    }
    world.pass().await;
    (rides, runs)
}

#[tokio::test]
async fn a_loop_from_home_is_offered_through_its_area_inferred_or_declared() {
    let world = World::new().await;
    let (rides, runs) = rides_from_home_into_an_area(&world).await;
    let area = world
        .places()
        .await
        .into_iter()
        .find(|place| meters((place.centroid_lat, place.centroid_lon), DOOR) > 700.0)
        .expect("bug: precondition: the rides from the door must make an area");
    // The search is centred where discover_routes centres it with no place
    // asked: on a place's centroid, the area's, never the door.
    let centre = (area.centroid_lat, area.centroid_lon);

    for declared in [false, true] {
        if declared {
            declare_home(
                world.repos(),
                &world.tenant,
                world.user,
                HomeChoice::Here(PlaceTarget::Activity(rides[0].clone())),
            )
            .await
            .unwrap()
            .unwrap();
        }
        let loops = world.loops(&search(centre, Some(Effort::Easy))).await;
        assert_eq!(
            ids(&loops),
            rides.iter().map(String::as_str).collect::<Vec<_>>(),
            "bug: the rides from home into the area are not offered, easiest first (declared: {declared})"
        );
        for found in &loops {
            assert_eq!(
                found.place.id, area.id,
                "bug: a loop from home carries the area's place, never home's"
            );
            assert!(
                found.line.len() > 2 && meters(found.line[0], DOOR) >= 150.0,
                "bug: the line is the privacy-trimmed track, never starting at the door: {:?}",
                found.line.first()
            );
            assert!(
                !runs.contains(&found.activity_id),
                "bug: a trail run is offered for a ride"
            );
        }
    }
}

#[tokio::test]
async fn a_loop_around_the_block_from_home_is_never_offered() {
    let world = World::new().await;
    let (rides, _) = rides_from_home_into_an_area(&world).await;
    let walks = world.loops(&search(DOOR, None)).await;
    let mut offered = ids(&walks);
    offered.sort_unstable();
    let mut expected: Vec<&str> = rides.iter().map(String::as_str).collect();
    expected.sort_unstable();
    assert_eq!(
        offered, expected,
        "bug: only the three rides into the area are loops; a walk around the block is offered: {walks:?}"
    );
}

#[tokio::test]
async fn another_tenant_or_athlete_never_sees_the_loops() {
    let world = World::new().await;
    three_sutton_loops(&world).await;
    let other = world.beside("00000000-0000-4000-8000-0000000016a5").await;
    let at_sutton = search(SUTTON, None);
    assert_eq!(
        world.loops(&at_sutton).await.len(),
        3,
        "bug: precondition: the athlete's own three loops must be offered, or this proves nothing"
    );

    assert!(
        other.loops(&at_sutton).await.is_empty(),
        "bug: another tenant's athlete is offered this athlete's loops"
    );
    assert!(
        world
            .loops_of(&other.tenant, world.user, &at_sutton)
            .await
            .is_empty(),
        "bug: the athlete's loops are read under another tenant"
    );
    assert!(
        world
            .loops_of(&world.tenant, other.user, &at_sutton)
            .await
            .is_empty(),
        "bug: another athlete reads this athlete's loops in their tenant"
    );
}

#[tokio::test]
async fn copies_of_one_ride_are_one_loop_and_a_ride_again_is_offered_once() {
    let world = World::new().await;
    world.home_rides().await;
    let easy = world.sutton_loop(12.0, 60.0).await;
    let moderate = world.sutton_loop(30.0, 450.0).await;
    // The hard loop recorded twice, by Strava and by Garmin.
    let start = world.next_start();
    let mut hard_copies = Vec::new();
    for provider in ["strava", "garmin"] {
        let id = Uuid::new_v4().simple().to_string();
        world
            .store(
                &world.tenant,
                provider,
                ActivityBuilder::new(&id, "Ride", SportType::Ride, start, 3_600, provider)
                    .distance_meters(62_000.0)
                    .elevation_gain(1_300.0)
                    .summary_polyline(encode_polyline(&loop_from(SUTTON, 0)))
                    .build(),
            )
            .await;
        hard_copies.push(id);
    }
    // The easy loop again, another day and a few metres longer.
    world.sutton_loop(12.2, 65.0).await;
    world.pass().await;

    let loops = world.loops(&search(SUTTON, Some(Effort::Easy))).await;
    let offered = ids(&loops);
    assert_eq!(
        offered.len(),
        3,
        "bug: a ride again and a copy are each one more loop: {offered:?}"
    );
    assert_eq!(
        &offered[..2],
        &[easy.as_str(), moderate.as_str()],
        "bug: the easy loop, ridden twice, leads once, then the moderate one: {offered:?}"
    );
    assert!(
        hard_copies.iter().any(|id| id == offered[2]),
        "bug: the hard loop is offered by one of its copies: {offered:?}"
    );
}

// The track-read cap counts reads: one short loop ridden more often than the
// cap must not hide the athlete's other loops there.
#[tokio::test]
async fn a_loop_ridden_many_times_never_hides_the_next_one() {
    let world = World::new().await;
    // Home must stay the densest spot: twice the usual home rides outnumber
    // the loop's twenty-six rides' endpoints.
    world.home_rides().await;
    world.home_rides().await;
    let mut repeats = Vec::new();
    for _ in 0..26 {
        repeats.push(world.sutton_loop(12.0, 60.0).await);
    }
    let moderate = world.sutton_loop(30.0, 450.0).await;
    world.pass().await;

    let loops = world.loops(&search(SUTTON, Some(Effort::Easy))).await;
    let offered = ids(&loops);
    assert_eq!(
        offered.len(),
        2,
        "bug: twenty-six rides of one loop and one other loop are two loops: {offered:?}"
    );
    assert!(
        repeats.iter().any(|id| id == offered[0]),
        "bug: the loop ridden twenty-six times leads for easy: {offered:?}"
    );
    assert_eq!(
        offered[1], moderate,
        "bug: the other loop follows, past the twenty-six rides of the first"
    );
}
