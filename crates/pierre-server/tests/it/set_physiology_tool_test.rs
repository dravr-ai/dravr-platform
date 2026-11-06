// ABOUTME: Integration tests for the set_physiology MCP tool — the write path for user_physiological_profiles
// ABOUTME: Content-asserting: real values through the executor, read-modify-write, derived zones, TSS effect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::{Duration, NaiveDate, Utc};
use chrono_tz::Australia::Brisbane;
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::zones::{HrZoneTier, HrZones, PowerZones, Vo2maxSource};
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::zones::{HrZoneModel, HrZoneSet, PowerZoneModel, PowerZoneSet};
use pierre_core::models::{
    Activity, AnchorOrigin, AnchorStamp, DailyTrainingState, DataSource, DeviceType, SportType,
    StoredHealthMetrics, StoredRecoveryMetrics, TenantId, UserPhysiologicalProfile,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::WellnessWriteMode;
use pierre_fitness_compute::training_history_compute::{compute_training_history, AthleteInputs};
use pierre_mcp_server::services::anchor_promotion::run_anchor_promotion_pass;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_providers::ai_scope;
use pierre_providers::precedence::{Ingress, Service, Source};
use pierre_services::effective_physiology::effective_physiology;
use pierre_services::stored_health::{self, Anchors, SourceFilter};
use pierre_tool_runtime::anchor_promotion::{
    promote_anchors, read_promotion, save_promotion, AnchorPromotion,
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::scopes::{missing_scope, required_scopes};
use pierre_tool_runtime::training_history_compute::HistoryRefresh;
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
    assert_eq!(profile.ftp_watts, Some(285), "bug: the stored FTP was lost");
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

    // Coggan's seven levels at 250 W, cut by cageux: 55 / 75 / 90 / 105 /
    // 120 / 150 % of FTP rounded to the watt, level 7 open above 375 W.
    assert_eq!(
        zones,
        PowerZoneSet::from_zones(&PowerZones::coggan(250.0).unwrap())
    );
    assert_eq!(
        [
            zones.z1_max,
            zones.z2_max,
            zones.z3_max,
            zones.z4_max,
            zones.z5_max
        ],
        [138, 188, 225, 263, 300]
    );
    assert_eq!(zones.z6_max, Some(375));
    assert_eq!(zones.model, PowerZoneModel::Coggan);
    assert!(
        zones.z3_max < 250 && zones.z4_max >= 250,
        "the threshold level should bracket the 250 W FTP, got z3={} z4={}",
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

    // No VO2max saved: cageux estimates one from the heart rates (Uth:
    // 15.3 x 192 / 48 = 61.2, the elite tier) and cuts its zones from it.
    let expected = HrZones::for_athlete(None, 48, 192).unwrap();
    assert_eq!(zones, HrZoneSet::from_zones(&expected));
    assert_eq!(zones.tier, Some(HrZoneTier::Elite));
    assert_eq!(zones.vo2_max_source, Some(Vo2maxSource::HeartRateRatio));
    assert_eq!(zones.model, HrZoneModel::KarvonenVo2maxTiered);
    assert!(
        zones.z1_max < zones.z2_max
            && zones.z2_max < zones.z3_max
            && zones.z3_max < zones.z4_max
            && zones.z4_max < zones.z5_max,
        "HR zones must be strictly increasing, got {zones:?}"
    );
    assert!(
        zones.z1_min.is_some_and(|floor| floor > 48),
        "zone 1 must sit above the resting rate, got {zones:?}"
    );
    assert_eq!(
        zones.z6_max,
        Some(192),
        "the elite tier's zone 6 tops out at the maximum heart rate"
    );
    assert!(zones.z5_max < 192, "zone 5 stops short of the maximum");
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

    // The mapping every training-load reader uses in production, so this
    // asserts the loop the write path closes rather than a parallel one.
    let inputs = AthleteInputs::from_profile(Some(&profile));
    assert_eq!(
        inputs.ftp_watts,
        Some(250.0),
        "the engine's inputs must carry the saved FTP"
    );

    let activities: Vec<Activity> = (1..=14).map(|d| power_run(&format!("a{d}"), d)).collect();
    let to = Utc::now().date_naive();
    let from = to - Duration::days(13);
    let config = AlgorithmConfig::default();

    let with_physiology = compute_training_history(&activities, inputs, from, to, &config, None)
        .expect("training-load series");
    let without = compute_training_history(
        &activities,
        AthleteInputs::default(),
        from,
        to,
        &config,
        None,
    )
    .expect("training-load series");

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
// The agent can actually call it
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

// ============================================================================
// The profile access is scope-gated (carnet#363)
// ============================================================================

#[test]
fn the_tool_declares_the_profile_access_it_performs() {
    // writes `user_physiological_profiles` and returns the profile re-read from storage,
    // so the call touches identity data. Declaring only the runtime
    // requirements yields fitness:write, which the
    // read-only default grant an RFC 7591 registration receives satisfies —
    // and that grant is exactly what the profile split exists to hold back.
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let (_, _, caps, _) = registry
        .all_tool_metadata()
        .into_iter()
        .find(|(name, _, _, _)| name == "set_physiology")
        .expect("the tool is registered");

    assert!(
        required_scopes(caps).contains(&OAuthScope::ProfileWrite),
        "touching the stored profile requires profile:write, not fitness:write"
    );
    assert!(
        required_scopes(caps).contains(&OAuthScope::ProfileRead),
        "the reply is the profile re-read from storage, so the read is declared too"
    );
    // The tool needs both profile scopes, and `missing_scope` names the first
    // one the caller lacks — so a fitness-write grant is turned away at the
    // read, before the write is ever considered.
    assert_eq!(
        missing_scope(&[OAuthScope::FitnessWrite], caps),
        Some(OAuthScope::ProfileRead),
        "a fitness-write grant must be refused and told what it needed"
    );
    // Holding the read is not enough to write the athlete's profile; this is
    // the assertion that would fail if PROFILE were dropped and the write
    // resolved back to fitness:write.
    assert_eq!(
        missing_scope(&[OAuthScope::ProfileRead], caps),
        Some(OAuthScope::ProfileWrite),
        "profile:read alone must still be refused the write"
    );
    assert_eq!(
        missing_scope(&OAuthScope::self_grant(), caps),
        None,
        "the athlete's own grant covers it"
    );
}

// ============================================================================
// update_user_configuration merges what it is given into what is saved
// ============================================================================

/// Two updates naming different overrides keep both.
///
/// The tool rebuilt the whole configuration from each call's arguments, so a
/// second override erased the first, and a call naming no profile reset the
/// saved one. Overrides are merged now, a null removes one, and the applied
/// template is kept unless a call names another.
#[tokio::test]
async fn update_user_configuration_merges_overrides_into_the_saved_ones() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let update =
        |params: Value| make_request("update_user_configuration", params, user_id, &tenant_id);

    let first = executor
        .execute_tool(update(json!({
            "profile": "Elite Athlete",
            "parameters": {"heart_rate.anaerobic_threshold": 88.0, "pace.easy_zone_low": 0.6}
        })))
        .await?;
    assert!(first.success, "{:?}", first.error);
    let second = executor
        .execute_tool(update(json!({
            "parameters": {"insights.min_confidence": 0.7, "pace.easy_zone_low": null}
        })))
        .await?;
    assert!(second.success, "{:?}", second.error);
    assert_eq!(
        second.result.unwrap()["changes_applied"],
        2,
        "the count is of what this call changed"
    );

    let saved: Value = serde_json::from_str(
        &executor
            .resources
            .repos()
            .profiles
            .get_configuration(&user_id.to_string())
            .await?
            .expect("the configuration is saved"),
    )?;
    assert_eq!(
        saved["session_overrides"],
        json!({"heart_rate.anaerobic_threshold": 88.0, "insights.min_confidence": 0.7}),
        "the first override survives the second update and the nulled one is gone"
    );
    assert_eq!(
        saved["active_profile"], "elite",
        "no profile named keeps the saved one"
    );
    Ok(())
}

// ============================================================================
// Anchors and measurements (Athlete Data Consolidation §5, S9; carnet#1306)
// ============================================================================

/// Every anchor the effective read covers.
const fn every_anchor() -> Anchors {
    Anchors {
        resting_hr: true,
        vo2_max: true,
        weight: true,
    }
}

/// A data source of `provider` for the athlete, as a sync records it.
async fn source_of(
    executor: &UniversalToolExecutor,
    tenant: &TenantId,
    user_id: Uuid,
    provider: &str,
) -> Result<String> {
    Ok(executor
        .resources
        .repos()
        .data_sources
        .upsert_data_source(
            tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: provider.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await?)
}

/// A Garmin recovery day holding only a resting heart rate.
fn garmin_morning(
    user_id: Uuid,
    source_id: &str,
    date: NaiveDate,
    rhr: u32,
) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: String::new(),
        user_id: user_id.to_string(),
        data_source_id: source_id.to_owned(),
        date,
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: None,
        resting_heart_rate: Some(rhr),
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: "garmin".to_owned(),
        recorded_at: Utc::now(),
    }
}

/// A resting heart rate the athlete states is that day's measurement too:
/// filed under `athlete` on their own calendar day, and standing over the
/// watch's reading of the same morning.
#[tokio::test]
async fn a_stated_resting_hr_is_the_days_measurement_over_the_watchs() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let repos = executor.resources.repos();
    repos
        .users
        .set_timezone(user_id, "Australia/Brisbane")
        .await?;
    let today = Utc::now().with_timezone(&Brisbane).date_naive();
    let garmin = source_of(&executor, &tenant, user_id, "garmin").await?;
    repos
        .recovery
        .upsert_recovery_metrics(
            &tenant,
            &garmin_morning(user_id, &garmin, today, 55),
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await?;

    set_physiology(&executor, user_id, &tenant_id, json!({ "resting_hr": 48 })).await?;

    let now = Utc::now();
    let read = stored_health::merged_recovery(
        repos,
        executor.resources.provider_registry().as_ref(),
        user_id,
        &tenant,
        (now - Duration::days(3), now + Duration::days(1)),
        SourceFilter::All,
        stored_health::as_stored,
    )
    .await?;
    assert_eq!(
        read.records.len(),
        1,
        "one morning, merged: {:?}",
        read.records
    );
    let morning = &read.records[0];
    assert_eq!(
        morning.record.date, today,
        "bug: dated by the athlete's own day"
    );
    assert_eq!(
        morning.record.resting_heart_rate,
        Some(48),
        "bug: the athlete's stated reading stands over the watch's"
    );
    assert_eq!(
        morning.primary,
        Source::new(Service::Athlete, Ingress::Manual),
        "the manual entry is the day's primary"
    );
    assert!(
        morning.sources.iter().any(|s| s == "garmin"),
        "the watch's day is merged in: {:?}",
        morning.sources
    );
    Ok(())
}

/// The profile's anchor stands over a newer measurement; an anchor the
/// profile lacks is read from the latest one, however old.
#[tokio::test]
async fn an_empty_anchor_reads_the_latest_measurement_and_a_stored_one_stands() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let repos = executor.resources.repos();
    let garmin = source_of(&executor, &tenant, user_id, "garmin").await?;
    // The athlete's own export: served over every transport, as the tool
    // reads it here.
    let export = source_of(&executor, &tenant, user_id, "garmin_archive").await?;
    let long_ago = Utc::now().date_naive() - Duration::days(200);
    repos
        .recovery
        .upsert_recovery_metrics(
            &tenant,
            &garmin_morning(user_id, &garmin, long_ago, 55),
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await?;
    repos
        .health_snapshots
        .upsert_health_snapshot(
            &tenant,
            &StoredHealthMetrics {
                id: String::new(),
                user_id: user_id.to_string(),
                data_source_id: export,
                date: long_ago,
                weight_kg: Some(71.8),
                body_fat_pct: None,
                muscle_mass_kg: None,
                bmi: None,
                bone_mass_kg: None,
                water_pct: None,
                systolic_bp: None,
                diastolic_bp: None,
                blood_glucose: None,
                vo2_max: Some(52.0),
                source_name: "garmin_archive".to_owned(),
                recorded_at: Utc::now(),
            },
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await?;
    let terms = executor.resources.provider_registry();

    let effective =
        effective_physiology(repos, terms.as_ref(), tenant, user_id, every_anchor()).await?;
    assert!(effective.profile.is_none(), "no profile is stored");
    assert_eq!(
        effective.resting_hr(),
        Some(55),
        "bug: with no anchor the latest measurement is read, 200 days back"
    );
    assert_eq!(effective.vo2_max(), Some(52.0), "the export's VO2max");
    assert_eq!(effective.weight(), Some(71.8), "the latest weigh-in");

    // The VO2max estimate takes the weigh-in when the profile has none.
    let estimate = executor
        .execute_tool(make_request(
            "estimate_vo2max",
            json!({ "method": "astrand_ryhming", "gender": "male",
                    "heart_rate": 150, "power_watts": 200 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(estimate.success, "{:?}", estimate.error);
    let estimate = estimate.result.expect("a payload");
    assert!(
        estimate["defaults_from_profile"]
            .as_array()
            .is_some_and(|d| d.iter().any(|v| v == "weight_kg")),
        "bug: the weigh-in fills the estimate's weight: {estimate}"
    );

    // An anchor the athlete sets stands, however much newer a measurement is.
    let mut profile = UserPhysiologicalProfile::new(user_id, SportType::Run);
    profile.resting_hr = Some(48);
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant, user_id, &profile)
        .await?;
    repos
        .recovery
        .upsert_recovery_metrics(
            &tenant,
            &garmin_morning(user_id, &garmin, Utc::now().date_naive(), 61),
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await?;
    let effective =
        effective_physiology(repos, terms.as_ref(), tenant, user_id, every_anchor()).await?;
    assert_eq!(
        effective.resting_hr(),
        Some(48),
        "bug: a newer watch reading replaced the athlete's anchor"
    );
    assert_eq!(
        effective.measured.resting_hr, None,
        "no measurement is read for an anchor the profile holds"
    );
    Ok(())
}

/// An executor serving the athlete's own web app.
async fn own_app_executor() -> Result<UniversalToolExecutor> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(UniversalToolExecutor::new(resources)
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp))
}

/// The athlete's live recovery and body rows filed under `athlete`.
async fn athlete_rows(
    executor: &UniversalToolExecutor,
    tenant: &TenantId,
    user_id: Uuid,
) -> Result<(Vec<StoredRecoveryMetrics>, Vec<StoredHealthMetrics>)> {
    let repos = executor.resources.repos();
    let (start, end) = (
        Utc::now() - Duration::days(3),
        Utc::now() + Duration::days(2),
    );
    let recovery = repos
        .recovery
        .get_recovery_metrics(user_id, tenant, start, end)
        .await?
        .into_iter()
        .filter(|row| row.source_name == "athlete")
        .collect();
    let body = repos
        .health_snapshots
        .get_health_snapshots(user_id, tenant, start, end)
        .await?
        .into_iter()
        .filter(|row| row.source_name == "athlete")
        .collect();
    Ok((recovery, body))
}

/// A number stated in a turn that read first-party-only data (the athlete
/// reads WHOOP's resting heart rate and says "save that") is saved to the
/// profile, stamped first-party-only, and filed as no wellness row: a row
/// under `athlete` is served over every transport, so it would carry the
/// number past the stamp (carnet#769).
#[tokio::test]
async fn a_reading_stated_in_a_first_party_only_turn_files_no_open_row() -> Result<()> {
    let executor = own_app_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);

    // The turn read first-party-only data before the athlete asked to save.
    let tainted = ai_scope::Provenance::new();
    tainted.mark();
    // Built inside the turn, as the chat turn builds its tool surface.
    let in_turn = ai_scope::tracking(tainted, async {
        UniversalToolExecutor::new(Arc::clone(&executor.resources))
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp)
    })
    .await;
    let response = in_turn
        .execute_tool(make_request(
            "set_physiology",
            json!({ "resting_hr": 47, "weight": 70.5 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(response.success, "{:?}", response.error);
    let profile = executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .expect("the profile is saved");
    assert_eq!(profile.resting_hr, Some(47), "the anchor is saved");
    assert!(
        profile.transport_policy.is_first_party_only(),
        "and stamped first-party-only"
    );
    let (recovery, body) = athlete_rows(&executor, &tenant, user_id).await?;
    assert!(
        recovery.is_empty() && body.is_empty(),
        "bug: a first-party-only number was filed as an open row: {recovery:?} {body:?}"
    );

    // A later save reads that stamped profile, so it is a first-party-only
    // derivation too: still nothing filed.
    set_physiology(&executor, user_id, &tenant_id, json!({ "resting_hr": 48 })).await?;
    let (recovery, _) = athlete_rows(&executor, &tenant, user_id).await?;
    assert!(recovery.is_empty(), "the stamp holds: {recovery:?}");

    // The same statement from an athlete whose turn read nothing restricted
    // is filed.
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    set_physiology(&executor, user_id, &tenant_id, json!({ "resting_hr": 48 })).await?;
    let (recovery, _) = athlete_rows(&executor, &tenant, user_id).await?;
    assert_eq!(
        recovery
            .iter()
            .map(|r| r.resting_heart_rate)
            .collect::<Vec<_>>(),
        [Some(48)],
        "an untainted statement is the day's measurement"
    );
    Ok(())
}

/// A `VO2max` the athlete saves is an anchor, never a measurement row: the
/// number `set_physiology` is handed is as often an estimate (a Cooper test
/// `estimate_vo2max` computed) as a lab value, and an estimate is never
/// stored as a measurement (Athlete Data Consolidation §5). The watch's
/// measurement of the day stands.
#[tokio::test]
async fn a_saved_vo2max_is_an_anchor_not_the_days_measurement() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let repos = executor.resources.repos();
    repos
        .users
        .set_timezone(user_id, "Australia/Brisbane")
        .await?;
    let today = Utc::now().with_timezone(&Brisbane).date_naive();
    let garmin = source_of(&executor, &tenant, user_id, "garmin").await?;
    repos
        .health_snapshots
        .upsert_health_snapshot(
            &tenant,
            &StoredHealthMetrics {
                id: String::new(),
                user_id: user_id.to_string(),
                data_source_id: garmin,
                date: today,
                weight_kg: None,
                body_fat_pct: None,
                muscle_mass_kg: None,
                bmi: None,
                bone_mass_kg: None,
                water_pct: None,
                systolic_bp: None,
                diastolic_bp: None,
                blood_glucose: None,
                vo2_max: Some(54.0),
                source_name: "garmin".to_owned(),
                recorded_at: Utc::now(),
            },
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await?;

    let saved = set_physiology(&executor, user_id, &tenant_id, json!({ "vo2_max": 50.3 })).await?;
    assert_eq!(
        saved["profile"]["vo2_max"],
        json!(50.3),
        "the anchor is saved"
    );

    let now = Utc::now();
    let days = stored_health::merged_body(
        repos,
        executor.resources.provider_registry().as_ref(),
        user_id,
        &tenant,
        (now - Duration::days(3), now + Duration::days(1)),
        SourceFilter::All,
        stored_health::as_stored,
    )
    .await?
    .records;
    assert_eq!(
        days.iter().map(|d| d.record.vo2_max).collect::<Vec<_>>(),
        [Some(54.0)],
        "bug: the saved VO2max replaced the watch's measurement of the day"
    );
    let (_, body) = athlete_rows(&executor, &tenant, user_id).await?;
    assert!(body.is_empty(), "no athlete body row: {body:?}");
    Ok(())
}

/// Take `table` away, so every read or write of it fails.
async fn drop_table(executor: &UniversalToolExecutor, table: &str) -> Result<()> {
    let statement = format!("DROP TABLE {table}");
    match executor.resources.database().backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(&statement).execute(sqlite.pool()).await?;
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::query(&format!("{statement} CASCADE"))
                .execute(postgres.pool())
                .await?;
        }
    }
    Ok(())
}

/// A measurement read that fails costs only the measurements: the stored
/// profile still describes the athlete, and an estimate given its weight
/// still runs.
#[tokio::test]
async fn a_failed_measurement_read_keeps_the_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "ftp_watts": 285, "max_hr": 188 }),
    )
    .await?;
    drop_table(&executor, "recovery_metrics").await?;
    drop_table(&executor, "health_snapshots").await?;

    let effective = effective_physiology(
        executor.resources.repos(),
        executor.resources.provider_registry().as_ref(),
        tenant,
        user_id,
        every_anchor(),
    )
    .await?;
    let profile = effective
        .profile
        .clone()
        .expect("bug: the profile was dropped with the measurements");
    assert_eq!(profile.ftp_watts, Some(285), "bug: the stored FTP was lost");
    assert_eq!(profile.max_hr, Some(188), "bug: the stored max HR was lost");
    assert_eq!(effective.resting_hr(), None, "no measurement was read");

    let estimate = executor
        .execute_tool(make_request(
            "estimate_vo2max",
            json!({ "method": "astrand_ryhming", "gender": "male",
                    "heart_rate": 150, "power_watts": 200, "weight_kg": 72.0 }),
            user_id,
            &tenant_id,
        ))
        .await;
    assert!(
        estimate.as_ref().is_ok_and(|r| r.success),
        "bug: an estimate given its weight failed on the measurement read: {estimate:?}"
    );
    Ok(())
}

