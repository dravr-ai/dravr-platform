// ABOUTME: The verifier's snapshot reads the athlete's VDOT from a performance only, never from their VO2max (carnet#1693 item 5)
// ABOUTME: DB profile + cached runs → build_athlete_metrics → vdot and pace ranges: stated while fresh, else best run, else none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use anyhow::Result;
use dravr_cageux::config::intelligence::{AlgorithmConfig, TrainingZonesConfig};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::{Activity, ProfileVdot, SportType, TenantId, UserPhysiologicalProfile};
use pierre_evals::AthleteMetrics;
use pierre_services::athlete_snapshot::build_athlete_metrics;
use uuid::Uuid;

fn tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("00000000-0000-0000-0000-000000000043").unwrap())
}

/// A 10 km run `days_ago` days back in `seconds`.
fn run_10k(id: &str, days_ago: i64, seconds: u64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("10 km {id}"),
        SportType::Run,
        Timestamp::now() - Elapsed::days(days_ago),
        seconds,
        "synthetic".to_owned(),
    )
    .distance_meters(10_000.0)
    .build()
}

/// An athlete whose watch reads `VO2max` 60, with nothing else saved.
fn vo2max_60(user_id: Uuid) -> UserPhysiologicalProfile {
    let mut profile = UserPhysiologicalProfile::new(user_id, SportType::Run);
    profile.vo2_max = Some(60.0);
    profile
}

/// `profile` stating VDOT `vdot`, true `days_ago` days back.
fn stating(
    mut profile: UserPhysiologicalProfile,
    vdot: f64,
    days_ago: i64,
) -> UserPhysiologicalProfile {
    profile.vdot = Some(ProfileVdot {
        value: vdot,
        as_of: (Timestamp::now() - Elapsed::days(days_ago)).utc_date(),
        method: Some("race: 10 km in 41:21".to_owned()),
    });
    profile
}

/// Store `profile` and `runs`, then build the snapshot.
async fn snapshot(
    profile: impl FnOnce(Uuid) -> UserPhysiologicalProfile,
    runs: &[Activity],
) -> Result<AthleteMetrics> {
    common::init_server_config();
    let database = common::create_test_database().await?;
    let repos = database.repositories();
    let tenant_id = tenant();
    let (user_id, _user) = common::create_test_user(&database).await?;
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &profile(user_id))
        .await?;
    if !runs.is_empty() {
        repos
            .activity_cache
            .upsert_activities(user_id, &tenant_id, "strava", runs)
            .await?;
    }
    Ok(build_athlete_metrics(
        repos,
        &common::offline_provider_registry(),
        &AlgorithmConfig::default(),
        &TrainingZonesConfig::default(),
        tenant_id,
        user_id,
    )
    .await)
}

#[tokio::test]
async fn snapshot_has_no_vdot_without_a_performance() -> Result<()> {
    let metrics = snapshot(vo2max_60, &[]).await?;
    assert_eq!(metrics.vo2max, Some(60.0), "the VO2max is still the VO2max");
    assert_eq!(metrics.vdot, None, "bug: a VO2max was read as a VDOT");
    for (zone, range) in [
        ("easy", metrics.easy_pace_range),
        ("marathon", metrics.marathon_pace_range),
        ("threshold", metrics.threshold_pace_range),
        ("interval", metrics.interval_pace_range),
    ] {
        assert_eq!(
            range, None,
            "bug: the {zone} paces were cut with no VDOT, from the VO2max"
        );
    }
    Ok(())
}

#[tokio::test]
async fn snapshot_paces_follow_stated_vdot_not_vo2max() -> Result<()> {
    // VO2max 60 would put threshold near 3:40/km; a stated VDOT 50 puts it at
    // Daniels' 4:15 (255.2 s/km), its band 250.6-260.0 s/km at 0.88 ± 0.02.
    let metrics = snapshot(|u| stating(vo2max_60(u), 50.0, 3), &[]).await?;
    assert_eq!(
        metrics.vdot,
        Some(50.0),
        "bug: the snapshot must hold the fresh stated VDOT, not the VO2max"
    );
    let (fast, slow) = metrics
        .threshold_pace_range
        .expect("a stated VDOT cuts the paces");
    let (fast, slow) = (fast.min(slow), fast.max(slow));
    assert!(
        (fast - 250.6).abs() < 0.5 && (slow - 260.0).abs() < 0.5,
        "the threshold band follows VDOT 50, not VO2max 60: {fast}-{slow}"
    );
    for (zone, range) in [
        ("easy", metrics.easy_pace_range),
        ("marathon", metrics.marathon_pace_range),
        ("interval", metrics.interval_pace_range),
    ] {
        assert!(
            range.is_some(),
            "bug: a stated VDOT must cut the {zone} paces too"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_stated_vdot_older_than_12_weeks_yields_to_a_recent_run() -> Result<()> {
    // VDOT 50 stated 100 days ago; a 10 km in 45:00 last week reads VDOT
    // 45.3, and that recent run is what the snapshot now holds.
    let runs = [run_10k("recent", 7, 2700)];
    let metrics = snapshot(|u| stating(vo2max_60(u), 50.0, 100), &runs).await?;
    let vdot = metrics.vdot.expect("the recent run gives a VDOT");
    assert!(
        (vdot - 45.26).abs() < 0.05,
        "bug: a stale stated VDOT outranked the recent run: {vdot}"
    );

    // While it is fresh, the stated VDOT wins over the same run.
    let fresh = snapshot(|u| stating(vo2max_60(u), 50.0, 30), &runs).await?;
    assert_eq!(
        fresh.vdot,
        Some(50.0),
        "bug: a stated VDOT under 12 weeks old must outrank the recent run"
    );
    Ok(())
}
