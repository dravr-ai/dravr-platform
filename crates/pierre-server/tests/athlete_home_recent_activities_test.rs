// ABOUTME: GET /api/me/activities/recent — newest cached activities across providers, clamped, tenant-scoped, cache-first
// ABOUTME: A stale cache answers at once, says so, and refreshes once in the background through the recent-fetch path

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home recent-activities suite.
//!
//! Every test seeds real rows through the activity cache's own writer and
//! reads the route's JSON back, so a handler that answered an empty list, a
//! fixed row or the wrong tenant's rows fails on content.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::Query;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, TimeZone, Utc};
use futures_util::future::join_all;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserOAuthToken,
};
use pierre_database::backends::factory::Database;
use pierre_fitness_compute::{encode_polyline, trimmed_overview_polyline};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

/// Every key a Home activity row carries, null or not.
const HOME_ACTIVITY_KEYS: [&str; 10] = [
    "id",
    "provider",
    "name",
    "sport_type",
    "start_date",
    "duration_seconds",
    "distance_meters",
    "elevation_gain_meters",
    "has_gps",
    "summary_polyline",
];

/// One athlete: the account, the tenant they act in and a bearer for it.
struct Athlete {
    user: User,
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

async fn seed_athlete(resources: &Arc<ServerContext>, label: &str) -> Athlete {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let token = common::generate_test_token(resources, &user).await;
    Athlete {
        user,
        user_id,
        tenant,
        token,
    }
}

/// A second tenant the athlete owns, and a bearer acting in it.
async fn second_tenant(resources: &Arc<ServerContext>, athlete: &Athlete) -> (TenantId, String) {
    let tenant = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant,
            name: "second tenant".to_owned(),
            slug: format!("second-{tenant}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: athlete.user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &athlete.user,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap();
    (tenant, token)
}

async fn get_json(resources: &Arc<ServerContext>, token: &str, uri: &str) -> (StatusCode, Value) {
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn recent(resources: &Arc<ServerContext>, token: &str, query: &str) -> Value {
    let (status, body) = get_json(
        resources,
        token,
        &format!("/api/me/activities/recent{query}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn ids(body: &Value) -> Vec<String> {
    body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}

/// `days` days before a fixed hour today.
fn days_ago(days: i64) -> DateTime<Utc> {
    Utc::now()
        .date_naive()
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc()
        - Duration::days(days)
}

fn run(id: &str, provider: &str, started: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        id,
        format!("Run {id}"),
        SportType::Run,
        started,
        2_400,
        provider,
    )
    .distance_meters(8_000.0)
    .build()
}

async fn cache(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    provider: &str,
    rows: &[Activity],
) {
    cache_in(resources, athlete.user_id, athlete.tenant, provider, rows).await;
}

async fn cache_in(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
    rows: &[Activity],
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, provider, rows)
        .await
        .unwrap();
}

/// A northbound, weaving 3.3 km track out of Parc La Fontaine.
fn weaving_track(count: usize) -> Vec<(f64, f64)> {
    (0..count)
        .map(|i| {
            let step = i as f64;
            (
                0.0001f64.mul_add(step, 45.5259),
                0.0005f64.mul_add((step * 0.21).sin(), -73.5697),
            )
        })
        .collect()
}

#[tokio::test]
async fn the_newest_five_are_served_across_providers_newest_first() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-order").await;
    let strava: Vec<Activity> = [1, 3, 5, 7]
        .iter()
        .map(|&d| run(&format!("s{d}"), "strava", days_ago(d)))
        .collect();
    let whoop: Vec<Activity> = [2, 4, 6]
        .iter()
        .map(|&d| run(&format!("w{d}"), "whoop", days_ago(d)))
        .collect();
    cache(&resources, &athlete, "strava", &strava).await;
    cache(&resources, &athlete, "whoop", &whoop).await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(ids(&body), ["s1", "w2", "s3", "w4", "s5"]);
    let providers: Vec<&str> = body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["provider"].as_str().unwrap())
        .collect();
    assert_eq!(providers, ["strava", "whoop", "strava", "whoop", "strava"]);
}

#[tokio::test]
async fn the_limit_is_clamped_between_one_and_twenty() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-clamp").await;
    let rows: Vec<Activity> = (1..=22)
        .map(|d| run(&format!("a{d:02}"), "strava", days_ago(d)))
        .collect();
    cache(&resources, &athlete, "strava", &rows).await;