/// A statement whose measurement row cannot be written saves nothing: the
/// tool's error is the truth, and a retry writes both.
#[tokio::test]
async fn a_failed_measurement_write_saves_no_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    drop_table(&executor, "recovery_metrics").await?;

    let rejection = set_physiology_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "resting_hr": 48 }),
    )
    .await?;
    assert!(!rejection.is_empty(), "the failure is reported");
    let profile = executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?;
    assert!(
        profile.is_none(),
        "bug: the profile was saved although the tool reported a failure: {profile:?}"
    );
    Ok(())
}

// ── Anchor promotion (DECISION 5, carnet#1307) ───────────────────────────────

/// File `bpm` as the Garmin morning of each of the athlete's last days, newest
/// first: `mornings[0]` is today.
async fn garmin_mornings(
    executor: &UniversalToolExecutor,
    tenant: &TenantId,
    user_id: Uuid,
    today: NaiveDate,
    mornings: &[u32],
) -> Result<()> {
    let garmin = source_of(executor, tenant, user_id, "garmin").await?;
    for (days_ago, bpm) in (0_i64..).zip(mornings) {
        executor
            .resources
            .repos()
            .recovery
            .upsert_recovery_metrics(
                tenant,
                &garmin_morning(user_id, &garmin, today - Duration::days(days_ago), *bpm),
                WellnessWriteMode::FullDay,
                Utc::now(),
            )
            .await?;
    }
    Ok(())
}

