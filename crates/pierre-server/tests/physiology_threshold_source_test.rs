// ABOUTME: The thresholds training load scores against have one home: the profile set_physiology writes
// ABOUTME: Pins the lactate threshold as a fraction of VO2max, the measured LTHR, and the save reaching the load

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `lactate_threshold_percentage` was documented as a fraction of max HR by
//! `set_physiology`, which multiplied it into the LTHR, and read as a fraction
//! of `VO2max` by the pace-zone engine. It is a fraction of `VO2max` now, as
//! the model, the kernel and the catalogue define it; the LTHR is a measured
//! `threshold_hr`, else estimated from it by Swain et al. 1994.
//!
//! `analyze_training_load` and the recovery tools read their thresholds from
//! `update_user_configuration`'s overrides while `set_physiology` wrote the
//! profile, so a saved FTP changed nothing they reported; the fitness score,
//! the recommendations and the race-prediction confidence scored every session
//! with no threshold at all. Every training-load reader now reads the profile.

use anyhow::Result;
use chrono::{Duration, Utc};
use dravr_cageux::training_load::TrainingLoadCalculator;
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::{
    max_hr_fraction_at_vo2max_fraction, Activity, ConnectionType, SportType, TenantId,
    UserPhysiologicalProfile,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_fitness_compute::AthleteInputs;
use pierre_tool_runtime::implementations::analytics::analyze_detailed_training_load;
use pierre_tool_runtime::implementations::analytics::output::{FitnessScoreResult, ProvidersUsed};
use pierre_tool_runtime::implementations::analytics::{
    calculate_fitness_metrics, generate_training_recommendations, NutritionAthlete,
};
use pierre_tool_runtime::implementations::stored_physiology::stored_athlete_inputs;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::training_history_compute::{
    compute_and_persist_history, default_window, fetch_history_rows,
};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("threshold_source_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenant = executor
        .resources
        .repos()
        .tenants
        .get_all()
        .await?
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id.to_string()))
}

fn make_request(tool: &str, params: Value, user_id: Uuid, tenant_id: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_owned()),
    }
}

