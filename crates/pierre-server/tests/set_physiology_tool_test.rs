// ABOUTME: Integration tests for the set_physiology MCP tool — the write path for user_physiological_profiles
// ABOUTME: Content-asserting: real values through the executor, read-modify-write, derived zones, TSS effect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::{Duration, Utc};
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::{Activity, SportType, TenantId};
use pierre_fitness_compute::training_history_compute::{compute_training_history, AthleteInputs};
use pierre_intelligence::AlgorithmConfig;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::registry::ToolRegistry;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(UniversalToolExecutor::new(resources)))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("physio_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let user_tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, user_tenant.id.to_string()))
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

/// Run `set_physiology` and require it to succeed, returning its payload.
async fn set_physiology(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<Value> {
    let response = executor
        .execute_tool(make_request("set_physiology", params, user_id, tenant_id))
        .await?;
    assert!(
        response.success,
        "set_physiology should have succeeded: {:?}",
        response.error
    );
    Ok(response.result.expect("set_physiology returns a payload"))
}

/// Run `set_physiology` and require it to be rejected, returning the message.
///
/// An invalid-input error surfaces as `Err` from the executor rather than an
/// unsuccessful response, so both shapes count as a rejection and both carry
/// the message the athlete would be shown.
async fn set_physiology_expecting_rejection(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<String> {
    match executor
        .execute_tool(make_request("set_physiology", params, user_id, tenant_id))
        .await
    {
        Err(e) => Ok(e.to_string()),
        Ok(response) => {
            assert!(
                !response.success,
                "set_physiology should have rejected these values, got: {:?}",
                response.result
            );
            Ok(response.error.unwrap_or_default())
        }
    }
}

/// Read the stored row straight from the repository, bypassing the tool, so an
/// assertion about persistence cannot be satisfied by the tool echoing its own
/// arguments back.
async fn stored_ftp(
    executor: &UniversalToolExecutor,
    tenant_id: &str,
    user_id: Uuid,
) -> Result<Option<u32>> {
    let tenant = TenantId::from_uuid(Uuid::parse_str(tenant_id)?);
    Ok(executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .and_then(|p| p.ftp_watts))
}

// ============================================================================
// The write lands
// ============================================================================

#[tokio::test]
async fn saving_ftp_persists_it_to_the_table_every_reader_uses() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result =
        set_physiology(&executor, user_id, &tenant_id, json!({ "ftp_watts": 285 })).await?;

    assert_eq!(result["saved"], json!(true));
    assert_eq!(result["created"], json!(true));
    assert_eq!(result["profile"]["ftp_watts"], json!(285));

    // The row itself, not the tool's echo.
    assert_eq!(
        stored_ftp(&executor, &tenant_id, user_id).await?,
        Some(285),
        "the FTP must be readable from user_physiological_profiles"
    );
    Ok(())
}

#[tokio::test]
async fn an_llm_float_for_an_integer_field_is_accepted() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // Models routinely emit `285.0` where the schema says integer; rejecting
    // it would drop the measurement the tool exists to capture.
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "ftp_watts": 285.0 }),
    )
    .await?;

    assert_eq!(stored_ftp(&executor, &tenant_id, user_id).await?, Some(285));
    Ok(())
}

// ============================================================================
// An unset column stays unset
// ============================================================================

#[tokio::test]
async fn columns_the_athlete_never_supplied_read_back_as_unknown() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(&executor, user_id, &tenant_id, json!({ "ftp_watts": 285 })).await?;

    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let profile = executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .expect("profile exists after the save");

    // SQLite answers a NULL numeric column with 0 rather than an error, so a
    // reader that decodes a bare `f64` turns "never measured" into a measured
    // zero. A 0 kg weight or a 0 ml/kg/min VO2 max reaching the TSS engine is
    // worse than an absent one, because it looks like data.
    assert_eq!(profile.ftp_watts, Some(285));
    assert_eq!(
        profile.vo2_max, None,
        "an unmeasured VO2 max must not read as 0"
    );
    assert_eq!(
        profile.weight, None,
        "an unmeasured weight must not read as 0"
    );
    assert_eq!(profile.max_hr, None);
    assert_eq!(profile.resting_hr, None);
    assert_eq!(profile.age, None);
    assert_eq!(profile.lactate_threshold_percentage, None);
    assert_eq!(profile.threshold_pace_sec_per_km, None);
    assert_eq!(profile.training_experience_years, None);
    Ok(())
}

