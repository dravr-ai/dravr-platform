// ABOUTME: Dev/test fixture API serving seeded activities as Strava API responses
// ABOUTME: Real Strava provider points here in dev (PIERRE_STRAVA_API_BASE_URL) so seed data flows the real path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Dev/test fixture HTTP API.
//!
//! Serves the small slice of the Strava API that the real Strava provider
//! calls (`/athlete`, `/athlete/activities`, `/activities/{id}`,
//! `/athletes/{id}/stats`), backed by rows seeded into the
//! `synthetic_activities` table. In dev the Strava provider's base URL is
//! pointed here (`PIERRE_STRAVA_API_BASE_URL`), so seeded test users fetch
//! their activities through the exact same provider code path a real user
//! would — no synthetic-provider special-casing.
//!
//! The bearer token a seeded user carries is `devfixture:<user_id>`; the
//! fixture extracts the user id from it and returns that user's activities.
//!
//! `/activities/{id}/streams` is deliberately not served: its `404` is what
//! Strava answers for an activity recorded without samples, which the provider
//! reads as no GPS. A seeded activity with coordinates never reaches it — its
//! route is drawn from the summary polyline the list and detail carry.

// A pub item this binary never uses is reachable from nowhere: the lint is
// crate-level because a library's test harness is a binary too, where it
// would flag every pub item the unit tests do not call.
#![deny(dead_code_pub_in_binary)]

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::net::SocketAddr;

use std::f64::consts::TAU;

use axum::extract::{Path, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{serve, Json, Router};
use pierre_fitness_compute::polyline::encode_polyline;
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

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "dev fixture API listening (Strava + Garmin shape)");
    serve(listener, app(pool)).await?;
    Ok(())
}

