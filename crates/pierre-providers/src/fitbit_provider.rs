// ABOUTME: Clean Fitbit API provider implementation using unified provider architecture
// ABOUTME: Handles OAuth2 authentication with PKCE and data fetching with proper error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - HTTP client Arc sharing across async operations (shared_client().clone())
// - String ownership for API responses and error handling

use super::circuit_breaker::CircuitBreaker;
use super::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
    TokenRefreshCallback,
};
use super::errors::provider::ProviderError;
use crate::activity_paging::pages_for;
use crate::constants::{api_provider_limits, oauth_providers};
use crate::errors::{AppError, AppResult};
use crate::http_client::{shared_client, SharedHttpClient};
use crate::models::{
    Activity, ActivityBuilder, Athlete, HeartRateZone, PersonalRecord, SportType, Stats,
};
use crate::pagination::{CursorPage, PaginationParams};
use crate::utils;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::from_str;
use std::collections::HashSet;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

/// Fitbit API base URL
const FITBIT_API_BASE: &str = "https://api.fitbit.com/1";

/// Fitbit API error response format
#[derive(Debug, Deserialize)]
struct FitbitErrorResponse {
    errors: Option<Vec<FitbitError>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FitbitError {
    error_type: Option<String>,
    message: Option<String>,
}

/// Fitbit user profile API response wrapper
#[derive(Debug, Deserialize)]
struct FitbitUserResponse {
    user: FitbitUserProfile,
}

/// Fitbit user profile data
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FitbitUserProfile {
    encoded_id: String,
    display_name: String,
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
    avatar: Option<String>,
}

/// Fitbit activities list API response
#[derive(Debug, Deserialize)]
struct FitbitActivitiesResponse {
    activities: Vec<FitbitActivity>,
}

/// Fitbit activity data from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FitbitActivity {
    #[serde(rename = "logId")]
    log_id: u64,
    activity_name: String,
    activity_type_id: u32,
    start_time: String,
    original_start_time: Option<String>,
    duration: u64,         // milliseconds
    distance: Option<f64>, // km
    steps: Option<u32>,
    calories: Option<u32>,
    elevation_gain: Option<f64>, // meters
    average_heart_rate: Option<u32>,
    heart_rate_zones: Option<Vec<FitbitHeartRateZone>>,
}

/// Fitbit heart rate zone data
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FitbitHeartRateZone {
    name: String,
    min: u32,
    max: u32,
    minutes: u32,
}

/// Fitbit lifetime stats API response
#[derive(Debug, Deserialize)]
struct FitbitLifetimeStatsResponse {
    lifetime: FitbitLifetime,
}

#[derive(Debug, Deserialize)]
struct FitbitLifetime {
    total: FitbitLifetimeTotal,
}

#[derive(Debug, Deserialize)]
struct FitbitLifetimeTotal {
    distance: f64, // km
    floors: f64,
}

/// Clean Fitbit provider implementation
pub struct FitbitProvider {
    config: ProviderConfig,
    credentials: RwLock<Option<OAuth2Credentials>>,
    client: SharedHttpClient,
    circuit_breaker: CircuitBreaker,
    token_refresh_callback: OnceLock<TokenRefreshCallback>,
}

impl FitbitProvider {
    /// Create a new Fitbit provider with default configuration
    #[must_use]
    pub fn new() -> Self {
        let config = ProviderConfig {
            name: oauth_providers::FITBIT.to_owned(),
            auth_url: "https://www.fitbit.com/oauth2/authorize".to_owned(),
            token_url: "https://api.fitbit.com/oauth2/token".to_owned(),
            api_base_url: FITBIT_API_BASE.to_owned(),
            revoke_url: Some("https://api.fitbit.com/oauth2/revoke".to_owned()),
            default_scopes: oauth_providers::FITBIT_DEFAULT_SCOPES
                .split(' ')
                .map(str::to_owned)
                .collect(),
        };

        Self {
            circuit_breaker: CircuitBreaker::new(oauth_providers::FITBIT),
            config,
            credentials: RwLock::new(None),
            client: shared_client().clone(),
            token_refresh_callback: OnceLock::new(),
        }
    }