async fn set_physiology(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<Value> {
    let response = executor
        .execute_tool(make_request("set_physiology", params, user_id, tenant_id))
        .await?;
    assert!(response.success, "set_physiology: {:?}", response.error);
    Ok(response.result.expect("set_physiology returns a payload"))
}

async fn rejection(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    tool: &str,
    params: Value,
) -> Result<String> {
    match executor
        .execute_tool(make_request(tool, params, user_id, tenant_id))
        .await
    {
        Err(e) => Ok(e.to_string()),
        Ok(response) => {
            assert!(
                !response.success,
                "{tool} should have refused, got {:?}",
                response.result
            );
            Ok(response.error.unwrap_or_default())
        }
    }
}

async fn stored_profile(
    executor: &UniversalToolExecutor,
    tenant_id: &str,
    user_id: Uuid,
) -> Result<UserPhysiologicalProfile> {
    Ok(executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(TenantId::parse_str(tenant_id)?, user_id)
        .await?
        .expect("a profile exists"))
}

fn profile_with(
    lactate: Option<f64>,
    max_hr: Option<u16>,
    threshold_hr: Option<u16>,
) -> UserPhysiologicalProfile {
    let mut profile = UserPhysiologicalProfile::new(Uuid::new_v4(), SportType::Run);
    profile.lactate_threshold_percentage = lactate;
    profile.max_hr = max_hr;
    profile.threshold_hr = threshold_hr;
    profile
}

// ============================================================================
// The lactate threshold is a fraction of VO2max; the LTHR is measured first
// ============================================================================

#[test]
fn swain_places_a_fraction_of_vo2max_on_the_max_hr_scale() {
    // %HRmax = 0.6463 x %VO2max + 37.182 (Swain et al. 1994).
    assert!((max_hr_fraction_at_vo2max_fraction(0.85) - 0.921_175).abs() < 1e-9);
    assert!((max_hr_fraction_at_vo2max_fraction(0.65) - 0.791_915).abs() < 1e-9);
    assert!((max_hr_fraction_at_vo2max_fraction(0.50) - 0.694_97).abs() < 1e-9);
}

#[test]
fn the_lthr_is_estimated_from_the_lactate_threshold_on_the_vo2max_scale() {
    let lthr = profile_with(Some(0.85), Some(190), None)
        .lactate_threshold_hr()
        .expect("a lactate threshold and a max HR estimate one");
    // 190 x 0.921175, not the 190 x 0.85 = 161.5 a max-HR reading gave.
    assert!((lthr - 175.023_25).abs() < 1e-6, "got {lthr}");

    assert_eq!(
        profile_with(Some(0.85), None, None).lactate_threshold_hr(),
        None,
        "an estimate needs a measured max HR"
    );
    assert_eq!(
        profile_with(None, Some(190), None).lactate_threshold_hr(),
        None
    );
}

#[test]
fn a_measured_threshold_heart_rate_wins_over_the_estimate() {
    let profile = profile_with(Some(0.85), Some(190), Some(168));
    assert_eq!(profile.lactate_threshold_hr(), Some(168.0));
    assert!(
        (profile.estimated_lactate_threshold_hr().unwrap() - 175.023_25).abs() < 1e-6,
        "the estimate is still there to compare against"
    );

    let inputs = AthleteInputs::from_profile(Some(&profile));
    assert_eq!(inputs.lthr, Some(168.0), "training load scores against it");
    assert_eq!(inputs.max_hr, Some(190.0));
}

#[test]
fn no_profile_gives_no_thresholds() {
    let inputs = AthleteInputs::from_profile(None);
    assert_eq!(inputs.ftp_watts, None);
    assert_eq!(inputs.lthr, None);
    assert_eq!(inputs.weight_kg, None);
}

// ============================================================================
// set_physiology writes both, in their own units
// ============================================================================

#[tokio::test]
async fn set_physiology_saves_a_measured_threshold_heart_rate() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({"max_hr": 190, "resting_hr": 50, "threshold_hr": 168, "lactate_threshold_percentage": 0.82}),
    )
    .await?;

    assert_eq!(result["profile"]["threshold_hr"], json!(168));
    assert_eq!(
        result["profile"]["lactate_threshold_percentage"],
        json!(0.82)
    );
    assert!(
        result["updated_fields"]
            .as_array()
            .unwrap()
            .contains(&json!("threshold_hr")),
        "{:?}",
        result["updated_fields"]
    );
    let stored = stored_profile(&executor, &tenant_id, user_id).await?;
    assert_eq!(stored.threshold_hr, Some(168), "the row itself holds it");
    assert_eq!(stored.lactate_threshold_percentage, Some(0.82));
    Ok(())
}

#[tokio::test]
async fn a_threshold_heart_rate_at_or_above_max_is_refused() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        "set_physiology",
        json!({"max_hr": 180, "threshold_hr": 185}),
    )
    .await?;
    assert!(
        error.contains("threshold_hr (185) must be less than max_hr (180)"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
async fn a_lactate_threshold_is_refused_in_the_vo2max_unit_it_is_read_in() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        "set_physiology",
        json!({"lactate_threshold_percentage": 0.55}),
    )
    .await?;
    assert!(error.contains("fraction of VO2max"), "{error}");
    Ok(())
}

#[tokio::test]
async fn the_pace_zones_read_the_saved_lactate_threshold() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({"vo2_max": 55.0, "lactate_threshold_percentage": 0.80}),
    )
    .await?;

    let zones = |params: Value| {
        executor.execute_tool(make_request(
            "calculate_personalized_zones",
            params,
            user_id,
            &tenant_id,
        ))
    };
    let saved = zones(json!({})).await?.result.expect("a payload");
    assert_eq!(
        saved["input_sources"]["lactate_threshold"],
        json!("profile")
    );
    assert_eq!(saved["user_profile"]["lactate_threshold"], json!(0.80));

    let higher = zones(json!({"lactate_threshold": 0.90}))
        .await?
        .result
        .expect("a payload");
    assert_ne!(
        saved["personalized_zones"]["pace_zones"]["zone_3_threshold"],
        higher["personalized_zones"]["pace_zones"]["zone_3_threshold"],
        "threshold pace moves with the share of VO2max held at threshold"
    );
    Ok(())
}

