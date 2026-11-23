// ABOUTME: Exercises estimate_vo2max end to end through the tool executor — each field test against its published equation
// ABOUTME: Pins VO2max vs VDOT, Swain's heart-rate sources and window, the warnings, the profile defaults and chat reachability

// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use dravr_tronc::dates::CivilDate;
use pierre_core::models::{
    AnchorStamp, MeasurementKind, MetricProvenance, SportType, TenantId, UserPhysiologicalProfile,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::scopes::{missing_scope, required_scopes};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::common;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    // The dispatch chokepoint enforces OAuth scopes; the seeding write
    // (set_physiology) needs the athlete's own grant, exactly as its tests do.
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("vo2max_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
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

async fn estimate(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<Value> {
    let response = executor
        .execute_tool(make_request("estimate_vo2max", params, user_id, tenant_id))
        .await?;
    assert!(
        response.success,
        "estimate_vo2max should have succeeded: {:?}",
        response.error
    );
    Ok(response.result.expect("estimate_vo2max returns a payload"))
}

/// The text of the rejection, from whichever channel it arrives on.
async fn estimate_expecting_rejection(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<String> {
    match executor
        .execute_tool(make_request("estimate_vo2max", params, user_id, tenant_id))
        .await
    {
        Err(e) => Ok(e.to_string()),
        Ok(response) => {
            assert!(
                !response.success,
                "estimate_vo2max should have rejected these values, got: {:?}",
                response.result
            );
            Ok(format!("{:?}", response.error))
        }
    }
}

/// Kilograms in one pound: Kline's Rockport equation takes weight in pounds,
/// the tool takes it in kilograms.
const KG_PER_LB: f64 = 0.453_592_37;

fn vo2(result: &Value) -> f64 {
    result["vo2max_ml_kg_min"]
        .as_f64()
        .expect("vo2max_ml_kg_min is a number")
}

fn vdot(result: &Value) -> f64 {
    result["vdot"].as_f64().expect("vdot is a number")
}

fn warnings(result: &Value) -> Vec<String> {
    result["warnings"]
        .as_array()
        .expect("warnings is an array")
        .iter()
        .filter_map(|w| w.as_str().map(str::to_owned))
        .collect()
}

// ============================================================================
// Each method reproduces its published equation
// ============================================================================

#[tokio::test]
async fn cooper_reports_the_published_equation() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // Cooper (1968): VO2max = (distance_m − 504.9) / 44.73. 2,800 m → 51.31.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "cooper_test", "distance_meters": 2800 }),
    )
    .await?;

    let expected = (2800.0_f64 - 504.9) / 44.73;
    assert!(
        (vo2(&result) - expected).abs() < 0.06,
        "Cooper for 2,800 m should be {expected:.2}, got {}",
        vo2(&result)
    );
    assert_eq!(result["method"], json!("cooper_test"));
    assert_eq!(
        result["saved"],
        json!(false),
        "the tool estimates, it does not write"
    );
    assert_eq!(result["defaults_from_profile"], json!([]));
    assert!(
        result["formula"].as_str().unwrap_or("").contains("Cooper"),
        "the formula field names the method's source"
    );
    Ok(())
}

#[tokio::test]
async fn cooper_gives_a_vdot_and_a_threshold_pace() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // The 12-minute run read as a race through Daniels' curves: 2,800 m in
    // 12:00 is 233.3 m/min, oxygen cost 43.59 ml/kg/min, held at 99.19 % of
    // VO2max for 12 minutes, so VDOT 43.9 — beside Cooper's VO2max of 51.3.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "cooper_test", "distance_meters": 2800 }),
    )
    .await?;
    assert!(
        (vdot(&result) - 43.9).abs() < 0.11,
        "Cooper's run read as a 12-minute race should be VDOT 43.9: {result}"
    );
    let pace = result["implied_threshold_pace_sec_per_km"]
        .as_f64()
        .expect("Cooper's VDOT implies a threshold pace");
    assert!(
        (270.0..=300.0).contains(&pace),
        "VDOT 43.9 at Daniels' 0.88 runs threshold near 4:44/km, got {pace}"
    );
    let to_store = result["to_store"].as_str().unwrap_or("");
    assert!(
        to_store.contains("vo2_max_method 'cooper_test'") && to_store.contains("vdot_method"),
        "both numbers are storable, each with its method: {to_store}"
    );
    Ok(())
}

