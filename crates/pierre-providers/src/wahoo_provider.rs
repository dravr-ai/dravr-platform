// ABOUTME: Wahoo Cloud API provider — completed workouts with their FIT samples, and the calendar Dravr schedules plans into
// ABOUTME: OAuth2 with rotating refresh tokens refreshed just before a call; plans written neutral through wahoo_plan
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - HTTP client Arc sharing across async operations (shared_client().clone())
// - String ownership for API responses and error handling

//! # Wahoo provider
//!
//! Reads the athlete's completed Wahoo workouts — a *workout* record with its
//! *workout summary* (NP and TSS included) and the summary's activity FIT
//! file — and writes the training calendar: a structured session is a plan in
//! the athlete's Wahoo library plus a workout scheduled on its day
//! ([`crate::wahoo_plan`] renders both, neutral by construction).
//!
//! **Tokens.** Access tokens live two hours; a refresh returns a new pair,
//! and the previous pair is revoked once an API call is made with the new
//! access token. Wahoo also caps an app at ten unrevoked tokens per athlete,
//! so a token is refreshed only right before the call that uses it — never
//! ahead of time (Wahoo Cloud API, "Token Limits").
//!
//! **Listing.** `GET /v1/workouts` takes only `page` and `per_page` and
//! answers newest `starts` first, so a time window is read by walking pages
//! until the workouts start before it. A workout with no summary is a
//! scheduled one (Dravr's own, or the athlete's), never an activity.

use super::circuit_breaker::CircuitBreaker;
use super::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
    TokenRefreshCallback,
};
use super::errors::provider::ProviderError;
use crate::activity_paging::{max_activity_pages, MAX_ACTIVITY_PAGES_ENV};
use crate::constants::{api_provider_limits, oauth_providers};
use crate::errors::{AppError, AppResult, ErrorCode};
use crate::http_client::{shared_client, SharedHttpClient};
use crate::models::refresh_due;
use crate::models::{
    Activity, ActivityBuilder, Athlete, CalendarEventRef, PlannedSession, PlannedSessionKind,
    Stats, TimeSeriesData,
};
use crate::pagination::{Cursor, CursorPage, PaginationParams};
use crate::request_budget;
use crate::utils;
use crate::wahoo_plan::{
    self, is_dravr_token, opaque_id, sport_for_workout_type, WORKOUT_TYPE_UNKNOWN,
};
use crate::wahoo_streams::{is_wahoo_file_url, time_series_from_fit, MAX_FIT_BYTES};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::{Method, StatusCode};
use serde::de::{self, Deserializer};
use serde::Deserialize;
use std::sync::OnceLock;
use tokio::sync::{OnceCell, RwLock};
use tracing::{debug, info, instrument, warn};

// ============================================================================
// Wahoo API response structures
// ============================================================================

/// A Wahoo decimal: the API sends decimals as strings (`"450.00"`), and a
/// number or `null` is read the same way.
fn decimal<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Text(String),
        Number(f64),
    }
    match Option::<Raw>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Raw::Number(n)) => Ok(n.is_finite().then_some(n)),
        Some(Raw::Text(text)) if text.trim().is_empty() => Ok(None),
        Some(Raw::Text(text)) => text
            .trim()
            .parse::<f64>()
            .map(|n| n.is_finite().then_some(n))
            .map_err(de::Error::custom),
    }
}

/// `GET /v1/user`.
#[derive(Debug, Deserialize)]
struct WahooUser {
    id: i64,
    #[serde(default)]
    first: Option<String>,
    #[serde(default)]
    last: Option<String>,
    /// Present with the `email` scope.
    #[serde(default)]
    email: Option<String>,
}

/// A file Wahoo stores on its CDN.
#[derive(Debug, Clone, Default, Deserialize)]
struct WahooFile {
    #[serde(default)]
    url: Option<String>,
}

/// A workout summary: the results of a completed workout.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct WahooWorkoutSummary {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "decimal")]
    ascent_accum: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    cadence_avg: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    calories_accum: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    distance_accum: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    duration_total_accum: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    heart_rate_avg: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    power_bike_np_last: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    power_bike_tss_last: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    power_avg: Option<f64>,
    #[serde(default, deserialize_with = "decimal")]
    speed_avg: Option<f64>,
    #[serde(default)]
    file: Option<WahooFile>,
}

