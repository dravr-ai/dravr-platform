// ABOUTME: Integration tests for per-sport physiology anchors — set_physiology's sport_anchors through to the envelope check
// ABOUTME: Storage with range and confidence, twin mirroring and superseding, relationships, tenant scope, the save path (carnet#1496)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::periodization::{AnchorConfidence, SportFamily};
use pierre_core::models::{SportAnchorName, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::TransportPolicy;
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
    let email = format!("anchors_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id.to_string()))
}

fn request(tool: &str, params: Value, user_id: Uuid, tenant_id: &str) -> UniversalRequest {
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
        .execute_tool(request("set_physiology", params, user_id, tenant_id))
        .await?;
    assert!(
        response.success,
        "set_physiology should have succeeded: {:?}",
        response.error
    );
    Ok(response.result.expect("set_physiology returns a payload"))
}

async fn set_physiology_refused(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<String> {
    match executor
        .execute_tool(request("set_physiology", params, user_id, tenant_id))
        .await
    {
        Err(e) => Ok(e.to_string()),
        Ok(response) => {
            assert!(
                !response.success,
                "set_physiology should have refused: {:?}",
                response.result
            );
            Ok(response.error.unwrap_or_default())
        }
    }
}

fn anchors_of(payload: &Value) -> Vec<Value> {
    payload["profile"]["sport_anchors"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn find<'a>(anchors: &'a [Value], sport: &str, anchor: &str) -> Option<&'a Value> {
    anchors
        .iter()
        .find(|a| a["sport"] == sport && a["anchor"] == anchor)
}

#[tokio::test]
async fn a_stated_map_anchor_is_stored_with_its_range_method_and_confidence() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "sport_anchors": [{
            "sport": "ride", "anchor": "map", "value": 380, "low": 365, "high": 395,
            "kind": "measured", "confidence": "high", "method": "ramp peak",
            "source": "lab", "measured_on": "2026-11-02"
        }]}),
    )
    .await?;

    let fields = saved["updated_fields"].as_array().expect("field names");
    assert!(fields.iter().any(|f| f == "sport_anchors"), "{saved:#}");
    let anchors = anchors_of(&saved);
    let map = find(&anchors, "ride", "map").expect("the MAP anchor is echoed");
    assert_eq!(map["value"], 380.0);
    assert_eq!(map["low"], 365.0);
    assert_eq!(map["high"], 395.0);
    assert_eq!(map["kind"], "measured");
    assert_eq!(map["confidence"], "high");
    assert_eq!(map["method"], "ramp peak");
    assert_eq!(map["origin"], "lab");
    assert_eq!(map["as_of"], "2026-11-02");

    let tenant = TenantId::parse_str(&tenant_id)?;
    let stored = executor
        .resources
        .repos()
        .user_physiological_profile
        .list_sport_anchors(tenant, user_id)
        .await?;
    assert_eq!(stored.len(), 1, "the anchor is in storage, not only echoed");
    assert_eq!(stored[0].sport, SportFamily::Ride);
    assert_eq!(stored[0].anchor, SportAnchorName::Map);
    assert_eq!(stored[0].value.confidence, AnchorConfidence::High);
    Ok(())
}

#[tokio::test]
async fn a_ride_ftp_anchor_mirrors_onto_the_profile_ftp_and_dates_it() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "sport_anchors": [{
            "sport": "ride", "anchor": "ftp", "value": 290, "kind": "measured",
            "confidence": "medium", "method": "20min_test", "measured_on": "2026-11-01"
        }]}),
    )
    .await?;

    let profile = &saved["profile"];
    assert_eq!(
        profile["ftp_watts"], 290,
        "training load reads the same FTP the families do: {saved:#}"
    );
    assert_eq!(profile["threshold_stated_on"]["ftp_watts"], "2026-11-01");
    assert!(
        profile.get("threshold_provenance").is_none(),
        "a mirrored FTP is the athlete's own, never an import"
    );
    assert!(
        profile["power_zones"].is_object(),
        "the mirrored FTP cuts power zones as a plain one does"
    );
    Ok(())
}

#[tokio::test]
async fn a_plain_ftp_supersedes_the_stored_ride_ftp_anchor() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "sport_anchors": [
            { "sport": "ride", "anchor": "ftp", "value": 280, "kind": "measured", "confidence": "high" },
            { "sport": "run", "anchor": "max_heart_rate", "value": 194, "kind": "measured" }
        ]}),
    )
    .await?;

    let saved = set_physiology(&executor, user_id, &tenant_id, json!({ "ftp_watts": 300 })).await?;

    assert_eq!(saved["profile"]["ftp_watts"], 300);
    assert!(
        saved["profile"]["threshold_stated_on"]["ftp_watts"].is_string(),
        "a plain FTP is dated the day the athlete stated it: {saved:#}"
    );
    let anchors = anchors_of(&saved);
    assert!(
        find(&anchors, "ride", "ftp").is_none(),
        "the anchor's range and confidence described the replaced number: {saved:#}"
    );
    assert!(
        find(&anchors, "run", "max_heart_rate").is_some(),
        "an anchor with no twin in the call is untouched"
    );
    Ok(())
}

