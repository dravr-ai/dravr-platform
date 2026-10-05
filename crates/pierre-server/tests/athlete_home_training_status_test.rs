// ABOUTME: GET /api/me/training-status — form today as a share of fitness, its band, the form trend, load ratio and recovery days
// ABOUTME: Pins the figures against the rollup the agent reads, the honest empty answer on a thin history, tenancy, and that the read stores nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home training-status suite.
//!
//! The route's figures are checked two ways. `status_from_rows` is handed
//! rows with known loads, so each band edge and each recovery-day step is
//! asserted as a concrete value. The route itself is then checked against
//! `compute_and_persist_history` — the rollup `get_training_history` serves —
//! over the same stored activities, so Home and a conversation cannot quote
//! different numbers for the same day.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "tools-data")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use pierre_core::transport::TransportPolicy;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{Duration, NaiveDate, Utc};
use dravr_cageux::training_load::FormBand;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::{
    ActivityBuilder, ConnectionType, DailyTrainingState, SportType, TenantId, User,
    UserPhysiologicalProfile,
};
use pierre_fitness_compute::training_history_compute::{
    ACWR_ACUTE_DAYS, ACWR_CHRONIC_DAYS, CTL_WINDOW_DAYS,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::training_status::status_from_rows;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::{
    compute_and_persist_history, fetch_history_rows,
};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

/// The backend the seeded rows are written under.
const BACKEND: &str = "sciotte";

struct Athlete {
    user: User,
    user_id: Uuid,
    tenant: TenantId,
}

/// A user with real physiology and nothing cached; `connected` registers the
/// scrape backend the seeded rows are read under.
async fn athlete(resources: &Arc<ServerContext>, connected: bool) -> Athlete {
    let email = format!("status-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .expect("test user");
    if connected {
        resources
            .common
            .repos
            .provider_connections
            .register_connection(user_id, tenant, BACKEND, &ConnectionType::OAuth, None)
            .await
            .unwrap();
    }
    // Real physiology so TSS is HR-derived and the chronic load is non-zero.
    let profile = UserPhysiologicalProfile {
        user_id,
        vo2_max: Some(52.0),
        resting_hr: Some(50),
        max_hr: Some(190),
        lactate_threshold_percentage: Some(0.85),
        threshold_hr: None,
        age: Some(34),
        weight: Some(72.0),
        fitness_level: FitnessLevel::Advanced,
        primary_sport: SportType::Run,
        training_experience_years: Some(10),
        ftp_watts: Some(280),
        threshold_pace_sec_per_km: Some(225.0),
        hr_zones: None,
        power_zones: None,
        critical_power_watts: None,
        w_prime_joules: None,
        critical_speed_mps: None,
        d_prime_meters: None,
        transport_policy: TransportPolicy::AnyTransport,
    };
    resources
        .common
        .repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant, user_id, &profile)
        .await
        .unwrap();
    Athlete {
        user,
        user_id,
        tenant,
    }
}

/// Cache an hour's run on every day from `oldest_days_ago` up to today.
async fn seed_daily_runs(resources: &Arc<ServerContext>, athlete: &Athlete, oldest_days_ago: i64) {
    let activities: Vec<_> = (0..=oldest_days_ago)
        .map(|days_ago| {
            ActivityBuilder::new(
                format!("cached-{days_ago}"),
                format!("run {days_ago}d ago"),
                SportType::Run,
                Utc::now() - Duration::days(days_ago),
                3_600,
                BACKEND.to_owned(),
            )
            .distance_meters(12_000.0)
            .average_heart_rate(155)
            .build()
        })
        .collect();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, BACKEND, &activities)
        .await
        .unwrap();
}