/// Heart-rate runs the athlete uploaded on each of the last ten days, plus a
/// stored rollup day whose load no workout gives, so a recompute shows.
async fn stored_history_to_recompute(
    executor: &UniversalToolExecutor,
    tenant: TenantId,
    user_id: Uuid,
) -> Result<NaiveDate> {
    let repos = executor.resources.repos();
    let runs: Vec<Activity> = (1..=10)
        .map(|day| {
            ActivityBuilder::new(
                format!("upload-run-{day}"),
                "Run",
                SportType::Run,
                Utc::now() - Duration::days(day),
                3_600,
                "upload",
            )
            .distance_meters(10_000.0)
            .average_heart_rate(150)
            .build()
        })
        .collect();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "upload", &runs)
        .await?;
    let stale_day = Utc::now().date_naive() - Duration::days(1);
    let mut stale = DailyTrainingState::zero(stale_day);
    stale.daily_load = 999.0;
    repos
        .training_history
        .upsert_training_history_batch(tenant, user_id, &[stale])
        .await?;
    Ok(stale_day)
}

async fn stored_profile(
    executor: &UniversalToolExecutor,
    tenant: TenantId,
    user_id: Uuid,
) -> Result<UserPhysiologicalProfile> {
    Ok(executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .expect("a stored profile"))
}