// ============================================================================
// A value saved with set_physiology changes analyze_training_load's answer
// ============================================================================

/// Hour-long rides with heart rate and no power, so their stress is scored
/// against the LTHR when there is one.
fn heart_rate_rides() -> Vec<Activity> {
    (1..=21)
        .map(|day| {
            ActivityBuilder::new(
                format!("hr-ride-{day}"),
                format!("ride {day}"),
                SportType::Ride,
                Utc::now() - Duration::days(day),
                3_600,
                "strava".to_owned(),
            )
            .distance_meters(30_000.0)
            .average_heart_rate(152)
            .build()
        })
        .collect()
}

/// Power-only rides, scored against the FTP when there is one.
fn power_rides() -> Vec<Activity> {
    (1..=21)
        .map(|day| {
            ActivityBuilder::new(
                format!("power-ride-{day}"),
                format!("ride {day}"),
                SportType::Ride,
                Utc::now() - Duration::days(day),
                3_600,
                "strava".to_owned(),
            )
            .distance_meters(32_000.0)
            .average_power(230)
            .build()
        })
        .collect()
}

/// The CTL `analyze_training_load` reports for `activities`, scored against
/// what its reader returns for the athlete.
async fn analyzed_ctl(
    executor: &UniversalToolExecutor,
    tenant_id: &str,
    user_id: Uuid,
    activities: &[Activity],
) -> Result<f64> {
    let inputs = stored_athlete_inputs(&executor.resources, Some(tenant_id), user_id).await?;
    let payload = serde_json::to_value(analyze_detailed_training_load(
        activities,
        &inputs,
        &executor.cageux_config().algorithms,
        ProvidersUsed {
            activity_provider: "strava".to_owned(),
            sleep_provider: None,
        },
    ))?;
    Ok(payload["load_metrics"]["ctl"]
        .as_f64()
        .expect("an analysed load carries its CTL"))
}

/// Two whole-number CTLs, as the tools round them, are the same reading.
fn assert_same_ctl(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < f64::EPSILON,
        "{what}: {actual} vs {expected}"
    );
}

/// The calculator's own CTL for `activities` at `ftp` and `lthr`, rounded as
/// the tool rounds it.
fn calculator_ctl(
    executor: &UniversalToolExecutor,
    activities: &[Activity],
    ftp: Option<f64>,
    lthr: Option<f64>,
) -> Result<f64> {
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(Activity::start_date);
    Ok(TrainingLoadCalculator::from_config(
        executor.cageux_config().algorithms.clone(),
        Utc::now().date_naive(),
    )
    .calculate_training_load(&sorted, ftp, lthr, None, None, None)?
    .ctl
    .round())
}

#[tokio::test]
async fn a_threshold_heart_rate_saved_with_set_physiology_changes_the_training_load() -> Result<()>
{
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let rides = heart_rate_rides();

    let before = analyzed_ctl(&executor, &tenant_id, user_id, &rides).await?;
    set_physiology(&executor, user_id, &tenant_id, json!({"threshold_hr": 160})).await?;
    let after = analyzed_ctl(&executor, &tenant_id, user_id, &rides).await?;

    assert_same_ctl(
        after,
        calculator_ctl(&executor, &rides, None, Some(160.0))?,
        "the rides are scored against the saved LTHR",
    );
    assert!(
        (after - before).abs() >= 1.0,
        "saving an LTHR changes the load of heart-rate-only rides: {before} both ways"
    );
    Ok(())
}