async fn status_json(resources: &Arc<ServerContext>, token: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::builder().uri("/api/me/training-status");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn status_ok(resources: &Arc<ServerContext>, athlete: &Athlete) -> Value {
    let token = common::generate_test_token(resources, &athlete.user).await;
    let (status, body) = status_json(resources, Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn stored_rows(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<DailyTrainingState> {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    fetch_history_rows(&runtime.data(), athlete.tenant, athlete.user_id, from, to)
        .await
        .unwrap()
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// A day whose form is `tsb` on a fitness of `form_ctl`.
fn row(date: NaiveDate, tsb: f64, form_ctl: f64, acwr: Option<f64>) -> DailyTrainingState {
    DailyTrainingState {
        tsb,
        form_ctl,
        ctl: form_ctl,
        atl: form_ctl - tsb,
        acwr,
        ..DailyTrainingState::zero(date)
    }
}

fn day(n: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, n).unwrap()
}

/// Each band is read off form as a share of fitness, and the recovery days
/// are cageux's steps past the deep-fatigue edge.
#[test]
fn bands_and_recovery_days_follow_form_as_a_share_of_fitness() {
    // (tsb on a fitness of 100, band, recovery days)
    let cases = [
        (-55.0, FormBand::DeepFatigue, 3),
        (-45.0, FormBand::DeepFatigue, 2),
        (-35.0, FormBand::DeepFatigue, 1),
        (-25.0, FormBand::HeavyBlock, 0),
        (-15.0, FormBand::Productive, 0),
        (0.0, FormBand::Balanced, 0),
        (10.0, FormBand::Fresh, 0),
        (25.0, FormBand::Detraining, 0),
    ];
    for (tsb, band, recovery_days) in cases {
        let status = status_from_rows(day(20), &[row(day(20), tsb, 100.0, None)]);
        let form = status.form.expect("today's row gives a form reading");
        assert_eq!(form.band, band, "tsb {tsb} on fitness 100");
        assert_eq!(form.pct_of_fitness, Some(tsb), "tsb {tsb} on fitness 100");
        assert_eq!(status.recovery_days, Some(recovery_days), "tsb {tsb}");
    }
}

/// The same balance on a different fitness is a different band: -30 is the
/// deep end of a block at fitness 120 and the deepest band at fitness 60.
#[test]
fn the_same_balance_bands_differently_on_a_different_fitness() {
    let strong = status_from_rows(day(20), &[row(day(20), -30.0, 120.0, None)]);
    let modest = status_from_rows(day(20), &[row(day(20), -30.0, 60.0, None)]);
    let strong = strong.form.unwrap();
    let modest = modest.form.unwrap();
    assert_eq!(strong.band, FormBand::HeavyBlock);
    assert_eq!(strong.pct_of_fitness, Some(-25.0));
    assert_eq!(modest.band, FormBand::DeepFatigue);
    assert_eq!(modest.pct_of_fitness, Some(-50.0));
}

/// With no chronic base the reading says so: no percentage, no recovery
/// prescription, and never a band derived from the raw balance.
#[test]
fn a_fitness_too_low_to_scale_form_is_insufficient_history() {
    let status = status_from_rows(day(20), &[row(day(20), -8.0, 10.0, Some(1.6))]);
    let body = serde_json::to_value(&status).unwrap();
    assert_eq!(
        body["form"],
        json!({ "band": "insufficient_history", "pct_of_fitness": null })
    );
    assert_eq!(body["recovery_days"], Value::Null);
    assert_eq!(
        body["trend"],
        json!([{ "date": "2026-09-20", "band": "insufficient_history", "pct_of_fitness": null }])
    );
}

/// The trend carries every vouched day oldest first, and the load ratio is
/// today's, with the windows it was measured over.
#[test]
fn the_trend_and_load_ratio_come_from_the_rows_as_given() {
    let rows = [
        row(day(18), -12.0, 100.0, Some(0.9)),
        row(day(19), 6.0, 100.0, Some(1.1)),
        row(day(20), -22.4, 100.0, Some(1.37)),
    ];
    let body = serde_json::to_value(status_from_rows(day(20), &rows)).unwrap();
    assert_eq!(body["today"], "2026-09-20");
    assert_eq!(
        body["trend"],
        json!([
            { "date": "2026-09-18", "band": "productive", "pct_of_fitness": -12.0 },
            { "date": "2026-09-19", "band": "fresh", "pct_of_fitness": 6.0 },
            { "date": "2026-09-20", "band": "heavy_block", "pct_of_fitness": -22.0 },
        ])
    );
    assert_eq!(
        body["load_ratio"],
        json!({ "ratio": 1.37, "acute_days": ACWR_ACUTE_DAYS, "chronic_days": ACWR_CHRONIC_DAYS })
    );
}

/// A series that stops before today is not passed off as today's reading.
#[test]
fn rows_that_stop_short_of_today_give_no_reading() {
    let rows = [
        row(day(18), -12.0, 100.0, Some(0.9)),
        row(day(19), 6.0, 100.0, Some(1.1)),
    ];
    let body = serde_json::to_value(status_from_rows(day(20), &rows)).unwrap();
    assert_eq!(
        body,
        json!({
            "today": "2026-09-20",
            "form": null,
            "trend": [],
            "load_ratio": null,
            "recovery_days": null,
        })
    );
}

/// A deep cache answers the whole chronic window, with the figures the
/// agent's rollup holds for the same days — and the read stores nothing.
#[tokio::test]
async fn a_deep_history_answers_the_rollups_own_figures_and_stores_nothing() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, true).await;
    seed_daily_runs(&resources, &athlete, 200).await;

    let body = status_ok(&resources, &athlete).await;
    let today = Utc::now().date_naive();
    let from = today - Duration::days(CTL_WINDOW_DAYS);
    assert_eq!(body["today"], ymd(today));

    assert!(
        stored_rows(&resources, &athlete, from, today)
            .await
            .is_empty(),
        "a Home read must not write the rollup"
    );

    // The rollup the agent reads, over the same stored activities.
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    compute_and_persist_history(&runtime, athlete.tenant, athlete.user_id, from, today)
        .await
        .unwrap();
    let rows = stored_rows(&resources, &athlete, from, today).await;
    assert_eq!(rows.len(), usize::try_from(CTL_WINDOW_DAYS).unwrap() + 1);

    let trend = body["trend"].as_array().unwrap();
    assert_eq!(trend.len(), rows.len(), "one point per day of the window");
    for (point, row) in trend.iter().zip(&rows) {
        let reading = row.form_reading();
        assert_eq!(point["date"], ymd(row.date));
        assert_eq!(point["band"], serde_json::to_value(reading.band).unwrap());
        assert_eq!(
            point["pct_of_fitness"].as_f64(),
            reading.form_pct.map(f64::round)
        );
    }

    let last = rows.last().unwrap();
    let reading = last.form_reading();
    assert!(
        last.form_ctl > 20.0,
        "a daily hour must build a chronic base form can be scaled against, got {}",
        last.form_ctl
    );
    // A steady daily hour for 200 days is a flat load: fatigue sits level
    // with fitness, the ratio at 1, and nothing calls for a lighter day.
    assert_eq!(body["form"]["band"], "balanced");
    assert_eq!(
        body["form"]["pct_of_fitness"].as_f64(),
        reading.form_pct.map(f64::round)
    );
    assert_eq!(body["recovery_days"], 0);
    let ratio = body["load_ratio"]["ratio"].as_f64().unwrap();
    assert!((ratio - last.acwr.unwrap()).abs() < 1e-9);
    assert!((ratio - 1.0).abs() < 0.01, "flat load, got ratio {ratio}");
    assert_eq!(body["load_ratio"]["acute_days"], ACWR_ACUTE_DAYS);
    assert_eq!(body["load_ratio"]["chronic_days"], ACWR_CHRONIC_DAYS);
}

/// A history too shallow to warm today's chronic load answers nothing rather
/// than a low reading built from a zero seed.
#[tokio::test]
async fn a_shallow_history_answers_no_reading() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, true).await;
    seed_daily_runs(&resources, &athlete, 30).await;

    let body = status_ok(&resources, &athlete).await;
    assert_eq!(body["form"], Value::Null);
    assert_eq!(body["trend"], json!([]));
    assert_eq!(body["load_ratio"], Value::Null);
    assert_eq!(body["recovery_days"], Value::Null);
}

/// An athlete with no provider connected is answered the same empty status,
/// not refused: Home has its own connect prompt.
#[tokio::test]
async fn no_connected_provider_answers_no_reading() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete(&resources, false).await;

    let body = status_ok(&resources, &athlete).await;
    assert_eq!(body["form"], Value::Null);
    assert_eq!(body["trend"], json!([]));
}

/// One athlete's history never answers another's read.
#[tokio::test]
async fn another_athletes_history_is_not_read() {
    let resources = common::create_test_server_resources().await.unwrap();
    let trained = athlete(&resources, true).await;
    seed_daily_runs(&resources, &trained, 200).await;
    let other = athlete(&resources, true).await;

    let own = status_ok(&resources, &trained).await;
    assert_eq!(own["form"]["band"], "balanced");
    let theirs = status_ok(&resources, &other).await;
    assert_eq!(theirs["form"], Value::Null);
    assert_eq!(theirs["trend"], json!([]));
}

#[tokio::test]
async fn an_unauthenticated_read_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (status, _) = status_json(&resources, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