/// The anchors `set_physiology` sets are stamped as the athlete's own, on
/// their own calendar day, and the stamp survives the round trip through the
/// table; an anchor it does not set carries none.
#[tokio::test]
async fn the_anchors_set_physiology_sets_are_stamped_as_the_athletes() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    executor
        .resources
        .repos()
        .users
        .set_timezone(user_id, "Australia/Brisbane")
        .await?;
    let today = Utc::now().with_timezone(&Brisbane).date_naive();

    let saved = set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "resting_hr": 47, "weight": 70.5 }),
    )
    .await?;

    let profile = stored_profile(&executor, tenant, user_id).await?;
    assert_eq!(
        profile.anchor_provenance.resting_hr,
        Some(AnchorStamp::athlete(today)),
        "bug: a stated resting heart rate is not stamped as the athlete's"
    );
    assert_eq!(
        profile.anchor_provenance.weight,
        Some(AnchorStamp::athlete(today)),
        "bug: a stated weight is not stamped as the athlete's"
    );
    assert_eq!(
        profile.anchor_provenance.vo2_max, None,
        "an anchor the call did not set carries no stamp"
    );
    assert_eq!(
        saved["profile"]["anchor_provenance"]["resting_hr"]["origin"],
        json!("athlete"),
        "the tool echoes where the anchor came from: {saved}"
    );
    Ok(())
}

