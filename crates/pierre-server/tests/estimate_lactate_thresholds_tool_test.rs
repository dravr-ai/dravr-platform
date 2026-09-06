// ABOUTME: Exercises estimate_lactate_thresholds end to end through the tool executor — each construct against numbers computed in the test
// ABOUTME: Pins the four-method reply shape, the band table, the power zones, the stored-profile echo, every rejection, and chat reachability

// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::scopes::{missing_scope, required_scopes};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

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
    let email = format!("lactate_test_{}@example.com", Uuid::new_v4());
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
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

async fn analyze(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<Value> {
    let response = executor
        .execute_tool(make_request(
            "estimate_lactate_thresholds",
            params,
            user_id,
            tenant_id,
        ))
        .await?;
    assert!(
        response.success,
        "estimate_lactate_thresholds should have succeeded: {:?}",
        response.error
    );
    Ok(response
        .result
        .expect("estimate_lactate_thresholds returns a payload"))
}

/// The text of the rejection, from whichever channel it arrives on.
async fn analyze_expecting_rejection(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<String> {
    match executor
        .execute_tool(make_request(
            "estimate_lactate_thresholds",
            params,
            user_id,
            tenant_id,
        ))
        .await
    {
        Err(e) => Ok(e.to_string()),
        Ok(response) => {
            assert!(
                !response.success,
                "estimate_lactate_thresholds should have rejected these values, got: {:?}",
                response.result
            );
            Ok(format!("{:?}", response.error))
        }
    }
}

/// Six cycling stages whose lactate crosses 4.0 mmol/L a quarter of the way
/// from 250 W (3.6) to 275 W (5.2), heart rate on every stage.
fn cycling_stages() -> Value {
    json!([
        { "intensity": 150, "lactate_mmol": 1.0, "heart_rate": 120 },
        { "intensity": 175, "lactate_mmol": 1.2, "heart_rate": 130 },
        { "intensity": 200, "lactate_mmol": 1.6, "heart_rate": 140 },
        { "intensity": 225, "lactate_mmol": 2.4, "heart_rate": 150 },
        { "intensity": 250, "lactate_mmol": 3.6, "heart_rate": 160 },
        { "intensity": 275, "lactate_mmol": 5.2, "heart_rate": 170 },
    ])
}

fn lt2(result: &Value, method: &str) -> Value {
    result["lt2"]
        .as_array()
        .expect("lt2 is a list of constructs")
        .iter()
        .find(|entry| entry["method"] == json!(method))
        .cloned()
        .unwrap_or_else(|| panic!("lt2 carries {method}: {}", result["lt2"]))
}

fn approx(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected} ± {tolerance}, got {actual}"
    );
}

// ============================================================================
// The reply keeps the four constructs apart and computes each from the stages
// ============================================================================

#[tokio::test]
async fn obla_is_the_exact_interpolated_crossing_of_four_mmol() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "watts", "stages": cycling_stages() }),
    )
    .await?;

    assert_eq!(result["unit"], json!("watts"));
    assert_eq!(result["stage_count"], json!(6));
    let obla = lt2(&result, "obla_4mmol");
    assert_eq!(obla["outcome"], json!("determined"));
    assert_eq!(obla["marks"], json!("LT2"));
    // 3.6 → 5.2 crosses 4.0 at 0.25 of 250 → 275 W; heart rate 160 → 170 at 0.25.
    approx(obla["intensity"].as_f64().unwrap(), 256.3, 1e-9);
    approx(obla["lactate_mmol"].as_f64().unwrap(), 4.0, 1e-9);
    approx(obla["heart_rate"].as_f64().unwrap(), 163.0, 1e-9);
    assert!(
        obla["reference"].as_str().unwrap().contains("Heck"),
        "the convention names its paper: {}",
        obla["reference"]
    );
    assert_eq!(
        result["saved"],
        json!(false),
        "the tool never writes the profile"
    );
    assert!(
        result["to_store"]
            .as_str()
            .unwrap()
            .contains("set_physiology"),
        "{}",
        result["to_store"]
    );
    assert!(result["to_store"].as_str().unwrap().contains("ftp_watts"));
    Ok(())
}

