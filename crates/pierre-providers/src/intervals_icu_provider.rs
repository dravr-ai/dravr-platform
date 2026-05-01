// ABOUTME: Intervals.icu pull-only provider — implements FitnessProvider for athlete profile + activities + streams + wellness
// ABOUTME: Uses HTTP Basic auth (athlete_id : api_key) over reqwest; push surface lands in Phase 5 alongside the workout library
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// Clippy allowances for this module:
// - cast_possible_truncation: Intervals.icu returns f64 for HR/power/cadence; truncating to u32 is safe within sensor ranges
// - cast_sign_loss: same — HR/power/cadence are always non-negative
// - cast_precision_loss: same — single-second timestamps fit in f64 without loss
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use reqwest::Client;
use reqwest::{RequestBuilder, Response};
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, TokenRefreshCallback,
};
use crate::errors::{AppError, AppResult};
use crate::http_client::shared_client;
use crate::models::{
    Activity, ActivityBuilder, Athlete, PersonalRecord, SportType, Stats, TimeSeriesData,
};
use crate::pagination::{CursorPage, PaginationParams};

/// Default base URL for Intervals.icu's REST API (overridable by tests).
pub const DEFAULT_API_BASE_URL: &str = "https://intervals.icu";

const DEFAULT_PAGE_LIMIT: usize = 30;
const MAX_PAGE_LIMIT: usize = 200;

/// One-shot guard that fires `warn!` the first time an
/// [`IntervalsIcuProvider`] is constructed in this process. Endurance
/// Phase 4 risk #4: this provider has zero real-API soak time, so the
/// log gives operators a Cloud-Run-visible breadcrumb that someone has
/// linked an Intervals.icu account for the first time and we should
/// watch for unexpected response shapes / rate-limit headers.
static FIRST_INSTANTIATION: OnceLock<()> = OnceLock::new();

/// Wrap a `reqwest` request with structured before/after tracing so every
/// Intervals.icu API call surfaces in Cloud Run logs with op + url +
/// HTTP status. Credentials never enter the log line — `RequestBuilder`
/// owns the `basic_auth` header and we only ever read the URL we built.
async fn send_traced(
    req: RequestBuilder,
    op: &'static str,
    url: &str,
) -> reqwest::Result<Response> {
    info!(provider = "intervals_icu", op, url, "request");
    match req.send().await {
        Ok(response) => {
            info!(
                provider = "intervals_icu",
                op,
                url,
                status = response.status().as_u16(),
                content_length = response.content_length().unwrap_or(0),
                "response"
            );
            Ok(response)
        }
        Err(e) => {
            warn!(provider = "intervals_icu", op, url, error = %e, "transport failure");
            Err(e)
        }
    }
}

/// Provider configuration helper for Intervals.icu credentials.
///
/// Intervals.icu doesn't use `OAuth2` in the classical sense (athletes
/// generate a personal API key in their account settings), but
/// `OAuth2Credentials::access_token` carries the API key so we can
/// reuse the existing storage path.
#[must_use]
pub fn default_config() -> ProviderConfig {
    ProviderConfig {
        name: "intervals_icu".to_owned(),
        auth_url: format!("{DEFAULT_API_BASE_URL}/account/api"),
        token_url: format!("{DEFAULT_API_BASE_URL}/api/v1/oauth/token"),
        api_base_url: DEFAULT_API_BASE_URL.to_owned(),
        revoke_url: None,
        default_scopes: Vec::new(),
    }
}

/// Intervals.icu activity payload shape (subset we map to `Activity`).
#[derive(Debug, Deserialize)]
struct IntervalsIcuActivity {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    activity_type: Option<String>,
    start_date_local: String,
    #[serde(default)]
    elapsed_time: Option<u64>,
    #[serde(default)]
    distance: Option<f64>,
    #[serde(default)]
    total_elevation_gain: Option<f64>,
    #[serde(default)]
    average_heartrate: Option<f64>,
    #[serde(default)]
    max_heartrate: Option<f64>,
    #[serde(default)]
    average_speed: Option<f64>,
    #[serde(default)]
    max_speed: Option<f64>,
    #[serde(default)]
    calories: Option<u32>,
    #[serde(default)]
    average_cadence: Option<f64>,
    #[serde(default)]
    average_watts: Option<f64>,
    #[serde(default)]
    max_watts: Option<f64>,
    #[serde(default)]
    weighted_average_watts: Option<f64>,
    #[serde(default)]
    icu_ftp: Option<f64>,
}

