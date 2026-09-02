// ABOUTME: Intervals.icu provider — FitnessProvider for athlete profile + activities + streams + wellness + the training-calendar write surface
// ABOUTME: Uses HTTP Basic auth (literal "API_KEY" : api_key) over reqwest; calendar writes create, update, and delete events keyed by Dravr's external_id
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

//! # Intervals.icu Provider Module
//!
//! [`FitnessProvider`] for Intervals.icu (athlete profile, activities, streams,
//! wellness, and the training-calendar write surface) authenticated via HTTP
//! Basic auth with the literal username [`BASIC_AUTH_USERNAME`] and the
//! athlete's API key as the password.
//!
//! ## Calendar writes
//!
//! A [`PlannedSession`] becomes one calendar event: `POST /events` creates it,
//! `PUT /events/{id}` replaces it in place, `PUT /events/bulk-delete` removes
//! a batch by id, and `GET /events` reads the window back so the ledger can be
//! reconciled against the calendar. Every event carries Dravr's `external_id`,
//! and a session's steps go out in Intervals.icu's workout text DSL (see
//! [`render_description`]) so the calendar parses them into targets and
//! computes planned load. The DSL is parsed on every write and cannot be
//! disabled, which is why coach prose is escaped before it is sent.
//!
//! ## Authentication
//!
//! Intervals.icu does not use OAuth. An athlete generates a personal API key in
//! their account settings; the platform stores the athlete id in
//! `user_oauth_tokens.provider_user_id` and the API key in the encrypted
//! access-token column. At request time the registry builds the provider via
//! [`IntervalsIcuProviderFactory`] and the serving path feeds those two values
//! in as [`OAuth2Credentials`] `client_id` / `access_token`.
//!
//! The two values play different roles on the wire: the athlete id addresses
//! the athlete-scoped URL path (`/api/v1/athlete/{id}/...`), while the Basic
//! credential pair is always `API_KEY:<api key>`. Intervals.icu rejects an
//! athlete id in the username position with 401 on every endpoint.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use reqwest::Response;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

use serde_json::json;

use super::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
    TokenRefreshCallback,
};
use crate::activity_paging::pages_for;
use crate::constants::api_provider_limits;
use crate::errors::{AppError, AppResult};
use crate::http_client::{shared_client, SharedHttpClient, SharedHttpError, SharedRequestBuilder};
use crate::intervals_icu_calendar::{
    event_body, event_id_segment, CreatedEvent, DeleteEventsResponse,
};
use crate::models::{
    Activity, ActivityBuilder, Athlete, CalendarEventRef, PersonalRecord, PlannedSession,
    SportType, Stats, TimeSeriesData,
};
use crate::pagination::{CursorPage, PaginationParams};

/// Default base URL for Intervals.icu's REST API (overridable by tests).
pub const DEFAULT_API_BASE_URL: &str = "https://intervals.icu";

/// HTTP Basic auth username for Intervals.icu's API-key scheme.
///
/// Intervals.icu authenticates personal API keys with the *literal* string
/// `API_KEY` as the Basic username and the athlete's key as the password —
/// the athlete id belongs in the URL path, never in the credential pair.
/// Sending the athlete id as the username makes every call 401.
pub const BASIC_AUTH_USERNAME: &str = "API_KEY";

/// Local aliases for the centralised page sizes — every provider's live in
/// `api_provider_limits` so a walk's arithmetic reads one source.
const DEFAULT_PAGE_LIMIT: usize = api_provider_limits::intervals_icu::DEFAULT_ACTIVITIES_PER_PAGE;
const MAX_PAGE_LIMIT: usize = api_provider_limits::intervals_icu::MAX_ACTIVITIES_PER_REQUEST;