#[tokio::test]
async fn every_construct_is_present_and_named() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "watts", "stages": cycling_stages() }),
    )
    .await?;

    assert_eq!(result["lt1"]["method"], json!("log_log"));
    assert_eq!(result["lt1"]["marks"], json!("LT1"));
    assert!(result["lt1"]["reference"]
        .as_str()
        .unwrap()
        .contains("Beaver"));
    // A textbook curve resolves LT1, and it lands where the literature puts
    // it: 191 W at 1.33 mmol/L, inside the 1.0-2.0 mmol/L LT1 band
    // (Seiler-Viken 2025) and between the stages that bracket the rise.
    assert_eq!(
        result["lt1"]["outcome"],
        json!("determined"),
        "{}",
        result["lt1"]
    );
    let lt1_watts = result["lt1"]["intensity"].as_f64().unwrap();
    let lt1_mmol = result["lt1"]["lactate_mmol"].as_f64().unwrap();
    approx(lt1_watts, 191.0, 0.5);
    approx(lt1_mmol, 1.33, 0.02);
    assert!(
        (1.0..=2.0).contains(&lt1_mmol),
        "LT1 sits in the 1.0-2.0 mmol/L band: {lt1_mmol}"
    );
    let methods: Vec<&str> = result["lt2"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["method"].as_str().unwrap())
        .collect();
    assert_eq!(methods, vec!["modified_dmax", "dmax", "obla_4mmol"]);
    for entry in result["lt2"].as_array().unwrap() {
        assert_eq!(entry["marks"], json!("LT2"));
        assert!(
            matches!(
                entry["outcome"].as_str(),
                Some("determined" | "not_determinable")
            ),
            "{entry}"
        );
    }
    // On an accelerating curve every LT2 construct resolves, and modified
    // Dmax sits at or right of Dmax.
    let modified = lt2(&result, "modified_dmax");
    let classic = lt2(&result, "dmax");
    assert_eq!(modified["outcome"], json!("determined"), "{modified}");
    assert_eq!(classic["outcome"], json!("determined"), "{classic}");
    assert!(
        modified["intensity"].as_f64().unwrap() >= classic["intensity"].as_f64().unwrap(),
        "modified Dmax {} vs Dmax {}",
        modified["intensity"],
        classic["intensity"]
    );
    // The physiological ordering every construct must respect: LT1 is the
    // first rise, LT2 the second, so LT1 sits below every LT2 on the same
    // test. A construct that mixed the two up would fail here.
    for entry in result["lt2"].as_array().unwrap() {
        let lt2_watts = entry["intensity"].as_f64().unwrap();
        assert!(
            lt1_watts < lt2_watts,
            "LT1 {lt1_watts} W must sit below LT2 by {}: {lt2_watts} W",
            entry["method"]
        );
    }
    assert!(
        result["framing"]
            .as_str()
            .unwrap()
            .contains("do not coincide"),
        "{}",
        result["framing"]
    );
    Ok(())
}

#[tokio::test]
async fn a_construct_the_stages_cannot_support_says_why() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // Lactate peaks at 2.8 mmol/L: the 4.0 convention has nothing to cross.
    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "unit": "watts",
            "stages": [
                { "intensity": 150, "lactate_mmol": 1.0 },
                { "intensity": 180, "lactate_mmol": 1.3 },
                { "intensity": 210, "lactate_mmol": 1.9 },
                { "intensity": 240, "lactate_mmol": 2.8 },
            ]
        }),
    )
    .await?;

    let obla = lt2(&result, "obla_4mmol");
    assert_eq!(obla["outcome"], json!("not_determinable"));
    let reason = obla["reason"].as_str().unwrap();
    assert!(reason.contains("2.8"), "names the peak: {reason}");
    assert!(
        obla.get("intensity").is_none(),
        "no number is invented: {obla}"
    );
    Ok(())
}

// ============================================================================
// Band table, power zones and the stored profile
// ============================================================================

#[tokio::test]
async fn band_table_rows_are_interpolated_from_the_stages() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "watts", "stages": cycling_stages() }),
    )
    .await?;

    let rows = result["band_table"].as_array().unwrap();
    // The first stage already sits at 1.0 mmol/L, so 1.5 … 4.0 are the rows.
    assert_eq!(rows.len(), 6, "{rows:?}");
    let two = rows
        .iter()
        .find(|r| r["lactate_mmol"] == json!(2.0))
        .expect("a 2.0 mmol row");
    // 1.6 → 2.4 crosses 2.0 halfway through 200 → 225 W; heart rate 140 → 150.
    approx(two["intensity"].as_f64().unwrap(), 212.5, 1e-9);
    approx(two["heart_rate"].as_f64().unwrap(), 145.0, 1e-9);
    let levels: Vec<f64> = rows
        .iter()
        .map(|r| r["lactate_mmol"].as_f64().unwrap())
        .collect();
    assert!(
        levels.windows(2).all(|w| w[0] < w[1]),
        "ascending: {levels:?}"
    );
    Ok(())
}

