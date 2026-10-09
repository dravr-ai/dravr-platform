// ABOUTME: Training status and the persisted training-load rollup read every connected provider, merged once per workout
// ABOUTME: Phil's shape — GPS copies on Strava and Garmin, WHOOP used last — answers the same series as one clean history

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#836. The CTL/ATL/TSB series used to be computed from the cached
//! rows of one elected connection, so an athlete whose elected provider held
//! a shallow history read "not enough history" while another connection held
//! months of it, and a session recorded on a provider other than the elected
//! one never reached their load.
//!
//! Each test compares a multi-provider athlete against a control athlete
//! holding the same workouts once, on a single provider: merged and
//! deduplicated, the two must read the same series — not shallower (the
//! elected provider alone) and not heavier (a workout counted per copy).
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
use std::time::Duration as StdDuration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, DailyTrainingState, SportType, TenantId, User,
    UserPhysiologicalProfile,
};
use pierre_core::transport::TransportPolicy;
use pierre_fitness_compute::training_history_compute::CTL_WINDOW_DAYS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::{
    compute_and_persist_history, fetch_history_rows,
};
use serde_json::Value;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

const STRAVA: &str = "strava";
const GARMIN: &str = "sciotte_garmin";
const WHOOP: &str = "whoop";

/// Days of history the deep provider holds: well past the 72-day warm-up.
const DEEP_DAYS: i64 = 200;

struct Athlete {
    user: User,
    user_id: Uuid,
    tenant: TenantId,
}

/// A user with real physiology, so every session scores a non-zero load.
async fn athlete(resources: &Arc<ServerContext>) -> Athlete {
    let email = format!("multi-{}@example.com", Uuid::new_v4());
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
    Athlete {
        user,
        user_id,
        tenant,
    }
}

/// 07:00 UTC `days_ago` days before today.
fn morning(days_ago: i64) -> DateTime<Utc> {
    let day = Utc::now().date_naive() - Duration::days(days_ago);
    Utc.from_utc_datetime(&day.and_time(NaiveTime::from_hms_opt(7, 0, 0).unwrap()))
}

/// An outdoor ride as a GPS recorder stores it, under `provider`.
fn ride(provider: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        format!("{provider}-ride-{days_ago}"),
        format!("Morning ride {days_ago}d ago"),
        SportType::Ride,
        morning(days_ago),
        3_600,
        provider.to_owned(),
    )
    .distance_meters(30_000.0)
    .average_heart_rate(150)
    .build()
}

/// The workout WHOOP detected during that ride: wrist-classified as a run,
/// starting a little late and ending a little early.
fn detected(days_ago: i64) -> Activity {
    ActivityBuilder::new(
        format!("whoop-{days_ago}"),
        "Running".to_owned(),
        SportType::Run,
        morning(days_ago) + Duration::minutes(2),
        3_300,
        WHOOP.to_owned(),
    )
    .average_heart_rate(150)
    .build()
}

impl Athlete {
    async fn connect(&self, resources: &Arc<ServerContext>, provider: &str) {
        sleep(StdDuration::from_millis(20)).await;
        resources
            .common
            .repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
    }

    /// Mark `provider` used now, as a background refresh does on every visit.
    async fn touch(&self, resources: &Arc<ServerContext>, provider: &str) {
        sleep(StdDuration::from_millis(20)).await;
        resources
            .common
            .repos
            .provider_connections
            .touch_last_used(self.user_id, self.tenant, provider)
            .await
            .unwrap();
    }

    async fn cache(&self, resources: &Arc<ServerContext>, provider: &str, rows: &[Activity]) {
        resources
            .common
            .repos
            .activity_cache
            .upsert_activities(self.user_id, &self.tenant, provider, rows)
            .await
            .unwrap();
    }

