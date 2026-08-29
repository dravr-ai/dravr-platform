// ABOUTME: Unit tests for the personalized coach recommender (sport profile + scoring)
// ABOUTME: Covers provider gating, sport matching, sport-agnostic coaches, and cold start
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use std::collections::{HashMap, HashSet};

use pierre_config::coach_recommendations::CoachRecommendationConfig;
use pierre_core::models::coaches::CoachPrerequisites;
use pierre_core::models::{SportProfile, SportType};
use pierre_services::coaches::score_coach;

/// Build a sport profile directly from `(canonical_sport_label, count)` pairs.
fn profile(counts: &[(&str, u32)]) -> SportProfile {
    let sport_counts: HashMap<String, u32> =
        counts.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
    let total_activities = counts.iter().map(|(_, v)| *v).sum();
    SportProfile {
        total_activities,
        window_days: 90,
        sport_counts,
    }
}

fn providers(list: &[&str]) -> HashSet<String> {
    list.iter().map(|p| (*p).to_owned()).collect()
}

fn prereqs(
    provider_list: &[&str],
    min_activities: u32,
    activity_types: &[&str],
) -> CoachPrerequisites {
    CoachPrerequisites {
        providers: provider_list.iter().map(|s| (*s).to_owned()).collect(),
        min_activities,
        activity_types: activity_types.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[test]
fn locked_when_no_provider_connected() {
    // A provider-gated coach is not eligible for a user with zero connected
    // providers (cold start), regardless of any (empty) profile.
    let cfg = CoachRecommendationConfig::default();
    let runner = profile(&[("run", 10)]);
    let rec = score_coach(
        &prereqs(&["strava"], 5, &["Run"]),
        Some(&runner),
        &providers(&[]),
        &cfg,
    );
    assert!(
        !rec.eligible,
        "coach needing a provider is not eligible with no provider connected"
    );
    assert!(rec.match_score.abs() < f32::EPSILON);
}

#[test]
fn cross_provider_running_coach_matches_garmin_user() {
    // A coach whose prerequisite names `strava` must still match a Garmin
    // runner — the provider name expresses "needs activity data", and the
    // running activities establish relevance. (Regression: Garmin users used
    // to fail the provider gate and get cold-start recs.)
    let cfg = CoachRecommendationConfig::default();
    let runner = profile(&[("run", 10)]);
    let rec = score_coach(
        &prereqs(&["strava"], 5, &["Run"]),
        Some(&runner),
        &providers(&["garmin"]),
        &cfg,
    );
    assert!(
        rec.eligible,
        "garmin runner should match a strava-prereq Run coach"
    );
    assert!((rec.match_score - 1.0).abs() < f32::EPSILON);
}

#[test]
fn sport_match_is_eligible_with_full_score() {
    let cfg = CoachRecommendationConfig::default();
    let runner = profile(&[("run", 10)]);
    let rec = score_coach(
        &prereqs(&["strava"], 5, &["Run"]),
        Some(&runner),
        &providers(&["strava"]),
        &cfg,
    );
    assert!(rec.eligible);
    assert!((rec.match_score - 1.0).abs() < f32::EPSILON);
}

#[test]
fn sport_mismatch_not_eligible_even_with_provider() {
    // Pure cyclist should not see a Run coach recommended.
    let cfg = CoachRecommendationConfig::default();
    let cyclist = profile(&[("ride", 10)]);
    let rec = score_coach(
        &prereqs(&["strava"], 5, &["Run"]),
        Some(&cyclist),
        &providers(&["strava"]),
        &cfg,
    );
    assert!(!rec.eligible);
    assert!(rec.match_score.abs() < f32::EPSILON);
}

#[test]
fn sport_agnostic_coach_eligible_once_provider_connected() {
    // Recovery-style coach: provider-gated, no activity types.
    let cfg = CoachRecommendationConfig::default();
    let cyclist = profile(&[("ride", 10)]);
    let rec = score_coach(
        &prereqs(&["strava"], 5, &[]),
        Some(&cyclist),
        &providers(&["strava"]),
        &cfg,
    );
    assert!(rec.eligible);
    assert!((rec.match_score - cfg.sport_agnostic_base_score).abs() < f32::EPSILON);
}

#[test]
fn cold_start_recommends_only_provider_free_starters() {
    let cfg = CoachRecommendationConfig::default();
    // Sleep / Yoga: no providers, no activity types -> starter set.
    let starter = score_coach(&prereqs(&[], 0, &[]), None, &providers(&[]), &cfg);
    assert!(starter.eligible);
    // Provider- or sport-gated coaches wait until we can see activity.
    let gated = score_coach(
        &prereqs(&["strava"], 5, &["Run"]),
        None,
        &providers(&[]),
        &cfg,
    );
    assert!(!gated.eligible);
}

#[test]
fn below_min_activities_is_penalized_but_still_eligible() {
    let cfg = CoachRecommendationConfig::default();
    // Runner with only 2 activities; coach asks for 5.
    let novice = profile(&[("run", 2)]);
    let rec = score_coach(
        &prereqs(&["strava"], 5, &["Run"]),
        Some(&novice),
        &providers(&["strava"]),
        &cfg,
    );
    assert!(rec.eligible);
    assert!((rec.match_score - cfg.below_min_activities_penalty).abs() < f32::EPSILON);
}

#[test]
fn active_sports_applies_count_and_share_thresholds() {
    let cfg = CoachRecommendationConfig::default();
    // 10 runs + 1 ride: ride is below both the count (3) and share (0.15) floors.
    let mixed = profile(&[("run", 10), ("ride", 1)]);
    let active = mixed.active_sports(cfg.min_activities_for_sport, cfg.min_share_for_sport);
    assert!(active.contains(&SportType::Run));
    assert!(!active.contains(&SportType::Ride));
}

/// A coach asking for `Ride` must match the athlete whose rides are logged
/// ONLY as mountain-bike and gravel.
///
/// No plain `ride` entry on purpose. An athlete who also logs road rides was
/// always eligible — `active` held `Ride` outright and exact equality found it
/// — which is why this has to be the pure case to mean anything. It is the
/// mirror of the 2026-08-27 grounding defect: that one hid an athlete's rides
/// from his coach, this hides the cycling coaches from a rider whose provider
/// tags every ride as a discipline.
#[test]
fn a_ride_coach_matches_an_athlete_who_logs_only_mountain_bike_and_gravel() {
    let cfg = CoachRecommendationConfig::default();
    let off_road_only = profile(&[("mountain_bike", 22), ("gravel_ride", 7), ("walk", 4)]);

    let ride_overlap = off_road_only.activity_type_overlap(
        &["Ride".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!(
        (ride_overlap - 1.0).abs() < f32::EPSILON,
        "a cycling coach must be eligible for an athlete who only rides off-road, got {ride_overlap}"
    );
}

/// The same rule on foot: trail sessions are running.
///
/// An athlete who logs every run as `TrailRunning` matched no `Run` coach at
/// all, which is most trail runners.
#[test]
fn a_run_coach_matches_an_athlete_who_logs_only_trail_running() {
    let cfg = CoachRecommendationConfig::default();
    let trail_runner = profile(&[("trail_running", 19), ("hike", 4)]);

    let run_overlap = trail_runner.activity_type_overlap(
        &["Run".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!(
        (run_overlap - 1.0).abs() < f32::EPSILON,
        "a running coach must be eligible for a trail runner, got {run_overlap}"
    );
}

/// Family-aware widens the head of a family, never a specific discipline.
///
/// A mountain-bike specialist must not be recommended to a road cyclist just
/// because both are cycling — that asymmetry is the whole point of the rule.
#[test]
fn a_discipline_specific_coach_stays_exact() {
    let cfg = CoachRecommendationConfig::default();
    let road_cyclist = profile(&[("ride", 20)]);

    let mtb_overlap = road_cyclist.activity_type_overlap(
        &["MountainBike".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!(
        mtb_overlap.abs() < f32::EPSILON,
        "a mountain-bike coach must not match a road-only cyclist, got {mtb_overlap}"
    );
}

/// Eligibility must not depend on how granularly the provider tagged the rides.
///
/// THE INVARIANT, and the reason the other tests in this group exist. Same
/// athlete, same thirty activities, same six rides — described two ways. Before
/// the family pass these scored 0.0 vs 1.0, 0.5 vs 1.0 and 0.333 vs 0.667: the
/// rider was punished for owning three bikes, because Strava derives `sport_type`
/// from default gear.
#[test]
fn sport_eligibility_does_not_depend_on_provider_tag_granularity() {
    let cfg = CoachRecommendationConfig::default();
    let fragmented = profile(&[
        ("run", 20),
        ("walk", 4),
        ("ride", 2),
        ("mountain_bike", 2),
        ("gravel_ride", 2),
    ]);
    let lumped = profile(&[("run", 20), ("walk", 4), ("ride", 6)]);

    for shape in [
        vec!["Ride".to_owned()],
        vec!["Run".to_owned(), "Ride".to_owned()],
        vec!["Run".to_owned(), "Ride".to_owned(), "Swim".to_owned()],
    ] {
        let a = fragmented.activity_type_overlap(
            &shape,
            cfg.min_activities_for_sport,
            cfg.min_share_for_sport,
        );
        let b = lumped.activity_type_overlap(
            &shape,
            cfg.min_activities_for_sport,
            cfg.min_share_for_sport,
        );
        assert!(
            (a - b).abs() < f32::EPSILON,
            "coach {shape:?} scored {a} for the athlete whose rides are split across \
             three bikes and {b} for the same six rides tagged plain — the tag is not \
             the training"
        );
    }
}

/// Rides split across disciplines clear the floor together.
///
/// Non-vacuous by construction: every cycling label is 2, under the count floor
/// of 3, and 2/30 = 6.7%, under the share floor of 15%. No single label can
/// satisfy this — only the family total of 6 (20%) can. Measured 0.0 before.
#[test]
fn a_ride_coach_matches_an_athlete_whose_rides_are_split_across_disciplines() {
    let cfg = CoachRecommendationConfig::default();
    let three_bikes = profile(&[
        ("run", 20),
        ("walk", 4),
        ("ride", 2),
        ("mountain_bike", 2),
        ("gravel_ride", 2),
    ]);

    let overlap = three_bikes.activity_type_overlap(
        &["Ride".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!(
        (overlap - 1.0).abs() < f32::EPSILON,
        "six rides across three bikes is a cyclist, got {overlap}"
    );
}

/// The same on foot — proves the rule is family-generic, not cycling-special-cased.
///
/// A cycling-only patch would pass the ride test and fail this one, which is
/// precisely what it is for. Each on-foot label is 2 (<3) at 6.7% (<15%); only
/// the family total of 6 clears. Measured 0.0 before.
#[test]
fn a_run_coach_matches_an_athlete_whose_runs_are_split_road_trail_treadmill() {
    let cfg = CoachRecommendationConfig::default();
    let mixed_runner = profile(&[
        ("run", 2),
        ("trail_running", 2),
        ("virtual_run", 2),
        ("ride", 24),
    ]);

    let overlap = mixed_runner.activity_type_overlap(
        &["Run".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!(
        (overlap - 1.0).abs() < f32::EPSILON,
        "road plus trail plus treadmill is a runner, got {overlap}"
    );
}

/// The stated intent, unchanged: one cross-training ride surfaces no cycling coach.
///
/// Passes both before and after — a guard on the bar, not proof of the fix. The
/// family total here is 1, which clears neither floor, so nothing is inserted.
#[test]
fn one_cross_training_ride_in_thirty_still_surfaces_no_cycling_coach() {
    let cfg = CoachRecommendationConfig::default();
    let runner = profile(&[("run", 29), ("ride", 1)]);

    let overlap = runner.activity_type_overlap(
        &["Ride".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!(
        overlap.abs() < f32::EPSILON,
        "the floor must not move — one ride in thirty is not a cyclist, got {overlap}"
    );
}

/// A discipline-specific coach stays exact even when the head was inserted.
///
/// Passes both before and after — it guards the family pass against widening
/// into specificity. The head `Ride` enters the set for this athlete, and a
/// `MountainBike` coach must still not match through it.
#[test]
fn an_inserted_family_head_does_not_make_a_specialist_coach_match() {
    let cfg = CoachRecommendationConfig::default();
    let three_bikes = profile(&[
        ("run", 20),
        ("walk", 4),
        ("ride", 2),
        ("mountain_bike", 2),
        ("gravel_ride", 2),
    ]);

    let overlap = three_bikes.activity_type_overlap(
        &["MountainBike".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!(
        overlap.abs() < f32::EPSILON,
        "a mountain-bike specialist must not match through an inserted Ride head, got {overlap}"
    );
}

/// The primary sport is the family the athlete trains most, named by the
/// discipline they actually log.
///
/// A bare `max_by_key` over labels answered the wrong question for anyone whose
/// provider splits one sport across several. These counts are the 2026-08-28
/// athlete's shape: 20 runs against 33 rides spread over three bikes. Cycling is
/// 62% of his training and the old code returned "run", which reached him as
/// "Welcome! Based on your recent Run training".
#[test]
fn a_split_cycling_athlete_is_not_reported_as_a_runner() {
    let mixed = profile(&[
        ("run", 20),
        ("mountain_bike", 12),
        ("gravel_ride", 11),
        ("ride", 10),
    ]);

    assert_eq!(
        mixed.primary_sport().as_deref(),
        Some("mountain_bike"),
        "cycling is 33 of 53 activities; the athlete should be told the bike he \
         rides most, not the one sport whose label happens to lead"
    );
}

/// Naming the family head would trade one wrong label for another.
///
/// An athlete who logs every run as `TrailRunning` is a trail runner. Reporting
/// "Run" because that is the family head is the same class of error in the
/// opposite direction, which is why the winner is resolved to a family and then
/// back down to a discipline.
#[test]
fn a_trail_runner_keeps_the_discipline_they_actually_log() {
    let trail = profile(&[("trail_running", 18), ("hike", 3)]);

    assert_eq!(
        trail.primary_sport().as_deref(),
        Some("trail_running"),
        "the family won, but the label reported must be the one he logs"
    );
}

/// Ties resolve the same way every time.
///
/// `sport_counts` is a `HashMap`, so without an explicit tie-break the answer
/// would depend on iteration order — and this string is shown to the athlete.
#[test]
fn an_exact_tie_is_stable_across_calls() {
    let tied = profile(&[("run", 10), ("swim", 10)]);

    let first = tied.primary_sport();
    assert!(first.is_some(), "a profile with rows has a primary sport");
    for _ in 0..20 {
        assert_eq!(
            tied.primary_sport(),
            first,
            "a tie must not depend on hash order"
        );
    }
}

#[test]
fn overlap_normalizes_titlecase_coach_activity_types() {
    let cfg = CoachRecommendationConfig::default();
    let runner = profile(&[("run", 10)]);
    // Coach frontmatter uses TitleCase "Run"; user activities are snake_case "run".
    let run_overlap = runner.activity_type_overlap(
        &["Run".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!((run_overlap - 1.0).abs() < f32::EPSILON);
    let ride_overlap = runner.activity_type_overlap(
        &["Ride".to_owned()],
        cfg.min_activities_for_sport,
        cfg.min_share_for_sport,
    );
    assert!(ride_overlap.abs() < f32::EPSILON);
}
