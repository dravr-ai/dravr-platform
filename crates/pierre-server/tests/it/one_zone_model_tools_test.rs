// ABOUTME: The tools render cageux's one zone model: Coggan's seven levels, tiered HR zones with zone 6, no defaults
// ABOUTME: Drives calculate_personalized_zones, the max-HR estimate and the intensity readers through their seams

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::{Duration, Utc};
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::zones::{HrZones, COGGAN_POWER_ZONE_NAMES};
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::zones::HrZonesUnavailable;
use pierre_core::models::{Activity, SportType};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_fitness_compute::training_history_compute::AthleteInputs;
use pierre_tool_runtime::implementations::analytics::{
    generate_training_recommendations, NutritionAthlete,
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::common;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("one_zone_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id.to_string()))
}

async fn call(
    executor: &UniversalToolExecutor,
    tool: &str,
    params: Value,
    user_id: Uuid,
    tenant_id: &str,
) -> Result<Value> {
    let response = executor
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters: params,
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant_id.to_owned()),
        })
        .await?;
    assert!(response.success, "{tool} failed: {:?}", response.error);
    Ok(response.result.expect("a payload"))
}

#[tokio::test]
async fn coggan_seven_levels_at_250_watts_through_the_tool() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = call(
        &executor,
        "calculate_personalized_zones",
        json!({ "ftp": 250 }),
        user_id,
        &tenant_id,
    )
    .await?;
    let zones = &result["personalized_zones"]["power_zones"];
    // 55 / 75 / 90 / 105 / 120 / 150 % of 250 W; level 7 has no ceiling.
    let expected = [
        (0.0, Some(137.5)),
        (137.5, Some(187.5)),
        (187.5, Some(225.0)),
        (225.0, Some(262.5)),
        (262.5, Some(300.0)),
        (300.0, Some(375.0)),
        (375.0, None),
    ];
    for (index, (min, max)) in expected.into_iter().enumerate() {
        let level = &zones[format!("zone_{}", index + 1)];
        assert_eq!(
            level["min_watts"],
            json!(min),
            "level {}: {level}",
            index + 1
        );
        assert_eq!(
            level["max_watts"],
            max.map_or(Value::Null, |watts| json!(watts)),
            "level {}: {level}",
            index + 1
        );
        assert_eq!(level["name"], json!(COGGAN_POWER_ZONE_NAMES[index]));
    }
    Ok(())
}

#[tokio::test]
async fn heart_rate_zones_carry_zone_6_the_tier_and_where_the_vo2max_came_from() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = call(
        &executor,
        "calculate_personalized_zones",
        json!({ "resting_hr": 50, "max_hr": 190 }),
        user_id,
        &tenant_id,
    )
    .await?;
    // No VO2max anywhere: cageux estimates 15.3 x 190 / 50 = 58.1 from the
    // heart rates, the advanced tier, which carries a zone 6.
    let expected = HrZones::for_athlete(None, 50, 190).unwrap();
    let rendered = &result["personalized_zones"]["heart_rate_zones"];
    assert_eq!(expected.zones().len(), 6);
    for band in expected.zones() {
        let zone = &rendered[format!("zone_{}", band.zone)];
        assert_eq!(zone["min_hr"], json!(band.lower_bpm), "{zone}");
        assert_eq!(zone["max_hr"], json!(band.upper_bpm), "{zone}");
        assert_eq!(zone["name"], json!(band.name));
    }
    assert_eq!(rendered["zone_6"]["max_hr"], json!(190));
    assert_eq!(rendered["tier"], json!("advanced"));
    assert_eq!(rendered["vo2_max_source"], json!("heart_rate_ratio"));
    assert_eq!(
        result["input_sources"]["vo2_max"],
        json!("heart_rate_ratio")
    );

    let calculations = &result["zone_calculations"];
    assert_eq!(calculations["method"], json!("karvonen_vo2max_tiered"));
    assert_eq!(calculations["tier"], json!("advanced"));
    assert_eq!(
        calculations["aerobic_threshold_hr"],
        json!(expected.aerobic_threshold_bpm)
    );
    assert_eq!(
        calculations["lactate_threshold_hr"],
        json!(expected.lactate_threshold_bpm)
    );
    // The estimate only chose the tier: pace zones still need a real VO2max.
    assert!(result["personalized_zones"]["pace_zones"].is_null());
    assert!(result["unavailable"]["pace_zones"]
        .as_str()
        .unwrap_or_default()
        .contains("vo2_max"));

    // A supplied VO2max below 50 is the intermediate tier: five zones only.
    let intermediate = call(
        &executor,
        "calculate_personalized_zones",
        json!({ "resting_hr": 50, "max_hr": 190, "vo2_max": 45.0 }),
        user_id,
        &tenant_id,
    )
    .await?;
    let rendered = &intermediate["personalized_zones"]["heart_rate_zones"];
    assert!(rendered["zone_6"].is_null(), "{rendered}");
    assert_eq!(rendered["tier"], json!("intermediate"));
    assert_eq!(rendered["vo2_max_source"], json!("provided"));
    assert_eq!(rendered["zone_5"]["max_hr"], json!(190));
    Ok(())
}

