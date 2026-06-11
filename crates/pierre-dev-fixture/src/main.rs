// ABOUTME: Dev/test fixture API serving seeded activities as Strava API responses
// ABOUTME: Real Strava provider points here in dev (PIERRE_STRAVA_API_BASE_URL) so seed data flows the real path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Dev/test fixture HTTP API.
//!
//! Serves the small slice of the Strava API that the real Strava provider
//! calls (`/athlete`, `/athlete/activities`, `/athletes/{id}/stats`), backed by
//! rows seeded into the
//! `synthetic_activities` table. In dev the Strava provider's base URL is
//! pointed here (`PIERRE_STRAVA_API_BASE_URL`), so seeded test users fetch
//! their activities through the exact same provider code path a real user
//! would — no synthetic-provider special-casing.
//!
//! The bearer token a seeded user carries is `devfixture:<user_id>`; the
//! fixture extracts the user id from it and returns that user's activities.

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::net::SocketAddr;

use axum::extract::{Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{serve, Json, Router};
use serde_json::{json, Value};
use sqlx::sqlite::{SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::{fmt, EnvFilter};

/// Bearer-token prefix a seeded dev user carries; the suffix is the user id.
const BEARER_PREFIX: &str = "devfixture:";

/// Default port the fixture binds when `FIXTURE_PORT` is unset.
const DEFAULT_PORT: u16 = 9555;

/// Max activities returned for a single request (mirrors a sane Strava page).
const MAX_ACTIVITIES: i64 = 200;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let database_url = env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set (e.g. sqlite:./data/users.db)".to_owned())?;
    let port: u16 = env::var("FIXTURE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?;

    let app = Router::new()
        .route("/athlete", get(athlete))
        .route("/athlete/activities", get(athlete_activities))
        .route("/athletes/{id}/stats", get(athlete_stats))
        .route(
            "/activitylist-service/activities/search/activities",
            get(garmin_activities),
        )
        .route("/health", get(|| async { "ok" }))
        .with_state(pool);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "dev fixture API listening (Strava + Garmin shape)");
    serve(listener, app).await?;
    Ok(())
}

/// Minimal Strava athlete profile — the provider only needs an id/name here.
async fn athlete(headers: HeaderMap) -> Json<Value> {
    let user_id = user_from_bearer(&headers).unwrap_or_else(|| "unknown".to_owned());
    Json(json!({
        "id": stable_id(&user_id),
        "username": user_id,
        "firstname": "Dev",
        "lastname": "Fixture",
    }))
}

/// `GET /athlete/activities` — returns the bearer user's seeded activities in
/// Strava summary-activity JSON shape.
async fn athlete_activities(
    State(pool): State<SqlitePool>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Json<Value> {
    let Some(user_id) = user_from_bearer(&headers) else {
        warn!("missing or malformed bearer; returning empty activity list");
        return Json(json!([]));
    };

    let limit = params
        .get("per_page")
        .and_then(|p| p.parse::<i64>().ok())
        .unwrap_or(MAX_ACTIVITIES)
        .clamp(1, MAX_ACTIVITIES);

    let rows = sqlx::query(
        "SELECT id, name, sport_type, start_date, duration_seconds, distance_meters, \
         elevation_gain, average_heart_rate, max_heart_rate, average_speed, max_speed, \
         calories, city, region, country, start_latitude, start_longitude \
         FROM synthetic_activities WHERE user_id = ? ORDER BY start_date DESC LIMIT ?",
    )
    .bind(&user_id)
    .bind(limit)
    .fetch_all(&pool)
    .await;

    match rows {
        Ok(rows) => {
            let activities: Vec<Value> = rows.iter().map(row_to_strava_activity).collect();
            info!(user_id = %user_id, count = activities.len(), "served seeded activities");
            Json(Value::Array(activities))
        }
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "activity query failed");
            Json(json!([]))
        }
    }
}