#[tokio::test]
async fn power_zones_anchor_on_modified_dmax_for_watts() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "watts", "stages": cycling_stages() }),
    )
    .await?;

    let zones = &result["power_zones"];
    assert_eq!(zones["anchor"], json!("lt2_modified_dmax"));
    assert_eq!(zones["available"], json!(true), "{zones}");
    let anchor_watts = lt2(&result, "modified_dmax")["intensity"]
        .as_f64()
        .unwrap()
        .round();
    approx(zones["ftp_watts"].as_f64().unwrap(), anchor_watts, 0.0);
    let bounds: Vec<u64> = (1..=5)
        .map(|z| {
            zones["zones"][format!("zone_{z}")]["max_watts"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert!(
        bounds.windows(2).all(|w| w[0] < w[1]),
        "zone ceilings increase: {bounds:?}"
    );
    assert!(
        bounds[3] >= zones["ftp_watts"].as_u64().unwrap(),
        "zone 4 reaches the threshold itself: {bounds:?}"
    );
    Ok(())
}

#[tokio::test]
async fn pace_stages_report_pace_and_no_power_zones() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "unit": "seconds_per_km",
            "stages": [
                { "intensity": 360, "lactate_mmol": 1.0 },
                { "intensity": 340, "lactate_mmol": 1.2 },
                { "intensity": 320, "lactate_mmol": 1.6 },
                { "intensity": 300, "lactate_mmol": 2.4 },
                { "intensity": 280, "lactate_mmol": 3.6 },
                { "intensity": 260, "lactate_mmol": 5.2 },
            ]
        }),
    )
    .await?;

    assert_eq!(result["unit"], json!("seconds_per_km"));
    let obla = lt2(&result, "obla_4mmol");
    let pace = obla["intensity"].as_f64().unwrap();
    // Interpolated on the speed axis a quarter of the way from 280 to 260 s/km.
    let expected_speed = (1000.0_f64 / 260.0 - 1000.0 / 280.0).mul_add(0.25, 1000.0 / 280.0);
    approx(pace, (1000.0 / expected_speed * 10.0).round() / 10.0, 1e-9);
    assert!(pace > 260.0 && pace < 280.0, "{pace}");
    assert_eq!(result["power_zones"]["available"], json!(false));
    assert!(
        result["power_zones"]["reason"]
            .as_str()
            .unwrap()
            .contains("watts"),
        "{}",
        result["power_zones"]
    );
    assert!(
        result["to_store"]
            .as_str()
            .unwrap()
            .contains("threshold_pace_sec_per_km"),
        "{}",
        result["to_store"]
    );
    Ok(())
}

#[tokio::test]
async fn the_stored_profile_is_echoed_beside_the_estimate() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = executor
        .execute_tool(make_request(
            "set_physiology",
            json!({ "ftp_watts": 240, "max_hr": 185 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(saved.success, "{:?}", saved.error);

    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "watts", "stages": cycling_stages() }),
    )
    .await?;

    assert_eq!(result["stored_profile"]["ftp_watts"], json!(240));
    assert_eq!(result["stored_profile"]["max_hr"], json!(185));
    assert_eq!(
        result["stored_profile"]["threshold_pace_sec_per_km"],
        Value::Null
    );
    // The estimate does not touch the stored value: re-saving one unchanged
    // field reads the row back, and the FTP is still the athlete's own 240.
    let after = executor
        .execute_tool(make_request(
            "set_physiology",
            json!({ "max_hr": 185 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(after.success, "{:?}", after.error);
    assert_eq!(after.result.unwrap()["profile"]["ftp_watts"], json!(240));
    Ok(())
}

// ============================================================================
// Rejections name what the athlete can fix
// ============================================================================

#[tokio::test]
async fn fewer_than_four_stages_are_rejected_with_the_counts() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = analyze_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "unit": "watts",
            "stages": [
                { "intensity": 150, "lactate_mmol": 1.0 },
                { "intensity": 200, "lactate_mmol": 1.6 },
                { "intensity": 250, "lactate_mmol": 3.6 },
            ]
        }),
    )
    .await?;
    assert!(
        message.contains("cannot analyze the lactate test"),
        "an analysis error surfaces as invalid input, not a server fault: {message}"
    );
    assert!(message.contains("at least 4"), "{message}");
    assert!(message.contains("got 3"), "{message}");
    Ok(())
}

#[tokio::test]
async fn a_stage_that_is_not_harder_is_rejected_by_position() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = analyze_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "unit": "watts",
            "stages": [
                { "intensity": 150, "lactate_mmol": 1.0 },
                { "intensity": 200, "lactate_mmol": 1.6 },
                { "intensity": 190, "lactate_mmol": 2.4 },
                { "intensity": 250, "lactate_mmol": 3.6 },
            ]
        }),
    )
    .await?;
    assert!(message.contains("stages[2].intensity"), "{message}");
    assert!(message.contains("more watts"), "{message}");
    Ok(())
}

