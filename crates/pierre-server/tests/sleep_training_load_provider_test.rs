// ABOUTME: Pins that the recovery tools read training load from the provider every activity tool elects
// ABOUTME: No hard-coded priority list and no silent Strava fallback for an athlete who never connected it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The sleep and recovery tools used to pick their activity provider from a
//! private priority list gated on a valid OAuth token, falling back to
//! "strava" when none matched. An Intervals.icu, TrainingPeaks or
//! scrape-connected Garmin athlete was therefore never selected and was told
//! to reconnect a Strava account they never had (carnet#575). They now elect
//! the provider through `resolve_provider_for_request`, as every activity tool
//! does.

use anyhow::Result;
use chrono::{Duration, Utc};
use dravr_cageux::training_load::TrainingLoadCalculator;
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::slice;
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    // The athlete's own app: the rides are Strava's, which no external
    // transport serves (carnet#765).
    Ok(Arc::new(
        UniversalToolExecutor::new(resources)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp),
    ))
}

async fn user_with_tenant(executor: &UniversalToolExecutor) -> Result<(Uuid, TenantId)> {
    let email = format!("recovery_provider_{}@example.com", Uuid::new_v4());
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
        .ok_or_else(|| anyhow::anyhow!("user should have tenant"))?
        .id;
    Ok((user_id, TenantId::parse_str(&tenant.to_string())?))
}

fn request(tool: &str, user_id: Uuid, tenant: &TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: json!({}),
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant.to_string()),
    }
}

fn auth_tag(metadata: Option<&HashMap<String, Value>>) -> Option<String> {
    metadata?
        .get("auth_required_provider")?
        .as_str()
        .map(str::to_owned)
}

/// Every tool that computes training load elects the same provider.
const TRAINING_LOAD_TOOLS: [&str; 3] = [
    "calculate_recovery_score",
    "suggest_rest_day",
    "optimize_sleep_schedule",
];

#[tokio::test]
async fn an_athlete_with_no_connection_gets_the_reconnect_refusal_not_a_strava_fetch() -> Result<()>
{
    let executor = executor().await?;
    let (user_id, tenant) = user_with_tenant(&executor).await?;

    for tool in TRAINING_LOAD_TOOLS {
        let response = executor
            .execute_tool(request(tool, user_id, &tenant))
            .await?;
        assert!(!response.success, "{tool}: nothing is connected");
        let error = response.error.clone().unwrap_or_default();
        assert!(
            error.starts_with("No fitness provider connected"),
            "{tool}: an athlete with no connection is asked to connect one, not sent to a Strava token they never had: {error}"
        );
        assert_eq!(
            auth_tag(response.metadata.as_ref()).as_deref(),
            Some("sciotte"),
            "{tool}: the canonical no-provider refusal carries the reconnect tag: {error}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_athletes_own_provider_is_elected_whatever_its_name() -> Result<()> {
    let executor = executor().await?;
    let (user_id, tenant) = user_with_tenant(&executor).await?;
    executor
        .resources
        .repos()
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            "intervals_icu",
            &ConnectionType::OAuth,
            None,
        )
        .await?;

    for tool in TRAINING_LOAD_TOOLS {
        let response = executor
            .execute_tool(request(tool, user_id, &tenant))
            .await?;
        let error = response.error.clone().unwrap_or_default();
        // The connection row has no credential behind it, so the read fails —
        // but it fails on the athlete's own provider.
        assert!(!response.success, "{tool}: no credential is stored");
        assert!(
            error.contains("intervals_icu"),
            "{tool}: the elected provider is the athlete's Intervals.icu connection: {error}"
        );
        assert!(
            !error.to_lowercase().contains("strava"),
            "{tool}: no Strava fallback: {error}"
        );
    }
    Ok(())
}

/// The recovery tools score each session against the thresholds saved with
/// `set_physiology`, the reading `analyze_training_load` gives.
///
/// They used to read an undeclared `user_config` argument that no
/// schema-following caller could send, and then the configuration's session
/// overrides, which `set_physiology` never writes — so an FTP the athlete
/// saved never reached their TSB: a power-only ride scored the same with and
/// without one. The session here is served from the activity cache (the
/// connection has no credential, so the live read fails and the cached rows
/// are served), and the tool's CTL must be the calculator's own for that ride
/// at the saved FTP.
#[tokio::test]
async fn the_recovery_tools_score_sessions_against_the_saved_thresholds() -> Result<()> {
    let executor = executor().await?;
    let (user_id, tenant) = user_with_tenant(&executor).await?;
    let repos = executor.resources.repos();
    repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await?;
    let ride = ActivityBuilder::new(
        "threshold-ride".to_owned(),
        "Threshold ride".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(36_000.0)
    .average_power(250)
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", slice::from_ref(&ride))
        .await?;

    let ctl_of = |response: UniversalResponse| -> f64 {
        assert!(response.success, "{:?}", response.error);
        response.result.unwrap()["training_load"]["ctl"]
            .as_f64()
            .expect("the recovery score carries the CTL behind it")
    };
    let without_thresholds = ctl_of(
        executor
            .execute_tool(request("calculate_recovery_score", user_id, &tenant))
            .await?,
    );

    let saved = executor
        .execute_tool(UniversalRequest {
            parameters: json!({"ftp_watts": 250}),
            ..request("set_physiology", user_id, &tenant)
        })
        .await?;
    assert!(saved.success, "set_physiology: {:?}", saved.error);
    let with_ftp = ctl_of(
        executor
            .execute_tool(request("calculate_recovery_score", user_id, &tenant))
            .await?,
    );

    let expected = TrainingLoadCalculator::from_config(
        executor.cageux_config().algorithms.clone(),
        Utc::now().date_naive(),
    )
    .calculate_training_load(&[ride], Some(250.0), None, None, None, None)?;
    assert!(
        (with_ftp - expected.ctl).abs() < 1e-9,
        "the ride is scored at the saved FTP: tool {with_ftp}, calculator {}",
        expected.ctl
    );
    assert!(
        (with_ftp - without_thresholds).abs() > 1e-6,
        "saving an FTP changes the score of a power-only ride ({without_thresholds} both ways)"
    );
    Ok(())
}