/// Intervals.icu athlete profile shape.
#[derive(Debug, Deserialize)]
struct IntervalsIcuAthlete {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    profile_medium: Option<String>,
}

/// Provider implementation for Intervals.icu.
pub struct IntervalsIcuProvider {
    config: ProviderConfig,
    /// Stored credentials. `access_token` holds the API key; `client_id`
    /// holds the athlete id (Intervals.icu uses HTTP Basic auth with the
    /// athlete id as username and the API key as password).
    credentials: Arc<RwLock<Option<OAuth2Credentials>>>,
    http: Client,
}

impl IntervalsIcuProvider {
    /// Construct with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(default_config())
    }

    /// Construct with a custom configuration (test override for the base URL).
    #[must_use]
    pub fn with_config(config: ProviderConfig) -> Self {
        FIRST_INSTANTIATION.get_or_init(|| {
            warn!(
                provider = "intervals_icu",
                "Intervals.icu provider instantiated for the first time in this process — \
                 beta integration with no real-API soak time; watch logs for unexpected \
                 response shapes and rate-limit headers"
            );
        });
        Self {
            config,
            credentials: Arc::new(RwLock::new(None)),
            http: shared_client().clone(),
        }
    }

    async fn require_credentials(&self) -> AppResult<(String, String)> {
        let guard = self.credentials.read().await;
        let creds = guard.as_ref().ok_or_else(|| {
            AppError::auth_invalid("intervals.icu credentials not set — link your account first")
        })?;
        let athlete_id = creds.client_id.clone();
        let api_key = creds
            .access_token
            .clone()
            .ok_or_else(|| AppError::auth_invalid("intervals.icu API key missing"))?;
        if athlete_id.is_empty() {
            return Err(AppError::auth_invalid(
                "intervals.icu athlete id missing (set OAuth2Credentials.client_id to the i123456 athlete id)",
            ));
        }
        Ok((athlete_id, api_key))
    }

    fn athlete_url(&self, athlete_id: &str, suffix: &str) -> String {
        format!(
            "{}/api/v1/athlete/{}{}",
            self.config.api_base_url, athlete_id, suffix
        )
    }

    fn activity_url(&self, activity_id: &str, suffix: &str) -> String {
        format!(
            "{}/api/v1/activity/{}{}",
            self.config.api_base_url, activity_id, suffix
        )
    }

    /// List activities between `oldest` and `newest` (inclusive) — wraps the
    /// `/api/v1/athlete/{id}/activities` endpoint with athlete-scoped Basic
    /// auth.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing, the upstream
    /// HTTP call fails, or the response cannot be deserialised.
    pub async fn list_activities(
        &self,
        oldest: Option<DateTime<Utc>>,
        newest: Option<DateTime<Utc>>,
        limit: usize,
    ) -> AppResult<Vec<Activity>> {
        let (athlete_id, api_key) = self.require_credentials().await?;
        let mut query: Vec<(String, String)> =
            vec![("limit".to_owned(), limit.min(MAX_PAGE_LIMIT).to_string())];
        if let Some(o) = oldest {
            query.push(("oldest".to_owned(), o.to_rfc3339()));
        }
        if let Some(n) = newest {
            query.push(("newest".to_owned(), n.to_rfc3339()));
        }
        let url = self.athlete_url(&athlete_id, "/activities");
        let req = self
            .http
            .get(&url)
            .basic_auth(&athlete_id, Some(&api_key))
            .header("Accept", "application/json")
            .query(&query);
        let response = send_traced(req, "list_activities", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("list_activities: {e}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("list_activities returned {}", response.status()),
            ));
        }
        let raw: Vec<IntervalsIcuActivity> = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("list_activities decode: {e}"))
        })?;
        Ok(raw.into_iter().filter_map(map_activity).collect())
    }

    /// Fetch the per-second time-series streams for a given activity.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing or the upstream
    /// HTTP call fails. Returns `Ok(None)` for HTTP 404 (activity has no
    /// stream data).
    pub async fn get_streams(&self, activity_id: &str) -> AppResult<Option<TimeSeriesData>> {
        let (athlete_id, api_key) = self.require_credentials().await?;
        let url = self.activity_url(activity_id, "/streams.json");
        let req = self
            .http
            .get(&url)
            .basic_auth(&athlete_id, Some(&api_key))
            .header("Accept", "application/json");
        let response = send_traced(req, "get_streams", &url).await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_streams: {e}"))
        })?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("get_streams returned {}", response.status()),
            ));
        }
        let raw: Vec<IntervalsIcuStream> = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_streams decode: {e}"))
        })?;
        Ok(Some(streams_to_time_series(&raw)))
    }

    /// Fetch the daily wellness rows for the date range — Intervals.icu's
    /// canonical feed for HRV / RHR / sleep / weight.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing or the upstream
    /// HTTP call fails.
    pub async fn get_wellness(
        &self,
        oldest: NaiveDate,
        newest: NaiveDate,
    ) -> AppResult<Vec<IntervalsIcuWellness>> {
        let (athlete_id, api_key) = self.require_credentials().await?;
        let url = self.athlete_url(&athlete_id, "/wellness");
        let req = self
            .http
            .get(&url)
            .basic_auth(&athlete_id, Some(&api_key))
            .header("Accept", "application/json")
            .query(&[
                ("oldest", oldest.format("%Y-%m-%d").to_string()),
                ("newest", newest.format("%Y-%m-%d").to_string()),
            ]);
        let response = send_traced(req, "get_wellness", &url).await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_wellness: {e}"))
        })?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("get_wellness returned {}", response.status()),
            ));
        }
        response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_wellness decode: {e}"))
        })
    }

    /// Fetch the calendar events (planned workouts + races) for the date range.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing or the upstream
    /// HTTP call fails.
    pub async fn get_events(
        &self,
        oldest: NaiveDate,
        newest: NaiveDate,
    ) -> AppResult<Vec<IntervalsIcuEvent>> {
        let (athlete_id, api_key) = self.require_credentials().await?;
        let url = self.athlete_url(&athlete_id, "/events");
        let req = self
            .http
            .get(&url)
            .basic_auth(&athlete_id, Some(&api_key))
            .header("Accept", "application/json")
            .query(&[
                ("oldest", oldest.format("%Y-%m-%d").to_string()),
                ("newest", newest.format("%Y-%m-%d").to_string()),
            ]);
        let response = send_traced(req, "get_events", &url)
            .await
            .map_err(|e| AppError::external_service("intervals_icu", format!("get_events: {e}")))?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("get_events returned {}", response.status()),
            ));
        }
        response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_events decode: {e}"))
        })
    }
}