/// `strftime` format for the `oldest` / `newest` bounds on a range query.
///
/// Intervals.icu parses these as *local* date-times: no UTC offset, no
/// fractional seconds. `to_rfc3339()` produces both, and the API answers a
/// request carrying them with 422 — which is what every activity list call
/// against the live service got (prod, 2026-08-26: `GET
/// /api/v1/athlete/{id}/activities` → 422). The same file already speaks this
/// dialect everywhere else: `/wellness` and `/events` send `%Y-%m-%d`, and
/// [`parse_local_dt`] reads responses back with this exact pattern.
const QUERY_DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// How far back an activity list reaches when the caller names no lower bound.
///
/// Intervals.icu requires a bounded range — an unbounded list is a 422, not a
/// full-history dump — so every entry point defaults rather than passing the
/// absence through.
const DEFAULT_LOOKBACK_DAYS: i64 = 90;

/// How far forward an activity list reaches when the caller names no upper
/// bound. One day, so an activity uploaded earlier today is inside the window.
const DEFAULT_LOOKAHEAD_DAYS: i64 = 1;

/// Slack subtracted from the lower bound before it goes on the wire, absorbing
/// the local-vs-UTC reading of [`QUERY_DATETIME_FORMAT`].
///
/// What we hold is a UTC instant; what the format puts on the wire is a naive
/// wall clock, and Intervals.icu reads that as *athlete-local*. For an athlete
/// west of UTC our wall clock runs ahead of theirs, so an unpadded `oldest` is
/// read as up to twelve hours later than the caller asked for and activities
/// inside that gap never come back. The upper bound already carries
/// [`DEFAULT_LOOKAHEAD_DAYS`] of slack for a different reason; this is its
/// counterpart on the lower bound.
///
/// Widening is the safe direction. Callers dedupe by activity id on the way
/// into the cache (`write_through_activity_cache`), so an extra day of overlap
/// costs a few redundant rows, while a missing one costs an activity the
/// athlete uploaded this morning — the incremental `after` fetches
/// (`fetch_recent_activities_all_providers`, the fresh-head data path) pass a
/// real lower bound and are exactly the callers that would lose it.
const QUERY_LOCAL_OFFSET_SLACK_DAYS: i64 = 1;

/// Resolve the `(oldest, newest)` bounds an activity list query runs with.
///
/// One source of truth for the defaulting, because the two entry points
/// disagreed: [`FitnessProvider::get_activities_with_params`] defaulted both
/// bounds while `get_activities_cursor` passed `None, None` straight through
/// and asked Intervals.icu for an unbounded range.
fn activity_window(
    after: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    (
        after.unwrap_or_else(|| now - Duration::days(DEFAULT_LOOKBACK_DAYS)),
        before.unwrap_or_else(|| now + Duration::days(DEFAULT_LOOKAHEAD_DAYS)),
    )
}