#[tokio::test]
async fn from_vdot_reports_a_vdot_not_a_vo2max() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // A VDOT is a performance index, not a VO2max: it comes back under its
    // own name, unchanged, and no VO2max is claimed from it.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "from_vdot", "vdot": 50 }),
    )
    .await?;
    assert!(
        (vdot(&result) - 50.0).abs() < f64::EPSILON,
        "bug: a stated VDOT must come back unchanged: {result}"
    );
    assert_eq!(
        result["vo2max_ml_kg_min"],
        Value::Null,
        "a stated VDOT gives no VO2max"
    );
    Ok(())
}

// ============================================================================
// Swain's VO2-reserve cycle stage (carnet#1693 item 1)
// ============================================================================

/// One 6-minute stage at 200 W ending at 148 bpm, for an athlete of maximum
/// 190 and resting 50: 70 % of the 140-beat reserve, so a maximal 285.7 W,
/// and ACSM's 10.8 × 285.7 / 70 + 7 = 51.08 ml/kg/min.
const SWAIN_VO2MAX: f64 = 10.8 * (200.0 / 0.7) / 70.0 + 7.0;

#[tokio::test]
async fn swain_names_the_heart_rates_it_used_and_where_each_came_from() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let saved = executor
        .execute_tool(make_request(
            "set_physiology",
            json!({ "max_hr": 190 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(saved.success, "seeding failed: {:?}", saved.error);

    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "swain_vo2_reserve",
            "power_watts": 200,
            "heart_rate": 148,
            "resting_hr": 50,
            "weight_kg": 70,
        }),
    )
    .await?;
    assert!(
        (vo2(&result) - SWAIN_VO2MAX).abs() < 0.06,
        "Swain should read {SWAIN_VO2MAX:.2}: {result}"
    );
    let used = &result["heart_rates_used"];
    assert_eq!(
        used["max_hr"]["bpm"],
        json!(190.0),
        "bug: the saved maximum heart rate must be the one used: {result}"
    );
    assert_eq!(
        used["max_hr"]["source"],
        json!("profile"),
        "the saved maximum is the athlete's own: {result}"
    );
    assert_eq!(
        used["resting_hr"]["bpm"],
        json!(50.0),
        "bug: the call's resting heart rate must be the one used: {result}"
    );
    assert_eq!(
        used["resting_hr"]["source"],
        json!("call"),
        "bug: a resting heart rate the call gave must be named as the call's: {result}"
    );
    assert_eq!(
        result["defaults_from_profile"],
        json!(["max_hr"]),
        "bug: only the maximum heart rate came from stored data: {result}"
    );
    assert_eq!(
        result["implied_threshold_pace_sec_per_km"],
        Value::Null,
        "a cycling VO2max implies no running pace"
    );
    assert_eq!(
        result["vdot"],
        Value::Null,
        "bug: a cycling VO2max must give no VDOT: {result}"
    );
    assert!(
        warnings(&result).is_empty(),
        "bug: an estimate inside 20-90 must carry no warning: {result}"
    );
    Ok(())
}