    async fn status(&self, resources: &Arc<ServerContext>) -> Value {
        let token = common::generate_test_token(resources, &self.user).await;
        let request = Request::builder()
            .uri("/api/me/training-status")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let response = athlete_home_routes()
            .with_state(Arc::clone(resources))
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The persisted rollup over the chronic window ending today, as
    /// `compute_training_history` writes it and the agent reads it.
    async fn rollup(&self, resources: &Arc<ServerContext>) -> Vec<DailyTrainingState> {
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        let to = Utc::now().date_naive();
        let from = to - Duration::days(CTL_WINDOW_DAYS);
        compute_and_persist_history(&runtime, self.tenant, self.user_id, from, to)
            .await
            .unwrap();
        fetch_history_rows(&runtime.data(), self.tenant, self.user_id, from, to)
            .await
            .unwrap()
    }
}

/// Rides every other day across the deep history.
fn ride_days() -> impl Iterator<Item = i64> {
    (0..=DEEP_DAYS).filter(|d| d % 2 == 0)
}

/// The figures two rollups must agree on, day by day.
fn figures(rows: &[DailyTrainingState]) -> Vec<(NaiveDate, String)> {
    rows.iter()
        .map(|row| {
            (
                row.date,
                format!(
                    "load {:.6} ctl {:.6} atl {:.6} tsb {:.6}",
                    row.daily_load, row.ctl, row.atl, row.tsb
                ),
            )
        })
        .collect()
}

/// Phil's shape: years on Strava, the same rides copied from a Garmin
/// connected last month, WHOOP detecting the same sessions over the last ten
/// days — every connection refreshed the same evening, WHOOP last, Garmin
/// the last recorder. The elected Garmin alone held 20 days, short of the
/// 72-day warm-up, so Home answered "not enough history". Merged, it reads
/// exactly what one clean copy of the rides reads.
#[tokio::test]
async fn copies_across_strava_garmin_and_whoop_read_as_one_history() {
    let resources = common::create_test_server_resources().await.unwrap();

    let phil = athlete(&resources).await;
    phil.connect(&resources, STRAVA).await;
    phil.connect(&resources, GARMIN).await;
    phil.connect(&resources, WHOOP).await;
    let strava: Vec<_> = ride_days().map(|d| ride(STRAVA, d)).collect();
    let garmin: Vec<_> = ride_days()
        .filter(|d| *d <= 20)
        .map(|d| ride(GARMIN, d))
        .collect();
    let whoop: Vec<_> = ride_days().filter(|d| *d <= 10).map(detected).collect();
    phil.cache(&resources, STRAVA, &strava).await;
    phil.cache(&resources, GARMIN, &garmin).await;
    phil.cache(&resources, WHOOP, &whoop).await;
    phil.touch(&resources, STRAVA).await;
    phil.touch(&resources, GARMIN).await;
    phil.touch(&resources, WHOOP).await;

    // The same rides, once, on one provider.
    let control = athlete(&resources).await;
    control.connect(&resources, STRAVA).await;
    let once: Vec<_> = ride_days().map(|d| ride(STRAVA, d)).collect();
    control.cache(&resources, STRAVA, &once).await;

    let phil_status = phil.status(&resources).await;
    let control_status = control.status(&resources).await;
    assert!(
        !phil_status["form"].is_null(),
        "months of stored rides warm today's form whichever connection was used last: {phil_status}"
    );
    assert_eq!(
        phil_status, control_status,
        "the merged history reads exactly one copy of each ride"
    );

    let phil_rollup = phil.rollup(&resources).await;
    let control_rollup = control.rollup(&resources).await;
    assert_eq!(
        phil_rollup.len(),
        usize::try_from(CTL_WINDOW_DAYS).unwrap() + 1,
        "the persisted rollup is warmed for the whole window"
    );
    assert_eq!(
        figures(&phil_rollup),
        figures(&control_rollup),
        "the rollup the agent reads scores each ride once, not once per copy"
    );
}

/// A session only one provider holds is part of the athlete's load even when
/// another connection was used last: an indoor ride uploaded to Strava alone
/// counts beside the rides Garmin and Strava both hold.
#[tokio::test]
async fn a_session_on_a_provider_used_less_recently_still_counts() {
    let resources = common::create_test_server_resources().await.unwrap();
    let indoor_days = [1_i64, 3, 5];

    let athlete_a = athlete(&resources).await;
    athlete_a.connect(&resources, STRAVA).await;
    athlete_a.connect(&resources, GARMIN).await;
    let mut strava: Vec<_> = ride_days().map(|d| ride(STRAVA, d)).collect();
    strava.extend(indoor_days.iter().map(|d| ride(STRAVA, *d)));
    let garmin: Vec<_> = ride_days().map(|d| ride(GARMIN, d)).collect();
    athlete_a.cache(&resources, STRAVA, &strava).await;
    athlete_a.cache(&resources, GARMIN, &garmin).await;
    athlete_a.touch(&resources, STRAVA).await;
    athlete_a.touch(&resources, GARMIN).await;

    let control = athlete(&resources).await;
    control.connect(&resources, STRAVA).await;
    let mut once: Vec<_> = ride_days().map(|d| ride(STRAVA, d)).collect();
    once.extend(indoor_days.iter().map(|d| ride(STRAVA, *d)));
    control.cache(&resources, STRAVA, &once).await;

    let rollup = athlete_a.rollup(&resources).await;
    let today = Utc::now().date_naive();
    for days_ago in indoor_days {
        let day = today - Duration::days(days_ago);
        let row = rollup
            .iter()
            .find(|row| row.date == day)
            .expect("the window covers the indoor ride");
        assert!(
            row.daily_load > 0.0,
            "the Strava-only ride {days_ago}d ago adds load though Garmin was used last"
        );
    }
    assert_eq!(
        figures(&rollup),
        figures(&control.rollup(&resources).await),
        "every workout counts once: the union of providers, never a copy twice"
    );
    assert_eq!(
        athlete_a.status(&resources).await,
        control.status(&resources).await,
        "Home quotes the same series"
    );
}
