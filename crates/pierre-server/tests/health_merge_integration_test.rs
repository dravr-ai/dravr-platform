// ABOUTME: Pins that synced sleep and recovery from two sources reach the tools as one record
// ABOUTME: Persistence round-trips every stored metric; readers merge per night and per day

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An athlete wearing a WHOOP strap and a Garmin watch syncs one night and one
//! morning twice. Before the merge, HRV never survived the database (it was
//! written as a string and read back as nothing), stage durations and the
//! athlete's note were dropped, and the readiness reader summed both sources'
//! hours into one double night. These tests hold the stored metrics and the
//! merged read to their values.

use anyhow::Result;
use chrono::{Duration, NaiveDate, Utc};
use pierre_core::models::{
    ConnectionType, DataSource, DeviceType, StoredRecoveryMetrics, StoredSleepSession, TenantId,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn connected_user(executor: &UniversalToolExecutor) -> Result<(Uuid, TenantId)> {
    let email = format!("merge_test_{}@example.com", Uuid::new_v4());
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
    let tenant = TenantId::parse_str(&tenant.to_string())?;
    for provider in ["whoop", "garmin"] {
        executor
            .resources
            .repos()
            .provider_connections
            .register_connection(user_id, tenant, provider, &ConnectionType::OAuth, None)
            .await?;
    }
    Ok((user_id, tenant))
}

async fn data_source(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant: &TenantId,
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

fn request(tool: &str, params: Value, user_id: Uuid, tenant: &TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

fn sleep(
    user_id: Uuid,
    source: &str,
    ds: &str,
    hours_ago: i64,
    span_hours: i64,
) -> StoredSleepSession {
    let start = Utc::now() - Duration::hours(hours_ago);
    StoredSleepSession {
        id: format!("{source}-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: ds.to_owned(),
        is_nap: false,
        start_datetime: start,
        end_datetime: start + Duration::hours(span_hours),
        total_sleep_seconds: None,
        deep_sleep_seconds: None,
        light_sleep_seconds: None,
        rem_sleep_seconds: None,
        awake_seconds: None,
        sleep_efficiency: None,
        avg_heart_rate: None,
        min_heart_rate: None,
        avg_hrv: None,
        sleep_score: None,
        stages: Vec::new(),
        source_name: source.to_owned(),
    }
}

fn recovery(user_id: Uuid, source: &str, ds: &str, date: NaiveDate) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: format!("{source}-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: ds.to_owned(),
        date,
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: None,
        resting_heart_rate: None,
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: source.to_owned(),
        recorded_at: Utc::now(),
    }
}

#[tokio::test]
async fn every_stored_recovery_and_sleep_metric_survives_the_database() -> Result<()> {
    let executor = executor().await?;
    let (user_id, tenant) = connected_user(&executor).await?;
    let ds = data_source(&executor, user_id, &tenant, "whoop").await?;
    let repos = executor.resources.repos();
    let today = Utc::now().date_naive();

    let mut day = recovery(user_id, "whoop", &ds, today);
    day.hrv_ms = Some(58.4);
    day.hrv_rmssd = Some(58.4);
    day.body_battery = Some(41);
    day.spo2 = Some(97.5);
    day.daily_strain = Some(14.2);
    day.athlete_note = Some("legs heavy".to_owned());
    repos
        .recovery
        .upsert_recovery_metrics(&tenant, &day)
        .await?;

    let mut night = sleep(user_id, "whoop", &ds, 10, 8);
    night.total_sleep_seconds = Some(26_000);
    night.deep_sleep_seconds = Some(5_400);
    night.rem_sleep_seconds = Some(6_100);
    night.awake_seconds = Some(1_200);
    night.min_heart_rate = Some(44);
    night.avg_heart_rate = Some(51.5);
    repos.sleep.upsert_sleep_session(&tenant, &night).await?;

    let window = (
        Utc::now() - Duration::days(2),
        Utc::now() + Duration::hours(1),
    );
    let read = repos
        .recovery
        .get_recovery_metrics(user_id, &tenant, window.0, window.1)
        .await?;
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].hrv_ms, Some(58.4));
    assert_eq!(read[0].hrv_rmssd, Some(58.4));
    assert_eq!(read[0].body_battery, Some(41));
    assert_eq!(read[0].spo2, Some(97.5));
    assert_eq!(read[0].daily_strain, Some(14.2));
    assert_eq!(read[0].athlete_note.as_deref(), Some("legs heavy"));

    let nights = repos
        .sleep
        .get_sleep_sessions(user_id, &tenant, window.0, window.1)
        .await?;
    assert_eq!(nights.len(), 1);
    assert_eq!(nights[0].total_sleep_seconds, Some(26_000));
    assert_eq!(nights[0].deep_sleep_seconds, Some(5_400));
    assert_eq!(nights[0].rem_sleep_seconds, Some(6_100));
    assert_eq!(nights[0].awake_seconds, Some(1_200));
    assert_eq!(nights[0].min_heart_rate, Some(44));
    assert_eq!(nights[0].avg_heart_rate, Some(51.5));
    // Absent metrics read back absent, never as a measured zero.
    assert_eq!(nights[0].sleep_efficiency, None);
    assert_eq!(nights[0].light_sleep_seconds, None);
    Ok(())
}

#[tokio::test]
async fn one_night_and_one_morning_from_two_sources_reach_the_tools_once() -> Result<()> {
    let executor = executor().await?;
    let (user_id, tenant) = connected_user(&executor).await?;
    let whoop_ds = data_source(&executor, user_id, &tenant, "whoop").await?;
    let garmin_ds = data_source(&executor, user_id, &tenant, "garmin").await?;
    let repos = executor.resources.repos();

    let mut whoop_night = sleep(user_id, "whoop", &whoop_ds, 10, 8);
    whoop_night.total_sleep_seconds = Some(26_000);
    whoop_night.sleep_score = Some(84);
    whoop_night.sleep_efficiency = Some(91.0);
    let mut garmin_night = sleep(user_id, "garmin", &garmin_ds, 9, 7);
    garmin_night.total_sleep_seconds = Some(24_500);
    garmin_night.deep_sleep_seconds = Some(5_400);
    repos
        .sleep
        .upsert_sleep_session(&tenant, &whoop_night)
        .await?;
    repos
        .sleep
        .upsert_sleep_session(&tenant, &garmin_night)
        .await?;

    let morning = whoop_night.end_datetime.date_naive();
    let mut whoop_day = recovery(user_id, "whoop", &whoop_ds, morning);
    whoop_day.recovery_score = Some(71);
    whoop_day.hrv_rmssd = Some(62.0);
    let mut garmin_day = recovery(user_id, "garmin", &garmin_ds, morning);
    garmin_day.recovery_score = Some(40);
    garmin_day.body_battery = Some(40);
    repos
        .recovery
        .upsert_recovery_metrics(&tenant, &whoop_day)
        .await?;
    repos
        .recovery
        .upsert_recovery_metrics(&tenant, &garmin_day)
        .await?;

    let sessions = executor
        .execute_tool(request("get_sleep_sessions", json!({}), user_id, &tenant))
        .await?;
    assert!(sessions.success, "{:?}", sessions.error);
    let sessions = sessions.result.unwrap();
    assert_eq!(sessions["count"], 1, "{sessions:#}");
    let night = &sessions["sessions"][0];
    assert_eq!(night["source_name"], "whoop");
    assert_eq!(night["total_sleep_seconds"], 26_000);
    assert_eq!(night["deep_sleep_seconds"], 5_400);
    assert_eq!(night["sources"], json!(["whoop", "garmin"]));

    let metrics = executor
        .execute_tool(request("get_recovery_metrics", json!({}), user_id, &tenant))
        .await?;
    assert!(metrics.success, "{:?}", metrics.error);
    let metrics = metrics.result.unwrap();
    assert_eq!(metrics["count"], 1, "{metrics:#}");
    let day = &metrics["metrics"][0];
    // WHOOP's recovery score stands over Garmin's Body Battery proxy.
    assert_eq!(day["recovery_score"], 71);
    assert_eq!(day["hrv_rmssd"], 62.0);
    assert_eq!(day["body_battery"], 40);
    assert_eq!(day["filled"][0]["metric"], "body_battery");
    assert_eq!(day["filled"][0]["source"], "garmin");

    // The scoring tools read the same merged night: WHOOP's duration and
    // efficiency, Garmin's deep sleep, and HRV from the morning's recovery.
    let quality = executor
        .execute_tool(request(
            "analyze_sleep_quality",
            json!({}),
            user_id,
            &tenant,
        ))
        .await?;
    assert!(quality.success, "{:?}", quality.error);
    let quality = quality.result.unwrap();
    let text = quality.to_string();
    assert!(
        text.contains("7.2"),
        "duration 26 000 s ≈ 7.2 h: {quality:#}"
    );
    Ok(())
}

#[tokio::test]
async fn a_named_sleep_provider_narrows_the_scoring_tools_to_that_source() -> Result<()> {
    let executor = executor().await?;
    let (user_id, tenant) = connected_user(&executor).await?;
    let garmin_ds = data_source(&executor, user_id, &tenant, "garmin").await?;
    let mut garmin_night = sleep(user_id, "garmin", &garmin_ds, 9, 7);
    garmin_night.total_sleep_seconds = Some(24_500);
    executor
        .resources
        .repos()
        .sleep
        .upsert_sleep_session(&tenant, &garmin_night)
        .await?;

    let whoop_only = executor
        .execute_tool(request(
            "analyze_sleep_quality",
            json!({ "sleep_provider": "whoop" }),
            user_id,
            &tenant,
        ))
        .await?;
    assert!(!whoop_only.success);
    assert!(
        whoop_only
            .error
            .as_deref()
            .is_some_and(|e| e.contains("No sleep synced from whoop")),
        "{:?}",
        whoop_only.error
    );

    let garmin_only = executor
        .execute_tool(request(
            "analyze_sleep_quality",
            json!({ "sleep_provider": "garmin" }),
            user_id,
            &tenant,
        ))
        .await?;
    assert!(garmin_only.success, "{:?}", garmin_only.error);
    Ok(())
}