#[tokio::test]
async fn swain_says_when_its_heart_rates_are_a_providers() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::parse_str(&tenant_id)?;
    // intervals.icu's maximum, and a resting heart rate the weekly promotion
    // filed from the athlete's morning readings.
    let mut profile = UserPhysiologicalProfile::new(user_id, SportType::Ride);
    profile.max_hr = Some(190);
    profile.threshold_provenance.max_hr = Some(MetricProvenance {
        kind: MeasurementKind::Estimated,
        origin: Some("intervals.icu".to_owned()),
        as_of: CivilDate::from_ymd(2026, 11, 1).ok(),
    });
    profile.resting_hr = Some(50);
    profile.anchor_provenance.resting_hr = CivilDate::from_ymd(2026, 11, 15)
        .ok()
        .map(AnchorStamp::promoted);
    profile.weight = Some(70.0);
    executor
        .resources
        .repos()
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant, user_id, &profile)
        .await?;

    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "swain_vo2_reserve", "power_watts": 200, "heart_rate": 148 }),
    )
    .await?;
    let used = &result["heart_rates_used"];
    assert_eq!(
        used["max_hr"]["source"],
        json!("provider_import"),
        "bug: an imported maximum heart rate must be named as an import: {result}"
    );
    assert_eq!(
        used["max_hr"]["origin"],
        json!("intervals.icu"),
        "bug: an imported maximum heart rate must name its provider: {result}"
    );
    assert_eq!(
        used["resting_hr"]["source"],
        json!("device_readings"),
        "bug: a promoted resting heart rate must be named as the device's: {result}"
    );
    assert_eq!(
        used["resting_hr"]["as_of"],
        json!("2026-11-15"),
        "bug: a promoted resting heart rate must carry its day: {result}"
    );
    assert!(
        (vo2(&result) - SWAIN_VO2MAX).abs() < 0.06,
        "the stored heart rates and weight give the same estimate: {result}"
    );
    Ok(())
}

#[tokio::test]
async fn a_swain_stage_outside_its_window_is_refused_with_the_athletes_range() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // 170 bpm is 86 % of a 50-190 reserve; the method was validated at
    // 65-75 %, which is 141-155 bpm for this athlete.
    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "swain_vo2_reserve",
            "power_watts": 200,
            "heart_rate": 170,
            "max_hr": 190,
            "resting_hr": 50,
            "weight_kg": 70,
        }),
    )
    .await?;
    assert!(
        message.contains("141-155 bpm"),
        "the refusal names the athlete's own window: {message}"
    );
    Ok(())
}

#[tokio::test]
async fn a_swain_estimate_outside_the_human_range_is_shown_unclamped_with_a_warning() -> Result<()>
{
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // 30 W at 70 % of reserve for 100 kg: 10.8 × 42.9 / 100 + 7 = 11.6, below
    // anything a person measures — almost certainly a wrong input.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "swain_vo2_reserve",
            "power_watts": 30,
            "heart_rate": 148,
            "max_hr": 190,
            "resting_hr": 50,
            "weight_kg": 100,
        }),
    )
    .await?;
    let expected = 10.8 * (30.0 / 0.7) / 100.0 + 7.0;
    assert!(
        (vo2(&result) - expected).abs() < 0.06,
        "never clamped to 20: {result}"
    );
    let warnings = warnings(&result);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("watts") && w.contains("body weight") && w.contains("heart rates")),
        "the warning names the inputs to recheck: {warnings:?}"
    );
    Ok(())
}

#[tokio::test]
async fn swain_with_no_maximum_heart_rate_asks_for_a_measured_one() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "swain_vo2_reserve",
            "power_watts": 200,
            "heart_rate": 148,
            "resting_hr": 50,
            "weight_kg": 70,
        }),
    )
    .await?;
    assert!(
        message.contains("max_hr"),
        "bug: Swain with no maximum heart rate must ask for max_hr: {message}"
    );
    Ok(())
}

// ============================================================================
// Profile defaults
// ============================================================================

#[tokio::test]
async fn rockport_takes_weight_and_age_from_the_profile_and_says_so() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = executor
        .execute_tool(make_request(
            "set_physiology",
            json!({ "weight": 70.0, "age": 30 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(
        saved.success,
        "seeding the profile failed: {:?}",
        saved.error
    );

    // Kline et al. (1987), with weight in pounds and time in minutes:
    // 132.853 − 0.0769·lb − 0.3877·age + 6.315·gender − 3.2649·min − 0.1565·HR
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "male",
            "time_seconds": 780,
            "heart_rate": 140,
        }),
    )
    .await?;

    // Summed term by term so the test reads like the equation; clippy's
    // mul_add preference does not apply to a plain sum.
    let expected: f64 = [
        132.853,
        -0.0769 * 70.0 / KG_PER_LB,
        -0.3877 * 30.0,
        6.315,
        -3.2649 * 13.0,
        -0.1565 * 140.0,
    ]
    .iter()
    .sum();
    assert!(
        (vo2(&result) - expected).abs() < 0.06,
        "Rockport should be {expected:.2}, got {}",
        vo2(&result)
    );
    assert_eq!(
        result["defaults_from_profile"],
        json!(["weight_kg", "age"]),
        "the response must name every input that came from the profile"
    );
    Ok(())
}