#[tokio::test]
async fn an_ftp_saved_with_set_physiology_changes_the_training_load() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let rides = power_rides();

    let before = analyzed_ctl(&executor, &tenant_id, user_id, &rides).await?;
    set_physiology(&executor, user_id, &tenant_id, json!({"ftp_watts": 250})).await?;
    let after = analyzed_ctl(&executor, &tenant_id, user_id, &rides).await?;

    assert_same_ctl(
        after,
        calculator_ctl(&executor, &rides, Some(250.0), None)?,
        "the rides are scored against the saved FTP",
    );
    assert!(
        (after - before).abs() >= 1.0,
        "saving an FTP changes the load of power-only rides: {before} both ways"
    );
    Ok(())
}

#[tokio::test]
async fn configuration_overrides_are_not_a_second_source_of_thresholds() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(&executor, user_id, &tenant_id, json!({"ftp_watts": 250})).await?;

    // A document written before the configuration refused measurements.
    executor
        .resources
        .repos()
        .profiles
        .save_configuration(
            &user_id.to_string(),
            &json!({"session_overrides": {"ftp": 320, "threshold_hr": 150}}).to_string(),
        )
        .await?;
    let inputs = stored_athlete_inputs(&executor.resources, Some(&tenant_id), user_id).await?;
    assert_eq!(inputs.ftp_watts, Some(250.0), "the profile's FTP, not 320");
    assert_eq!(inputs.lthr, None, "no LTHR was saved with set_physiology");

    // And the configuration tool refuses them, naming where they belong.
    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        "update_user_configuration",
        json!({"parameters": {"ftp": 300, "max_hr": 190}}),
    )
    .await?;
    assert!(
        error.contains("ftp, max_hr belong to the athlete's physiological profile")
            && error.contains("set_physiology"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
async fn a_request_without_a_tenant_reads_no_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(&executor, user_id, &tenant_id, json!({"ftp_watts": 250})).await?;

    let inputs = stored_athlete_inputs(&executor.resources, None, user_id).await?;
    assert_eq!(
        inputs.ftp_watts, None,
        "profiles are tenant-scoped; no tenant, no profile"
    );
    let inputs = stored_athlete_inputs(&executor.resources, Some(&tenant_id), user_id).await?;
    assert_eq!(inputs.ftp_watts, Some(250.0));
    Ok(())
}

// ============================================================================
// Every other training-load surface reads the same saved thresholds
// ============================================================================

fn providers() -> ProvidersUsed {
    ProvidersUsed {
        activity_provider: "strava".to_owned(),
        sleep_provider: None,
    }
}

#[tokio::test]
async fn the_fitness_score_reads_the_ftp_saved_with_set_physiology() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let rides = power_rides();
    let config = &executor.cageux_config().algorithms;
    let ctl = |inputs: &AthleteInputs| match calculate_fitness_metrics(
        &rides,
        "month",
        inputs,
        config,
        providers(),
    ) {
        FitnessScoreResult::Scored(detail) => detail.metrics.ctl,
        FitnessScoreResult::NoData(empty) => panic!("21 rides score: {}", empty.message),
    };

    let before = ctl(&stored_athlete_inputs(&executor.resources, Some(&tenant_id), user_id).await?);
    set_physiology(&executor, user_id, &tenant_id, json!({"ftp_watts": 250})).await?;
    let after = ctl(&stored_athlete_inputs(&executor.resources, Some(&tenant_id), user_id).await?);

    assert_same_ctl(
        after,
        calculator_ctl(&executor, &rides, Some(250.0), None)?,
        "the rides are scored against the saved FTP",
    );
    assert_same_ctl(
        after,
        analyzed_ctl(&executor, &tenant_id, user_id, &rides).await?,
        "the fitness score and analyze_training_load report one CTL",
    );
    assert!((after - before).abs() >= 1.0, "{before} both ways");
    Ok(())
}

