// ABOUTME: carnet#582 — a synced run faster than the athlete's stored best at a standard distance sends one "New PR" notice
// ABOUTME: Drives PersonalBests over the real repository and notification pipeline; seeding and slower runs send nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Personal-record notices (carnet#582).
//!
//! Each test builds a run whose cumulative distance grows at a constant pace,
//! so cageux's best effort at 5 km is exactly `5 × pace`, and hands it to
//! [`PersonalBests`] as the webhook sync does. What the athlete was told is
//! read back from the notification rows the real pipeline persisted, and the
//! stored bests from the real repository.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "client-notifications")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use chrono::{DateTime, TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId, TimeSeriesData};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::events::event_params;
use pierre_notifications::models::Notification;
use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
use pierre_routes_groups::NotificationRoutes;
use pierre_services::notification_localizer::UserLocaleNotificationLocalizer;
use pierre_services::personal_bests::{format_effort_time, BestOutcome, PersonalBests};
use pierre_services::provider_rate_limiter::ProviderRateLimiter;
use serde_json::Value;
use tokio::time::sleep;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_tenant};
use crate::helpers::axum_test::AxumTestRequest;

/// How long the fire-and-forget dispatch task is given to persist its row.
const DISPATCH_SETTLE: Duration = Duration::from_millis(400);

/// The notification service the server boots: the pipeline plus the localizer
/// that renders each event in the recipient's language.
fn notification_service(resources: &ServerContext) -> Arc<NotificationService> {
    let service = match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => NotificationService::from_sqlite(sqlite.pool().clone()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => NotificationService::from_postgres(pg.pool().clone()),
    };
    Arc::new(
        service.with_localizer(Arc::new(UserLocaleNotificationLocalizer::new(
            Arc::clone(&resources.common.repos),
            Arc::clone(&resources.mcp.messaging_strings_registry),
        ))),
    )
}

struct Fixture {
    resources: Arc<ServerContext>,
    service: Arc<NotificationService>,
    bests: PersonalBests,
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

async fn fixture(email: &str) -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (user, token) = create_test_tenant(&resources, email).await.unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    let service = notification_service(&resources);
    let bests = PersonalBests::new(
        Arc::clone(&resources.common.repos.personal_bests),
        Some(Arc::clone(&service)),
        Arc::new(ProviderRateLimiter::new(Arc::clone(
            &resources.common.repos.usage_counters,
        ))),
        Arc::clone(&resources.common.repos.worker_runs),
    );
    Fixture {
        resources,
        service,
        bests,
        user_id: user.id,
        tenant,
        token: format!("Bearer {token}"),
    }
}

fn day(n: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, n, 7, 0, 0).unwrap()
}

/// A Strava run of `km` kilometres at a constant `seconds_per_km`, sampled
/// once a second with its cumulative distance, as the Strava mapper builds it.
fn run(id: &str, start: DateTime<Utc>, seconds_per_km: u32, km: u32) -> (Activity, TimeSeriesData) {
    let duration = seconds_per_km * km;
    let timestamps: Vec<u32> = (0..=duration).collect();
    let metres_per_second = 1000.0 / f64::from(seconds_per_km);
    let distance: Vec<f64> = timestamps
        .iter()
        .map(|t| f64::from(*t) * metres_per_second)
        .collect();
    let activity = ActivityBuilder::new(
        id.to_owned(),
        "Morning Run".to_owned(),
        SportType::Run,
        start,
        u64::from(duration),
        "strava".to_owned(),
    )
    .distance_meters(f64::from(km) * 1000.0)
    .build();
    let series = TimeSeriesData {
        timestamps,
        heart_rate: None,
        power: None,
        cadence: None,
        speed: None,
        altitude: None,
        temperature: None,
        gps_coordinates: None,
        distance: Some(distance),
    };
    (activity, series)
}

/// The `personal_record` rows the athlete holds.
async fn record_rows(f: &Fixture) -> Vec<Notification> {
    sleep(DISPATCH_SETTLE).await;
    let (rows, _, _) = f
        .service
        .list_notifications(
            f.user_id,
            CommereTenantId(f.tenant.as_uuid()),
            50,
            0,
            None,
            false,
        )
        .await
        .unwrap();
    rows.into_iter()
        .filter(|row| row.notification_type == "personal_record")
        .collect()
}

async fn stored_5k(f: &Fixture) -> Option<f64> {
    f.resources
        .common
        .repos
        .personal_bests
        .personal_bests(f.user_id, f.tenant)
        .await
        .unwrap()
        .into_iter()
        .find(|best| best.distance == "5k")
        .map(|best| best.elapsed_seconds)
}

/// A run measured with nothing stored yet sets the baseline at every
/// distance it covers, tells nothing, and does not by itself make the
/// athlete's history walk complete: only the walk reaching their first
/// activity does.
#[tokio::test]
async fn a_first_measured_run_sets_the_baseline_and_tells_nothing() {
    let f = fixture("pr_seed@example.com").await;
    let (activity, series) = run("run-1", day(1), 300, 11);

    let results = f
        .bests
        .record_run(f.user_id, f.tenant, &activity, Some(&series), false)
        .await
        .unwrap();

    let distances: Vec<&str> = results.iter().map(|r| r.distance).collect();
    assert_eq!(
        distances,
        vec!["5k", "10k"],
        "an 11 km run covers two standard distances"
    );
    assert!(results.iter().all(|r| r.outcome == BestOutcome::Seeded));
    assert_eq!(stored_5k(&f).await, Some(1500.0), "5 km at 5:00/km");
    assert!(!f
        .bests
        .is_seed_complete(f.user_id, f.tenant, "strava")
        .await
        .unwrap());
    assert!(
        record_rows(&f).await.is_empty(),
        "a baseline is not a record"
    );
}