    /// Create provider with custom configuration
    #[must_use]
    pub fn with_config(config: ProviderConfig) -> Self {
        let provider_name = config.name.clone();
        Self {
            circuit_breaker: CircuitBreaker::new(&provider_name),
            config,
            credentials: RwLock::new(None),
            client: shared_client().clone(),
            token_refresh_callback: OnceLock::new(),
        }
    }

    /// Retrieve the current access token from credentials
    async fn get_access_token(&self) -> AppResult<String> {
        let token = self
            .credentials
            .read()
            .await
            .as_ref()
            .ok_or_else(|| AppError::internal("No credentials available for Fitbit API request"))?
            .access_token
            .clone();

        token.ok_or_else(|| AppError::internal("No access token available"))
    }

    /// Handle non-success API responses
    /// Fitbit names the failure in its `errors[]` body — an expired token, a
    /// scope the grant does not carry, or a message worth showing — which the
    /// status code alone does not say. A 401 never reaches this hook:
    /// [`utils::api_request_with_retry`] maps it to the re-auth error first,
    /// so `expired_token` is only seen here on some other status. Everything
    /// else is the shared behaviour in [`utils::api_error`].
    fn vendor_error(_status: reqwest::StatusCode, text: &str) -> Option<AppError> {
        let first_error = from_str::<FitbitErrorResponse>(text)
            .ok()?
            .errors?
            .into_iter()
            .next()?;
        let error_type = first_error.error_type.unwrap_or_default();
        let message = first_error.message.unwrap_or_default();
        Some(match error_type.as_str() {
            "expired_token" => AppError::external_service(
                oauth_providers::FITBIT,
                "Access token expired. Please refresh token.".to_owned(),
            ),
            "insufficient_scope" => AppError::external_service(
                oauth_providers::FITBIT,
                format!("Insufficient permissions: {message}"),
            ),
            _ => AppError::external_service(oauth_providers::FITBIT, message),
        })
    }

    /// Make authenticated API request with circuit breaker protection
    async fn api_request<T>(&self, endpoint: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        debug!("Starting Fitbit API request to endpoint: {endpoint}");

        // Check circuit breaker before making request
        if !self.circuit_breaker.is_allowed() {
            let err = ProviderError::CircuitBreakerOpen {
                provider: oauth_providers::FITBIT.to_owned(),
                retry_after_secs: 30,
            };
            return Err(AppError::external_service("Fitbit", err.to_string()));
        }

        self.refresh_token_if_needed().await?;

        let access_token = self.get_access_token().await?;

        let url = format!(
            "{}/{}",
            self.config.api_base_url,
            endpoint.trim_start_matches('/')
        );

        let retry_config = utils::RetryConfig {
            estimated_block_duration_secs:
                api_provider_limits::fitbit::ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS,
            ..utils::RetryConfig::default()
        };
        let result = utils::api_request_with_retry(
            &self.client,
            &url,
            &access_token,
            oauth_providers::FITBIT,
            &retry_config,
            Self::vendor_error,
        )
        .await;

        // Record success/failure for circuit breaker
        match &result {
            Ok(_) => self.circuit_breaker.record_success(),
            Err(_) => self.circuit_breaker.record_failure(),
        }

        result
    }