    assert_eq!(
        ids(&recent(&resources, &athlete.token, "?limit=2").await),
        ["a01", "a02"]
    );
    assert_eq!(
        ids(&recent(&resources, &athlete.token, "?limit=0").await),
        ["a01"]
    );
    assert_eq!(
        ids(&recent(&resources, &athlete.token, "?limit=-4").await),
        ["a01"]
    );
    let widest = ids(&recent(&resources, &athlete.token, "?limit=500").await);
    assert_eq!(widest.len(), 20);
    assert_eq!(widest.first().map(String::as_str), Some("a01"));
    assert_eq!(widest.last().map(String::as_str), Some("a20"));
}

#[tokio::test]
async fn a_row_carries_every_contract_field_with_nulls_where_nothing_was_recorded() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-fields").await;
    let track = weaving_track(300);
    let raw = encode_polyline(&track);
    let started = Utc.with_ymd_and_hms(2026, 9, 20, 7, 30, 0).unwrap();
    let outdoor = ActivityBuilder::new("o1", "Hill loop", SportType::Run, started, 3_000, "strava")
        .distance_meters(9_500.0)
        .elevation_gain(212.0)
        .start_latitude(45.5259)
        .start_longitude(-73.5697)
        .summary_polyline(raw.clone())
        .build();
    let indoor = ActivityBuilder::new(
        "i1",
        "Zwift",
        SportType::VirtualRide,
        started - Duration::days(1),
        3_600,
        "strava",
    )
    .build();
    let gravel = ActivityBuilder::new(
        "g1",
        "Gravel grind",
        SportType::Other("GravelGrind".to_owned()),
        started - Duration::days(2),
        7_200,
        "strava",
    )
    .build();
    cache(&resources, &athlete, "strava", &[outdoor, indoor, gravel]).await;

    let body = recent(&resources, &athlete.token, "").await;
    let rows = body["activities"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        let keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for key in HOME_ACTIVITY_KEYS {
            assert!(keys.contains(&key), "{key} missing from {row}");
        }
        assert_eq!(keys.len(), HOME_ACTIVITY_KEYS.len(), "{row}");
    }

    let outdoor = &rows[0];
    assert_eq!(outdoor["id"], "o1");
    assert_eq!(outdoor["provider"], "strava");
    assert_eq!(outdoor["name"], "Hill loop");
    assert_eq!(outdoor["sport_type"], "run");
    assert_eq!(
        DateTime::parse_from_rfc3339(outdoor["start_date"].as_str().unwrap()).unwrap(),
        started
    );
    assert_eq!(outdoor["duration_seconds"], 3_000);
    assert_eq!(outdoor["distance_meters"], 9_500.0);
    assert_eq!(outdoor["elevation_gain_meters"], 212.0);
    assert_eq!(outdoor["has_gps"], true);
    let sketch = outdoor["summary_polyline"].as_str().unwrap();
    assert_eq!(Some(sketch.to_owned()), trimmed_overview_polyline(&raw));
    assert_ne!(
        sketch, raw,
        "the sketch is trimmed at its ends before it leaves"
    );

    let indoor = &rows[1];
    assert_eq!(indoor["sport_type"], "virtual_ride");
    assert_eq!(indoor["has_gps"], false);
    assert!(indoor["distance_meters"].is_null());
    assert!(indoor["elevation_gain_meters"].is_null());
    assert!(indoor["summary_polyline"].is_null());

    assert_eq!(
        rows[2]["sport_type"], "GravelGrind",
        "an unmapped sport reads as the provider's own word"
    );
}

