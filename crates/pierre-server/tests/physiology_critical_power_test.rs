// ABOUTME: Critical power, W′, critical speed and D′ land on the profile only with whether they were measured
// ABOUTME: Pins the provenance round-trip, the full-row upsert keeping them, and every refusal set_physiology owes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#714. Vekta, intervals.icu and WKO all model a critical power from
//! training data; a lab or a 3-minute all-out test measures one. The profile
//! keeps that difference, because "your CP is 312 W" and "Vekta estimates your
//! CP at 312 W" are different claims and only the second is true of a model
//! output.
//!
//! The upsert writes every column from `EXCLUDED.*`, so a column missing from
//! any of the INSERT list, the VALUES, the SET list or the SELECT is nulled by
//! the next unrelated save. `an_ftp_save_keeps_the_stored_critical_power` is
//! the guard on that.

use anyhow::Result;
use chrono::{NaiveDate, Utc};
use pierre_core::models::{MeasurementKind, TenantId, UserPhysiologicalProfile};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
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
    let email = format!("critical_power_{}@example.com", Uuid::new_v4());
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

fn make_request(params: Value, user_id: Uuid, tenant_id: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: "set_physiology".to_owned(),
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
        .execute_tool(make_request(params, user_id, tenant_id))
        .await?;
    assert!(response.success, "set_physiology: {:?}", response.error);
    Ok(response.result.expect("set_physiology returns a payload"))
}

async fn rejection(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<String> {
    match executor
        .execute_tool(make_request(params, user_id, tenant_id))
        .await
    {
        Err(e) => Ok(e.to_string()),
        Ok(response) => {
            assert!(
                !response.success,
                "set_physiology should have refused, got {:?}",
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
) -> Result<Option<UserPhysiologicalProfile>> {
    Ok(executor
        .resources
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(TenantId::parse_str(tenant_id)?, user_id)
        .await?)
}

#[tokio::test]
async fn an_estimated_critical_power_round_trips_with_its_source() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let result = set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "critical_power_watts": 312,
            "w_prime_joules": 21500,
            "measurement_kind": "estimated",
            "measurement_source": "vekta",
            "measured_on": "2026-10-01"
        }),
    )
    .await?;

    assert_eq!(
        result["profile"]["critical_power_watts"],
        json!({"value": 312, "kind": "estimated", "origin": "vekta", "as_of": "2026-10-01"}),
        "the reply carries the kind beside the value, so the agent can frame it"
    );
    assert_eq!(result["profile"]["w_prime_joules"]["value"], json!(21500));

    let stored = stored_profile(&executor, &tenant_id, user_id)
        .await?
        .expect("a profile exists");
    let cp = stored.critical_power_watts.expect("the row holds CP");
    assert_eq!(cp.value, 312);
    assert_eq!(cp.provenance.kind, MeasurementKind::Estimated);
    assert_eq!(cp.provenance.origin.as_deref(), Some("vekta"));
    assert_eq!(cp.provenance.as_of, NaiveDate::from_ymd_opt(2026, 10, 1));
    let w_prime = stored.w_prime_joules.expect("the row holds W′");
    assert_eq!(w_prime.value, 21500);
    assert_eq!(w_prime.provenance.kind, MeasurementKind::Estimated);
    assert!(stored.critical_speed_mps.is_none());
    assert!(stored.d_prime_meters.is_none());
    Ok(())
}

#[tokio::test]
async fn a_measured_critical_speed_round_trips() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "critical_speed_mps": 4.17,
            "d_prime_meters": 616.0,
            "measurement_kind": "Measured",
            "measurement_source": "track test 3600/2400/1200 m"
        }),
    )
    .await?;

    let stored = stored_profile(&executor, &tenant_id, user_id)
        .await?
        .expect("a profile exists");
    let cs = stored.critical_speed_mps.expect("the row holds CS");
    assert!((cs.value - 4.17).abs() < 1e-9);
    assert_eq!(cs.provenance.kind, MeasurementKind::Measured);
    assert_eq!(
        cs.provenance.as_of, None,
        "no date was given, none is invented"
    );
    let d_prime = stored.d_prime_meters.expect("the row holds D′");
    assert!((d_prime.value - 616.0).abs() < 1e-9);
    Ok(())
}

/// The full-row upsert trap: a save that never mentions CP must keep it,
/// provenance and all.
#[tokio::test]
async fn an_ftp_save_keeps_the_stored_critical_power() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "critical_power_watts": 290,
            "w_prime_joules": 18000,
            "critical_speed_mps": 4.5,
            "d_prime_meters": 250.0,
            "measurement_kind": "measured",
            "measurement_source": "lab",
            "measured_on": "2026-09-20"
        }),
    )
    .await?;
    set_physiology(&executor, user_id, &tenant_id, json!({"ftp_watts": 280})).await?;

    let stored = stored_profile(&executor, &tenant_id, user_id)
        .await?
        .expect("a profile exists");
    assert_eq!(stored.ftp_watts, Some(280));
    let cp = stored
        .critical_power_watts
        .expect("CP survives a save that did not mention it");
    assert_eq!(cp.value, 290);
    assert_eq!(cp.provenance.kind, MeasurementKind::Measured);
    assert_eq!(cp.provenance.origin.as_deref(), Some("lab"));
    assert_eq!(cp.provenance.as_of, NaiveDate::from_ymd_opt(2026, 9, 20));
    assert_eq!(stored.w_prime_joules.map(|v| v.value), Some(18000));
    assert!(stored.critical_speed_mps.is_some());
    assert!(stored.d_prime_meters.is_some());
    Ok(())
}