#[tokio::test]
async fn a_restated_weight_overrides_the_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = executor
        .execute_tool(make_request(
            "set_physiology",
            json!({ "weight": 90.0, "age": 30 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(saved.success);

    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "female",
            "weight_kg": 60.0,
            "time_seconds": 900,
            "heart_rate": 150,
        }),
    )
    .await?;

    // Weight from the call (60 kg, in pounds), age from the profile (30),
    // gender 0.
    let expected: f64 = [
        132.853,
        -0.0769 * 60.0 / KG_PER_LB,
        -0.3877 * 30.0,
        -3.2649 * 15.0,
        -0.1565 * 150.0,
    ]
    .iter()
    .sum();
    assert!(
        (vo2(&result) - expected).abs() < 0.06,
        "got {}",
        vo2(&result)
    );
    assert_eq!(result["defaults_from_profile"], json!(["age"]));
    Ok(())
}

// ============================================================================
// Rejections say what to do
// ============================================================================

#[tokio::test]
async fn rockport_with_no_weight_anywhere_points_at_set_physiology() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "male",
            "age": 30,
            "time_seconds": 780,
            "heart_rate": 140,
        }),
    )
    .await?;
    assert!(
        message.contains("weight_kg"),
        "names the missing field: {message}"
    );
    assert!(
        message.contains("set_physiology"),
        "tells the coach how to fix it: {message}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_method_lists_the_real_ones() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "treadmill", "distance_meters": 2800 }),
    )
    .await?;
    assert!(message.contains("cooper_test"), "{message}");
    assert!(message.contains("rockport_walk"), "{message}");
    Ok(())
}

/// No `VO2max` comes from a pace, and Åstrand–Ryhming is replaced by Swain
/// (carnet#1693 items 1 and 2): both retired methods are refused, and the
/// refusal no longer offers them.
#[tokio::test]
async fn the_retired_pace_and_astrand_methods_are_refused() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    for params in [
        json!({ "method": "from_pace", "max_speed_ms": 5.0, "recovery_speed_ms": 3.0 }),
        json!({ "method": "astrand_ryhming", "gender": "male", "heart_rate": 150,
                "power_watts": 200, "weight_kg": 75 }),
    ] {
        let message =
            estimate_expecting_rejection(&executor, user_id, &tenant_id, params.clone()).await?;
        let offered = message
            .split_once("expected one of")
            .map_or("", |(_, methods)| methods);
        assert!(
            message.contains("unknown method") && offered.contains("swain_vo2_reserve"),
            "bug: {params} must be refused as unknown, listing the methods that exist: {message}"
        );
        assert!(
            !offered.contains("from_pace") && !offered.contains("astrand_ryhming"),
            "bug: the refusal must not offer a retired method: {message}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn gender_outside_the_fitted_equation_is_rejected() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "other",
            "weight_kg": 70,
            "age": 30,
            "time_seconds": 780,
            "heart_rate": 140,
        }),
    )
    .await?;
    assert!(message.contains("female or male"), "{message}");
    Ok(())
}

