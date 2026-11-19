// ABOUTME: Covers declare_home: here, elsewhere and forget, read by the places recompute, through the service the tool calls
// ABOUTME: The trailhead case the issue was filed for, home winning over a named place, refusals, and a declaration seen once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's declared home (carnet#1593), on the places world of
//! `athlete_places_test`: a door at `DOOR`, a trailhead at `OKA`, a club at
//! `SUTTON`. Every assertion reads what a reader can see, a row's place and
//! the places list, never the seed, except the refusals, which check that
//! nothing was stored.

use pierre_core::models::{ActivityBuilder, HomeDeclaration, SportType};
use pierre_services::athlete_places::{
    declare_home, HomeChoice, HomeState, PlaceRefusal, PlaceTarget,
};
use uuid::Uuid;

use crate::athlete_places_test::{meters, out_and_back, World, DOOR, OKA, SUTTON};

impl World {
    async fn declare(&self, choice: HomeChoice) -> Result<HomeState, PlaceRefusal> {
        declare_home(self.repos(), &self.tenant, self.user, choice)
            .await
            .unwrap()
    }

    /// Whether each ride of `ids` carries a place, in order.
    async fn placed(&self, ids: &[String]) -> Vec<bool> {
        let mut placed = Vec::new();
        for id in ids {
            placed.push(self.places_of(id).await.iter().all(Option::is_some));
        }
        placed
    }
}

/// Rides out and back from the trailhead and none from home: the trailhead
/// is the densest endpoint cluster, so it is taken for home and excluded.
async fn trailhead_only(world: &World) -> Vec<String> {
    let mut rides = Vec::new();
    for _ in 0..5 {
        rides.push(world.ride_along(&out_and_back(OKA), |b| b).await);
    }
    world.pass().await;
    assert!(
        world.places().await.is_empty(),
        "bug: precondition: the trailhead must be taken for home, or this proves nothing"
    );
    rides
}

#[tokio::test]
async fn elsewhere_brings_back_the_trailhead_an_athlete_with_no_home_rides_drives_to() {
    let world = World::new().await;
    let rides = trailhead_only(&world).await;

    assert_eq!(
        world.declare(HomeChoice::Elsewhere).await,
        Ok(HomeState::Elsewhere),
        "bug: elsewhere answers its state"
    );
    let place = world.place_at(OKA).await;
    assert_eq!(
        place.activity_count, 5,
        "bug: with home elsewhere the trailhead's rides form a place"
    );
    assert_eq!(
        world.placed(&rides).await,
        vec![true; 5],
        "bug: with home elsewhere every trailhead ride carries a place"
    );
    assert_eq!(
        world.pass().await.users,
        0,
        "bug: the recompute the declaration ran saw it, so the athlete is not due again"
    );
}

#[tokio::test]
async fn here_on_a_home_start_moves_the_exclusion_and_forget_restores_the_guess() {
    // Sixteen trailhead rides outnumber three home rides, so the trailhead
    // is guessed for home and the door's three starts make a place.
    let world = World::new().await;
    let mut trailhead = Vec::new();
    for _ in 0..16 {
        trailhead.push(world.ride_along(&out_and_back(OKA), |b| b).await);
    }
    let mut home = Vec::new();
    for _ in 0..3 {
        home.push(world.ride(DOOR, 0).await);
    }
    world.pass().await;
    let guessed = world.places().await;
    assert_eq!(
        (world.placed(&trailhead).await, world.placed(&home).await),
        (vec![false; 16], vec![true; 3]),
        "bug: precondition: the trailhead must be guessed for home, or this proves nothing: {guessed:?}"
    );

    assert_eq!(
        world
            .declare(HomeChoice::Here(PlaceTarget::Activity(home[0].clone())))
            .await,
        Ok(HomeState::Declared),
        "bug: here answers its state"
    );
    assert_eq!(
        (world.placed(&trailhead).await, world.placed(&home).await),
        (vec![true; 16], vec![false; 3]),
        "bug: a declared home excludes the door's starts and frees the trailhead"
    );
    assert_eq!(
        world.place_at(OKA).await.activity_count,
        16,
        "bug: the trailhead is a place of its sixteen rides"
    );
    assert!(
        world
            .places()
            .await
            .iter()
            .all(|p| meters((p.centroid_lat, p.centroid_lon), DOOR) > 1_000.0),
        "bug: a place is left at the declared door"
    );

    assert_eq!(
        world.declare(HomeChoice::Forget).await,
        Ok(HomeState::Inferred),
        "bug: forget answers the automatic state"
    );
    assert_eq!(
        (world.placed(&trailhead).await, world.placed(&home).await),
        (vec![false; 16], vec![true; 3]),
        "bug: forget did not bring back the automatic guess"
    );
    let restored = world.places().await;
    assert_eq!(
        restored
            .iter()
            .map(|p| (p.activity_count, p.name.clone()))
            .collect::<Vec<_>>(),
        guessed
            .iter()
            .map(|p| (p.activity_count, p.name.clone()))
            .collect::<Vec<_>>(),
        "bug: forget did not restore the places the guess made"
    );
}