// ============================================================================
// Read-modify-write — the regression the naive upsert would fail
// ============================================================================

#[tokio::test]
async fn a_second_save_keeps_the_fields_the_first_one_set() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "ftp_watts": 285, "weight": 72.5 }),
    )
    .await?;

    // A save that mentions only max_hr must not null out FTP, weight, or the
    // power zones the first call derived. The underlying upsert writes every
    // column from EXCLUDED.*, so without the read-modify-write in the tool
    // this call would wipe them.
    let second = set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "max_hr": 190, "resting_hr": 48 }),
    )
    .await?;

    assert_eq!(second["created"], json!(false));
    assert_eq!(
        second["profile"]["ftp_watts"],
        json!(285),
        "the earlier FTP must survive a later single-field save"
    );
    assert_eq!(second["profile"]["weight"], json!(72.5));
    assert_eq!(second["profile"]["max_hr"], json!(190));
    assert!(
        !second["profile"]["power_zones"].is_null(),
        "power zones derived by the first save must survive the second"
    );
    assert_eq!(stored_ftp(&executor, &tenant_id, user_id).await?, Some(285));
    Ok(())
}

#[tokio::test]
async fn a_call_that_sets_nothing_is_rejected() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let error =
        set_physiology_expecting_rejection(&executor, user_id, &tenant_id, json!({})).await?;
    assert!(
        error.contains("at least one measurement"),
        "the error should say what to supply, got: {error}"
    );
    Ok(())
}

// ============================================================================
// Validation
// ============================================================================

#[tokio::test]
async fn out_of_range_ftp_is_rejected_and_leaves_the_stored_row_untouched() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(&executor, user_id, &tenant_id, json!({ "ftp_watts": 285 })).await?;

    for absurd in [20, 2000] {
        let error = set_physiology_expecting_rejection(
            &executor,
            user_id,
            &tenant_id,
            json!({ "ftp_watts": absurd }),
        )
        .await?;
        assert!(
            error.contains("ftp must be between"),
            "expected the shared FTP range message, got: {error}"
        );
    }

    assert_eq!(
        stored_ftp(&executor, &tenant_id, user_id).await?,
        Some(285),
        "a rejected save must not disturb the stored profile"
    );
    Ok(())
}

#[tokio::test]
async fn a_resting_rate_at_or_above_the_maximum_is_rejected() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // Both values sit inside their own ranges, so only the relationship check
    // can catch this pair.
    let error = set_physiology_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "resting_hr": 100, "max_hr": 100 }),
    )
    .await?;
    assert!(
        error.contains("resting_hr") && error.contains("must be less than max_hr"),
        "expected the resting/max relationship message, got: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn an_inverted_pair_outside_the_ranges_is_rejected_on_the_range_first() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // A 55 bpm maximum is below the 100 bpm floor, so the range check answers
    // before the relationship check does. The more specific message is the
    // better one to show the athlete, and the pair is still refused.
    let error = set_physiology_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "resting_hr": 60, "max_hr": 55 }),
    )
    .await?;
    assert!(
        error.contains("max_hr must be between"),
        "expected the max_hr range message, got: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn a_contradiction_spread_across_two_calls_is_still_rejected() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(&executor, user_id, &tenant_id, json!({ "max_hr": 190 })).await?;

    // Validating the merged row rather than this call's arguments is what
    // catches a resting rate that only conflicts with what is already stored.
    let error = set_physiology_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "resting_hr": 195 }),
    )
    .await?;
    assert!(
        error.contains("resting_hr"),
        "expected a resting-rate error, got: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_fitness_level_is_rejected_with_the_allowed_values() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let error = set_physiology_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "fitness_level": "superhuman" }),
    )
    .await?;
    assert!(
        error.contains("beginner") && error.contains("professional"),
        "the error should list the accepted levels, got: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn a_fitness_level_is_accepted_in_the_casing_a_model_writes() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "fitness_level": "advanced" }),
    )
    .await?;
    assert_eq!(result["profile"]["fitness_level"], json!("Advanced"));
    Ok(())
}