/// A workout record: scheduled, or completed when it carries a summary.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WahooWorkout {
    id: i64,
    starts: String,
    #[serde(default)]
    minutes: Option<f64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    plan_id: Option<i64>,
    #[serde(default)]
    workout_token: Option<String>,
    #[serde(default)]
    workout_type_id: Option<u16>,
    #[serde(default)]
    workout_summary: Option<WahooWorkoutSummary>,
    #[serde(default)]
    updated_at: Option<String>,
}

/// One page of `GET /v1/workouts`.
#[derive(Debug, Deserialize)]
struct WahooWorkoutPage {
    #[serde(default)]
    workouts: Vec<WahooWorkout>,
    #[serde(default)]
    total: Option<usize>,
}

/// A plan in the athlete's library, as `GET /v1/plans` lists it.
#[derive(Debug, Deserialize)]
struct WahooPlan {
    id: i64,
    #[serde(default)]
    deleted: bool,
}

/// A created or updated record: only its id is read.
#[derive(Debug, Deserialize)]
struct WahooCreated {
    id: i64,
}

/// One of the athlete's power-zone sets.
#[derive(Debug, Deserialize)]
struct WahooPowerZone {
    #[serde(default, deserialize_with = "decimal")]
    ftp: Option<f64>,
    #[serde(default)]
    workout_type_family_id: Option<u16>,
    #[serde(default)]
    updated_at: Option<String>,
}