/// Wrap a `reqwest` request with structured before/after tracing so every
/// Intervals.icu API call surfaces in Cloud Run logs with op + url +
/// HTTP status. Credentials never enter the log line — `RequestBuilder`
/// owns the `basic_auth` header and we only ever read the URL we built.
async fn send_traced(
    req: SharedRequestBuilder,
    op: &'static str,
    url: &str,
) -> Result<Response, SharedHttpError> {
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
///
/// LIMITATION(registre#46): `IntervalsIcuActivity` and `IntervalsIcuWellness` deserialize the
/// numeric fields only — `feel` (inverted: 1 is best), `icu_rpe`, `session_rpe`, `description`,
/// the per-activity message stream and wellness `comments` / `lactate` are dropped on read.
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
    /// holds the athlete id, which addresses the athlete-scoped URL path. The
    /// HTTP Basic username is the constant [`BASIC_AUTH_USERNAME`], never the
    /// athlete id.
    credentials: Arc<RwLock<Option<OAuth2Credentials>>>,
    http: SharedHttpClient,
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
    /// Both bounds are required and are serialised with
    /// [`QUERY_DATETIME_FORMAT`]. Callers that hold an open-ended range resolve
    /// it through [`activity_window`] first, so no request can reach the API
    /// with an offset-bearing timestamp or an unbounded range — the two shapes
    /// Intervals.icu answers with 422.
    ///
    /// `oldest` goes on the wire [`QUERY_LOCAL_OFFSET_SLACK_DAYS`] earlier than
    /// asked: the format is read as athlete-local while the argument is a UTC
    /// instant, and the compensation belongs here, at the boundary that owns
    /// the format, rather than in the window the caller reasons about.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing, the upstream
    /// HTTP call fails, or the response cannot be deserialised.
    pub async fn list_activities(
        &self,
        oldest: DateTime<Utc>,
        newest: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<Activity>> {
        let (athlete_id, api_key) = self.require_credentials().await?;
        let query: Vec<(String, String)> = vec![
            ("limit".to_owned(), limit.min(MAX_PAGE_LIMIT).to_string()),
            (
                "oldest".to_owned(),
                (oldest - Duration::days(QUERY_LOCAL_OFFSET_SLACK_DAYS))
                    .format(QUERY_DATETIME_FORMAT)
                    .to_string(),
            ),
            (
                "newest".to_owned(),
                newest.format(QUERY_DATETIME_FORMAT).to_string(),
            ),
        ];
        let url = self.athlete_url(&athlete_id, "/activities");
        let req = self
            .http
            .get(&url)
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
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
        Ok(raw
            .into_iter()
            .filter_map(|a| map_activity(a, None))
            .collect())
    }

    /// Walk the activity feed backwards through the window until `limit`
    /// activities are collected or the window is exhausted.
    ///
    /// Intervals.icu answers at most [`MAX_PAGE_LIMIT`] activities per request,
    /// and [`Self::list_activities`] clamped the caller's limit to it silently.
    /// That truncation is not local: the historical backfill asks every provider
    /// for two thousand activities and then reads `fetched_count < requested_limit` as
    /// proof the window was exhausted, so a provider that quietly returns 200
    /// makes the backfill record a depth it never reached — and the gate then
    /// serves that shallow slice as a complete season, permanently. Strava and
    /// Garmin already page internally for this reason; this puts Intervals.icu
    /// on the same contract.
    ///
    /// The walk steps `newest` down to the oldest activity of each page
    /// *inclusively* and de-duplicates by activity id. An exclusive step would
    /// be tidier but drops rows: several activities can share one start time,
    /// and a page boundary can fall between them. Repeating a boundary row costs
    /// one duplicate that the id filter removes; skipping past it loses an
    /// activity outright.
    ///
    /// Terminates on any of: the limit reached, a short page (nothing older in
    /// the window), a page contributing no new id (the window stopped advancing),
    /// an exhausted window, or the shared ceiling from [`pages_for`].
    async fn list_activities_paged(
        &self,
        oldest: DateTime<Utc>,
        newest: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<Activity>> {
        let mut collected: Vec<Activity> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut cursor = newest;
        // One page of slack over the caller's limit: the step is inclusive, so
        // each boundary re-delivers a row the id filter then drops, and without
        // it a walk can finish a page short of what was asked for. `pages_for`
        // applies the ceiling every provider shares.
        let pages = pages_for(limit.saturating_add(MAX_PAGE_LIMIT), MAX_PAGE_LIMIT);

        for _ in 0..pages {
            if collected.len() >= limit || cursor <= oldest {
                break;
            }
            let page = self.list_activities(oldest, cursor, MAX_PAGE_LIMIT).await?;
            let page_len = page.len();
            let mut oldest_in_page: Option<DateTime<Utc>> = None;
            let mut added = 0_usize;
            for activity in page {
                let start = activity.start_date();
                oldest_in_page = Some(oldest_in_page.map_or(start, |cur| cur.min(start)));
                if seen.insert(activity.id().to_owned()) {
                    collected.push(activity);
                    added += 1;
                }
            }
            // A short page means the window holds nothing older. `added == 0`
            // means a full page repeated what we already had, so `cursor` is no
            // longer advancing — the guard that makes the pathological
            // same-timestamp feed terminate instead of spinning to the cap.
            if page_len < MAX_PAGE_LIMIT || added == 0 {
                break;
            }
            let Some(oldest_in_page) = oldest_in_page else {
                break;
            };
            cursor = oldest_in_page;
        }

        collected.truncate(limit);
        Ok(collected)
    }

    /// Fetch the per-second time-series streams for a given activity.
    ///
    /// # Errors
    ///
    /// Returns [`AppError`] when credentials are missing or the upstream
    /// HTTP call fails. Returns `Ok(None)` for HTTP 404 (activity has no
    /// stream data).
    pub async fn get_streams(&self, activity_id: &str) -> AppResult<Option<TimeSeriesData>> {
        let (_, api_key) = self.require_credentials().await?;
        let url = self.activity_url(activity_id, "/streams.json");
        let req = self
            .http
            .get(&url)
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
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
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
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
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
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
    /// The writer's own key for the event, when one was set (Dravr sets
    /// [`PlannedSession::external_id`] on every event it writes).
    #[serde(default)]
    pub external_id: Option<String>,
    /// When the event last changed, as Intervals.icu reports it.
    #[serde(default)]
    pub updated: Option<String>,
}

impl IntervalsIcuEvent {
    /// The identity-and-freshness view a reconcile needs.
    ///
    /// # Errors
    ///
    /// Returns an error when `start_date_local` does not begin with a civil
    /// date — an event the calendar cannot place on a day cannot be reconciled.
    fn calendar_event_ref(self) -> AppResult<CalendarEventRef> {
        let day = self.start_date_local.get(..10).unwrap_or_default();
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|e| {
            AppError::external_service(
                "intervals_icu",
                format!(
                    "event {} has no civil date in '{}': {e}",
                    self.id, self.start_date_local
                ),
            )
        })?;
        Ok(CalendarEventRef {
            provider_event_id: self.id.to_string(),
            external_id: self.external_id,
            date,
            updated_at: self.updated.as_deref().and_then(parse_local_dt),
        })
    }
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
        // latlng is a FLAT interleaved list: its sample count is half its
        // raw length. Counting it raw synthesised a timestamp axis twice as
        // long as the real recording whenever GPS was present.
        let sample_len = if stream.stream_type == "latlng" {
            stream.data.len() / 2
        } else {
            stream.data.len()
        };
        max_len = max_len.max(sample_len);
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

fn map_activity(raw: IntervalsIcuActivity, streams: Option<TimeSeriesData>) -> Option<Activity> {
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
    builder = builder.time_series_data_opt(streams);
    Some(builder.build())
}

fn parse_local_dt(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, QUERY_DATETIME_FORMAT)
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
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
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
        // Not clamped to MAX_PAGE_LIMIT: a caller asking for a season gets a
        // season. The walk bounds itself by MAX_ACTIVITY_PAGES instead.
        let limit = params.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
        let (oldest, newest) = activity_window(
            params
                .after
                .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0)),
            params
                .before
                .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0)),
        );
        self.list_activities_paged(oldest, newest, limit).await
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        let limit = params.limit;
        let (oldest, newest) = activity_window(None, None);
        let activities = self.list_activities(oldest, newest, limit).await?;
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
        let (_, api_key) = self.require_credentials().await?;
        let url = self.activity_url(id, "");
        let req = self
            .http
            .get(&url)
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
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
        map_activity(raw, None)
            .ok_or_else(|| AppError::external_service("intervals_icu", "could not map activity"))
    }

    // The streams endpoint is a second round trip, so only this tier pays
    // it. Best-effort: a streams failure degrades to the plain activity —
    // stale-less summary beats a dead export.
    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        let (_, api_key) = self.require_credentials().await?;
        let url = self.activity_url(id, "");
        let req = self
            .http
            .get(&url)
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
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
        let streams = match self.get_streams(id).await {
            Ok(streams) => streams,
            Err(e) => {
                warn!(activity_id = %id, error = %e, "intervals_icu streams fetch failed; serving the activity without them");
                None
            }
        };
        map_activity(raw, streams)
            .ok_or_else(|| AppError::external_service("intervals_icu", "could not map activity"))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        // Intervals.icu doesn't expose an aggregate-stats endpoint; derive
        // a 90-day rollup from the activity list so the trait surface
        // returns real data instead of synthetic zeros.
        let (oldest, newest) = activity_window(None, None);
        let activities = self.list_activities(oldest, newest, MAX_PAGE_LIMIT).await?;
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
            year_to_date: None,
        })
    }

    async fn get_personal_records(&self) -> AppResult<Vec<PersonalRecord>> {
        // LIMITATION(registre#45): `IntervalsIcuProvider::get_personal_records` returns an empty
        // list — nothing derives records from intervals.icu's best-effort curves, so a caller
        // cannot tell a recordless athlete from one this provider never read.
        Ok(Vec::new())
    }

    async fn list_calendar_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        self.get_events(from, to)
            .await?
            .into_iter()
            .map(IntervalsIcuEvent::calendar_event_ref)
            .collect()
    }

    async fn push_planned_session(&self, session: &PlannedSession) -> AppResult<String> {
        let (athlete_id, api_key) = self.require_credentials().await?;
        let url = self.athlete_url(&athlete_id, "/events");
        let req = self
            .http
            .post(&url)
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
            .header("Accept", "application/json")
            .json(&event_body(session));
        let response = send_traced(req, "push_planned_session", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("push_planned_session: {e}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("push_planned_session returned {}", response.status()),
            ));
        }
        let created: CreatedEvent = response.json().await.map_err(|e| {
            AppError::external_service("intervals_icu", format!("push_planned_session decode: {e}"))
        })?;
        Ok(created.id.to_string())
    }

    async fn update_planned_session(
        &self,
        provider_event_id: &str,
        session: &PlannedSession,
    ) -> AppResult<()> {
        let event_id = event_id_segment(provider_event_id)?;
        let (athlete_id, api_key) = self.require_credentials().await?;
        let url = self.athlete_url(&athlete_id, &format!("/events/{event_id}"));
        let req = self
            .http
            .put(&url)
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
            .header("Accept", "application/json")
            .json(&event_body(session));
        let response = send_traced(req, "update_planned_session", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("update_planned_session: {e}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("update_planned_session returned {}", response.status()),
            ));
        }
        Ok(())
    }

    async fn delete_planned_sessions(&self, provider_event_ids: &[String]) -> AppResult<u64> {
        if provider_event_ids.is_empty() {
            return Ok(0);
        }
        // By id, never by `external_id`: id deletion is authentication-agnostic,
        // whereas the `external_id` form only reaches events "created by the
        // calling OAuth application", and this provider authenticates with the
        // athlete's API key. Never the date-range delete either — it would take
        // events Dravr did not write.
        let doomed = provider_event_ids
            .iter()
            .map(|id| event_id_segment(id).map(|n| json!({ "id": n })))
            .collect::<AppResult<Vec<_>>>()?;
        let (athlete_id, api_key) = self.require_credentials().await?;
        let url = self.athlete_url(&athlete_id, "/events/bulk-delete");
        let req = self
            .http
            .put(&url)
            .basic_auth(BASIC_AUTH_USERNAME, Some(&api_key))
            .header("Accept", "application/json")
            .json(&doomed);
        let response = send_traced(req, "delete_planned_sessions", &url)
            .await
            .map_err(|e| {
                AppError::external_service("intervals_icu", format!("delete_planned_sessions: {e}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                "intervals_icu",
                format!("delete_planned_sessions returned {}", response.status()),
            ));
        }
        let deleted: DeleteEventsResponse = response.json().await.map_err(|e| {
            AppError::external_service(
                "intervals_icu",
                format!("delete_planned_sessions decode: {e}"),
            )
        })?;
        Ok(deleted.events_deleted)
    }
}

/// Factory that builds [`IntervalsIcuProvider`] instances for the
/// [`ProviderRegistry`](crate::registry::ProviderRegistry).
pub struct IntervalsIcuProviderFactory;

impl ProviderFactory for IntervalsIcuProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(IntervalsIcuProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["intervals_icu"]
    }
}