// ============================================================================
// Derived zones are persisted, with real boundaries
// ============================================================================

#[tokio::test]
async fn saving_ftp_stores_power_zones_with_real_watt_boundaries() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(&executor, user_id, &tenant_id, json!({ "ftp_watts": 250 })).await?;

    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let profile = executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .expect("profile exists after the save");

    let zones = profile
        .power_zones
        .expect("saving FTP must populate power_zones");

    // Real watts, not placeholders: strictly increasing, and the threshold
    // zone bracketing the FTP that produced them.
    assert!(
        zones.z1_max < zones.z2_max
            && zones.z2_max < zones.z3_max
            && zones.z3_max < zones.z4_max
            && zones.z4_max < zones.z5_max,
        "power zones must be strictly increasing, got {zones:?}"
    );
    assert!(
        zones.z1_max > 0,
        "zone 1 must have a real upper bound, got {}",
        zones.z1_max
    );
    assert!(
        zones.z3_max < 250 && zones.z4_max >= 250,
        "the threshold zone should bracket the 250 W FTP, got z3={} z4={}",
        zones.z3_max,
        zones.z4_max
    );
    Ok(())
}

#[tokio::test]
async fn saving_both_heart_rates_stores_hr_zones_between_them() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "resting_hr": 48, "max_hr": 192 }),
    )
    .await?;

    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let profile = executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .expect("profile exists after the save");

    let zones = profile
        .hr_zones
        .expect("saving both heart rates must populate hr_zones");

    assert!(
        zones.z1_max < zones.z2_max
            && zones.z2_max < zones.z3_max
            && zones.z3_max < zones.z4_max
            && zones.z4_max < zones.z5_max,
        "HR zones must be strictly increasing, got {zones:?}"
    );
    assert!(
        zones.z1_max > 48,
        "zone 1 must sit above the resting rate, got {}",
        zones.z1_max
    );
    assert_eq!(
        zones.z5_max, 192,
        "zone 5 tops out at the maximum heart rate"
    );
    Ok(())
}

#[tokio::test]
async fn a_single_heart_rate_alone_does_not_fabricate_zones() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = set_physiology(&executor, user_id, &tenant_id, json!({ "max_hr": 192 })).await?;
    assert!(
        result["profile"]["hr_zones"].is_null(),
        "HR zones need both bounds; deriving them from one is guesswork"
    );
    Ok(())
}

// ============================================================================
// The written profile actually reaches the training-load engine
// ============================================================================

fn power_run(id: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("session {id}"),
        SportType::Ride,
        Utc::now() - Duration::days(days_ago),
        3600,
        "synthetic".to_owned(),
    )
    .distance_meters(30_000.0)
    .average_heart_rate(150)
    .average_power(210)
    .normalized_power(225)
    .build()
}

#[tokio::test]
async fn a_saved_ftp_changes_the_training_load_the_engine_computes() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(&executor, user_id, &tenant_id, json!({ "ftp_watts": 250 })).await?;

    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let profile = executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .expect("profile exists after the save");

    // Built exactly as `training_history_compute` builds it in production, so
    // this asserts the loop the write path closes rather than a parallel one.
    let inputs = AthleteInputs {
        ftp_watts: profile.ftp_watts.map(f64::from),
        lthr: profile
            .lactate_threshold_percentage
            .and_then(|pct| profile.max_hr.map(|mhr| f64::from(mhr) * pct)),
        max_hr: profile.max_hr.map(f64::from),
        resting_hr: profile.resting_hr.map(f64::from),
        weight_kg: profile.weight,
    };
    assert_eq!(
        inputs.ftp_watts,
        Some(250.0),
        "the engine's inputs must carry the saved FTP"
    );

    let activities: Vec<Activity> = (1..=14).map(|d| power_run(&format!("a{d}"), d)).collect();
    let to = Utc::now().date_naive();
    let from = to - Duration::days(13);
    let config = AlgorithmConfig::default();

    let with_physiology = compute_training_history(&activities, inputs, from, to, &config);
    let without =
        compute_training_history(&activities, AthleteInputs::default(), from, to, &config);

    let ctl_with = with_physiology.last().expect("a final day").ctl;
    let ctl_without = without.last().expect("a final day").ctl;

    assert!(
        ctl_with > 0.0,
        "the physiology-aware run must produce real load, got {ctl_with}"
    );
    assert!(
        (ctl_with - ctl_without).abs() > 1.0,
        "a saved FTP must change the computed load; got {ctl_with} with physiology vs {ctl_without} without — \
         if these are equal the write path is not reaching the engine"
    );
    Ok(())
}