    /// Convert Fitbit activity type ID to our `SportType` enum
    fn parse_sport_type(activity_type_id: u32, activity_name: &str) -> SportType {
        // Fitbit activity type IDs based on their API documentation
        // See: https://dev.fitbit.com/build/reference/web-api/activity/
        match activity_type_id {
            90009 | 90019 | 3001 => SportType::Run, // Run, Running, Treadmill
            90001 => SportType::Walk,               // Walk
            1 | 1071 => SportType::Ride,            // Bike, Cycling
            90024 | 18120 => SportType::Swim,       // Swimming, Walking(water)
            90013 | 17180 => SportType::Hike,       // Hiking
            52001 | 17190 => SportType::Yoga,       // Yoga
            15680 => SportType::StrengthTraining,   // Weight Training
            15000 | 15010 | 15020 => SportType::Workout, // Workout types
            _ => SportType::Other(activity_name.to_owned()),
        }
    }

    /// Convert Fitbit activity to internal Activity model
    fn convert_fitbit_activity(activity: FitbitActivity) -> AppResult<Activity> {
        // Parse start time - Fitbit uses ISO 8601 format
        let start_time_str = activity
            .original_start_time
            .as_ref()
            .unwrap_or(&activity.start_time);

        // Parse start time - try RFC3339 first, then fall back to naive datetime (assume UTC)
        let start_date = DateTime::parse_from_rfc3339(start_time_str)
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|_| {
                // Try alternative Fitbit format: "2024-01-15T10:30:00.000" (assume UTC)
                chrono::NaiveDateTime::parse_from_str(start_time_str, "%Y-%m-%dT%H:%M:%S%.f")
                    .map(|naive| naive.and_utc())
                    .or_else(|_| {
                        // Try without milliseconds: "2024-01-15T10:30:00"
                        chrono::NaiveDateTime::parse_from_str(start_time_str, "%Y-%m-%dT%H:%M:%S")
                            .map(|naive| naive.and_utc())
                    })
            })
            .map_err(|e| {
                AppError::internal(format!(
                    "Failed to parse activity start time '{start_time_str}': {e}"
                ))
            })?;

        let duration_seconds = activity.duration / 1000; // Convert ms to seconds

        Ok(ActivityBuilder::new(
            activity.log_id.to_string(),
            activity.activity_name.clone(),
            Self::parse_sport_type(activity.activity_type_id, &activity.activity_name),
            start_date,
            duration_seconds,
            oauth_providers::FITBIT,
        )
        .distance_meters_opt(activity.distance.map(|d| d * 1000.0)) // Convert km to meters
        .elevation_gain_opt(activity.elevation_gain)
        .average_speed_opt(activity.distance.and_then(|d| {
            if duration_seconds > 0 {
                #[allow(clippy::cast_precision_loss)]
                Some((d * 1000.0) / (duration_seconds as f64)) // m/s
            } else {
                None
            }
        }))
        .average_heart_rate_opt(activity.average_heart_rate)
        .calories_opt(activity.calories)
        .steps_opt(activity.steps)
        .heart_rate_zones_opt(activity.heart_rate_zones.map(|zones| {
            zones
                .into_iter()
                .map(|zone| HeartRateZone {
                    name: zone.name,
                    min_hr: zone.min,
                    max_hr: zone.max,
                    minutes: zone.minutes,
                })
                .collect()
        }))
        .sport_type_detail(activity.activity_name.clone())
        .build())
    }
}