#[tokio::test]
async fn home_declared_on_a_named_place_keeps_its_name_and_forget_returns_its_rides() {
    let world = World::new().await;
    world.home_rides().await;
    let mut club = Vec::new();
    for _ in 0..3 {
        club.push(world.ride(SUTTON, 0).await);
    }
    world.pass().await;
    let named = world.place_at(SUTTON).await;
    world
        .name(PlaceTarget::Place(named.id.clone()), Some("Club"))
        .await
        .unwrap();

    for target in [
        PlaceTarget::Place(named.id.clone()),
        PlaceTarget::Activity(club[1].clone()),
    ] {
        assert_eq!(
            world.declare(HomeChoice::Here(target.clone())).await,
            Ok(HomeState::Declared),
            "bug: here on {target:?} answers its state"
        );
        let kept = world.place_at(SUTTON).await;
        assert_eq!(
            (kept.id.as_str(), kept.name.as_deref(), kept.activity_count),
            (named.id.as_str(), Some("Club"), 0),
            "bug: home declared on {target:?} must keep the place and its name, empty"
        );
        assert_eq!(
            world.placed(&club).await,
            vec![false; 3],
            "bug: home declared on {target:?} must exclude the club's starts"
        );

        world.declare(HomeChoice::Forget).await.unwrap();
        let back = world.place_at(SUTTON).await;
        assert_eq!(
            (back.id.as_str(), back.name.as_deref(), back.activity_count),
            (named.id.as_str(), Some("Club"), 3),
            "bug: forget after {target:?} returns the club's rides under its id and name"
        );
    }
}

#[tokio::test]
async fn naming_a_home_start_after_declaring_never_brings_the_door_back() {
    let world = World::new().await;
    let home = world.home_rides().await;
    world.pass().await;
    world
        .declare(HomeChoice::Here(PlaceTarget::Activity(home[0].clone())))
        .await
        .unwrap();

    let porch = world
        .name(PlaceTarget::Activity(home[1].clone()), Some("Porch"))
        .await
        .expect("bug: a home start with a route can be named");
    let kept = world
        .places()
        .await
        .into_iter()
        .find(|p| p.id == porch.id)
        .expect("bug: the named place exists");
    assert_eq!(
        (kept.name.as_deref(), kept.activity_count),
        (Some("Porch"), 0),
        "bug: a place named at a home start holds no ride while home is declared"
    );
    assert_eq!(
        world.placed(&home).await,
        vec![false; 16],
        "bug: naming a home start brought the door's starts back"
    );
}

#[tokio::test]
async fn here_refuses_what_it_cannot_place_and_stores_nothing() {
    let world = World::new().await;
    world.home_rides().await;
    let raw_only = Uuid::new_v4().simple().to_string();
    world
        .store(
            &world.tenant,
            "strava",
            ActivityBuilder::new(
                &raw_only,
                "Ride",
                SportType::Ride,
                world.next_start(),
                3_600,
                "strava",
            )
            .distance_meters(4_000.0)
            .start_latitude(DOOR.0)
            .start_longitude(DOOR.1)
            .build(),
        )
        .await;

    for (target, refused) in [
        (
            PlaceTarget::Activity(raw_only),
            PlaceRefusal::ActivityHasNoRoute,
        ),
        (
            PlaceTarget::Activity("no-such-activity".to_owned()),
            PlaceRefusal::ActivityNotFound,
        ),
        (
            PlaceTarget::Place(Uuid::new_v4().to_string()),
            PlaceRefusal::PlaceNotFound,
        ),
    ] {
        assert_eq!(
            world.declare(HomeChoice::Here(target.clone())).await,
            Err(refused),
            "bug: here on {target:?} is refused for its reason"
        );
    }
    let stored = world
        .repos()
        .athlete_places
        .home_declaration(&world.tenant, world.user)
        .await
        .unwrap();
    assert_eq!(
        (stored.home, stored.declared_at),
        (HomeDeclaration::Inferred, None),
        "bug: a refused declaration stored something"
    );
}