#[tokio::test]
async fn another_tenants_and_another_users_activities_are_invisible() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-home").await;
    let stranger = seed_athlete(&resources, "recent-stranger").await;
    let (elsewhere, elsewhere_token) = second_tenant(&resources, &athlete).await;
    cache(
        &resources,
        &athlete,
        "strava",
        &[
            run("mine-1", "strava", days_ago(1)),
            run("mine-2", "strava", days_ago(2)),
        ],
    )
    .await;
    cache_in(
        &resources,
        athlete.user_id,
        elsewhere,
        "strava",
        &[run("elsewhere-1", "strava", days_ago(1))],
    )
    .await;
    cache(
        &resources,
        &stranger,
        "strava",
        &[run("theirs-1", "strava", days_ago(1))],
    )
    .await;

    assert_eq!(
        ids(&recent(&resources, &athlete.token, "").await),
        ["mine-1", "mine-2"]
    );
    assert_eq!(
        ids(&recent(&resources, &elsewhere_token, "").await),
        ["elsewhere-1"]
    );
    assert_eq!(
        ids(&recent(&resources, &stranger.token, "").await),
        ["theirs-1"]
    );
}

#[tokio::test]
async fn a_mirror_backend_row_reads_as_the_provider_it_mirrors() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-mirror").await;
    cache(
        &resources,
        &athlete,
        "sciotte",
        &[run("m1", "sciotte", days_ago(1))],
    )
    .await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(body["activities"][0]["provider"], "strava");
}

#[tokio::test]
async fn freshness_follows_the_last_successful_fetch() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-fresh").await;

    // Nothing connected, nothing fetched: nothing to refresh either.
    let body = recent(&resources, &athlete.token, "").await;
    assert!(body["as_of"].is_null());
    assert_eq!(body["stale"], false);
    assert_eq!(body["activities"], json!([]));

    // A fetch a moment ago is fresh, and `as_of` is when it happened.
    let fetched_at = Utc::now() - Duration::minutes(5);
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "whoop",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    resources
        .common
        .repos
        .activity_cache
        .record_activity_fetch(athlete.user_id, &athlete.tenant, "whoop", fetched_at)
        .await
        .unwrap();
    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(body["stale"], false);
    let as_of = DateTime::parse_from_rfc3339(body["as_of"].as_str().unwrap()).unwrap();
    assert!((as_of.with_timezone(&Utc) - fetched_at).num_seconds().abs() < 1);
}

#[tokio::test]
async fn a_connected_provider_never_fetched_is_stale() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-never").await;
    // A connection with no token: the background refresh finds nothing to
    // authenticate with and leaves the cache as it was.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "whoop",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let body = recent(&resources, &athlete.token, "").await;
    assert!(body["as_of"].is_null());
    assert_eq!(body["stale"], true);
    await_background(&resources).await;
}

// ---------------------------------------------------------------------------
// The background refresh, against a Strava-shaped mock
// ---------------------------------------------------------------------------

/// Environment set for the duration of one test and removed after it.
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, String)]) -> Self {
        for (key, value) in vars {
            env::set_var(key, value);
        }
        Self {
            keys: vars.iter().map(|(key, _)| *key).collect(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            env::remove_var(key);
        }
    }
}

