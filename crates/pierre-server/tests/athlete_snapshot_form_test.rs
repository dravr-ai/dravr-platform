// ABOUTME: The athlete snapshot carries the same FormReading the tools render, and a reply is checked against it
// ABOUTME: DB profile + activities → build_athlete_metrics → form reading → personalized verdict on the product's own prose
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use anyhow::Result;
use chrono::{Duration, Utc};
use dravr_cageux::config::intelligence::{AlgorithmConfig, TrainingZonesConfig};
use pierre_core::civil_time::{clock_date, resolve_zone};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::{Activity, SportType, TenantId, UserPhysiologicalProfile};
use pierre_core::transport::TransportPolicy;
use pierre_evals::personalized::check as personalized_check;
use pierre_evals::{ClaimSource, ConservativeStrategy, ExtractedClaim, PersonalizedContext};
use pierre_fitness_compute::{compute_training_history, AthleteInputs};
use pierre_memory::claims::{ClaimCategory, ClaimStatus};
use pierre_providers::ai_scope;
use pierre_providers::registry::global_registry;
use pierre_services::athlete_snapshot::build_athlete_metrics;
use uuid::Uuid;

/// Days of training the athlete has behind them, one run a day.
const TRAINING_DAYS: i64 = 16;

fn tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("00000000-0000-0000-0000-000000000042").unwrap())
}

fn synthetic_run(id: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("synthetic {id}"),
        SportType::Run,
        Utc::now() - Duration::days(days_ago),
        3600,
        "synthetic".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(150)
    .average_power(200)
    .normalized_power(205)
    .build()
}

fn profile(user_id: Uuid) -> UserPhysiologicalProfile {
    UserPhysiologicalProfile {
        user_id,
        vo2_max: Some(50.0),
        resting_hr: Some(48),
        max_hr: Some(192),
        lactate_threshold_percentage: Some(0.85),
        threshold_hr: None,
        age: Some(35),
        weight: Some(70.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sport: SportType::Run,
        training_experience_years: Some(8),
        ftp_watts: Some(250),
        threshold_pace_sec_per_km: Some(240.0),
        hr_zones: None,
        power_zones: None,
        critical_power_watts: None,
        w_prime_joules: None,
        critical_speed_mps: None,
        d_prime_meters: None,
        transport_policy: TransportPolicy::AnyTransport,
    }
}

fn claim(text: &str) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category: ClaimCategory::Physiological,
        source: ClaimSource::Reply,
    }
}

/// The reading the snapshot carries is the one the daily row hands every other
/// surface, and the prose the tools render from it is supported by the checker
/// while the same prose with its share changed is contradicted.
#[tokio::test]
async fn the_snapshot_carries_the_reading_the_tools_render() -> Result<()> {
    common::init_server_config();
    let database = common::create_test_database().await?;
    let repos = database.repositories();
    let tenant_id = tenant();
    let (user_id, _user) = common::create_test_user(&database).await?;
    let profile = profile(user_id);

    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &profile)
        .await?;
    let activities: Vec<Activity> = (1..=TRAINING_DAYS)
        .map(|d| synthetic_run(&format!("run-{d}"), d))
        .collect();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant_id, "strava", &activities)
        .await?;

    let metrics = build_athlete_metrics(
        repos,
        global_registry().as_ref(),
        &AlgorithmConfig::default(),
        &TrainingZonesConfig::default(),
        tenant_id,
        user_id,
    )
    .await;
    let form = metrics
        .form
        .expect("sixteen training days give a form reading");

    // The same stored activities, through the same rollup the Home block and
    // `get_training_history` read, banded by the row's own `form_reading`.
    let now = Utc::now();
    let stored = ai_scope::filter_activities(
        global_registry().as_ref(),
        repos
            .activity_cache
            .get_cached_activities(
                user_id,
                &tenant_id,
                None,
                now - Duration::days(180),
                now,
                500,
            )
            .await?,
    );
    let today = clock_date(now, resolve_zone(None));
    let states = compute_training_history(
        &stored,
        AthleteInputs::from_profile(Some(&profile)),
        today - Duration::days(90),
        today,
        &AlgorithmConfig::default(),
        None,
    )
    .expect("the default training-load parameters are accepted");
    let expected = states.last().unwrap().form_reading();
    assert_eq!(
        form, expected,
        "the snapshot must carry the daily row's reading, CTL and share and band"
    );
    assert!(
        form.form_pct.is_some(),
        "sixteen training days are a chronic base, so the share can be judged: {form:?}"
    );

    // A reply that repeats what the tools rendered is true of this athlete.
    let strategy = ConservativeStrategy::default();
    let ctx = PersonalizedContext {
        metrics: &metrics,
        tolerance: &strategy,
    };
    let prose = form.inline();
    let verdict = personalized_check(&claim(&prose), &ctx)
        .unwrap_or_else(|| panic!("the rendered form was not checked: {prose}"));
    assert_eq!(verdict.status, ClaimStatus::Supported, "{prose}");

    // And one figure changed is not.
    let pct = form.form_pct.unwrap().round();
    let altered = prose.replace(&format!("{pct:.0}%"), &format!("{:.0}%", pct + 40.0));
    assert_ne!(
        altered, prose,
        "the share must appear in the prose: {prose}"
    );
    let verdict = personalized_check(&claim(&altered), &ctx)
        .unwrap_or_else(|| panic!("the altered form was not checked: {altered}"));
    assert_eq!(verdict.status, ClaimStatus::Contradicted, "{altered}");

    Ok(())
}