/// `GET /athletes/{id}/stats` — returns the bearer user's all-time ride and run
/// totals in Strava's athlete-stats JSON shape. The path id is ignored; the user
/// is resolved from the bearer like the other handlers. Only `all_ride_totals`
/// and `all_run_totals` are emitted because those are the only fields the Strava
/// provider's `get_stats` reads — matching how the real provider sums ride+run.
async fn athlete_stats(State(pool): State<SqlitePool>, headers: HeaderMap) -> Json<Value> {
    let Some(user_id) = user_from_bearer(&headers) else {
        warn!("missing or malformed bearer; returning zeroed stats");
        return Json(strava_stats_json(None));
    };

    // Aggregate ride-like and run-like activities separately. Substring matches
    // (`%ride%`, `%run%`) catch the seeded variants — gravel_ride, virtual_ride,
    // mountain_bike_ride, trail_run — the same way real Strava folds them into
    // its ride/run buckets. SQLite SUM ignores NULL distance/elevation (e.g. yoga).
    let row = sqlx::query(
        "SELECT \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' THEN 1 ELSE 0 END), 0) AS ride_count, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' THEN distance_meters END), 0.0) AS ride_distance, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' THEN duration_seconds END), 0) AS ride_time, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' THEN elevation_gain END), 0.0) AS ride_elev, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' THEN 1 ELSE 0 END), 0) AS run_count, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' THEN distance_meters END), 0.0) AS run_distance, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' THEN duration_seconds END), 0) AS run_time, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' THEN elevation_gain END), 0.0) AS run_elev \
         FROM synthetic_activities WHERE user_id = ?",
    )
    .bind(&user_id)
    .fetch_one(&pool)
    .await;

    match row {
        Ok(row) => {
            info!(user_id = %user_id, "served seeded athlete stats");
            Json(strava_stats_json(Some(&row)))
        }
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "stats query failed");
            Json(strava_stats_json(None))
        }
    }
}

/// Build Strava athlete-stats JSON from an aggregate row, or all-zero totals when
/// the row is absent (missing bearer or query error). Column names follow the
/// `<bucket>_<metric>` aliases the stats query emits.
fn strava_stats_json(row: Option<&SqliteRow>) -> Value {
    let totals = |count_col, dist_col, time_col, elev_col| {
        let (count, distance, moving_time, elevation_gain) =
            row.map_or((0_i64, 0.0_f64, 0_i64, 0.0_f64), |r| {
                (
                    r.try_get::<i64, _>(count_col).unwrap_or(0),
                    r.try_get::<f64, _>(dist_col).unwrap_or(0.0),
                    r.try_get::<i64, _>(time_col).unwrap_or(0),
                    r.try_get::<f64, _>(elev_col).unwrap_or(0.0),
                )
            });
        json!({
            "count": count,
            "distance": distance,
            "moving_time": moving_time,
            "elevation_gain": elevation_gain,
        })
    };

    json!({
        "all_ride_totals": totals("ride_count", "ride_distance", "ride_time", "ride_elev"),
        "all_run_totals": totals("run_count", "run_distance", "run_time", "run_elev"),
    })
}

/// `GET /activitylist-service/activities/search/activities` — returns the
/// bearer user's seeded activities in Garmin Connect summary JSON shape (the
/// flat array of `GarminActivityResponse` the Garmin provider deserializes).
async fn garmin_activities(
    State(pool): State<SqlitePool>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Json<Value> {
    let Some(user_id) = user_from_bearer(&headers) else {
        warn!("missing or malformed bearer; returning empty activity list");
        return Json(json!([]));
    };

    let limit = params
        .get("limit")
        .and_then(|p| p.parse::<i64>().ok())
        .unwrap_or(MAX_ACTIVITIES)
        .clamp(1, MAX_ACTIVITIES);

    let rows = sqlx::query(
        "SELECT id, name, sport_type, start_date, duration_seconds, distance_meters, \
         elevation_gain, average_heart_rate, max_heart_rate, average_speed, max_speed \
         FROM synthetic_activities WHERE user_id = ? ORDER BY start_date DESC LIMIT ?",
    )
    .bind(&user_id)
    .bind(limit)
    .fetch_all(&pool)
    .await;

    match rows {
        Ok(rows) => {
            let activities: Vec<Value> = rows.iter().map(row_to_garmin_activity).collect();
            info!(user_id = %user_id, count = activities.len(), "served seeded garmin activities");
            Json(Value::Array(activities))
        }
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "garmin activity query failed");
            Json(json!([]))
        }
    }
}