impl Default for FitbitProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FitnessProvider for FitbitProvider {
    fn name(&self) -> &'static str {
        oauth_providers::FITBIT
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        info!("Setting Fitbit credentials");
        *self.credentials.write().await = Some(credentials);
        Ok(())
    }

    fn set_token_refresh_callback(&self, callback: TokenRefreshCallback) {
        let _ = self.token_refresh_callback.set(callback);
    }

    async fn is_authenticated(&self) -> bool {
        let credentials = self.credentials.read().await;
        utils::is_authenticated(&credentials)
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        // Check if refresh is needed and extract credentials
        let (needs_refresh, credentials) = {
            let guard = self.credentials.read().await;
            let needs_refresh = if let Some(creds) = guard.as_ref() {
                creds.expires_at.is_some_and(|expires_at| {
                    Utc::now() + chrono::Duration::minutes(5) > expires_at
                })
            } else {
                let err = ProviderError::ConfigurationError {
                    provider: oauth_providers::FITBIT.to_owned(),
                    details: "No credentials available".to_owned(),
                };
                return Err(AppError::external_service("Fitbit", err.to_string()));
            };

            let credentials = guard
                .as_ref()
                .ok_or_else(|| AppError::internal("No credentials available for refresh"))?
                .clone(); // Safe: OAuth2Credentials ownership for refresh operation
            drop(guard); // Release lock early to avoid contention

            (needs_refresh, credentials)
        };

        if !needs_refresh {
            return Ok(());
        }

        let refresh_token = credentials
            .refresh_token
            .ok_or_else(|| AppError::internal("No refresh token available"))?;

        // Fitbit authenticates the refresh with an HTTP Basic header and
        // rejects the client credentials repeated in the body.
        let mut new_credentials = utils::refresh_oauth_token(
            &self.client,
            &utils::RefreshRequest {
                token_url: &self.config.token_url,
                client_id: &credentials.client_id,
                client_secret: &credentials.client_secret,
                refresh_token: &refresh_token,
                provider_name: oauth_providers::FITBIT,
                client_auth: utils::ClientAuth::BasicHeader,
                extra_form: &[],
            },
        )
        .await?;
        // Fitbit omits the refresh token when it is unchanged, and the
        // exchange does not re-issue scopes.
        new_credentials.refresh_token = new_credentials.refresh_token.or(Some(refresh_token));
        new_credentials.scopes = credentials.scopes;

        *self.credentials.write().await = Some(new_credentials.clone());

        // Persist refreshed token to database via callback
        if let Some(callback) = self.token_refresh_callback.get() {
            callback(new_credentials).await;
        }

        Ok(())
    }

    #[instrument(skip(self), fields(provider = "fitbit", api_call = "get_athlete"))]
    async fn get_athlete(&self) -> AppResult<Athlete> {
        let response: FitbitUserResponse = self.api_request("user/-/profile.json").await?;

        Ok(Athlete {
            id: response.user.encoded_id,
            username: response.user.display_name.clone(),
            firstname: response.user.first_name,
            lastname: response.user.last_name,
            profile_picture: response.user.avatar,
            provider: oauth_providers::FITBIT.to_owned(),
        })
    }

    #[instrument(
        skip(self, params),
        fields(
            provider = "fitbit",
            api_call = "get_activities",
            limit = ?params.limit,
            offset = ?params.offset,
        )
    )]
    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        // Fitbit API uses date-based pagination with beforeDate/afterDate
        let fitbit_offset = params.offset.unwrap_or(0);

        // Use before/after timestamps if provided, otherwise default to last 30 days
        let (start_date, end_date) = if params.before.is_some() || params.after.is_some() {
            let end = params.before.map_or_else(
                || chrono::Utc::now().date_naive(),
                |ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map_or_else(|| chrono::Utc::now().date_naive(), |dt| dt.date_naive())
                },
            );
            let start = params.after.map_or_else(
                || end - chrono::Duration::days(365),
                |ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map_or_else(|| end - chrono::Duration::days(365), |dt| dt.date_naive())
                },
            );
            (start, end)
        } else {
            let end = chrono::Utc::now().date_naive();
            (end - chrono::Duration::days(30), end)
        };

        let requested = params
            .limit
            .unwrap_or(api_provider_limits::fitbit::DEFAULT_ACTIVITIES_PER_PAGE);
        let page_size = api_provider_limits::fitbit::MAX_ACTIVITIES_PER_REQUEST;

        // Fitbit caps `limit` at 100 server-side, so passing the caller's limit
        // straight through returned one page and said nothing about it — the
        // silent truncation that makes the historical backfill record a window
        // it never read. Walk `beforeDate` backwards instead. Fitbit's own
        // cursor is the `pagination.next` URL, which this client does not model,
        // and its `offset` must be 0 on this endpoint; the date bound uses only
        // parameters already in use here.
        let pages = pages_for(requested, page_size);
        let mut activities: Vec<Activity> = Vec::with_capacity(requested.min(page_size * pages));
        let mut seen: HashSet<String> = HashSet::new();
        let mut before_cursor = end_date;

        for _ in 0..pages {
            if activities.len() >= requested || before_cursor < start_date {
                break;
            }
            let endpoint = format!(
                "user/-/activities/list.json?beforeDate={}&afterDate={}&sort=desc&limit={page_size}&offset={fitbit_offset}",
                before_cursor.format("%Y-%m-%d"),
                start_date.format("%Y-%m-%d"),
            );

            let response: FitbitActivitiesResponse = self.api_request(&endpoint).await?;
            let page_len = response.activities.len();
            let mut oldest_in_page: Option<NaiveDate> = None;
            let mut added = 0_usize;

            for fitbit_activity in response.activities {
                match Self::convert_fitbit_activity(fitbit_activity) {
                    Ok(activity) => {
                        let day = activity.start_date().date_naive();
                        oldest_in_page = Some(oldest_in_page.map_or(day, |cur| cur.min(day)));
                        if seen.insert(activity.id().to_owned()) {
                            activities.push(activity);
                            added += 1;
                        }
                    }
                    Err(e) => {
                        warn!("Failed to convert Fitbit activity: {e}");
                    }
                }
            }

            // Short page: nothing older in the window. No new id: `beforeDate` is
            // day-granular, so a day holding more than one page cannot advance
            // the cursor — stop rather than re-request it to the ceiling.
            if page_len < page_size || added == 0 {
                break;
            }
            let Some(oldest_in_page) = oldest_in_page else {
                break;
            };
            before_cursor = oldest_in_page;
        }

        activities.truncate(requested);
        Ok(activities)
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        // Fitbit API uses date-based pagination - delegate to offset-based approach
        let activities = self.get_activities(Some(params.limit), None).await?;
        let has_more = activities.len() == params.limit;
        Ok(CursorPage::new(activities, None, None, has_more))
    }

    #[instrument(
        skip(self),
        fields(provider = "fitbit", api_call = "get_activity", activity_id = %id)
    )]
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        // Fitbit doesn't have a direct single activity endpoint
        // We need to use the activity log endpoint
        let endpoint = format!("user/-/activities/{id}.json");
        let response: FitbitActivitiesResponse = self.api_request(&endpoint).await?;

        response
            .activities
            .into_iter()
            .next()
            .ok_or_else(|| AppError::not_found(format!("Activity {id} not found")))
            .and_then(Self::convert_fitbit_activity)
    }

    #[instrument(skip(self), fields(provider = "fitbit", api_call = "get_stats"))]
    async fn get_stats(&self) -> AppResult<Stats> {
        let response: FitbitLifetimeStatsResponse =
            self.api_request("user/-/activities.json").await?;

        // Fitbit provides lifetime totals
        Ok(Stats {
            total_activities: 0, // Fitbit doesn't provide activity count in lifetime stats
            total_distance: response.lifetime.total.distance * 1000.0, // Convert km to meters
            total_duration: 0,   // Not available in lifetime stats
            total_elevation_gain: response.lifetime.total.floors * 3.0, // Estimate: 1 floor ≈ 3m
            year_to_date: None,
        })
    }

    async fn get_personal_records(&self) -> AppResult<Vec<PersonalRecord>> {
        // Fitbit doesn't have a direct personal records API
        // This would need to be calculated from activity history
        Ok(vec![])
    }
}

// ============================================================================
// Provider Factory
// ============================================================================

/// Factory for creating Fitbit provider instances
pub struct FitbitProviderFactory;

impl ProviderFactory for FitbitProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(FitbitProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[oauth_providers::FITBIT]
    }
}