/// An empty resting heart rate takes the median of the last seven mornings,
/// stamped as promoted and as of the newest; the zones are recut from it and
/// the stored training history recomputed against it. The Garmin rows it came
/// from keep the profile on Dravr's own surfaces (carnet#769).
#[tokio::test]
async fn an_empty_resting_hr_is_promoted_from_the_weeks_median_and_recomputes_history() -> Result<()>
{
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let today = Utc::now().date_naive();
    set_physiology(&executor, user_id, &tenant_id, json!({ "max_hr": 190 })).await?;
    let zones_before = stored_profile(&executor, tenant, user_id).await?.hr_zones;
    // An eighth, older morning sits outside the window.
    garmin_mornings(
        &executor,
        &tenant,
        user_id,
        today,
        &[50, 52, 49, 51, 70, 50, 53, 40],
    )
    .await?;
    let stale_day = stored_history_to_recompute(&executor, tenant, user_id).await?;
    let runtime = Arc::clone(&executor.resources);

    let outcome = promote_anchors(&runtime, tenant, user_id).await?;

    assert!(
        matches!(
            outcome,
            AnchorPromotion::Promoted(HistoryRefresh::Recomputed { .. })
        ),
        "bug: the promotion did not recompute the stored history: {outcome:?}"
    );
    let profile = stored_profile(&executor, tenant, user_id).await?;
    assert_eq!(
        profile.resting_hr,
        Some(51),
        "bug: not the median of the last seven mornings"
    );
    assert_eq!(
        profile.anchor_provenance.resting_hr,
        Some(AnchorStamp::promoted(today)),
        "stamped as promoted, as of the newest morning"
    );
    assert_eq!(
        AthleteInputs::from_profile(Some(&profile)).resting_hr,
        Some(51.0),
        "bug: stored training load does not read the promoted anchor"
    );
    assert_ne!(
        profile.hr_zones, zones_before,
        "bug: the zones were not recut from the promoted resting heart rate"
    );
    assert!(
        profile.transport_policy.is_first_party_only(),
        "bug: a number promoted from Garmin rows left the profile open to external callers"
    );
    let day = executor
        .resources
        .repos()
        .training_history
        .get_training_history(tenant, user_id, stale_day, stale_day)
        .await?;
    assert!(
        day.iter()
            .all(|state| (state.daily_load - 999.0).abs() > f64::EPSILON),
        "bug: the stored day was not recomputed: {day:?}"
    );

    let again = promote_anchors(&runtime, tenant, user_id).await?;
    assert_eq!(
        again,
        AnchorPromotion::Unchanged,
        "the same week promoted twice writes nothing"
    );
    Ok(())
}