/// Every route the fixture answers, over the seeded `pool`.
fn app(pool: SqlitePool) -> Router {
    Router::new()
        .route("/athlete", get(athlete))
        .route("/athlete/activities", get(athlete_activities))
        .route("/activities/{id}", get(activity_detail))
        .route("/athletes/{id}/stats", get(athlete_stats))
        .route(
            "/activitylist-service/activities/search/activities",
            get(garmin_activities),
        )
        .route("/health", get(|| async { "ok" }))
        .with_state(pool)
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

/// `GET /activities/{id}` — one of the bearer user's seeded activities in
/// Strava's detailed-activity shape: the summary fields the list serves, which
/// the provider's `DetailedActivityResponse` flattens, and no laps or splits,
/// since a seed records none.
///
/// The id is the numeric one the list handed out ([`stable_id`]): the first 16
/// hex digits of the row's UUID. An id the bearer user holds no row for is
/// Strava's `404 Record Not Found`, and a request without the fixture's bearer
/// is its `401`, so the provider meets the same answers it would in prod.
async fn activity_detail(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(user_id) = user_from_bearer(&headers) else {
        warn!("missing or malformed bearer; answering unauthorized");
        return strava_error(
            StatusCode::UNAUTHORIZED,
            "Authorization Error",
            ("Athlete", "access_token", "invalid"),
        );
    };
    let Ok(numeric) = id.parse::<u64>() else {
        return record_not_found();
    };
    let row = sqlx::query(
        "SELECT id, name, sport_type, start_date, duration_seconds, distance_meters, \
         elevation_gain, average_heart_rate, max_heart_rate, average_speed, max_speed, \
         calories, city, region, country, start_latitude, start_longitude \
         FROM synthetic_activities \
         WHERE user_id = ? AND substr(lower(replace(id, '-', '')), 1, 16) = ?",
    )
    .bind(&user_id)
    .bind(format!("{numeric:016x}"))
    .fetch_optional(&pool)
    .await;

    match row {
        Ok(Some(row)) => {
            info!(user_id = %user_id, activity_id = numeric, "served seeded activity detail");
            Json(row_to_strava_activity(&row)).into_response()
        }
        Ok(None) => record_not_found(),
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "activity detail query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Strava's `404` for an activity the caller holds no record of.
fn record_not_found() -> Response {
    strava_error(
        StatusCode::NOT_FOUND,
        "Record Not Found",
        ("Activity", "id", "invalid"),
    )
}

/// An error body in Strava's shape: a message and one `{resource, field, code}`.
fn strava_error(
    status: StatusCode,
    message: &str,
    (resource, field, code): (&str, &str, &str),
) -> Response {
    let body = json!({
        "message": message,
        "errors": [{ "resource": resource, "field": field, "code": code }],
    });
    (status, Json(body)).into_response()
}

/// `GET /athletes/{id}/stats` — returns the bearer user's ride and run totals in
/// Strava's athlete-stats JSON shape, for both all-time and the current calendar
/// year (`ytd_*`). The path id is ignored; the user is resolved from the bearer
/// like the other handlers. Ride and run buckets are emitted because those are the
/// fields the Strava provider's `get_stats` reads — matching how the real provider
/// sums ride+run. The `ytd_*` totals re-run the same aggregation filtered to the
/// current year so dev exercises the annual-vs-lifetime distinction.
async fn athlete_stats(State(pool): State<SqlitePool>, headers: HeaderMap) -> Json<Value> {
    let Some(user_id) = user_from_bearer(&headers) else {
        warn!("missing or malformed bearer; returning zeroed stats");
        return Json(strava_stats_json(None));
    };

    // Aggregate ride-like and run-like activities separately. Substring matches
    // (`%ride%`, `%run%`) catch the seeded variants — gravel_ride, virtual_ride,
    // mountain_bike_ride, trail_run — the same way real Strava folds them into
    // its ride/run buckets. SQLite SUM ignores NULL distance/elevation (e.g. yoga).
    // The `ytd_*` aggregates reuse the ride/run buckets but additionally require
    // the activity's calendar year to match the current year (SQLite `strftime`),
    // so they form a strict subset of the all-time totals.
    let row = sqlx::query(
        "SELECT \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' THEN 1 ELSE 0 END), 0) AS ride_count, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' THEN distance_meters END), 0.0) AS ride_distance, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' THEN duration_seconds END), 0) AS ride_time, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' THEN elevation_gain END), 0.0) AS ride_elev, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' THEN 1 ELSE 0 END), 0) AS run_count, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' THEN distance_meters END), 0.0) AS run_distance, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' THEN duration_seconds END), 0) AS run_time, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' THEN elevation_gain END), 0.0) AS run_elev, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' AND strftime('%Y', start_date) = strftime('%Y', 'now') THEN 1 ELSE 0 END), 0) AS ytd_ride_count, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' AND strftime('%Y', start_date) = strftime('%Y', 'now') THEN distance_meters END), 0.0) AS ytd_ride_distance, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' AND strftime('%Y', start_date) = strftime('%Y', 'now') THEN duration_seconds END), 0) AS ytd_ride_time, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%ride%' AND strftime('%Y', start_date) = strftime('%Y', 'now') THEN elevation_gain END), 0.0) AS ytd_ride_elev, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' AND strftime('%Y', start_date) = strftime('%Y', 'now') THEN 1 ELSE 0 END), 0) AS ytd_run_count, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' AND strftime('%Y', start_date) = strftime('%Y', 'now') THEN distance_meters END), 0.0) AS ytd_run_distance, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' AND strftime('%Y', start_date) = strftime('%Y', 'now') THEN duration_seconds END), 0) AS ytd_run_time, \
         COALESCE(SUM(CASE WHEN sport_type LIKE '%run%' AND strftime('%Y', start_date) = strftime('%Y', 'now') THEN elevation_gain END), 0.0) AS ytd_run_elev \
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
        "ytd_ride_totals": totals("ytd_ride_count", "ytd_ride_distance", "ytd_ride_time", "ytd_ride_elev"),
        "ytd_run_totals": totals("ytd_run_count", "ytd_run_distance", "ytd_run_time", "ytd_run_elev"),
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
    let distance: Option<f64> = row
        .try_get::<Option<f64>, _>("distance_meters")
        .ok()
        .flatten();
    let (start_latlng, map) = match (lat, lng) {
        (Some(lat), Some(lng)) => (
            json!([lat, lng]),
            json!({
                "id": format!("a{}", stable_id(&id)),
                "summary_polyline": loop_route((lat, lng), distance.unwrap_or(0.0), stable_id(&id)),
                "resource_state": 2,
            }),
        ),
        // Strava sends an empty polyline for an activity recorded without GPS.
        _ => (Value::Null, json!({ "summary_polyline": "" })),
    };

    json!({
        "id": stable_id(&id),
        "name": row.try_get::<String, _>("name").unwrap_or_default(),
        "type": row.try_get::<String, _>("sport_type").unwrap_or_else(|_| "Workout".to_owned()),
        "start_date": rfc3339_z(&row.try_get::<String, _>("start_date").unwrap_or_default()),
        "distance": distance,
        "elapsed_time": row.try_get::<Option<i64>, _>("duration_seconds").ok().flatten(),
        "total_elevation_gain": row.try_get::<Option<f64>, _>("elevation_gain").ok().flatten(),
        "average_speed": row.try_get::<Option<f64>, _>("average_speed").ok().flatten(),
        "max_speed": row.try_get::<Option<f64>, _>("max_speed").ok().flatten(),
        "average_heartrate": row.try_get::<Option<f64>, _>("average_heart_rate").ok().flatten(),
        "max_heartrate": row.try_get::<Option<f64>, _>("max_heart_rate").ok().flatten(),
        "calories": row.try_get::<Option<f64>, _>("calories").ok().flatten(),
        "start_latlng": start_latlng,
        "map": map,
        "location_city": row.try_get::<Option<String>, _>("city").ok().flatten(),
        "location_state": row.try_get::<Option<String>, _>("region").ok().flatten(),
        "location_country": row.try_get::<Option<String>, _>("country").ok().flatten(),
    })
}

/// Points on a seeded route's loop: enough to read as a route on a map card
/// and a sketch, few enough that the polyline stays the size Strava's are.
const ROUTE_POINTS: u32 = 72;

/// A seeded route's loop is never wider than this, so a long ride still fits
/// the map around its start.
const MAX_LOOP_RADIUS_METERS: f64 = 12_000.0;

/// Metres in one degree of latitude.
const METERS_PER_DEGREE: f64 = 111_320.0;

/// A seeded activity's route: a closed loop that leaves from and returns to
/// its start, its length about the activity's distance, bent by a wave whose
/// phase comes from the activity id so no two routes share a shape. Encoded
/// the way Strava's `map.summary_polyline` is, so the real provider reads it.
fn loop_route(start: (f64, f64), distance_meters: f64, seed: u64) -> String {
    let radius = (distance_meters / TAU).clamp(150.0, MAX_LOOP_RADIUS_METERS);
    let degrees = u32::try_from(seed % 360).unwrap_or(0);
    let phase = f64::from(degrees) / 360.0 * TAU;
    let meters_per_degree_longitude = METERS_PER_DEGREE * start.0.to_radians().cos();
    // The loop's centre sits one radius from the start, so the loop passes
    // through the start at angle zero.
    let (centre_north, centre_east) = (radius * phase.sin(), radius * phase.cos());
    let points: Vec<(f64, f64)> = (0..=ROUTE_POINTS)
        .map(|step| {
            let angle = f64::from(step) / f64::from(ROUTE_POINTS) * TAU;
            let wave = (0.18 * 3.0_f64.mul_add(angle, phase).sin()).mul_add(angle.sin(), 1.0);
            let north = (radius * wave).mul_add(-(phase + angle).sin(), centre_north);
            let east = (radius * wave).mul_add(-(phase + angle).cos(), centre_east);
            (
                start.0 + north / METERS_PER_DEGREE,
                start.1 + east / meters_per_degree_longitude,
            )
        })
        .collect();
    encode_polyline(&points)
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

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{Duration, Utc};
    use pierre_config::environment::HttpClientConfig;
    use pierre_config::utils::http_client::initialize_http_clients;
    use pierre_providers::core::{
        CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig,
    };
    use pierre_providers::strava_provider::StravaProvider;
    use std::sync::Once;

    static HTTP_CLIENTS: Once = Once::new();

    const ATHLETE: &str = "6f1c2d3e-4a5b-4c6d-8e9f-0a1b2c3d4e5f";
    const ROAD_RIDE: &str = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const TREADMILL_RUN: &str = "1b2c3d4e-5f6a-4b7c-9d8e-0f1a2b3c4d5e";
    const STRANGER: &str = "9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c6b";

    /// A pool holding the seed table's read columns, with one ride recorded
    /// with GPS and one treadmill run recorded without, both the athlete's.
    async fn seeded_pool() -> SqlitePool {
        // One connection: an in-memory database lives and dies with it.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        sqlx::query(
            "CREATE TABLE synthetic_activities (id TEXT PRIMARY KEY, user_id TEXT NOT NULL, \
             name TEXT, sport_type TEXT, start_date TEXT, duration_seconds INTEGER, \
             distance_meters REAL, elevation_gain REAL, average_heart_rate REAL, \
             max_heart_rate REAL, average_speed REAL, max_speed REAL, calories REAL, city TEXT, \
             region TEXT, country TEXT, start_latitude REAL, start_longitude REAL)",
        )
        .execute(&pool)
        .await
        .expect("create table");
        for (id, name, sport, lat, lng) in [
            (ROAD_RIDE, "Morning ride", "ride", Some(45.5), Some(-73.6)),
            (TREADMILL_RUN, "Treadmill run", "run", None, None),
        ] {
            sqlx::query(
                "INSERT INTO synthetic_activities (id, user_id, name, sport_type, start_date, \
                 duration_seconds, distance_meters, start_latitude, start_longitude) \
                 VALUES (?, ?, ?, ?, '2026-09-28T10:00:00', 3600, 10000.0, ?, ?)",
            )
            .bind(id)
            .bind(ATHLETE)
            .bind(name)
            .bind(sport)
            .bind(lat)
            .bind(lng)
            .execute(&pool)
            .await
            .expect("seed row");
        }
        pool
    }

    /// The real Strava provider, its base URL on a fixture serving `pool`,
    /// holding the bearer a seeded `user` carries.
    async fn provider_on_fixture(pool: SqlitePool, user: &str) -> StravaProvider {
        HTTP_CLIENTS.call_once(|| initialize_http_clients(HttpClientConfig::default()));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move { serve(listener, app(pool)).await });

        let provider = StravaProvider::with_config(ProviderConfig {
            name: "strava".to_owned(),
            auth_url: "https://www.strava.com/oauth/authorize".to_owned(),
            token_url: "https://www.strava.com/oauth/token".to_owned(),
            api_base_url: format!("http://{addr}"),
            revoke_url: None,
            default_scopes: vec!["read".to_owned()],
        });
        provider
            .set_credentials(OAuth2Credentials {
                client_id: "dev".to_owned(),
                client_secret: "dev".to_owned(),
                access_token: Some(format!("{BEARER_PREFIX}{user}")),
                refresh_token: Some("dev".to_owned()),
                expires_at: Some(Utc::now() + Duration::days(1)),
                scopes: vec!["read".to_owned()],
                kind: CredentialKind::OAuthBearer,
            })
            .await
            .expect("credentials");
        provider
    }

    #[tokio::test]
    async fn a_seeded_activity_without_gps_reads_as_no_samples_not_a_failed_read() {
        let provider = provider_on_fixture(seeded_pool().await, ATHLETE).await;
        let id = stable_id(TREADMILL_RUN).to_string();

        // The route read: the detail, then the streams. The streams' 404 is
        // Strava's word that nothing was recorded, so the read carries an empty
        // stream set — the activity view settles that as "no GPS". Before the
        // detail route, the detail itself 404'd and the view could only say the
        // map failed to load.
        let activity = provider
            .get_activity_with_streams(&id)
            .await
            .expect("the detail read succeeds");
        assert_eq!(activity.name(), "Treadmill run");
        let streams = activity.time_series_data().expect("a stream set");
        assert!(streams.timestamps.is_empty(), "no samples recorded");
        assert!(streams.gps_coordinates.is_none(), "no track recorded");
    }

    #[tokio::test]
    async fn a_seeded_activity_with_gps_answers_its_detail_and_route() {
        let provider = provider_on_fixture(seeded_pool().await, ATHLETE).await;
        let activity = provider
            .get_activity_detailed(&stable_id(ROAD_RIDE).to_string())
            .await
            .expect("the detail read succeeds");

        assert_eq!(activity.name(), "Morning ride");
        assert_eq!(activity.duration_seconds(), 3600);
        assert!(
            activity
                .summary_polyline()
                .is_some_and(|line| !line.is_empty()),
            "the detail carries the route the list does"
        );
    }

    #[tokio::test]
    async fn another_athletes_activity_and_an_unknown_id_are_not_found() {
        let stranger = provider_on_fixture(seeded_pool().await, STRANGER).await;
        let foreign = stranger
            .get_activity_detailed(&stable_id(ROAD_RIDE).to_string())
            .await
            .expect_err("a row the bearer does not own is not served");
        // The provider reads Strava's `Record Not Found` body as the activity
        // missing, not as a failed request.
        assert!(
            foreign.to_string().contains("not found in strava"),
            "{foreign}"
        );

        let owner = provider_on_fixture(seeded_pool().await, ATHLETE).await;
        let unknown = owner
            .get_activity_detailed("42")
            .await
            .expect_err("an id no row carries is not served");
        assert!(
            unknown.to_string().contains("not found in strava"),
            "{unknown}"
        );
    }

    #[tokio::test]
    async fn the_detail_refuses_a_request_without_the_fixture_bearer() {
        let response = activity_detail(
            State(seeded_pool().await),
            Path(stable_id(ROAD_RIDE).to_string()),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