#[tokio::test]
async fn an_out_of_range_input_is_the_athletes_to_correct() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // A stated VDOT is bounded to the 20–90 race times are predicted for.
    for vdot in [19.9, 90.1, 200.0] {
        let message = estimate_expecting_rejection(
            &executor,
            user_id,
            &tenant_id,
            json!({ "method": "from_vdot", "vdot": vdot }),
        )
        .await?;
        assert!(
            message.contains("20-90"),
            "VDOT {vdot} surfaces as invalid input naming the range: {message}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_slow_5k_is_a_vdot_below_daniels_tables_with_a_warning() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // 5 km in 35:00 reads VDOT 25.6: below the 30-85 Daniels' tables print,
    // inside the 20-90 race times are predicted for. It is reported, with
    // its threshold pace, and the warning says it is less certain — not that
    // it cannot be saved.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "race_result", "distance_meters": 5000, "time_seconds": 2100 }),
    )
    .await?;
    assert!(
        (vdot(&result) - 25.6).abs() < 0.05,
        "a 35:00 5 km reads VDOT 25.6: {result}"
    );
    assert!(
        result["implied_threshold_pace_sec_per_km"].is_number(),
        "a VDOT below the tables still implies a threshold pace: {result}"
    );
    let race_warnings = warnings(&result);
    assert!(
        race_warnings.iter().any(|w| w.contains("less certain")),
        "the warning says the VDOT is less certain: {race_warnings:?}"
    );
    assert!(
        !race_warnings.iter().any(|w| w.contains("does not save")),
        "bug: a VDOT of 25.6 is saveable and predictable: {race_warnings:?}"
    );

    // Stated, the same VDOT is accepted with the same warning.
    let stated = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "from_vdot", "vdot": 25.6 }),
    )
    .await?;
    assert!(
        warnings(&stated).iter().any(|w| w.contains("less certain")),
        "a stated VDOT below the tables carries the warning: {stated}"
    );
    Ok(())
}

// ============================================================================
// Reachability
// ============================================================================

#[test]
fn estimate_vo2max_is_reachable_from_a_chat_turn() {
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let names: Vec<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(
        names.iter().any(|n| n == "estimate_vo2max"),
        "estimate_vo2max must sit in a chat-callable category or the coach can never see it; got {} tools",
        names.len()
    );
}

// ============================================================================
// The profile access is scope-gated (carnet#363)
// ============================================================================

#[test]
fn the_tool_declares_the_profile_access_it_performs() {
    // reads the stored profile for weight and age defaults and echoes `stored_vo2_max`,
    // so the call touches identity data. Declaring only the runtime
    // requirements yields an empty scope list, which the
    // read-only default grant an RFC 7591 registration receives satisfies —
    // and that grant is exactly what the profile split exists to hold back.
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let (_, _, caps, _) = registry
        .all_tool_metadata()
        .into_iter()
        .find(|(name, _, _, _)| name == "estimate_vo2max")
        .expect("the tool is registered");

    assert!(
        required_scopes(caps).contains(&OAuthScope::ProfileRead),
        "touching the stored profile requires profile:read, not fitness:read"
    );
    assert_eq!(
        missing_scope(&[OAuthScope::FitnessRead], caps),
        Some(OAuthScope::ProfileRead),
        "a fitness-read grant must be refused and told what it needed"
    );
    assert_eq!(
        missing_scope(&OAuthScope::self_grant(), caps),
        None,
        "the athlete's own grant covers it"
    );
}

// ============================================================================
// A race the athlete ran is a field test, and it seeds their threshold
// ============================================================================

