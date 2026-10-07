// ABOUTME: The readiness rail's alerts through the platform path — each alert reads its own window of the athlete's series
// ABOUTME: Sleep and HRV read the trailing week out of the 28 days gathered; the strain baseline is dated from the day judged

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The rail gathers one span of history — the 28-day baseline — for every
//! signal, and the kernel windows each alert by date. Handed 28 days of
//! recovery readings, the week-long alerts once averaged all of them: a good
//! week behind a bad month read as a deficit, and a bad week behind a good
//! month read as fine. These tests seed the athlete's series in the database
//! and read the alerts where the agent reads them, in the plan's state block,
//! on one of Dravr's own surfaces: the wearable is a WHOOP strap, whose terms
//! keep its records off every external transport (carnet#766).

use anyhow::Result;
use chrono::{Duration, NaiveDate, Utc};
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::models::{
    ConnectionType, DailyTrainingState, DataSource, DeviceType, StoredRecoveryMetrics,
    StoredSleepSession, TenantId,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

/// Seconds in an hour.
const HOUR: i64 = 3600;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::Messaging),
    ))
}

/// An athlete with a connected wearable and the data source its rows carry.
async fn athlete_with_wearable(
    executor: &UniversalToolExecutor,
) -> Result<(Uuid, TenantId, String)> {
    let email = format!("alert_windows_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let repos = executor.resources.repos();
    let tenant = repos
        .tenants
        .get_all()
        .await?
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?
        .id;
    let tenant = TenantId::parse_str(&tenant.to_string())?;
    repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await?;
    // WHOOP connects, and health sync keeps its rows, only under its owner
    // authorization — also the consent to hand them to a model (carnet#726).
    repos
        .users
        .record_provider_terms(
            user_id,
            "whoop",
            provider_terms_version("whoop").expect("WHOOP carries a notice"),
        )
        .await?;
    let data_source = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: "whoop".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await?;
    Ok((user_id, tenant, data_source))
}

fn request(tool: &str, params: Value, user_id: Uuid, tenant: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant.to_string()),
    }
}

fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// A plan with a flavour, so the ladder has one to read; the week itself is
/// easy and never the point.
async fn save_plan(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant: TenantId,
) -> Result<()> {
    let week = (today() + Duration::days(7)).format("%Y-%m-%d").to_string();
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            json!({
                "coach_id": "endurance-coach",
                "outline": {
                    "goal_race": { "name": "Parkrun PB", "date": "2027-03-14", "discipline": "run_5k", "priority": "A" },
                    "strategy": "polarised build",
                    "flavour": { "id": "polarized-classic", "selected_by": "coach", "override_reason": "house style" },
                    "phases": [
                        { "kind": "build", "start": week, "weeks": 6, "intent": "two hard days, the rest easy",
                          "target_hours": 8.0, "hard_sessions_max": 2 }
                    ]
                },
                "weeks": [{ "week_start": week, "focus": "build week", "phase_index": 0, "days": [
                    {"date": week, "sport": "run", "workout": "easy", "duration_min": 60, "intensity": "Z2"}
                ]}]
            }),
            user_id,
            tenant,
        ))
        .await?;
    assert!(saved.success, "save failed: {:?}", saved.error);
    Ok(())
}