/// Parse one of Wahoo's ISO 8601 instants.
fn parse_instant(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// A non-negative reading as a whole number.
fn whole(value: Option<f64>) -> Option<u32> {
    value
        .filter(|v| *v >= 0.0)
        .map(|v| utils::conversions::f64_to_u32(v.round()))
}

impl WahooWorkout {
    /// When the workout started.
    fn started(&self) -> AppResult<DateTime<Utc>> {
        parse_instant(&self.starts).ok_or_else(|| {
            AppError::external_service(
                oauth_providers::WAHOO,
                format!(
                    "workout {} has no readable start '{}'",
                    self.id, self.starts
                ),
            )
        })
    }

    /// The workout as a Dravr activity, or `None` for a scheduled workout
    /// that has no summary yet.
    fn to_activity(&self) -> AppResult<Option<Activity>> {
        let Some(summary) = self.workout_summary.as_ref() else {
            return Ok(None);
        };
        Ok(Some(self.activity_builder(summary)?.build()))
    }

    /// The activity builder of a completed workout. Names render as Wahoo
    /// sends them (the agreement forbids editing displayed Wahoo data).
    fn activity_builder(&self, summary: &WahooWorkoutSummary) -> AppResult<ActivityBuilder> {
        let type_id = self.workout_type_id.unwrap_or(WORKOUT_TYPE_UNKNOWN);
        let sport = sport_for_workout_type(type_id);
        let duration = summary
            .duration_total_accum
            .or_else(|| self.minutes.map(|minutes| minutes * 60.0))
            .map_or(0, |seconds| utils::conversions::f64_to_u64(seconds.round()));
        let name = summary
            .name
            .clone()
            .or_else(|| self.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("Wahoo {}", sport.display_name()));
        Ok(ActivityBuilder::new(
            self.id.to_string(),
            name,
            sport,
            self.started()?,
            duration,
            oauth_providers::WAHOO,
        )
        .distance_meters_opt(summary.distance_accum)
        .elevation_gain_opt(summary.ascent_accum)
        .average_heart_rate_opt(whole(summary.heart_rate_avg))
        .average_power_opt(whole(summary.power_avg))
        .normalized_power_opt(whole(summary.power_bike_np_last))
        .training_stress_score_opt(
            summary
                .power_bike_tss_last
                .map(utils::conversions::f64_to_f32),
        )
        .average_cadence_opt(whole(summary.cadence_avg))
        .average_speed_opt(summary.speed_avg)
        .calories_opt(whole(summary.calories_accum))
        .sport_type_detail_opt(Some(format!("wahoo_workout_type_{type_id}"))))
    }

    /// The calendar view of a workout Dravr scheduled, or `None` for one it
    /// did not write.
    fn calendar_event(&self) -> Option<CalendarEventRef> {
        let token = self
            .workout_token
            .as_deref()
            .filter(|t| is_dravr_token(t))?;
        let date = self.started().ok()?.date_naive();
        Some(CalendarEventRef {
            provider_event_id: self.id.to_string(),
            external_id: Some(token.to_owned()),
            date,
            updated_at: self.updated_at.as_deref().and_then(parse_instant),
        })
    }
}

/// A Wahoo workout id from a provider event id or an activity id, which are
/// the integers Wahoo assigned. Anything else never came from Wahoo and is
/// never interpolated into a path.
fn workout_id(id: &str) -> AppResult<i64> {
    id.trim()
        .parse::<i64>()
        .map_err(|_| AppError::invalid_input(format!("'{id}' is not a Wahoo workout id")))
}

// ============================================================================
// Wahoo provider
// ============================================================================

/// Wahoo Cloud API provider.
pub struct WahooProvider {
    config: ProviderConfig,
    credentials: RwLock<Option<OAuth2Credentials>>,
    client: SharedHttpClient,
    circuit_breaker: CircuitBreaker,
    token_refresh_callback: OnceLock<TokenRefreshCallback>,
    /// The athlete's FTP as their Wahoo power zones hold it, read once per
    /// provider: the only athlete number a pushed plan file carries.
    wahoo_ftp: OnceCell<Option<u32>>,
}

impl WahooProvider {
    /// Create a Wahoo provider with the given configuration.
    #[must_use]
    pub fn with_config(config: ProviderConfig) -> Self {
        Self {
            circuit_breaker: CircuitBreaker::new(oauth_providers::WAHOO),
            config,
            credentials: RwLock::new(None),
            client: shared_client().clone(),
            token_refresh_callback: OnceLock::new(),
            wahoo_ftp: OnceCell::new(),
        }
    }

    /// The current access token.
    async fn access_token(&self) -> AppResult<String> {
        self.credentials
            .read()
            .await
            .as_ref()
            .and_then(|credentials| credentials.access_token.clone())
            .ok_or_else(|| AppError::provider_auth_required(oauth_providers::WAHOO))
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.config.api_base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    /// Refuse while the circuit is open, then refresh a token about to
    /// expire: right before the call that will use it, as Wahoo asks.
    async fn prepare_call(&self) -> AppResult<String> {
        if !self.circuit_breaker.is_allowed() {
            let err = ProviderError::CircuitBreakerOpen {
                provider: oauth_providers::WAHOO.to_owned(),
                retry_after_secs: 30,
            };
            return Err(AppError::external_service(
                oauth_providers::WAHOO,
                err.to_string(),
            ));
        }
        self.refresh_token_if_needed().await?;
        self.access_token().await
    }

    /// Wahoo's 404 is a record that does not exist (or is not this app's to
    /// read): a not-found the caller can act on, not an API failure.
    fn not_found(status: StatusCode, _body: &str) -> Option<AppError> {
        (status == StatusCode::NOT_FOUND).then(|| AppError::not_found("Wahoo record"))
    }

    /// An authenticated `GET` of a JSON resource.
    async fn get_json<T>(&self, path: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let access_token = self.prepare_call().await?;
        let url = self.url(path);
        debug!("Wahoo GET {path}");
        let retry_config = utils::RetryConfig {
            estimated_block_duration_secs:
                api_provider_limits::wahoo::ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS,
            ..utils::RetryConfig::default()
        };
        let budget = request_budget::carried_by(&self.credentials).await;
        let result = utils::api_request_with_retry(
            &self.client,
            &url,
            &access_token,
            oauth_providers::WAHOO,
            &retry_config,
            budget.as_ref(),
            Self::not_found,
        )
        .await;
        match &result {
            Ok(_) => self.circuit_breaker.record_success(),
            Err(e) => self.circuit_breaker.record_error(e),
        }
        result
    }

    /// An authenticated form-encoded write (`POST`/`PUT`/`DELETE`). Returns
    /// the response of a success; a `404` on `DELETE` is `Ok(None)` — the
    /// record is already gone.
    async fn write(
        &self,
        method: Method,
        path: &str,
        form: &[(&str, String)],
    ) -> AppResult<Option<reqwest::Response>> {
        let access_token = self.prepare_call().await?;
        let url = self.url(path);
        let budget = request_budget::carried_by(&self.credentials).await;
        request_budget::admit(budget.as_ref(), oauth_providers::WAHOO).await?;
        debug!("Wahoo {method} {path}");
        let mut request = self
            .client
            .request(method.clone(), &url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Accept", "application/json");
        if !form.is_empty() {
            request = request.form(form);
        }
        let response = request.send().await.map_err(|e| {
            AppError::external_service(oauth_providers::WAHOO, format!("{method} {path}: {e}"))
        })?;
        let status = response.status();
        if status.is_success() {
            self.circuit_breaker.record_success();
            return Ok(Some(response));
        }
        if method == Method::DELETE && status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let error = if status == StatusCode::TOO_MANY_REQUESTS {
            AppError::new(
                ErrorCode::ExternalRateLimited,
                format!("Wahoo is rate limiting {method} {path}"),
            )
            .with_retry_after(api_provider_limits::wahoo::ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS)
        } else {
            let body = response.text().await.unwrap_or_default();
            utils::api_error(status, &body, oauth_providers::WAHOO)
        };
        self.circuit_breaker.record_error(&error);
        Err(error)
    }

    /// A write whose response carries the record's id.
    async fn write_for_id(
        &self,
        method: Method,
        path: &str,
        form: &[(&str, String)],
    ) -> AppResult<i64> {
        let response = self
            .write(method, path, form)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Wahoo record at {path}")))?;
        let created: WahooCreated = response.json().await.map_err(|e| {
            AppError::external_service(oauth_providers::WAHOO, format!("{path} response: {e}"))
        })?;
        Ok(created.id)
    }

    /// One page of `per_page` of the athlete's workouts, newest first.
    async fn workout_page(&self, page: usize, per_page: usize) -> AppResult<WahooWorkoutPage> {
        self.get_json(&format!("workouts?page={page}&per_page={per_page}"))
            .await
    }

    /// Walk the workouts newest first, page by page, handing each to `keep`
    /// until it answers [`Walk::Stop`], the pages run out, or the shared page
    /// ceiling ([`max_activity_pages`]) is reached — which is logged, since a
    /// walk cut short reads downstream as a history that ended there.
    ///
    /// Scheduled workouts share the listing with completed ones (a pushed
    /// plan puts every future session at its head), so how many pages hold
    /// what the caller wants is unknown up front: `keep` decides when it has
    /// enough.
    async fn walk_workouts<F>(&self, mut keep: F) -> AppResult<()>
    where
        F: FnMut(&WahooWorkout) -> Walk,
    {
        let per_page = api_provider_limits::wahoo::WORKOUTS_PER_PAGE;
        let max_pages = max_activity_pages();
        for page in 1..=max_pages {
            let listing = self.workout_page(page, per_page).await?;
            if listing.workouts.is_empty() {
                return Ok(());
            }
            for workout in &listing.workouts {
                if keep(workout) == Walk::Stop {
                    return Ok(());
                }
            }
            let seen = page.saturating_mul(per_page);
            if listing.total.is_some_and(|total| seen >= total) {
                return Ok(());
            }
        }
        warn!(
            pages_allowed = max_pages,
            "Wahoo workout walk stopped at the {MAX_ACTIVITY_PAGES_ENV} ceiling; older workouts may be unread"
        );
        Ok(())
    }

    /// One workout record.
    async fn workout(&self, id: &str) -> AppResult<WahooWorkout> {
        let id = workout_id(id)?;
        self.get_json(&format!("workouts/{id}")).await
    }

    /// The athlete's FTP as their Wahoo power zones hold it, read once: the
    /// cycling zone set when there is one, else the most recently updated
    /// set that names an FTP. `None` when they hold none, or the read fails —
    /// a pushed plan then goes out without FTP targets rather than not at all.
    async fn wahoo_ftp(&self) -> Option<u32> {
        *self
            .wahoo_ftp
            .get_or_init(|| async {
                match self.get_json::<Vec<WahooPowerZone>>("power_zones").await {
                    Ok(mut zones) => {
                        zones.retain(|zone| zone.ftp.is_some_and(|ftp| ftp > 0.0));
                        zones.sort_by_key(|zone| {
                            (
                                zone.workout_type_family_id == Some(0),
                                zone.updated_at.as_deref().and_then(parse_instant),
                            )
                        });
                        zones.last().and_then(|zone| whole(zone.ftp))
                    }
                    Err(e) => {
                        warn!(error = %e, "Wahoo power zones unreadable; plan targets go out without FTP");
                        None
                    }
                }
            })
            .await
    }

    /// Put the session's plan in the athlete's library, updating Dravr's
    /// existing copy rather than adding a second one: Wahoo's library holds
    /// one copy of a plan, found by its `external_id` (reconcile, never
    /// append — carnet#48). Returns the plan's id, or `None` when the session
    /// has no plan Wahoo can carry.
    async fn upsert_plan(&self, session: &PlannedSession) -> AppResult<Option<i64>> {
        let ftp = if wahoo_plan::targets_ftp(session) {
            self.wahoo_ftp().await
        } else {
            None
        };
        let Some(file) = wahoo_plan::plan_file(session, ftp) else {
            return Ok(None);
        };
        let external_id = opaque_id(&session.external_id);
        let existing: Vec<WahooPlan> = self
            .get_json(&format!("plans?external_id={external_id}"))
            .await?;
        let now = Utc::now();
        if let Some(plan) = existing.iter().find(|plan| !plan.deleted) {
            let form = wahoo_plan::plan_form(session, &file, now, false);
            let id = self
                .write_for_id(Method::PUT, &format!("plans/{}", plan.id), &form)
                .await?;
            return Ok(Some(id));
        }
        let form = wahoo_plan::plan_form(session, &file, now, true);
        Ok(Some(self.write_for_id(Method::POST, "plans", &form).await?))
    }

    /// Download and decode a completed workout's activity FIT file.
    async fn fit_series(&self, summary: &WahooWorkoutSummary) -> AppResult<Option<TimeSeriesData>> {
        let Some(url) = summary.file.as_ref().and_then(|file| file.url.as_deref()) else {
            return Ok(None);
        };
        if !is_wahoo_file_url(url) {
            warn!("Wahoo workout file is not on a Wahoo host; not fetched");
            return Ok(None);
        }
        // A CDN download is not an API call: no budget, no token, and it
        // revokes nothing (Wahoo Cloud API, "Token Limits").
        let response = self.client.get(url).send().await.map_err(|e| {
            AppError::external_service(oauth_providers::WAHOO, format!("workout file: {e}"))
        })?;
        if !response.status().is_success() {
            return Err(AppError::external_service(
                oauth_providers::WAHOO,
                format!("workout file download answered {}", response.status()),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_FIT_BYTES as u64)
        {
            return Err(AppError::external_service(
                oauth_providers::WAHOO,
                "workout file is larger than any activity file".to_owned(),
            ));
        }
        let bytes = response.bytes().await.map_err(|e| {
            AppError::external_service(oauth_providers::WAHOO, format!("workout file: {e}"))
        })?;
        if bytes.len() > MAX_FIT_BYTES {
            return Err(AppError::external_service(
                oauth_providers::WAHOO,
                "workout file is larger than any activity file".to_owned(),
            ));
        }
        time_series_from_fit(&bytes).map(Some)
    }

    /// A completed workout, refusing a scheduled one.
    async fn completed_workout(&self, id: &str) -> AppResult<(WahooWorkout, WahooWorkoutSummary)> {
        let workout = self.workout(id).await?;
        let summary = workout.workout_summary.clone().ok_or_else(|| {
            AppError::not_found(format!(
                "Wahoo activity {id} (a scheduled workout with no results yet)"
            ))
        })?;
        Ok((workout, summary))
    }
}

/// Whether a workout walk goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Walk {
    Continue,
    Stop,
}

#[async_trait]
impl FitnessProvider for WahooProvider {
    fn name(&self) -> &'static str {
        oauth_providers::WAHOO
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        info!("Setting Wahoo credentials");
        *self.credentials.write().await = Some(credentials);
        Ok(())
    }

    fn set_token_refresh_callback(&self, callback: TokenRefreshCallback) {
        let _ = self.token_refresh_callback.set(callback);
    }

    async fn is_authenticated(&self) -> bool {
        utils::is_authenticated(&*self.credentials.read().await)
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        let credentials = self
            .credentials
            .read()
            .await
            .clone()
            .ok_or_else(|| AppError::provider_auth_required(oauth_providers::WAHOO))?;
        if !credentials.expires_at.is_some_and(refresh_due) {
            return Ok(());
        }
        let refresh_token = credentials
            .refresh_token
            .clone()
            .ok_or_else(|| AppError::provider_auth_required(oauth_providers::WAHOO))?;
        let mut refreshed = utils::refresh_oauth_token(
            &self.client,
            &utils::RefreshRequest::form_fields(
                oauth_providers::WAHOO,
                &self.config.token_url,
                &credentials.client_id,
                &credentials.client_secret,
                &refresh_token,
            ),
        )
        .await?;
        // Wahoo returns a new refresh token on every refresh; the stored
        // scopes and the signing app's budget carry over.
        refreshed.refresh_token = refreshed.refresh_token.or(Some(refresh_token));
        refreshed.scopes = credentials.scopes;
        refreshed.request_budget = credentials.request_budget;
        *self.credentials.write().await = Some(refreshed.clone());
        if let Some(callback) = self.token_refresh_callback.get() {
            callback(refreshed).await;
        }
        Ok(())
    }

    #[instrument(skip(self), fields(provider = "wahoo", api_call = "get_athlete"))]
    async fn get_athlete(&self) -> AppResult<Athlete> {
        let user: WahooUser = self.get_json("user").await?;
        Ok(Athlete {
            id: user.id.to_string(),
            username: user.email.unwrap_or_default(),
            firstname: user.first,
            lastname: user.last,
            profile_picture: None,
            provider: oauth_providers::WAHOO.to_owned(),
            preferred_units: None,
        })
    }

    #[instrument(
        skip(self, params),
        fields(provider = "wahoo", api_call = "get_activities", limit = ?params.limit)
    )]
    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        let requested = params
            .limit
            .unwrap_or(api_provider_limits::wahoo::DEFAULT_ACTIVITIES_PER_PAGE);
        let mut skip = params.offset.unwrap_or(0);
        // A hint only: `requested` is the caller's number, never an allocation.
        let mut activities =
            Vec::with_capacity(requested.min(api_provider_limits::wahoo::WORKOUTS_PER_PAGE));
        let mut failure: Option<AppError> = None;
        self.walk_workouts(|workout| {
            let Ok(started) = workout.started() else {
                return Walk::Continue;
            };
            let at = started.timestamp();
            if params.before.is_some_and(|before| at >= before) {
                return Walk::Continue;
            }
            if params.after.is_some_and(|after| at < after) {
                return Walk::Stop;
            }
            match workout.to_activity() {
                Ok(Some(_)) if skip > 0 => skip -= 1,
                Ok(Some(activity)) => activities.push(activity),
                Ok(None) => {}
                Err(e) => failure = Some(e),
            }
            if activities.len() >= requested {
                Walk::Stop
            } else {
                Walk::Continue
            }
        })
        .await?;
        if let Some(e) = failure {
            warn!("A Wahoo workout did not convert: {e}");
        }
        Ok(activities)
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        // Wahoo pages by number, and a page number counts pages of one size,
        // so the cursor carries both (`page:per_page`). A page asks Wahoo for
        // at most `limit` workouts — every workout it lists is returned or
        // was scheduled, and the next page starts right after it.
        let max_per_page = api_provider_limits::wahoo::WORKOUTS_PER_PAGE;
        let (page, per_page) = params
            .cursor
            .as_ref()
            .and_then(Cursor::decode)
            .and_then(|(_, position)| {
                let (page, per_page) = position.split_once(':')?;
                Some((page.parse::<usize>().ok()?, per_page.parse::<usize>().ok()?))
            })
            .filter(|(page, per_page)| *page >= 1 && (1..=max_per_page).contains(per_page))
            .unwrap_or_else(|| (1, params.limit.clamp(1, max_per_page)));
        let listing = self.workout_page(page, per_page).await?;
        let mut activities = Vec::new();
        for workout in &listing.workouts {
            match workout.to_activity() {
                Ok(Some(activity)) => activities.push(activity),
                Ok(None) => {}
                Err(e) => warn!("A Wahoo workout did not convert: {e}"),
            }
        }
        let seen = page.saturating_mul(per_page);
        let has_more =
            !listing.workouts.is_empty() && listing.total.is_none_or(|total| seen < total);
        let position = |page: usize| Cursor::new(Utc::now(), &format!("{page}:{per_page}"));
        let next = has_more.then(|| position(page.saturating_add(1)));
        let prev = (page > 1).then(|| position(page - 1));
        Ok(CursorPage::new(activities, next, prev, has_more))
    }

    #[instrument(skip(self), fields(provider = "wahoo", api_call = "get_activity", activity_id = %id))]
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        let (workout, summary) = self.completed_workout(id).await?;
        Ok(workout.activity_builder(&summary)?.build())
    }

    fn serves_activity_streams(&self) -> bool {
        true
    }

    async fn get_activity_with_streams(&self, id: &str) -> AppResult<Activity> {
        let (workout, summary) = self.completed_workout(id).await?;
        let series = self.fit_series(&summary).await?;
        Ok(workout
            .activity_builder(&summary)?
            .time_series_data_opt(series)
            .build())
    }

    async fn get_activity_streams(&self, id: &str) -> AppResult<Option<TimeSeriesData>> {
        let (_, summary) = self.completed_workout(id).await?;
        self.fit_series(&summary).await
    }

    #[instrument(skip(self), fields(provider = "wahoo", api_call = "get_stats"))]
    async fn get_stats(&self) -> AppResult<Stats> {
        // Wahoo has no aggregate endpoint: the rollup is of the recent
        // workouts the default listing returns.
        let activities = self
            .get_activities_with_params(&ActivityQueryParams::with_pagination(None, None))
            .await?;
        Ok(Stats {
            total_activities: activities.len() as u64,
            total_distance: activities
                .iter()
                .filter_map(Activity::distance_meters)
                .sum(),
            total_duration: activities.iter().map(Activity::duration_seconds).sum(),
            total_elevation_gain: activities.iter().filter_map(Activity::elevation_gain).sum(),
            year_to_date: None,
        })
    }

    fn calendar_key(&self, external_id: &str) -> String {
        opaque_id(external_id)
    }

    async fn list_calendar_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> AppResult<Vec<CalendarEventRef>> {
        let mut events = Vec::new();
        self.walk_workouts(|workout| {
            let Some(event) = workout.calendar_event() else {
                // Not Dravr's, or no readable start: stop only once the
                // listing is before the window.
                return match workout.started() {
                    Ok(started) if started.date_naive() < from => Walk::Stop,
                    _ => Walk::Continue,
                };
            };
            if event.date < from {
                return Walk::Stop;
            }
            if event.date <= to {
                events.push(event);
            }
            Walk::Continue
        })
        .await?;
        Ok(events)
    }

    async fn push_planned_session(&self, session: &PlannedSession) -> AppResult<String> {
        if session.kind == PlannedSessionKind::WeekNote {
            return Err(AppError::invalid_input(
                "Wahoo's calendar holds workouts only, not week notes",
            ));
        }
        let plan_id = self.upsert_plan(session).await?;
        let form = wahoo_plan::workout_form(session, plan_id);
        let id = self.write_for_id(Method::POST, "workouts", &form).await?;
        Ok(id.to_string())
    }

    async fn update_planned_session(
        &self,
        provider_event_id: &str,
        session: &PlannedSession,
    ) -> AppResult<()> {
        let id = workout_id(provider_event_id)?;
        if session.kind == PlannedSessionKind::WeekNote {
            return Err(AppError::invalid_input(
                "Wahoo's calendar holds workouts only, not week notes",
            ));
        }
        let plan_id = self.upsert_plan(session).await?;
        let form = wahoo_plan::workout_form(session, plan_id);
        self.write_for_id(Method::PUT, &format!("workouts/{id}"), &form)
            .await?;
        Ok(())
    }

    async fn delete_planned_sessions(&self, provider_event_ids: &[String]) -> AppResult<u64> {
        let mut deleted = 0;
        for event_id in provider_event_ids {
            let id = workout_id(event_id)?;
            // The plan goes with the workout: Dravr wrote both, and a plan
            // left in the library would be one Dravr no longer tracks.
            let plan_id = match self.workout(&id.to_string()).await {
                Ok(workout) => workout.plan_id,
                Err(e) if e.code == ErrorCode::ResourceNotFound => continue,
                Err(e) => return Err(e),
            };
            if self
                .write(Method::DELETE, &format!("workouts/{id}"), &[])
                .await?
                .is_some()
            {
                deleted += 1;
            }
            if let Some(plan_id) = plan_id {
                self.write(Method::DELETE, &format!("plans/{plan_id}"), &[])
                    .await?;
            }
        }
        Ok(deleted)
    }
}