/// Fewer than three mornings in the week promote nothing: one reading never
/// stands for the week.
#[tokio::test]
async fn two_mornings_promote_no_resting_hr() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    garmin_mornings(
        &executor,
        &tenant,
        user_id,
        Utc::now().date_naive(),
        &[50, 52],
    )
    .await?;

    let outcome = promote_anchors(&executor.resources, tenant, user_id).await?;

    assert_eq!(outcome, AnchorPromotion::Unchanged, "nothing qualifies");
    let profile = executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?;
    assert!(profile.is_none(), "no profile is invented: {profile:?}");
    Ok(())
}

/// The athlete's own number is never promoted over: neither one they stated
/// after a promotion, nor one stored before origins were recorded. Stated in
/// their own app: a profile promoted from Garmin rows is first-party-only.
#[tokio::test]
async fn a_promotion_never_overwrites_the_athletes_resting_hr() -> Result<()> {
    let executor = own_app_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let today = Utc::now().date_naive();
    garmin_mornings(&executor, &tenant, user_id, today, &[50, 52, 49, 51, 50]).await?;
    let runtime = Arc::clone(&executor.resources);
    promote_anchors(&runtime, tenant, user_id).await?;
    assert_eq!(
        stored_profile(&executor, tenant, user_id).await?.resting_hr,
        Some(50),
        "the empty anchor was promoted"
    );

    // The athlete states their own: it is theirs from now on.
    set_physiology(&executor, user_id, &tenant_id, json!({ "resting_hr": 46 })).await?;
    let outcome = promote_anchors(&runtime, tenant, user_id).await?;
    let profile = stored_profile(&executor, tenant, user_id).await?;
    assert_eq!(
        outcome,
        AnchorPromotion::Unchanged,
        "bug: a promotion wrote over the athlete's stated anchor: {outcome:?}"
    );
    assert_eq!(
        profile.resting_hr,
        Some(46),
        "bug: the promotion overwrote the resting heart rate the athlete stated"
    );
    assert_eq!(
        profile.anchor_provenance.resting_hr.map(|s| s.origin),
        Some(AnchorOrigin::Athlete),
        "bug: the athlete's statement did not reclaim the anchor"
    );

    // A value stored before origins were recorded is the athlete's too.
    let (legacy_user, legacy_tenant_id) = create_test_user(&executor).await?;
    let legacy_tenant = TenantId::from_uuid(Uuid::parse_str(&legacy_tenant_id)?);
    let mut legacy = UserPhysiologicalProfile::new(legacy_user, SportType::Run);
    legacy.resting_hr = Some(44);
    executor
        .resources
        .repos()
        .user_physiological_profile
        .upsert_user_physiological_profile(legacy_tenant, legacy_user, &legacy)
        .await?;
    garmin_mornings(&executor, &legacy_tenant, legacy_user, today, &[50, 52, 49]).await?;
    let outcome = promote_anchors(&runtime, legacy_tenant, legacy_user).await?;
    assert_eq!(
        outcome,
        AnchorPromotion::Unchanged,
        "bug: a promotion wrote over a value stored before origins: {outcome:?}"
    );
    assert_eq!(
        stored_profile(&executor, legacy_tenant, legacy_user)
            .await?
            .resting_hr,
        Some(44),
        "bug: a legacy value was promoted over"
    );
    Ok(())
}