#[tokio::test]
async fn a_stage_missing_its_lactate_names_the_stage() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = analyze_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "unit": "watts",
            "stages": [
                { "intensity": 150, "lactate_mmol": 1.0 },
                { "intensity": 200, "lactate_mmol": 1.6 },
                { "intensity": 250 },
                { "intensity": 300, "lactate_mmol": 5.6 },
            ]
        }),
    )
    .await?;
    assert!(message.contains("stages[2]"), "{message}");
    assert!(message.contains("lactate_mmol"), "{message}");
    Ok(())
}

#[tokio::test]
async fn more_stages_than_any_protocol_runs_are_refused() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // The LT1 search fits a regression pair at every split, so its cost is
    // quadratic in the stage count; the ceiling is what keeps one call from
    // spending minutes of CPU on a worker.
    let stages: Vec<Value> = (0..=51)
        .map(|i| json!({ "intensity": 100 + i, "lactate_mmol": f64::from(i).mul_add(0.1, 1.0) }))
        .collect();
    let message = analyze_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "watts", "stages": stages }),
    )
    .await?;
    assert!(message.contains("at most 50 stages"), "{message}");
    assert!(message.contains("52"), "names what was sent: {message}");

    // Exactly at the ceiling the test is still analysed.
    let stages: Vec<Value> = (0..50)
        .map(|i| json!({ "intensity": 100 + i, "lactate_mmol": f64::from(i).mul_add(0.1, 1.0) }))
        .collect();
    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "watts", "stages": stages }),
    )
    .await?;
    assert_eq!(result["stage_count"], json!(50));
    Ok(())
}

#[tokio::test]
async fn an_unknown_unit_lists_the_real_ones() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = analyze_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "km_per_hour", "stages": cycling_stages() }),
    )
    .await?;
    assert!(message.contains("watts"), "{message}");
    assert!(message.contains("seconds_per_km"), "{message}");
    Ok(())
}

#[tokio::test]
async fn missing_stages_are_rejected_by_name() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message =
        analyze_expecting_rejection(&executor, user_id, &tenant_id, json!({ "unit": "watts" }))
            .await?;
    assert!(message.contains("'stages' is required"), "{message}");
    Ok(())
}

#[tokio::test]
async fn a_coherent_test_carries_no_ordering_warning() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = analyze(
        &executor,
        user_id,
        &tenant_id,
        json!({ "unit": "watts", "stages": cycling_stages() }),
    )
    .await?;

    // LT1 at 191 W sits below every determined LT2 on this test, so there is
    // nothing to warn about and the field is explicitly null rather than
    // absent — a reader can tell "checked and fine" from "not checked".
    assert_eq!(
        result["ordering_warning"],
        Value::Null,
        "a textbook curve orders correctly: {}",
        result["ordering_warning"]
    );
    Ok(())
}

// ============================================================================
// The profile read is scope-gated
// ============================================================================

#[test]
fn the_tool_declares_the_profile_read_it_performs() {
    // The reply echoes the athlete's stored FTP, threshold pace and max HR, so
    // the call reads identity data. Declaring only the runtime requirements
    // would yield an empty scope list, and the read-only default grant
    // (fitness:read) would reach the profile — the split these scopes exist
    // to enforce.
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let (_, _, caps, _) = registry
        .all_tool_metadata()
        .into_iter()
        .find(|(name, _, _, _)| name == "estimate_lactate_thresholds")
        .expect("the tool is registered");

    assert_eq!(
        required_scopes(caps),
        vec![OAuthScope::ProfileRead],
        "reading the stored profile requires profile:read, not fitness:read"
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

#[tokio::test]
async fn a_grant_without_profile_read_is_refused_at_the_chokepoint() -> Result<()> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let executor = UniversalToolExecutor::new(resources).with_scopes(vec![OAuthScope::FitnessRead]);
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let error = executor
        .execute_tool(make_request(
            "estimate_lactate_thresholds",
            json!({ "unit": "watts", "stages": cycling_stages() }),
            user_id,
            &tenant_id,
        ))
        .await
        .expect_err("a grant without profile:read must not reach the profile");
    let message = error.to_string();
    assert!(
        message.contains("profile:read"),
        "the refusal names the grant that was needed: {message}"
    );
    Ok(())
}

// ============================================================================
// Reachability
// ============================================================================

#[test]
fn estimate_lactate_thresholds_is_reachable_from_a_chat_turn() {
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let names: Vec<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(
        names.iter().any(|n| n == "estimate_lactate_thresholds"),
        "estimate_lactate_thresholds must sit in a chat-callable category or the coach can never see it; got {} tools",
        names.len()
    );
}