#[tokio::test]
async fn a_maximum_estimated_from_age_uses_the_configured_algorithm() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    call(
        &executor,
        "set_physiology",
        json!({ "age": 40, "resting_hr": 50 }),
        user_id,
        &tenant_id,
    )
    .await?;

    let result = call(
        &executor,
        "calculate_personalized_zones",
        json!({}),
        user_id,
        &tenant_id,
    )
    .await?;
    // The default `algorithm.maxhr` is Tanaka: 208 - 0.7 x 40 = 180.
    assert_eq!(result["user_profile"]["max_hr"], json!(180));
    assert_eq!(
        result["input_sources"]["max_hr"],
        json!("estimated_from_age")
    );
    assert_eq!(result["input_sources"]["max_hr_algorithm"], json!("tanaka"));
    let expected = HrZones::for_athlete(None, 50, 180).unwrap();
    assert_eq!(
        result["personalized_zones"]["heart_rate_zones"]["zone_1"]["max_hr"],
        json!(expected.zones()[0].upper_bpm)
    );
    Ok(())
}

/// Ten 50-minute runs alternating 170 bpm and 130 bpm, newest first.
fn alternating_runs() -> Vec<Activity> {
    (0..10)
        .map(|day| {
            let hr = if day % 2 == 0 { 170 } else { 130 };
            ActivityBuilder::new(
                format!("run-{day}"),
                "Run",
                SportType::Run,
                Utc::now() - Duration::days(day),
                3000,
                "test",
            )
            .distance_meters(10_000.0)
            .average_heart_rate(hr)
            .build()
        })
        .collect()
}

#[test]
fn the_intensity_mode_reads_the_athletes_zones_and_says_why_when_it_cannot() {
    let runs = alternating_runs();
    let config = AlgorithmConfig::default();
    let athlete = AthleteInputs::default();
    let nutrition = NutritionAthlete::default();
    let zones = HrZones::for_athlete(None, 50, 190).unwrap();

    let with_zones = generate_training_recommendations(
        &runs,
        "intensity",
        &athlete,
        Ok(&zones),
        &config,
        None,
        &nutrition,
    );
    assert!(with_zones.hr_zones_unavailable.is_none());
    let metrics = with_zones.metrics.expect("intensity metrics");
    assert_eq!(metrics.pattern_detected, Some(true));
    assert_eq!(metrics.hard_percentage, Some(50.0));

    let missing = HrZonesUnavailable::MissingHeartRates {
        resting_hr: true,
        max_hr: true,
    };
    let without = generate_training_recommendations(
        &runs,
        "intensity",
        &athlete,
        Err(&missing),
        &config,
        None,
        &nutrition,
    );
    assert_eq!(
        without.hr_zones_unavailable.as_deref(),
        Some(missing.reason().as_str())
    );
    assert_ne!(
        without.metrics.and_then(|m| m.hard_percentage),
        Some(50.0),
        "without zones heart rate cannot make half the runs hard"
    );

    // A mode that reads no heart rate carries no zone note.
    let plan = generate_training_recommendations(
        &runs,
        "training_plan",
        &athlete,
        Err(&missing),
        &config,
        None,
        &nutrition,
    );
    assert!(plan.hr_zones_unavailable.is_none());
}