/// A promoted resting heart rate that contradicts the stored maximum is
/// refused whole: nothing is saved.
#[tokio::test]
async fn a_promotion_contradicting_the_stored_maximum_saves_nothing() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    set_physiology(&executor, user_id, &tenant_id, json!({ "max_hr": 120 })).await?;
    garmin_mornings(
        &executor,
        &tenant,
        user_id,
        Utc::now().date_naive(),
        &[125, 126, 124],
    )
    .await?;

    let outcome = promote_anchors(&executor.resources, tenant, user_id).await?;

    assert_eq!(
        outcome,
        AnchorPromotion::Refused,
        "bug: a promoted resting HR at or above the stored maximum was not refused"
    );
    assert_eq!(
        stored_profile(&executor, tenant, user_id).await?.resting_hr,
        None,
        "bug: a contradictory promotion was saved"
    );
    Ok(())
}

/// The weekly pass reaches an athlete holding stored history but no profile,
/// and the latest weigh-in and running `VO2max` fill their anchors too.
#[tokio::test]
async fn the_weekly_pass_reaches_a_history_holder_with_no_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let today = Utc::now().date_naive();
    garmin_mornings(&executor, &tenant, user_id, today, &[48, 49, 50]).await?;
    stored_history_to_recompute(&executor, tenant, user_id).await?;
    let garmin = source_of(&executor, &tenant, user_id, "garmin").await?;
    for (days_ago, weight, vo2_max) in [(9, 71.0, Some(52.0)), (2, 70.4, None)] {
        executor
            .resources
            .repos()
            .health_snapshots
            .upsert_health_snapshot(
                &tenant,
                &StoredHealthMetrics {
                    id: String::new(),
                    user_id: user_id.to_string(),
                    data_source_id: garmin.clone(),
                    date: today - Duration::days(days_ago),
                    weight_kg: Some(weight),
                    body_fat_pct: None,
                    muscle_mass_kg: None,
                    bmi: None,
                    bone_mass_kg: None,
                    water_pct: None,
                    systolic_bp: None,
                    diastolic_bp: None,
                    blood_glucose: None,
                    vo2_max,
                    source_name: "garmin".to_owned(),
                    recorded_at: Utc::now(),
                },
                WellnessWriteMode::FullDay,
                Utc::now(),
            )
            .await?;
    }
    let runtime = Arc::clone(&executor.resources);

    let report = run_anchor_promotion_pass(&runtime).await?;

    assert_eq!(
        report.promoted, 1,
        "bug: the pass did not promote the history holder's anchors: {report:?}"
    );
    assert_eq!(
        report.recomputed, 1,
        "bug: the promotion did not recompute the stored history: {report:?}"
    );
    assert_eq!(
        report.failed, 0,
        "bug: a promotion failed on a read or write: {report:?}"
    );
    let profile = stored_profile(&executor, tenant, user_id).await?;
    assert_eq!(
        profile.resting_hr,
        Some(49),
        "bug: not the median of the three mornings"
    );
    assert_eq!(profile.weight, Some(70.4), "the latest weigh-in");
    assert_eq!(
        profile.anchor_provenance.weight,
        Some(AnchorStamp::promoted(today - Duration::days(2))),
        "bug: the weight is not stamped as of its weigh-in"
    );
    assert_eq!(
        profile.vo2_max,
        Some(52.0),
        "the latest VO2max, however old"
    );
    assert_eq!(
        profile.anchor_provenance.vo2_max,
        Some(AnchorStamp::promoted(today - Duration::days(9))),
        "bug: the VO2max is not stamped as of its measurement"
    );

    let again = run_anchor_promotion_pass(&runtime).await?;
    assert_eq!(
        again.promoted, 0,
        "a second pass changes nothing: {again:?}"
    );
    assert_eq!(
        again.unchanged, 1,
        "bug: the second pass did not count the athlete as unchanged: {again:?}"
    );
    Ok(())
}