// ============================================================================
// The coach can actually call it
// ============================================================================

#[test]
fn set_physiology_is_reachable_from_a_chat_turn() {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);

    let chat_callable: Vec<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|t| t.name)
        .collect();

    assert!(
        chat_callable.iter().any(|n| n == "set_physiology"),
        "set_physiology must be chat-callable — an athlete states their FTP mid-conversation, \
         and a write tool the coach cannot see is the same failure that hid `groups`. Got: {chat_callable:?}"
    );

    // The sibling config writers stay off the natural-language surface; this
    // pins the distinction the `physiology` category exists to draw.
    for operator_tool in ["update_user_configuration", "set_fitness_config"] {
        assert!(
            !chat_callable.iter().any(|n| n == operator_tool),
            "{operator_tool} must stay off the chat surface"
        );
    }
}

// ============================================================================
// calculate_personalized_zones no longer invents the numbers it lacks
// ============================================================================

#[tokio::test]
async fn zone_calculation_reports_what_it_cannot_derive_instead_of_estimating() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let response = executor
        .execute_tool(make_request(
            "calculate_personalized_zones",
            json!({}),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(
        response.success,
        "the tool should answer honestly, not fail: {:?}",
        response.error
    );
    let result = response.result.expect("a payload");

    assert!(
        result["personalized_zones"]["power_zones"].is_null(),
        "with no FTP anywhere, power zones must be omitted rather than derived from a house number"
    );
    assert!(
        result["unavailable"]["power_zones"]
            .as_str()
            .unwrap_or_default()
            .contains("ftp"),
        "the caller should be told which input is missing, got {:?}",
        result["unavailable"]
    );
    assert!(
        result["personalized_zones"]["heart_rate_zones"].is_null(),
        "with no heart rates anywhere, HR zones must be omitted"
    );
    Ok(())
}

#[tokio::test]
async fn zone_calculation_uses_the_saved_profile_once_physiology_exists() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "ftp_watts": 250, "resting_hr": 48, "max_hr": 192, "vo2_max": 55.0 }),
    )
    .await?;

    let response = executor
        .execute_tool(make_request(
            "calculate_personalized_zones",
            json!({}),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(response.success, "failed: {:?}", response.error);
    let result = response.result.expect("a payload");

    assert_eq!(
        result["input_sources"]["ftp"],
        json!("profile"),
        "the saved FTP should be the source, not an estimate"
    );
    assert_eq!(result["personalized_zones"]["ftp"], json!(250));
    assert!(
        !result["personalized_zones"]["power_zones"].is_null(),
        "power zones must be derived from the saved FTP"
    );
    assert!(
        !result["personalized_zones"]["heart_rate_zones"].is_null(),
        "HR zones must be derived from the saved heart rates"
    );
    assert!(
        result["unavailable"]
            .as_object()
            .is_none_or(serde_json::Map::is_empty),
        "nothing should be unavailable once the profile is complete, got {:?}",
        result["unavailable"]
    );
    Ok(())
}

#[tokio::test]
async fn an_explicit_argument_still_overrides_the_saved_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(&executor, user_id, &tenant_id, json!({ "ftp_watts": 250 })).await?;

    let response = executor
        .execute_tool(make_request(
            "calculate_personalized_zones",
            json!({ "ftp": 300 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(response.success, "failed: {:?}", response.error);
    let result = response.result.expect("a payload");

    assert_eq!(result["input_sources"]["ftp"], json!("provided"));
    assert_eq!(result["personalized_zones"]["ftp"], json!(300));
    Ok(())
}