/// The alert labels the plan's state block reports.
async fn alerts(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant: TenantId,
) -> Result<Vec<String>> {
    let asked = executor
        .execute_tool(request(
            "get_training_plan",
            json!({"coach_id": "endurance-coach", "include_state": true}),
            user_id,
            tenant,
        ))
        .await?;
    let state = &asked.result.expect("a plan")["state"];
    assert!(
        state["readiness"].is_string(),
        "the ladder was read: {state}"
    );
    Ok(state["alerts"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}

/// One night of `hours` asleep, woken on the morning `days_ago` days back.
fn night(user_id: Uuid, data_source: &str, days_ago: i64, hours: i64) -> StoredSleepSession {
    let woke = today() - Duration::days(days_ago);
    let start = (woke - Duration::days(1))
        .and_hms_opt(23, 0, 0)
        .expect("a valid time")
        .and_utc();
    StoredSleepSession {
        id: format!("whoop-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: data_source.to_owned(),
        is_nap: false,
        start_datetime: start,
        end_datetime: start + Duration::hours(hours),
        total_sleep_seconds: Some(u32::try_from(hours * HOUR).expect("a night fits")),
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
        source_name: "whoop".to_owned(),
    }
}

/// One morning's rMSSD, `days_ago` days back.
fn morning(user_id: Uuid, data_source: &str, days_ago: i64, rmssd: f64) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: format!("whoop-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: data_source.to_owned(),
        date: today() - Duration::days(days_ago),
        recovery_score: None,
        readiness_score: None,
        hrv_ms: Some(rmssd),
        hrv_rmssd: Some(rmssd),
        resting_heart_rate: None,
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: "whoop".to_owned(),
        recorded_at: Utc::now(),
    }
}

/// Four weeks of nights: `this_week` hours a night for the last seven,
/// `before` hours a night for the three weeks behind them.
async fn seed_nights(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant: TenantId,
    data_source: &str,
    this_week: i64,
    before: i64,
) -> Result<()> {
    for days_ago in 0..28 {
        let hours = if days_ago < 7 { this_week } else { before };
        executor
            .resources
            .repos()
            .sleep
            .upsert_sleep_session(&tenant, &night(user_id, data_source, days_ago, hours))
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn a_short_week_of_sleep_is_a_deficit_a_good_month_does_not_hide() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant, data_source) = athlete_with_wearable(&executor).await?;
    save_plan(&executor, user_id, tenant).await?;

    // Five hours a night this week, nine for the three before: the 28-day
    // mean is eight hours, the week's is five.
    seed_nights(&executor, user_id, tenant, &data_source, 5, 9).await?;

    let raised = alerts(&executor, user_id, tenant).await?;
    assert!(
        raised.iter().any(|a| a == "sleep-deficit"),
        "the week slept under seven hours a night: {raised:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_bad_month_behind_a_good_week_is_not_this_weeks_deficit() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant, data_source) = athlete_with_wearable(&executor).await?;
    save_plan(&executor, user_id, tenant).await?;

    // Eight hours a night this week, four for the three before: the 28-day
    // mean is five hours, the week's is eight.
    seed_nights(&executor, user_id, tenant, &data_source, 8, 4).await?;

    let raised = alerts(&executor, user_id, tenant).await?;
    assert!(
        !raised.iter().any(|a| a == "sleep-deficit"),
        "a fortnight-old bad night is not this week's sleep deficit: {raised:?}"
    );
    Ok(())
}

#[tokio::test]
async fn hrv_trending_down_reads_the_trailing_week() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant, data_source) = athlete_with_wearable(&executor).await?;
    save_plan(&executor, user_id, tenant).await?;

    // rMSSD falls four milliseconds a day across the last week, from 74 to
    // 50, after three flat weeks at 30. Over the whole month the line rises;
    // over the week it falls twice as fast as the taxonomy's -2 ms/day.
    let repos = executor.resources.repos();
    for days_ago in 0..28_u8 {
        let rmssd = if days_ago < 7 {
            4.0_f64.mul_add(f64::from(days_ago), 50.0)
        } else {
            30.0
        };
        let reading = morning(user_id, &data_source, i64::from(days_ago), rmssd);
        repos
            .recovery
            .upsert_recovery_metrics(&tenant, &reading)
            .await?;
    }

    let raised = alerts(&executor, user_id, tenant).await?;
    assert!(
        raised.iter().any(|a| a == "hrv-trending-down"),
        "the week's rMSSD slope is -4 ms/day: {raised:?}"
    );
    Ok(())
}

#[tokio::test]
async fn the_strain_baseline_is_dated_from_the_day_judged() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant, _) = athlete_with_wearable(&executor).await?;
    save_plan(&executor, user_id, tenant).await?;

    // Twenty-eight days at 1000 behind a day at 1500: half again the
    // baseline, over the taxonomy's 1.2x. The kernel reads the baseline from
    // the days before the one judged, so today's own strain stays out of it.
    let states: Vec<DailyTrainingState> = (0..=28)
        .map(|days_ago| {
            let mut day = DailyTrainingState::zero(today() - Duration::days(days_ago));
            day.strain = Some(if days_ago == 0 { 1500.0 } else { 1000.0 });
            day
        })
        .collect();
    executor
        .resources
        .repos()
        .training_history
        .upsert_training_history_batch(tenant, user_id, &states)
        .await?;

    let raised = alerts(&executor, user_id, tenant).await?;
    assert!(
        raised.iter().any(|a| a == "strain-high"),
        "today's strain is 1.5x its 28-day baseline: {raised:?}"
    );
    Ok(())
}