/// File a weigh-in from `provider` on `date`.
async fn weigh_in(
    executor: &UniversalToolExecutor,
    tenant: &TenantId,
    user_id: Uuid,
    provider: &str,
    date: NaiveDate,
    weight_kg: f64,
) -> Result<()> {
    let source = source_of(executor, tenant, user_id, provider).await?;
    executor
        .resources
        .repos()
        .health_snapshots
        .upsert_health_snapshot(
            tenant,
            &StoredHealthMetrics {
                id: String::new(),
                user_id: user_id.to_string(),
                data_source_id: source,
                date,
                weight_kg: Some(weight_kg),
                body_fat_pct: None,
                muscle_mass_kg: None,
                bmi: None,
                bone_mass_kg: None,
                water_pct: None,
                systolic_bp: None,
                diastolic_bp: None,
                blood_glucose: None,
                vo2_max: None,
                source_name: provider.to_owned(),
                recorded_at: Utc::now(),
            },
            WellnessWriteMode::FullDay,
            Utc::now(),
        )
        .await?;
    Ok(())
}

/// Rows read for an anchor that does not move never stamp the profile: two
/// Garmin mornings promote no resting heart rate, so a profile whose weight
/// alone moved, from a provider open to every transport, stays open to every
/// transport.
#[tokio::test]
async fn rows_read_for_an_anchor_that_did_not_move_do_not_stamp_the_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let today = Utc::now().date_naive();
    garmin_mornings(&executor, &tenant, user_id, today, &[50, 52]).await?;
    weigh_in(&executor, &tenant, user_id, "wahoo", today, 70.0).await?;

    let outcome = promote_anchors(&executor.resources, tenant, user_id).await?;

    assert!(
        matches!(outcome, AnchorPromotion::Promoted(_)),
        "bug: the weigh-in from an open provider was not promoted: {outcome:?}"
    );
    let profile = stored_profile(&executor, tenant, user_id).await?;
    assert_eq!(profile.weight, Some(70.0), "the weigh-in is promoted");
    assert_eq!(profile.resting_hr, None, "two mornings promote nothing");
    assert!(
        !profile.transport_policy.is_first_party_only(),
        "bug: Garmin mornings that promoted nothing stamped the profile first-party-only"
    );
    Ok(())
}

/// A `set_physiology` that commits while the promotion reads the measurements
/// stands: the promotion is applied to the profile as it is when it saves, not
/// to the copy it read before.
#[tokio::test]
async fn a_set_physiology_during_the_measurement_read_is_not_reverted() -> Result<()> {
    let executor = own_app_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let today = Utc::now().date_naive();
    set_physiology(&executor, user_id, &tenant_id, json!({ "ftp_watts": 250 })).await?;
    garmin_mornings(&executor, &tenant, user_id, today, &[50, 52, 49, 51]).await?;
    let runtime = Arc::clone(&executor.resources);
    let promotion = read_promotion(
        &runtime,
        tenant,
        user_id,
        Anchors {
            resting_hr: true,
            ..Anchors::default()
        },
    )
    .await?;

    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "ftp_watts": 265, "resting_hr": 46 }),
    )
    .await?;
    let outcome = save_promotion(&runtime, tenant, user_id, &promotion).await?;

    assert_eq!(
        outcome,
        AnchorPromotion::Unchanged,
        "bug: the promotion was applied to a profile read before the athlete's save: {outcome:?}"
    );
    let profile = stored_profile(&executor, tenant, user_id).await?;
    assert_eq!(
        profile.ftp_watts,
        Some(265),
        "bug: the promotion reverted the FTP the athlete saved meanwhile"
    );
    assert_eq!(
        profile.resting_hr,
        Some(46),
        "bug: the promotion overwrote the resting heart rate the athlete saved meanwhile"
    );
    assert_eq!(
        profile.anchor_provenance.resting_hr.map(|s| s.origin),
        Some(AnchorOrigin::Athlete),
        "bug: the athlete's stamp was replaced"
    );
    Ok(())
}

/// A revision written before the origin columns rewrites an anchor without
/// naming them; the number it writes is the athlete's, and the next promotion
/// leaves it alone.
#[tokio::test]
async fn a_value_written_without_its_origin_is_never_promoted_over() -> Result<()> {
    let executor = own_app_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let tenant = TenantId::from_uuid(Uuid::parse_str(&tenant_id)?);
    let today = Utc::now().date_naive();
    garmin_mornings(&executor, &tenant, user_id, today, &[50, 52, 49]).await?;
    let runtime = Arc::clone(&executor.resources);
    promote_anchors(&runtime, tenant, user_id).await?;
    assert_eq!(
        stored_profile(&executor, tenant, user_id)
            .await?
            .anchor_provenance
            .resting_hr,
        Some(AnchorStamp::promoted(today)),
        "the empty anchor was promoted"
    );

    // The previous revision's upsert sets the value and names no origin.
    let statement = "UPDATE user_physiological_profiles SET resting_hr = 47 WHERE user_id = $1";
    match executor.resources.database().backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(statement)
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await?;
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::query(statement)
                .bind(user_id)
                .execute(postgres.pool())
                .await?;
        }
    }
    let outcome = promote_anchors(&runtime, tenant, user_id).await?;

    let profile = stored_profile(&executor, tenant, user_id).await?;
    assert_eq!(
        profile.resting_hr,
        Some(47),
        "bug: the promotion overwrote a number an origin-unaware writer stored ({outcome:?})"
    );
    assert_eq!(
        profile.anchor_provenance.resting_hr, None,
        "bug: a promoted origin was left beside the athlete's number"
    );
    Ok(())
}