/// A run faster than the stored 5 km replaces it and sends exactly one
/// notice, naming the distance in the reader's language and the new time;
/// the same row reads English once the athlete switches.
#[tokio::test]
async fn beating_the_stored_5k_sends_one_notice_with_the_time_and_the_distance() {
    let f = fixture("pr_beat@example.com").await;
    let (first, first_series) = run("run-1", day(1), 300, 6);
    f.bests
        .record_run(f.user_id, f.tenant, &first, Some(&first_series), false)
        .await
        .unwrap();

    let (faster, faster_series) = run("run-2", day(3), 270, 6);
    let results = f
        .bests
        .record_run(f.user_id, f.tenant, &faster, Some(&faster_series), true)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].outcome,
        BestOutcome::Improved {
            previous_seconds: 1500.0
        }
    );
    assert_eq!(stored_5k(&f).await, Some(1350.0), "5 km at 4:30/km");

    let rows = record_rows(&f).await;
    assert_eq!(rows.len(), 1, "one beaten best, one notice");
    let params = event_params(rows[0].data.as_ref()).unwrap();
    assert_eq!(params["distance"], "5k");
    assert_eq!(params["time_display"], "22:30");
    assert_eq!(rows[0].title, "Nouveau record personnel !");
    assert_eq!(rows[0].body, "Nouveau record sur 5 km : 22:30");

    let router = NotificationRoutes::routes(Arc::clone(&f.resources));
    f.resources
        .common
        .repos
        .users
        .update_locale(f.user_id, "en")
        .await
        .unwrap();
    let response = AxumTestRequest::get("/api/notifications")
        .header("authorization", &f.token)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let feed: Value = response.json();
    let row = feed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["notification_type"] == "personal_record")
        .expect("the feed returns the record");
    assert_eq!(row["body"], "New 5 km PR: 22:30");
}

/// A run slower than the stored 5 km changes nothing and tells nothing.
#[tokio::test]
async fn a_slower_run_keeps_the_best_and_tells_nothing() {
    let f = fixture("pr_slower@example.com").await;
    let (first, first_series) = run("run-1", day(1), 270, 6);
    f.bests
        .record_run(f.user_id, f.tenant, &first, Some(&first_series), false)
        .await
        .unwrap();

    let (slower, slower_series) = run("run-2", day(3), 300, 6);
    let results = f
        .bests
        .record_run(f.user_id, f.tenant, &slower, Some(&slower_series), true)
        .await
        .unwrap();

    assert_eq!(results[0].outcome, BestOutcome::Slower);
    assert_eq!(stored_5k(&f).await, Some(1350.0));
    assert!(record_rows(&f).await.is_empty());
}

/// A distance the athlete's whole history never covered becomes a baseline,
/// not a record, even once every past run was measured.
#[tokio::test]
async fn a_first_effort_at_a_distance_is_a_baseline_even_after_seeding() {
    let f = fixture("pr_new_distance@example.com").await;
    let (short, short_series) = run("run-1", day(1), 300, 6);
    f.bests
        .record_run(f.user_id, f.tenant, &short, Some(&short_series), false)
        .await
        .unwrap();

    let (long, long_series) = run("run-2", day(3), 300, 11);
    let results = f
        .bests
        .record_run(f.user_id, f.tenant, &long, Some(&long_series), true)
        .await
        .unwrap();

    let ten_k = results.iter().find(|r| r.distance == "10k").unwrap();
    assert_eq!(ten_k.outcome, BestOutcome::Seeded);
    assert!(record_rows(&f).await.is_empty());
}

/// A scanned run is never listed for scanning again, a ride is never listed
/// at all, and the rest come back oldest first.
#[tokio::test]
async fn a_run_is_scanned_once_and_rides_are_skipped() {
    let f = fixture("pr_scan_once@example.com").await;
    let (later, _) = run("run-later", day(5), 300, 6);
    let (earlier, earlier_series) = run("run-earlier", day(2), 300, 6);
    let ride = ActivityBuilder::new(
        "ride-1".to_owned(),
        "Lunch Ride".to_owned(),
        SportType::Ride,
        day(4),
        3_600,
        "strava".to_owned(),
    )
    .build();
    let fetched = vec![later.clone(), ride, earlier.clone()];

    let pending = f
        .bests
        .unscanned_runs(f.user_id, f.tenant, &fetched)
        .await
        .unwrap();
    let ids: Vec<&str> = pending.iter().map(|a| a.id()).collect();
    assert_eq!(ids, vec!["run-earlier", "run-later"]);

    f.bests
        .record_run(f.user_id, f.tenant, &earlier, Some(&earlier_series), false)
        .await
        .unwrap();
    let pending = f
        .bests
        .unscanned_runs(f.user_id, f.tenant, &fetched)
        .await
        .unwrap();
    let ids: Vec<&str> = pending.iter().map(|a| a.id()).collect();
    assert_eq!(ids, vec!["run-later"]);
}

/// Times read as an athlete writes them: `m:ss` under an hour, `h:mm:ss`
/// from one up, rounded to the second.
#[test]
fn effort_times_read_as_minutes_or_hours() {
    assert_eq!(format_effort_time(1350.4), "22:30");
    assert_eq!(format_effort_time(59.6), "1:00");
    assert_eq!(format_effort_time(5_532.0), "1:32:12");
    assert_eq!(format_effort_time(10_805.0), "3:00:05");
}