/// A new value replaces the old one together with its provenance: a lab CP
/// after a modelled one is a measurement, and the old source does not linger.
#[tokio::test]
async fn a_new_value_replaces_its_provenance_too() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "critical_power_watts": 300,
            "measurement_kind": "estimated",
            "measurement_source": "intervals.icu",
            "measured_on": "2026-09-01"
        }),
    )
    .await?;
    set_physiology(
        &executor,
        user_id,
        &tenant_id,
        json!({"critical_power_watts": 305, "measurement_kind": "measured"}),
    )
    .await?;

    let cp = stored_profile(&executor, &tenant_id, user_id)
        .await?
        .and_then(|p| p.critical_power_watts)
        .expect("CP is stored");
    assert_eq!(cp.value, 305);
    assert_eq!(cp.provenance.kind, MeasurementKind::Measured);
    assert_eq!(cp.provenance.origin, None);
    assert_eq!(cp.provenance.as_of, None);
    Ok(())
}

#[tokio::test]
async fn a_critical_power_without_its_kind_is_refused_and_nothing_is_written() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({"critical_power_watts": 300, "measurement_source": "vekta"}),
    )
    .await?;
    assert!(error.contains("measurement_kind"), "{error}");
    assert!(
        stored_profile(&executor, &tenant_id, user_id)
            .await?
            .is_none(),
        "a refused call writes no row"
    );

    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({"critical_speed_mps": 4.2}),
    )
    .await?;
    assert!(error.contains("measurement_kind"), "{error}");
    Ok(())
}

/// A kind with no critical-power family value would read as qualifying the
/// FTP beside it, which the profile has no column for.
#[tokio::test]
async fn a_kind_with_nothing_to_describe_is_refused() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({"ftp_watts": 280, "measurement_kind": "measured"}),
    )
    .await?;
    assert!(error.contains("sets none of them"), "{error}");
    Ok(())
}

#[tokio::test]
async fn an_unknown_kind_is_refused_not_guessed() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({"critical_power_watts": 300, "measurement_kind": "provider"}),
    )
    .await?;
    assert!(error.contains("measured or estimated"), "{error}");
    Ok(())
}

#[tokio::test]
async fn a_future_or_malformed_date_is_refused() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let in_two_days = (Utc::now().date_naive() + chrono::Duration::days(2)).to_string();

    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({"critical_power_watts": 300, "measurement_kind": "estimated", "measured_on": in_two_days}),
    )
    .await?;
    assert!(error.contains("future"), "{error}");

    let error = rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({"critical_power_watts": 300, "measurement_kind": "estimated", "measured_on": "01/10/2026"}),
    )
    .await?;
    assert!(error.contains("YYYY-MM-DD"), "{error}");
    Ok(())
}

/// The bounds the Methodology note sources (CP 30–600 W, W′ 2000–60000 J,
/// CS 1.5–7.0 m/s, D′ 30–1000 m), each pinned at its edge: the bound itself
/// is accepted and the step past it refused. W′ given in kilojoules (21.5)
/// and a CS given in km/h are the slips they catch.
#[tokio::test]
async fn out_of_range_values_are_refused() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    for (field, value) in [
        ("critical_power_watts", json!(29)),
        ("critical_power_watts", json!(601)),
        ("w_prime_joules", json!(1999)),
        ("w_prime_joules", json!(60001)),
        ("w_prime_joules", json!(22)),
        ("critical_speed_mps", json!(1.49)),
        ("critical_speed_mps", json!(7.01)),
        ("critical_speed_mps", json!(15.0)),
        ("d_prime_meters", json!(29.9)),
        ("d_prime_meters", json!(1000.1)),
    ] {
        let mut params = json!({"measurement_kind": "estimated"});
        params[field] = value;
        let error = rejection(&executor, user_id, &tenant_id, params).await?;
        assert!(
            error.contains(&format!("{field} must be between")),
            "{field}: {error}"
        );
    }
    assert!(
        stored_profile(&executor, &tenant_id, user_id)
            .await?
            .is_none(),
        "no refused value was written"
    );

    for bounds in [
        json!({"critical_power_watts": 30, "w_prime_joules": 2000, "critical_speed_mps": 1.5, "d_prime_meters": 30.0}),
        json!({"critical_power_watts": 600, "w_prime_joules": 60000, "critical_speed_mps": 7.0, "d_prime_meters": 1000.0}),
    ] {
        let mut params = bounds.clone();
        params["measurement_kind"] = json!("estimated");
        set_physiology(&executor, user_id, &tenant_id, params).await?;
        let stored = stored_profile(&executor, &tenant_id, user_id)
            .await?
            .expect("a value at its bound is saved");
        assert_eq!(
            stored.critical_power_watts.map(|v| u64::from(v.value)),
            bounds["critical_power_watts"].as_u64()
        );
        assert_eq!(
            stored.w_prime_joules.map(|v| u64::from(v.value)),
            bounds["w_prime_joules"].as_u64()
        );
        assert_eq!(
            stored.critical_speed_mps.map(|v| v.value),
            bounds["critical_speed_mps"].as_f64()
        );
        assert_eq!(
            stored.d_prime_meters.map(|v| v.value),
            bounds["d_prime_meters"].as_f64()
        );
    }
    Ok(())
}