/// A mock `/athlete/activities` answering one fresh outdoor run, with a hit
/// counter.
async fn mock_strava_list() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let polyline = encode_polyline(&weaving_track(300));
    let app = Router::new().route(
        "/athlete/activities",
        get(move |_: Query<HashMap<String, String>>| {
            let counter = Arc::clone(&counter);
            let polyline = polyline.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                // Slow enough that concurrent page loads overlap the refresh.
                sleep(StdDuration::from_millis(150)).await;
                Json(json!([{
                    "id": 777_001,
                    "name": "Morning loop",
                    "type": "Run",
                    "sport_type": "Run",
                    "start_date": (Utc::now() - Duration::hours(2)).to_rfc3339(),
                    "distance": 10_200.0,
                    "elapsed_time": 3_100,
                    "total_elevation_gain": 95.0,
                    "start_latlng": [45.5259, -73.5697],
                    "map": { "id": "a777001", "summary_polyline": polyline }
                }]))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), hits)
}

async fn link_strava(resources: &Arc<ServerContext>, athlete: &Athlete) {
    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: athlete.user_id,
            tenant_id: athlete.tenant.to_string(),
            provider: "strava".to_owned(),
            // >= 40 chars and not "at_"-prefixed: the provider's token validation.
            access_token: "strava_access_token_0123456789abcdef0123456789".to_owned(),
            refresh_token: Some("strava_refresh".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some("4242".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

/// Move every cached row's `synced_at` back to `at`, as if the write-through
/// that stored them happened then. No repository writes an old `synced_at`
/// (an upsert stamps now), so the fixture writes it in SQL.
async fn backdate_cached_rows(resources: &ServerContext, user_id: Uuid, at: DateTime<Utc>) {
    const SQL: &str = "UPDATE cached_activities SET synced_at = $1 WHERE user_id = $2";
    match resources.agent.database.as_ref() {
        Database::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(at)
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(postgres) => {
            sqlx::query(SQL)
                .bind(at)
                .bind(user_id.to_string())
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// Wait for every task the route spawned on the drain tracker to finish.
async fn await_background(resources: &ServerContext) {
    let deadline = Instant::now() + StdDuration::from_secs(20);
    while !resources.common.turns.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the background refresh did not finish within 20s"
        );
        sleep(StdDuration::from_millis(25)).await;
    }
}

#[tokio::test]
#[serial]
async fn a_stale_cache_answers_at_once_and_refreshes_once_in_the_background() {
    let (api_base, hits) = mock_strava_list().await;
    let _env = EnvGuard::set(&[
        ("PIERRE_STRAVA_API_BASE_URL", api_base),
        ("STRAVA_CLIENT_ID", "test_client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "test_secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-refresh").await;
    link_strava(&resources, &athlete).await;
    // What the cache held from a fetch two days ago.
    cache(
        &resources,
        &athlete,
        "strava",
        &[run("old-1", "strava", days_ago(3))],
    )
    .await;
    let two_days_ago = Utc::now() - Duration::days(2);
    backdate_cached_rows(&resources, athlete.user_id, two_days_ago).await;
    resources
        .common
        .repos
        .activity_cache
        .record_activity_fetch(athlete.user_id, &athlete.tenant, "strava", two_days_ago)
        .await
        .unwrap();

    // Several page loads while the cache is stale: each answers from the
    // cache at once, and together they start one provider read.
    let loads = (0..4).map(|_| recent(&resources, &athlete.token, ""));
    for body in join_all(loads).await {
        assert_eq!(body["stale"], true);
        assert_eq!(ids(&body), ["old-1"]);
    }
    await_background(&resources).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "one refresh for every stale load"
    );

    // The refresh wrote through: the next load is fresh and carries the run
    // the provider returned, route overview and all.
    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(body["stale"], false);
    assert_eq!(ids(&body), ["777001", "old-1"]);
    let fresh = &body["activities"][0];
    assert_eq!(fresh["has_gps"], true);
    assert!(fresh["summary_polyline"]
        .as_str()
        .is_some_and(|p| !p.is_empty()));
    let as_of = DateTime::parse_from_rfc3339(body["as_of"].as_str().unwrap()).unwrap();
    assert!(as_of.with_timezone(&Utc) > two_days_ago + Duration::days(1));
    await_background(&resources).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "a fresh cache reads no provider"
    );
}