#[tokio::test]
async fn one_value_stated_both_ways_in_one_call_is_refused() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let message = set_physiology_refused(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "ftp_watts": 300,
            "sport_anchors": [{ "sport": "ride", "anchor": "ftp", "value": 290, "kind": "measured" }]
        }),
    )
    .await?;
    assert!(message.contains("state it once"), "{message}");
    Ok(())
}

#[tokio::test]
async fn a_map_below_the_stored_critical_power_is_refused() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "critical_power_watts": 300, "measurement_kind": "measured" }),
    )
    .await?;

    let message = set_physiology_refused(
        &executor,
        user_id,
        &tenant_id,
        json!({ "sport_anchors": [{ "sport": "ride", "anchor": "map", "value": 280, "kind": "estimated" }] }),
    )
    .await?;
    assert!(message.contains("must be above"), "{message}");
    Ok(())
}

#[tokio::test]
async fn the_dossier_carries_the_stored_anchors_beside_the_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "sport_anchors": [{ "sport": "run", "anchor": "vvo2max", "value": 5.2, "kind": "measured" }] }),
    )
    .await?;

    let tenant = TenantId::parse_str(&tenant_id)?;
    let dossier = executor
        .resources
        .repos()
        .dossier
        .compose_dossier(tenant, user_id, TransportPolicy::AnyTransport, &|_| true)
        .await?;
    assert!(dossier.physiology.is_some());
    assert_eq!(
        dossier
            .sport_anchors
            .iter()
            .map(|a| (a.sport, a.anchor))
            .collect::<Vec<_>>(),
        vec![(SportFamily::Run, SportAnchorName::Vvo2max)],
        "a later turn reads the anchors through the dossier"
    );
    Ok(())
}

#[tokio::test]
async fn another_tenant_reads_none_of_the_athletes_anchors() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "sport_anchors": [{ "sport": "swim", "anchor": "critical_speed", "value": 1.3, "kind": "measured" }] }),
    )
    .await?;

    let other = TenantId::from_uuid(Uuid::new_v4());
    let read = executor
        .resources
        .repos()
        .user_physiological_profile
        .list_sport_anchors(other, user_id)
        .await?;
    assert!(read.is_empty(), "anchors are scoped by tenant: {read:?}");
    Ok(())
}

/// A cyclist's week whose Tuesday is a 30/15 built from the short-interval
/// family: its ride form targets "100-110% MAP".
fn family_week() -> Value {
    json!({
        "agent_id": "endurance-coach",
        "outline": {
            "goal_race": { "name": "Big Red", "date": "2026-08-08", "discipline": "gravel", "priority": "A" },
            "strategy": "rebuild volume, race-specific intensity, taper into Aug 8",
            "phases": [
                {"kind": "build", "start": "2026-07-13", "weeks": 3, "intent": "volume back up", "target_hours": 9.0},
                {"kind": "taper", "start": "2026-08-03", "weeks": 1, "intent": "shorter, sharper"}
            ]
        },
        "weeks": [{
            "week_start": "2026-07-13",
            "focus": "volume back up",
            "days": [
                {"date": "2026-07-13", "sport": "rest", "workout": "off"},
                {"date": "2026-07-14", "sport": "gravel", "workout": "30/15", "duration_min": 60,
                 "template_slug": "vo2max_short", "template_preset": "30_15", "template_params": {"sets": 3}}
            ]
        }]
    })
}

async fn save_family_week(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
) -> Result<Value> {
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            family_week(),
            user_id,
            tenant_id,
        ))
        .await?;
    assert!(saved.success, "save failed: {:?}", saved.error);
    Ok(saved.result.expect("save result"))
}

fn finding_codes(saved: &Value) -> Vec<String> {
    saved["envelope"]["days"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|day| day["findings"].as_array().cloned().unwrap_or_default())
        .filter_map(|finding| finding["code"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn a_map_with_no_definition_is_flagged_on_a_plan_that_targets_map() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "sport_anchors": [{ "sport": "ride", "anchor": "map", "value": 380, "kind": "estimated" }] }),
    )
    .await?;

    let saved = save_family_week(&executor, user_id, &tenant_id).await?;

    assert!(
        finding_codes(&saved).iter().any(|c| c == "map_without_definition"),
        "the save path reads the athlete's anchors, so a MAP with no definition is named: {saved:#}"
    );
    let anchor = &saved["envelope"]["days"][0]["anchor"];
    assert_eq!(
        anchor["anchor"], "map",
        "the notice names the anchor the day's targets sit on: {saved:#}"
    );
    assert_eq!(anchor["value"], 380.0);
    assert_eq!(anchor["origin"], "athlete");
    assert_eq!(anchor["confidence"], "low");
    assert_eq!(
        anchor["own_conversions"], false,
        "MAP alone states one end of no ratio"
    );
    Ok(())
}

#[tokio::test]
async fn a_defined_map_raises_no_definition_finding() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({ "sport_anchors": [{
            "sport": "ride", "anchor": "map", "value": 380, "kind": "measured",
            "method": "ramp peak", "confidence": "high"
        }]}),
    )
    .await?;

    let saved = save_family_week(&executor, user_id, &tenant_id).await?;

    assert!(
        !finding_codes(&saved)
            .iter()
            .any(|c| c == "map_without_definition"),
        "a MAP with its definition on file is not flagged: {saved:#}"
    );
    Ok(())
}