// ============================================================================
// Provider factory
// ============================================================================

/// Factory for Wahoo provider instances.
pub struct WahooProviderFactory;

impl ProviderFactory for WahooProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(WahooProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[oauth_providers::WAHOO]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SportType;

    fn workout(json: &str) -> WahooWorkout {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    /// The Cloud API reference's own workout sample, decimals as strings.
    const COMPLETED: &str = r#"{
        "id": 56519, "starts": "2015-08-12T09:00:00.000Z", "minutes": 12,
        "name": "Friday Fun", "plan_id": null, "plan_ids": [], "route_id": null,
        "workout_token": "123", "workout_type_id": 40,
        "workout_summary": {
            "id": 8297, "name": "Easy Ride", "ascent_accum": "450.00",
            "cadence_avg": "50.00", "calories_accum": "1500.00",
            "distance_accum": "24909.71", "duration_active_accum": "179.00",
            "duration_paused_accum": "95.25", "duration_total_accum": "275.00",
            "heart_rate_avg": "100.00", "power_bike_np_last": "150.00",
            "power_bike_tss_last": "304.90", "power_avg": "94.59",
            "speed_avg": "10.75", "work_accum": "1041480.00",
            "time_zone": "America/Denver", "manual": false, "edited": false,
            "fitness_app_id": 1002,
            "file": { "url": "https://cdn.wahooligan.com/x/4_Mile_Segment_.fit" },
            "created_at": "2018-10-23T20:43:50.000Z", "updated_at": "2018-10-23T20:43:50.000Z"
        },
        "created_at": "2018-10-23T20:41:55.000Z", "updated_at": "2018-10-23T20:41:55.000Z"
    }"#;

    #[test]
    fn a_completed_workout_reads_its_summary_decimals() {
        let activity = workout(COMPLETED)
            .to_activity()
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("a completed workout is an activity"));
        assert_eq!(activity.id(), "56519");
        assert_eq!(activity.name(), "Easy Ride");
        assert_eq!(activity.duration_seconds(), 275);
        assert_eq!(activity.distance_meters(), Some(24_909.71));
        assert_eq!(activity.elevation_gain(), Some(450.0));
        assert_eq!(activity.average_heart_rate(), Some(100));
        assert_eq!(activity.average_power(), Some(95));
        assert_eq!(activity.normalized_power(), Some(150));
        assert_eq!(activity.calories(), Some(1500));
        assert_eq!(activity.provider(), "wahoo");
    }

    #[test]
    fn a_scheduled_workout_is_no_activity() {
        let scheduled = workout(
            r#"{"id": 7, "starts": "2026-10-08T12:00:00.000Z", "minutes": 90,
                "name": "Ride 1 h 30", "plan_id": 3, "workout_token": "dravr-abc",
                "workout_type_id": 0, "workout_summary": null,
                "updated_at": "2026-10-07T14:00:00.000Z"}"#,
        );
        assert!(scheduled
            .to_activity()
            .unwrap_or_else(|e| panic!("{e}"))
            .is_none());
        let event = scheduled
            .calendar_event()
            .unwrap_or_else(|| panic!("Dravr's token makes it a calendar event"));
        assert_eq!(event.provider_event_id, "7");
        assert_eq!(event.external_id.as_deref(), Some("dravr-abc"));
        assert_eq!(
            event.date,
            NaiveDate::from_ymd_opt(2026, 10, 8).unwrap_or_default()
        );
    }

    #[test]
    fn a_workout_dravr_did_not_write_is_no_calendar_event() {
        assert!(workout(COMPLETED).calendar_event().is_none());
    }

    #[test]
    fn decimals_read_as_strings_numbers_or_null() {
        let summary: WahooWorkoutSummary = serde_json::from_str(
            r#"{"power_avg": 210, "heart_rate_avg": "141.5", "cadence_avg": null, "speed_avg": ""}"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(summary.power_avg, Some(210.0));
        assert_eq!(summary.heart_rate_avg, Some(141.5));
        assert_eq!(summary.cadence_avg, None);
        assert_eq!(summary.speed_avg, None);
    }

    #[test]
    fn only_wahoo_integer_ids_reach_a_path() {
        assert_eq!(workout_id(" 56519 ").ok(), Some(56_519));
        assert!(workout_id("56519/../../user").is_err());
        assert!(workout_id("dravr:plan:x").is_err());
    }

    #[test]
    fn a_known_workout_type_maps_to_its_sport() {
        let mut completed = workout(COMPLETED);
        completed.workout_type_id = Some(61);
        let activity = completed
            .to_activity()
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("activity"));
        assert_eq!(activity.sport_type(), &SportType::VirtualRide);
    }

    #[test]
    fn an_unknown_workout_type_is_kept_as_other() {
        let mut completed = workout(COMPLETED);
        completed.workout_type_id = Some(48);
        let activity = completed
            .to_activity()
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("activity"));
        assert_eq!(
            activity.sport_type(),
            &SportType::Other("wahoo_workout_type_48".to_owned())
        );
    }
}