/// Map a `synthetic_activities` row to a Garmin Connect summary-activity JSON
/// object. Garmin's parser keys on `activityTypeDTO.typeKey` and `summaryDTO`
/// (camelCase, with explicit `averageHR`/`maxHR`); `startTimeGMT` is emitted in
/// both `RFC3339` casings the provider may deserialize.
fn row_to_garmin_activity(row: &SqliteRow) -> Value {
    let id: String = row.try_get("id").unwrap_or_default();
    let sport: String = row
        .try_get("sport_type")
        .unwrap_or_else(|_| "other".to_owned());
    let start = rfc3339_z(&row.try_get::<String, _>("start_date").unwrap_or_default());

    json!({
        "activityId": stable_id(&id),
        "activityName": row.try_get::<String, _>("name").unwrap_or_default(),
        "activityTypeDTO": { "typeKey": garmin_type_key(&sport) },
        "summaryDTO": {
            "startTimeGMT": start,
            "startTimeGmt": start,
            "distance": row.try_get::<Option<f64>, _>("distance_meters").ok().flatten(),
            "duration": row.try_get::<Option<i64>, _>("duration_seconds").ok().flatten(),
            "elevationGain": row.try_get::<Option<f64>, _>("elevation_gain").ok().flatten(),
            "averageSpeed": row.try_get::<Option<f64>, _>("average_speed").ok().flatten(),
            "maxSpeed": row.try_get::<Option<f64>, _>("max_speed").ok().flatten(),
            "averageHR": row.try_get::<Option<f64>, _>("average_heart_rate").ok().flatten(),
            "maxHR": row.try_get::<Option<f64>, _>("max_heart_rate").ok().flatten(),
        }
    })
}

/// Translate a stored `sport_type` to a Garmin `typeKey` the Garmin provider's
/// `parse_sport_type` recognizes. Most synthetic labels (`run`, `trail_run`,
/// `mountain_bike_ride`, `walk`, `hike`, …) are already accepted; only `ride`
/// must become `cycling`.
fn garmin_type_key(sport: &str) -> &str {
    match sport {
        "ride" => "cycling",
        other => other,
    }
}

/// Extract the seeded user id from an `Authorization: Bearer devfixture:<id>` header.
fn user_from_bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ").unwrap_or(value);
    token.strip_prefix(BEARER_PREFIX).map(ToOwned::to_owned)
}

/// Map a `synthetic_activities` row to a Strava summary-activity JSON object.
fn row_to_strava_activity(row: &SqliteRow) -> Value {
    let id: String = row.try_get("id").unwrap_or_default();
    let lat: Option<f64> = row.try_get("start_latitude").ok().flatten();
    let lng: Option<f64> = row.try_get("start_longitude").ok().flatten();
    let start_latlng = match (lat, lng) {
        (Some(lat), Some(lng)) => json!([lat, lng]),
        _ => Value::Null,
    };

    json!({
        "id": stable_id(&id),
        "name": row.try_get::<String, _>("name").unwrap_or_default(),
        "type": row.try_get::<String, _>("sport_type").unwrap_or_else(|_| "Workout".to_owned()),
        "start_date": rfc3339_z(&row.try_get::<String, _>("start_date").unwrap_or_default()),
        "distance": row.try_get::<Option<f64>, _>("distance_meters").ok().flatten(),
        "elapsed_time": row.try_get::<Option<i64>, _>("duration_seconds").ok().flatten(),
        "total_elevation_gain": row.try_get::<Option<f64>, _>("elevation_gain").ok().flatten(),
        "average_speed": row.try_get::<Option<f64>, _>("average_speed").ok().flatten(),
        "max_speed": row.try_get::<Option<f64>, _>("max_speed").ok().flatten(),
        "average_heartrate": row.try_get::<Option<f64>, _>("average_heart_rate").ok().flatten(),
        "max_heartrate": row.try_get::<Option<f64>, _>("max_heart_rate").ok().flatten(),
        "calories": row.try_get::<Option<f64>, _>("calories").ok().flatten(),
        "start_latlng": start_latlng,
        "location_city": row.try_get::<Option<String>, _>("city").ok().flatten(),
        "location_state": row.try_get::<Option<String>, _>("region").ok().flatten(),
        "location_country": row.try_get::<Option<String>, _>("country").ok().flatten(),
    })
}

/// Normalize a stored `start_date` to `RFC3339` with an explicit `Z` offset,
/// the shape the Strava provider's chrono parser expects. Seeded rows store a
/// naive UTC timestamp with no offset (e.g. `2026-05-22T02:37:08.05570`).
fn rfc3339_z(start_date: &str) -> String {
    if start_date.ends_with('Z') || start_date.contains('+') {
        start_date.to_owned()
    } else {
        format!("{start_date}Z")
    }
}

/// Derive a stable numeric Strava-style activity id from a `UUID` string by
/// taking its first 64 bits. Deterministic so repeated fetches are stable.
fn stable_id(uuid: &str) -> u64 {
    let hex: String = uuid
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(16)
        .collect();
    u64::from_str_radix(&hex, 16).unwrap_or(1)
}