impl Default for IntervalsIcuProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Daily wellness row from Intervals.icu (`/api/v1/athlete/{id}/wellness`).
#[derive(Debug, Clone, Deserialize)]
pub struct IntervalsIcuWellness {
    /// Calendar date (YYYY-MM-DD).
    pub id: String,
    /// HRV root-mean-square (ms).
    #[serde(default)]
    pub hrv: Option<f64>,
    /// Resting heart rate (bpm).
    #[serde(default, rename = "restingHR")]
    pub resting_hr: Option<f64>,
    /// Body weight (kg).
    #[serde(default)]
    pub weight: Option<f64>,
    /// Sleep duration in seconds.
    #[serde(default)]
    pub sleep_secs: Option<u64>,
    /// Sleep quality (Intervals.icu 0-5 scale).
    #[serde(default)]
    pub sleep_quality: Option<u8>,
    /// Athlete's perceived form (1-10 scale).
    #[serde(default)]
    pub readiness: Option<f64>,
}

/// Calendar event row from Intervals.icu (`/api/v1/athlete/{id}/events`).
#[derive(Debug, Clone, Deserialize)]
pub struct IntervalsIcuEvent {
    /// Event id.
    pub id: i64,
    /// Event date (`YYYY-MM-DD` for races, full ISO 8601 for sessions).
    pub start_date_local: String,
    /// Free-form event name.
    #[serde(default)]
    pub name: Option<String>,
    /// Event category — `WORKOUT`, `RACE_A`, `NOTE`, etc.
    #[serde(default, rename = "type")]
    pub event_type: Option<String>,
    /// Sport label.
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IntervalsIcuStream {
    #[serde(rename = "type")]
    stream_type: String,
    data: Vec<f64>,
}

fn streams_to_time_series(streams: &[IntervalsIcuStream]) -> TimeSeriesData {
    let mut hr: Option<Vec<u32>> = None;
    let mut power: Option<Vec<u32>> = None;
    let mut cadence: Option<Vec<u32>> = None;
    let mut speed: Option<Vec<f32>> = None;
    let mut altitude: Option<Vec<f32>> = None;
    let mut latlng: Vec<(f64, f64)> = Vec::new();
    let mut latlng_pending: Option<f64> = None;
    let mut max_len = 0_usize;
    for stream in streams {
        max_len = max_len.max(stream.data.len());
        match stream.stream_type.as_str() {
            "heartrate" => {
                hr = Some(stream.data.iter().map(|v| *v as u32).collect());
            }
            "watts" => {
                power = Some(stream.data.iter().map(|v| *v as u32).collect());
            }
            "cadence" => {
                cadence = Some(stream.data.iter().map(|v| *v as u32).collect());
            }
            "velocity_smooth" => {
                speed = Some(stream.data.iter().map(|v| *v as f32).collect());
            }
            "altitude" => {
                altitude = Some(stream.data.iter().map(|v| *v as f32).collect());
            }
            "latlng" => {
                // Intervals.icu sends interleaved lat/lon as a flat list.
                for value in &stream.data {
                    if let Some(lat) = latlng_pending.take() {
                        latlng.push((lat, *value));
                    } else {
                        latlng_pending = Some(*value);
                    }
                }
            }
            _ => {}
        }
    }
    let timestamps: Vec<u32> = (0..max_len)
        .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
        .collect();
    TimeSeriesData {
        timestamps,
        heart_rate: hr,
        power,
        cadence,
        speed,
        altitude,
        temperature: None,
        gps_coordinates: if latlng.is_empty() {
            None
        } else {
            Some(latlng)
        },
    }
}

fn map_activity(raw: IntervalsIcuActivity) -> Option<Activity> {
    let start_date = parse_local_dt(&raw.start_date_local)?;
    let sport = sport_for(raw.activity_type.as_deref());
    let name = raw
        .name
        .unwrap_or_else(|| format!("Intervals.icu {}", raw.id));
    let mut builder = ActivityBuilder::new(
        raw.id,
        name,
        sport,
        start_date,
        raw.elapsed_time.unwrap_or(0),
        "intervals_icu".to_owned(),
    );
    if let Some(d) = raw.distance {
        builder = builder.distance_meters(d);
    }
    if let Some(g) = raw.total_elevation_gain {
        builder = builder.elevation_gain(g);
    }
    if let Some(hr) = raw.average_heartrate {
        builder = builder.average_heart_rate(hr as u32);
    }
    if let Some(hr) = raw.max_heartrate {
        builder = builder.max_heart_rate(hr as u32);
    }
    if let Some(s) = raw.average_speed {
        builder = builder.average_speed(s);
    }
    if let Some(s) = raw.max_speed {
        builder = builder.max_speed(s);
    }
    if let Some(c) = raw.calories {
        builder = builder.calories(c);
    }
    if let Some(c) = raw.average_cadence {
        builder = builder.average_cadence(c as u32);
    }
    if let Some(p) = raw.average_watts {
        builder = builder.average_power(p as u32);
    }
    if let Some(p) = raw.max_watts {
        builder = builder.max_power(p as u32);
    }
    if let Some(p) = raw.weighted_average_watts {
        builder = builder.normalized_power(p as u32);
    }
    if let Some(ftp) = raw.icu_ftp {
        builder = builder.ftp(ftp as u32);
    }
    Some(builder.build())
}

fn parse_local_dt(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
                .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
                .ok()
        })
}