#[tokio::test]
async fn the_recovery_recommendations_read_the_threshold_heart_rate_saved_with_set_physiology(
) -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let rides = heart_rate_rides();
    let config = &executor.cageux_config().algorithms;
    let recovery_ctl = |inputs: &AthleteInputs| {
        generate_training_recommendations(
            &rides,
            "recovery",
            inputs,
            config,
            None,
            &NutritionAthlete::default(),
        )
        .metrics
        .and_then(|m| m.ctl)
        .expect("the recovery mode reports the CTL it advises on")
    };

    let before =
        recovery_ctl(&stored_athlete_inputs(&executor.resources, Some(&tenant_id), user_id).await?);
    set_physiology(&executor, user_id, &tenant_id, json!({"threshold_hr": 160})).await?;
    let after =
        recovery_ctl(&stored_athlete_inputs(&executor.resources, Some(&tenant_id), user_id).await?);

    assert_same_ctl(
        after.round(),
        calculator_ctl(&executor, &rides, None, Some(160.0))?,
        "the rides are scored against the saved LTHR",
    );
    assert!((after - before).abs() >= 1.0, "{before} both ways");
    Ok(())
}

// ============================================================================
// The stored daily history follows a threshold change
// ============================================================================

/// A connected provider with a run cached every third day for 200 days —
/// deep enough to warm the default 90-day window — and the stored rollup
/// computed from it.
async fn seed_and_compute_history(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
) -> Result<()> {
    let tenant = TenantId::parse_str(tenant_id)?;
    let repos = executor.resources.repos();
    repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await?;
    let runs: Vec<Activity> = (0..=66)
        .map(|i| {
            let days_ago = i * 3;
            ActivityBuilder::new(
                format!("history-run-{days_ago}"),
                format!("run {days_ago}d ago"),
                SportType::Run,
                Utc::now() - Duration::days(days_ago),
                3_600,
                "strava".to_owned(),
            )
            .distance_meters(12_000.0)
            .average_heart_rate(155)
            .build()
        })
        .collect();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &runs)
        .await?;
    let (from, to) = default_window(&executor.resources, user_id).await?;
    compute_and_persist_history(&executor.resources, tenant, user_id, from, to).await?;
    Ok(())
}

/// The CTL of the latest stored day, as `get_training_history` reads it.
async fn latest_stored_ctl(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
) -> Result<f64> {
    let (from, to) = default_window(&executor.resources, user_id).await?;
    let rows = fetch_history_rows(
        &executor.resources.data(),
        TenantId::parse_str(tenant_id)?,
        user_id,
        from,
        to,
    )
    .await?;
    Ok(rows.last().expect("the window has stored rows").ctl)
}

#[tokio::test]
async fn a_threshold_saved_with_set_physiology_recomputes_the_stored_history() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    seed_and_compute_history(&executor, user_id, &tenant_id).await?;
    let before = latest_stored_ctl(&executor, user_id, &tenant_id).await?;

    let saved =
        set_physiology(&executor, user_id, &tenant_id, json!({"threshold_hr": 160})).await?;
    assert_eq!(
        saved["training_history"]["status"],
        json!("recomputed"),
        "{saved:#}"
    );
    assert!(
        saved["training_history"]["rows_upserted"]
            .as_u64()
            .is_some_and(|rows| rows > 0),
        "{saved:#}"
    );
    let after = latest_stored_ctl(&executor, user_id, &tenant_id).await?;
    assert!(
        (after - before).abs() >= 1.0,
        "the stored runs are rescored against the saved LTHR: {before} both ways"
    );

    // A save that moves no training-load input leaves the rows alone.
    let unrelated = set_physiology(&executor, user_id, &tenant_id, json!({"age": 34})).await?;
    assert_eq!(
        unrelated["training_history"],
        json!({"status": "unaffected"})
    );
    Ok(())
}

#[tokio::test]
async fn a_threshold_saved_before_any_history_is_stored_has_nothing_to_recompute() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = set_physiology(&executor, user_id, &tenant_id, json!({"ftp_watts": 250})).await?;
    assert_eq!(
        saved["training_history"],
        json!({"status": "nothing_stored"})
    );
    Ok(())
}
