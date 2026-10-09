// ABOUTME: Uploaded .fit activities count toward training status and the persisted training-load rollup
// ABOUTME: An uploads-only athlete gets a status; a ride uploaded and also synced by Strava scores once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#818 meets carnet#836. The training-load series is read from every
//! connection's cached rows, merged once per workout; an upload is cached
//! under the `upload` key with no connection behind it, so it must still be
//! read — on its own for an athlete who only uploads, and merged with a
//! provider's copy of the same ride for one who also syncs.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "tools-data")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, DailyTrainingState, SportType, TenantId,
    UserPhysiologicalProfile,
};
use pierre_core::transport::TransportPolicy;
use pierre_fitness_compute::training_history_compute::CTL_WINDOW_DAYS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_test_support::fit::FitWorkout;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::{
    compute_and_persist_history, fetch_history_rows,
};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

const STRAVA: &str = "strava";

/// Days of history behind today: past the 72-day warm-up of the whole
/// 42-day window.
const DEEP_DAYS: i64 = 130;

/// Length of every uploaded ride, in seconds.
const RIDE_SECONDS: u32 = 600;

struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

/// A user with real physiology, so every session scores a non-zero load.
async fn athlete(resources: &Arc<ServerContext>) -> Athlete {
    let email = format!("uploads-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .expect("test user");
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
        primary_sport: SportType::Ride,
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
    let token = common::generate_test_token(resources, &user).await;
    Athlete {
        user_id,
        tenant,
        token,
    }
}

/// 07:00 UTC `days_ago` days before today.
fn morning(days_ago: i64) -> DateTime<Utc> {
    let day = Utc::now().date_naive() - Duration::days(days_ago);
    Utc.from_utc_datetime(&day.and_time(NaiveTime::from_hms_opt(7, 0, 0).unwrap()))
}

/// Strava's copy of the ride uploaded `days_ago`: the same workout, its start
/// twenty seconds later, as a provider's sync records it.
fn strava_copy(days_ago: i64) -> Activity {
    let workout = FitWorkout::ride(morning(days_ago), RIDE_SECONDS);
    ActivityBuilder::new(
        format!("strava-{days_ago}"),
        "Morning Ride".to_owned(),
        SportType::Ride,
        morning(days_ago) + Duration::seconds(20),
        u64::from(RIDE_SECONDS),
        STRAVA.to_owned(),
    )
    .distance_meters(workout.distance_meters())
    .start_latitude(45.5)
    .start_longitude(-73.6)
    .average_heart_rate(150)
    .build()
}

/// A Strava ride on a day nothing was uploaded.
fn strava_ride(days_ago: i64) -> Activity {
    ActivityBuilder::new(
        format!("strava-{days_ago}"),
        "Morning Ride".to_owned(),
        SportType::Ride,
        morning(days_ago),
        3_600,
        STRAVA.to_owned(),
    )
    .distance_meters(30_000.0)
    .average_heart_rate(150)
    .build()
}

impl Athlete {
    async fn send(&self, resources: &Arc<ServerContext>, request: Request<Body>) -> Value {
        let response = athlete_home_routes()
            .with_state(Arc::clone(resources))
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        assert!(status.is_success(), "{status}: {body}");
        body
    }

    async fn delete_upload(&self, resources: &Arc<ServerContext>, id: &str) {
        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/api/me/activities/upload/{id}"))
            .header("authorization", format!("Bearer {}", self.token))
            .body(Body::empty())
            .unwrap();
        self.send(resources, request).await;
    }

    /// Upload the ride of `days_ago`; the id of the activity it became.
    async fn upload_ride(&self, resources: &Arc<ServerContext>, days_ago: i64) -> String {
        let file = FitWorkout::ride(morning(days_ago), RIDE_SECONDS).encode();
        let request = Request::builder()
            .method("POST")
            .uri("/api/me/activities/upload")
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/octet-stream")
            .body(Body::from(file))
            .unwrap();
        let body = self.send(resources, request).await;
        body["activities"][0]["id"].as_str().unwrap().to_owned()
    }

    async fn status(&self, resources: &Arc<ServerContext>) -> Value {
        let request = Request::builder()
            .uri("/api/me/training-status")
            .header("authorization", format!("Bearer {}", self.token))
            .body(Body::empty())
            .unwrap();
        self.send(resources, request).await
    }

    async fn connect_strava(&self, resources: &Arc<ServerContext>, rows: &[Activity]) {
        let repos = &resources.common.repos;
        repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                STRAVA,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
        repos
            .activity_cache
            .upsert_activities(self.user_id, &self.tenant, STRAVA, rows)
            .await
            .unwrap();
    }

    /// The persisted rollup over the chronic window ending today, computed as
    /// `compute_training_history` computes it.
    async fn rollup(&self, resources: &Arc<ServerContext>) -> Vec<DailyTrainingState> {
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let (from, to) = window();
        compute_and_persist_history(&runtime, self.tenant, self.user_id, from, to)
            .await
            .expect("an athlete with uploads has a history to compute");
        self.stored_rollup(resources).await
    }

    /// The persisted rollup as it stands, without computing it.
    async fn stored_rollup(&self, resources: &Arc<ServerContext>) -> Vec<DailyTrainingState> {
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let (from, to) = window();
        fetch_history_rows(&runtime.data(), self.tenant, self.user_id, from, to)
            .await
            .unwrap()
    }
}

fn window() -> (chrono::NaiveDate, chrono::NaiveDate) {
    let to = Utc::now().date_naive();
    (to - Duration::days(CTL_WINDOW_DAYS), to)
}

fn load_on(rollup: &[DailyTrainingState], days_ago: i64) -> f64 {
    let day = Utc::now().date_naive() - Duration::days(days_ago);
    rollup
        .iter()
        .find(|row| row.date == day)
        .unwrap_or_else(|| panic!("the rollup covers {day}"))
        .daily_load
}

/// Upload days: every other day across the deep history, so a ride sits on
/// the first day of the warm-up read itself.
fn upload_days() -> impl Iterator<Item = i64> {
    (2..=DEEP_DAYS).filter(|d| d % 2 == 0)
}

#[tokio::test]
async fn an_athlete_who_only_uploads_gets_a_training_status_and_a_rollup() {
    let resources = common::create_test_server_resources().await.unwrap();
    let rider = athlete(&resources).await;
    for days_ago in upload_days() {
        rider.upload_ride(&resources, days_ago).await;
    }

    let status = rider.status(&resources).await;
    assert!(
        !status["form"].is_null(),
        "months of uploaded rides warm today's form with no provider connected: {status}"
    );
    assert!(!status["trend"].as_array().unwrap().is_empty(), "{status}");

    assert!(
        !rider.stored_rollup(&resources).await.is_empty(),
        "an upload recomputes the persisted rollup, as a provider capture does"
    );
    let rollup = rider.rollup(&resources).await;
    assert_eq!(
        rollup.len(),
        usize::try_from(CTL_WINDOW_DAYS).unwrap() + 1,
        "the persisted rollup is warmed for the whole window"
    );
    assert!(
        load_on(&rollup, 2) > 0.0,
        "the ride uploaded two days ago adds load"
    );
}

#[tokio::test]
async fn a_ride_uploaded_and_synced_by_strava_counts_once() {
    let resources = common::create_test_server_resources().await.unwrap();
    // The deep history, on Strava, on days nothing was uploaded.
    let history: Vec<Activity> = (2..=DEEP_DAYS)
        .filter(|d| d % 2 == 0)
        .map(strava_ride)
        .collect();

    // Both copies: the ride uploaded yesterday, then Strava's sync of it.
    let both = athlete(&resources).await;
    both.upload_ride(&resources, 1).await;
    let mut synced = history.clone();
    synced.push(strava_copy(1));
    both.connect_strava(&resources, &synced).await;

    // Each copy alone.
    let uploaded_only = athlete(&resources).await;
    uploaded_only.upload_ride(&resources, 1).await;
    uploaded_only.connect_strava(&resources, &history).await;
    let strava_only = athlete(&resources).await;
    strava_only.connect_strava(&resources, &synced).await;

    let both_load = load_on(&both.rollup(&resources).await, 1);
    let upload_load = load_on(&uploaded_only.rollup(&resources).await, 1);
    let strava_load = load_on(&strava_only.rollup(&resources).await, 1);
    assert!(upload_load > 0.0 && strava_load > 0.0);
    assert!(
        (both_load - upload_load).abs() < 1e-9 || (both_load - strava_load).abs() < 1e-9,
        "the merged day scores one copy ({both_load}), the upload's ({upload_load}) or \
         Strava's ({strava_load})"
    );
    assert!(
        both_load < upload_load + strava_load - 1e-9,
        "never both copies"
    );
    assert!(
        !both.status(&resources).await["form"].is_null(),
        "Home reads the merged history"
    );
}

#[tokio::test]
async fn deleting_an_upload_recomputes_the_persisted_rollup_without_it() {
    let resources = common::create_test_server_resources().await.unwrap();
    let rider = athlete(&resources).await;
    let mut latest = String::new();
    for days_ago in upload_days() {
        let id = rider.upload_ride(&resources, days_ago).await;
        if days_ago == 2 {
            latest = id;
        }
    }
    assert!(load_on(&rider.stored_rollup(&resources).await, 2) > 0.0);

    rider.delete_upload(&resources, &latest).await;

    assert!(
        load_on(&rider.stored_rollup(&resources).await, 2).abs() < f64::EPSILON,
        "the deleted ride no longer adds load to the stored rollup"
    );
}
