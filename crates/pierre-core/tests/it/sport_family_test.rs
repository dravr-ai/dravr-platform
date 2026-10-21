// ABOUTME: Tests for SportFamily
// ABOUTME: Variants fold into their family and share the family head

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::SportType;
use pierre_core::models::{sport_family_head, SportFamily};

#[test]
fn variants_fold_into_their_family() {
    assert_eq!(
        SportFamily::of(&SportType::VirtualRide),
        SportFamily::Cycling
    );
    assert_eq!(
        SportFamily::of(&SportType::GravelRide),
        SportFamily::Cycling
    );
    assert_eq!(
        SportFamily::of(&SportType::VirtualRun),
        SportFamily::Running
    );
    assert_eq!(SportFamily::of(&SportType::Hike), SportFamily::Other);
}

#[test]
fn a_trail_run_is_running() {
    // carnet#418: this arm was missing while the cycling terrain variants
    // beside it were present, so a trail runner's primary sport reached
    // the flavour rule as Mixed and their sessions read as multi-sport.
    assert_eq!(
        SportFamily::of(&SportType::TrailRunning),
        SportFamily::Running
    );
    assert_eq!(
        SportFamily::distinct(&[SportType::Run, SportType::TrailRunning]),
        1,
        "road and trail sessions are one athlete training one sport"
    );
}

#[test]
fn every_sport_with_a_head_shares_that_head_s_family() {
    // The invariant that makes the membership list single-source: this
    // holds by construction now, and would fail loudly if `of` ever grew
    // its own list again.
    for (sport, head) in [
        (SportType::TrailRunning, SportType::Run),
        (SportType::VirtualRun, SportType::Run),
        (SportType::MountainBike, SportType::Ride),
        (SportType::GravelRide, SportType::Ride),
        (SportType::EbikeRide, SportType::Ride),
        (SportType::VirtualRide, SportType::Ride),
    ] {
        assert_eq!(
            sport_family_head(&sport),
            Some(head.clone()),
            "{sport:?} folds into {head:?}"
        );
        assert_eq!(
            SportFamily::of(&sport),
            SportFamily::of(&head),
            "{sport:?} must land in the same family as its head {head:?}"
        );
    }
}

#[test]
fn a_runner_who_also_rides_the_trainer_spans_two_families() {
    let sports = [SportType::Run, SportType::VirtualRide, SportType::Run];
    assert_eq!(SportFamily::distinct(&sports), 2);
    assert_eq!(
        SportFamily::distinct(&[SportType::Run, SportType::VirtualRun]),
        1
    );
    assert_eq!(SportFamily::distinct(&[]), 0);
}
