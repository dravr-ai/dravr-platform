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
fn locked_when_required_provider_missing() {
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
        "coach needing an unconnected provider is not eligible"
    );
    assert!(rec.match_score.abs() < f32::EPSILON);
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