fn sport_for(label: Option<&str>) -> SportType {
    match label.unwrap_or("").to_ascii_lowercase().as_str() {
        "ride" | "virtualride" | "ebikeride" | "cycling" => SportType::Ride,
        "run" | "trailrun" | "virtualrun" | "running" => SportType::Run,
        "swim" => SportType::Swim,
        "walk" | "hike" => SportType::Walk,
        "yoga" => SportType::Yoga,
        "weighttraining" | "workout" => SportType::Workout,
        other if !other.is_empty() => SportType::Other(other.to_owned()),
        _ => SportType::Other("intervals_icu".to_owned()),
    }
}

#[async_trait]
impl FitnessProvider for IntervalsIcuProvider {
    fn name(&self) -> &'static str {
        "intervals_icu"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        if credentials.client_id.is_empty() {
            return Err(AppError::invalid_input(
                "intervals_icu requires the athlete id (e.g. i123456) in OAuth2Credentials.client_id",
            ));
        }
        let api_key_set = credentials
            .access_token
            .as_ref()
            .is_some_and(|s| !s.is_empty());
        if !api_key_set {
            return Err(AppError::invalid_input(
                "intervals_icu requires an API key in OAuth2Credentials.access_token",
            ));
        }
        let mut guard = self.credentials.write().await;
        *guard = Some(credentials);
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        self.credentials.read().await.is_some()
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        // API keys don't expire; nothing to refresh.
        Ok(())
    }

    fn set_token_refresh_callback(&self, _callback: TokenRefreshCallback) {
        // No-op — API keys don't refresh.
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        let (athlete_id, api_key) = self.require_credentials().await?;
        let url = self.athlete_url(&athlete_id, "");
        let req = self
            .http
            .get(&url)
            .basic_auth(&athlete_id, Some(&api_key))
            .header("Accept", "application/json");
        let response = send_traced(req, "get_athlete", &url).await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_athlete: {e}"))
        })?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("get_athlete returned {}", response.status()),
            ));
        }
        let raw: IntervalsIcuAthlete = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_athlete decode: {e}"))
        })?;
        let display_name = raw.name.unwrap_or_else(|| raw.id.clone());
        Ok(Athlete {
            id: raw.id,
            username: raw.email.unwrap_or_default(),
            firstname: Some(display_name),
            lastname: None,
            profile_picture: raw.profile_medium,
            provider: "intervals_icu".to_owned(),
        })
    }

    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        let limit = params
            .limit
            .unwrap_or(DEFAULT_PAGE_LIMIT)
            .min(MAX_PAGE_LIMIT);
        let oldest = params
            .after
            .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0));
        let newest = params
            .before
            .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0));
        // Default newest to "now" + 1d so today's recently-uploaded activities surface.
        let newest = newest.or_else(|| Some(Utc::now() + Duration::days(1)));
        // Default oldest to 90 days back so list calls don't pull a full lifetime.
        let oldest = oldest.or_else(|| Some(Utc::now() - Duration::days(90)));
        self.list_activities(oldest, newest, limit).await
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        let limit = params.limit;
        let activities = self.list_activities(None, None, limit).await?;
        let count = activities.len();
        Ok(CursorPage {
            items: activities,
            next_cursor: None,
            prev_cursor: None,
            has_more: false,
            count,
        })
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        let (athlete_id, api_key) = self.require_credentials().await?;
        let url = self.activity_url(id, "");
        let req = self
            .http
            .get(&url)
            .basic_auth(&athlete_id, Some(&api_key))
            .header("Accept", "application/json");
        let response = send_traced(req, "get_activity", &url).await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_activity: {e}"))
        })?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("get_activity returned {}", response.status()),
            ));
        }
        let raw: IntervalsIcuActivity = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("get_activity decode: {e}"))
        })?;
        map_activity(raw)
            .ok_or_else(|| AppError::external_service("intervals_icu", "could not map activity"))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        // Intervals.icu doesn't expose an aggregate-stats endpoint; derive
        // a 90-day rollup from the activity list so the trait surface
        // returns real data instead of synthetic zeros.
        let activities = self
            .list_activities(
                Some(Utc::now() - Duration::days(90)),
                Some(Utc::now() + Duration::days(1)),
                MAX_PAGE_LIMIT,
            )
            .await?;
        let total_activities = activities.len() as u64;
        let total_distance: f64 = activities
            .iter()
            .map(|a| a.distance_meters().unwrap_or(0.0))
            .sum();
        let total_duration: u64 = activities.iter().map(Activity::duration_seconds).sum();
        let total_elevation_gain: f64 = activities
            .iter()
            .map(|a| a.elevation_gain().unwrap_or(0.0))
            .sum();
        Ok(Stats {
            total_activities,
            total_distance,
            total_duration,
            total_elevation_gain,
        })
    }

    async fn get_personal_records(&self) -> AppResult<Vec<PersonalRecord>> {
        // Personal records aren't part of the Intervals.icu pull-only
        // surface in Phase 4. Return an empty list so callers see "no
        // records yet" rather than an error.
        Ok(Vec::new())
    }

    async fn disconnect(&self) -> AppResult<()> {
        let mut guard = self.credentials.write().await;
        *guard = None;
        Ok(())
    }
}