#[tokio::test]
async fn a_race_result_gives_a_vdot_and_the_pace_it_implies() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // A 5 km in 19:30 — the personal best an athlete states in the season
    // walk, in the units they state it in.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "race_result", "distance_meters": 5000, "time_seconds": 1170 }),
    )
    .await?;

    assert_eq!(result["method"], json!("race_result"));
    // Daniels puts a sub-20 5 km in the low fifties; the assertion is a band
    // rather than a point because the curve is fitted, not exact. It is a
    // VDOT, never reported as a VO2max.
    let estimated = vdot(&result);
    assert!(
        (45.0..=60.0).contains(&estimated),
        "a 19:30 5 km should read in the high forties to high fifties, got {estimated}"
    );
    assert_eq!(
        result["vo2max_ml_kg_min"],
        Value::Null,
        "bug: a race gives a VDOT, never a VO2max: {result}"
    );
    assert!(
        warnings(&result).is_empty(),
        "a 5 km sits inside Daniels' tables: {result}"
    );
    assert_eq!(
        result["saved"],
        json!(false),
        "the tool estimates, it does not write"
    );

    // The threshold pace the estimate implies is the number set_physiology
    // stores, so the athlete can confirm both in one exchange.
    let pace = result["implied_threshold_pace_sec_per_km"]
        .as_f64()
        .expect("a race result implies a threshold pace");
    assert!(
        (180.0..=300.0).contains(&pace),
        "threshold for a sub-20 5 km runner sits between 3:00 and 5:00 per km, got {pace}"
    );
    // Threshold is slower than the 5 km race pace it was derived from (234
    // s/km), which is what makes it a threshold rather than a race effort.
    assert!(
        pace > 1170.0 / 5.0,
        "threshold pace must be slower than the race pace it came from, got {pace}"
    );
    assert!(
        result["to_store"]
            .as_str()
            .unwrap_or("")
            .contains("threshold_pace_sec_per_km"),
        "the tool must say the pace is storable: {}",
        result["to_store"]
    );
    Ok(())
}

#[tokio::test]
async fn a_race_outside_daniels_tables_is_read_with_a_warning() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // 1 km in 3:30: cageux reads it, but Daniels' tables start at 1500 m, so
    // the VDOT rests on his curves beyond them.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "race_result", "distance_meters": 1000, "time_seconds": 210 }),
    )
    .await?;
    assert!(
        result["vdot"].is_number(),
        "bug: a race of any distance must give a VDOT: {result}"
    );
    let warnings = warnings(&result);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("1500 m to the marathon")),
        "a 1 km effort carries the distance warning: {warnings:?}"
    );
    Ok(())
}

#[tokio::test]
async fn every_vdot_method_offers_the_threshold_pace_its_vdot_implies() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // The pace rides on a VDOT: a Cooper test and a stated VDOT both carry
    // it, so an athlete who confirms one never has to run a second test to
    // get a threshold.
    for params in [
        json!({ "method": "cooper_test", "distance_meters": 2800 }),
        json!({ "method": "from_vdot", "vdot": 50 }),
    ] {
        let result = estimate(&executor, user_id, &tenant_id, params.clone()).await?;
        let pace = result["implied_threshold_pace_sec_per_km"]
            .as_f64()
            .unwrap_or_else(|| panic!("{params} must imply a threshold pace: {result}"));
        assert!(
            (150.0..=600.0).contains(&pace),
            "{params} implied an unusable pace: {pace}"
        );
    }

    // With no lactate threshold saved, threshold pace is Daniels' T pace: the
    // speed whose oxygen cost is 88 % of VDOT, 255.2 s/km (4:15/km) at VDOT
    // 50 as his tables print — the pace cageux cuts the threshold zone
    // around, so the two agree.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "from_vdot", "vdot": 50 }),
    )
    .await?;
    assert_eq!(
        result["implied_threshold_pace_sec_per_km"],
        json!(255.2),
        "VDOT 50 threshold pace: {result}"
    );

    // A walk's VO2max says nothing about running economy: no pace.
    let walk = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "male",
            "weight_kg": 70,
            "age": 30,
            "time_seconds": 780,
            "heart_rate": 140,
        }),
    )
    .await?;
    assert_eq!(
        walk["implied_threshold_pace_sec_per_km"],
        Value::Null,
        "a VO2max implies no threshold pace: {walk}"
    );
    Ok(())
}

#[tokio::test]
async fn a_race_nobody_could_run_is_refused_rather_than_estimated() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // 5 km in 30 seconds is not a race result; the kernel refuses the VDOT
    // and the tool says so instead of inventing a number.
    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "race_result", "distance_meters": 5000, "time_seconds": 30 }),
    )
    .await?;
    assert!(
        message.contains("race_result") || message.contains("VDOT") || message.contains("vdot"),
        "the refusal must name what it could not do: {message}"
    );
    Ok(())
}
