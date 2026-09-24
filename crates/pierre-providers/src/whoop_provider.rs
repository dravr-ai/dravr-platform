// ABOUTME: WHOOP API provider implementation using unified provider architecture
// ABOUTME: Handles OAuth2 authentication and data fetching for sleep, recovery, workouts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - HTTP client Arc sharing across async operations (shared_client().clone())
// - String ownership for API responses and error handling
//
// Clippy allowances for this module:
// - cast_possible_truncation: WHOOP's energy arrives as f64 kJ, far inside u32 once converted to kcal
// - cast_sign_loss: Heart rate and energy values from WHOOP are always positive
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use super::circuit_breaker::CircuitBreaker;
use super::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, TokenRefreshCallback,
};
use super::errors::provider::ProviderError;
use crate::activity_paging::pages_for;
use crate::constants::{api_provider_limits, oauth_providers};
use crate::errors::{AppError, AppResult};
use crate::http_client::{shared_client, SharedHttpClient};
use crate::models::{Activity, ActivityBuilder, Athlete, PersonalRecord, SportType, Stats};
use crate::pagination::{Cursor, CursorPage, PaginationParams};
use crate::registry::ProviderRegistry;
use crate::utils;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fmt::Write;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

// ============================================================================
// WHOOP API Response Structures
// ============================================================================

/// WHOOP pagination wrapper for API responses
#[derive(Debug, Deserialize)]
struct WhoopPaginatedResponse<T> {
    /// Array of records
    records: Vec<T>,
    /// Token for fetching next page (None if no more pages)
    next_token: Option<String>,
}

/// WHOOP user profile response
#[derive(Debug, Deserialize)]
struct WhoopUserProfile {
    /// User ID (integer in WHOOP)
    user_id: i64,
    /// User's email address
    email: Option<String>,
    /// User's first name
    first_name: Option<String>,
    /// User's last name
    last_name: Option<String>,
}

/// WHOOP workout/activity response
#[derive(Debug, Deserialize)]
struct WhoopWorkout {
    /// Unique workout ID (UUID string in v2)
    id: String,
    /// Start time of workout (ISO 8601)
    start: String,
    /// End time of workout (ISO 8601)
    end: String,
    /// Sport ID (WHOOP internal sport classification, null for unclassified)
    sport_id: Option<i32>,
    /// Workout score details
    score: Option<WhoopWorkoutScore>,
}

/// The measurements a WHOOP workout score carries.
///
/// WHOOP's workout strain is deliberately not read: it is WHOOP's own
/// calculation, which WHOOP's API Terms (§4) leave only WHOOP able to
/// authorize storing, and a cached activity is stored. Training load comes
/// from the heart-rate and energy measurements below instead.
#[derive(Debug, Deserialize)]
struct WhoopWorkoutScore {
    /// Average heart rate during workout
    average_heart_rate: Option<i32>,
    /// Maximum heart rate during workout
    max_heart_rate: Option<i32>,
    /// Kilojoules burned
    kilojoule: Option<f64>,
    /// Distance in meters (for applicable activities)
    distance_meter: Option<f64>,
    /// Altitude gain in meters
    altitude_gain_meter: Option<f64>,
}

// ============================================================================
// WHOOP Provider Implementation
// ============================================================================

/// WHOOP fitness provider for sleep, recovery, and workout data
pub struct WhoopProvider {
    config: ProviderConfig,
    credentials: RwLock<Option<OAuth2Credentials>>,
    client: SharedHttpClient,
    circuit_breaker: CircuitBreaker,
    token_refresh_callback: OnceLock<TokenRefreshCallback>,
}

impl WhoopProvider {
    /// Create a new WHOOP provider with default configuration
    #[must_use]
    pub fn new() -> Self {
        let config = ProviderConfig {
            name: oauth_providers::WHOOP.to_owned(),
            auth_url: "https://api.prod.whoop.com/oauth/oauth2/auth".to_owned(),
            token_url: "https://api.prod.whoop.com/oauth/oauth2/token".to_owned(),
            api_base_url: "https://api.prod.whoop.com/developer/v2".to_owned(),
            revoke_url: Some("https://api.prod.whoop.com/developer/v2/user/access".to_owned()),
            default_scopes: oauth_providers::WHOOP_DEFAULT_SCOPES
                .split(' ')
                .map(str::to_owned)
                .collect(),
        };

        Self {
            circuit_breaker: CircuitBreaker::new(oauth_providers::WHOOP),
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
            .ok_or_else(|| AppError::internal("No credentials available for WHOOP API request"))?
            .access_token
            .clone();

        token.ok_or_else(|| AppError::internal("No access token available"))
    }

    /// Make authenticated API request to WHOOP with circuit breaker protection
    async fn api_request<T>(&self, endpoint: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        debug!("Starting WHOOP API request to endpoint: {endpoint}");

        // Check circuit breaker before making request
        if !self.circuit_breaker.is_allowed() {
            let err = ProviderError::CircuitBreakerOpen {
                provider: oauth_providers::WHOOP.to_owned(),
                retry_after_secs: 30,
            };
            return Err(AppError::external_service("WHOOP", err.to_string()));
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
                api_provider_limits::whoop::ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS,
            ..utils::RetryConfig::default()
        };
        let result = utils::api_request_with_retry(
            &self.client,
            &url,
            &access_token,
            oauth_providers::WHOOP,
            &retry_config,
            Self::no_data_for_period,
        )
        .await;

        // Record success/failure for circuit breaker
        match &result {
            Ok(_) => self.circuit_breaker.record_success(),
            Err(_) => self.circuit_breaker.record_failure(),
        }

        result
    }

    /// WHOOP answers 404 when the strap has nothing for the window asked
    /// about, which is an empty result rather than a failure — the generic
    /// mapping would render it as an API error the athlete cannot act on.
    fn no_data_for_period(status: reqwest::StatusCode, _body: &str) -> Option<AppError> {
        (status.as_u16() == 404).then(|| {
            let err = ProviderError::NoDataAvailable {
                provider: oauth_providers::WHOOP.to_owned(),
                message: "No data available from WHOOP for the requested time period. \
                          Your WHOOP strap may not have synced yet."
                    .to_owned(),
            };
            AppError::external_service(oauth_providers::WHOOP, err.to_string())
        })
    }

    /// Convert WHOOP sport ID to our `SportType` enum
    fn parse_sport_type(sport_id: i32) -> SportType {
        // WHOOP sport IDs (from their API documentation)
        match sport_id {
            0 => SportType::Workout,             // Generic activity
            1 | 33 => SportType::Run,            // Running / Outdoor run
            34 => SportType::VirtualRun,         // Indoor run/Treadmill
            16 => SportType::Ride,               // Cycling
            17 => SportType::VirtualRide,        // Indoor cycling/Spin
            18 => SportType::MountainBike,       // Mountain biking
            43 | 44 => SportType::Swim,          // Swimming / Open water swim
            48 => SportType::Rowing,             // Rowing
            63 => SportType::Yoga,               // Yoga
            64 => SportType::Pilates,            // Pilates
            71 => SportType::StrengthTraining,   // Weightlifting
            47 => SportType::CrossCountrySkiing, // Cross-country skiing
            46 => SportType::AlpineSkiing,       // Alpine skiing
            45 => SportType::Snowboarding,       // Snowboarding
            52 => SportType::Hike,               // Hiking
            50 => SportType::Walk,               // Walking
            82 => SportType::Golf,               // Golf
            83 => SportType::Tennis,             // Tennis
            84 => SportType::Basketball,         // Basketball
            85 => SportType::Soccer,             // Soccer
            54 => SportType::RockClimbing,       // Climbing
            _ => SportType::Other(format!("whoop_sport_{sport_id}")),
        }
    }

    /// Convert WHOOP workout to our Activity model
    fn convert_workout(workout: &WhoopWorkout) -> AppResult<Activity> {
        let start_date = DateTime::parse_from_rfc3339(&workout.start)
            .map_err(|e| AppError::internal(format!("Failed to parse workout start date: {e}")))?
            .with_timezone(&Utc);

        let end_date = DateTime::parse_from_rfc3339(&workout.end)
            .map_err(|e| AppError::internal(format!("Failed to parse workout end date: {e}")))?
            .with_timezone(&Utc);

        let duration_seconds = (end_date - start_date).num_seconds().unsigned_abs();

        let score = workout.score.as_ref();

        let sport_id = workout.sport_id.unwrap_or(0);
        Ok(ActivityBuilder::new(
            workout.id.clone(),
            format!("WHOOP {}", Self::parse_sport_type(sport_id).display_name()),
            Self::parse_sport_type(sport_id),
            start_date,
            duration_seconds,
            oauth_providers::WHOOP,
        )
        .distance_meters_opt(score.and_then(|s| s.distance_meter))
        .elevation_gain_opt(score.and_then(|s| s.altitude_gain_meter))
        .average_heart_rate_opt(score.and_then(|s| s.average_heart_rate).map(|hr| hr as u32))
        .max_heart_rate_opt(score.and_then(|s| s.max_heart_rate).map(|hr| hr as u32))
        .calories_opt(
            score
                .and_then(|s| s.kilojoule)
                .map(|kj| (kj * 0.239) as u32),
        )
        .sport_type_detail_opt(Some(format!("whoop_sport_{sport_id}")))
        .build())
    }
}

impl Default for WhoopProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FitnessProvider for WhoopProvider {
    fn name(&self) -> &'static str {
        oauth_providers::WHOOP
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        info!("Setting WHOOP credentials");
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
                    provider: oauth_providers::WHOOP.to_owned(),
                    details: "No credentials available".to_owned(),
                };
                return Err(AppError::external_service("WHOOP", err.to_string()));
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

        // WHOOP rotates refresh tokens and only returns a new one when
        // `scope=offline` is sent on the refresh; without it the single-use
        // refresh token is consumed but not replaced, so the next refresh
        // fails with HTTP 400 invalid_request.
        let mut new_credentials = utils::refresh_oauth_token(
            &self.client,
            &utils::RefreshRequest {
                token_url: &self.config.token_url,
                client_id: &credentials.client_id,
                client_secret: &credentials.client_secret,
                refresh_token: &refresh_token,
                provider_name: oauth_providers::WHOOP,
                client_auth: utils::ClientAuth::FormFields,
                extra_form: &[("scope", "offline")],
            },
        )
        .await?;
        // The rotated refresh token is kept when WHOOP sends one; scopes are
        // not re-issued by the exchange, so carry the stored set across.
        new_credentials.refresh_token = new_credentials.refresh_token.or(Some(refresh_token));
        new_credentials.scopes = credentials.scopes;

        *self.credentials.write().await = Some(new_credentials.clone());

        // Persist refreshed token to database via callback
        if let Some(callback) = self.token_refresh_callback.get() {
            callback(new_credentials).await;
        }
        Ok(())
    }

    #[instrument(skip(self), fields(provider = "whoop", api_call = "get_athlete"))]
    async fn get_athlete(&self) -> AppResult<Athlete> {
        let profile: WhoopUserProfile = self.api_request("user/profile/basic").await?;

        Ok(Athlete {
            id: profile.user_id.to_string(),
            username: profile.email.clone().unwrap_or_default(),
            firstname: profile.first_name,
            lastname: profile.last_name,
            profile_picture: None, // WHOOP doesn't provide profile pictures via API
            provider: oauth_providers::WHOOP.to_owned(),
        })
    }

    #[instrument(
        skip(self, params),
        fields(
            provider = "whoop",
            api_call = "get_activities",
            limit = ?params.limit,
            offset = ?params.offset,
        )
    )]
    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        let requested = params
            .limit
            .unwrap_or(api_provider_limits::whoop::DEFAULT_ACTIVITIES_PER_PAGE);
        let page_size = api_provider_limits::whoop::MAX_ACTIVITIES_PER_REQUEST;

        // WHOOP uses token-based pagination, offset is not directly supported
        // For offset support, we'd need to paginate through until we reach the offset
        // For simplicity, we'll fetch from the beginning
        if params.offset.is_some_and(|o| o > 0) {
            warn!("WHOOP provider offset pagination is limited - fetching from beginning");
        }

        // WHOOP answers at most 25 workouts per request. This used to clamp the
        // caller's limit to that and return quietly, which is not a local
        // truncation: the historical backfill asks for two thousand activities
        // and reads `fetched_count < fetch_limit` as proof the window was
        // exhausted, so a silent 25 made it record a season it never read. Walk
        // the `next_token` chain instead, bounded by the shared page ceiling.
        let pages = pages_for(requested, page_size);
        let mut activities = Vec::with_capacity(requested.min(page_size * pages));
        let mut next_token: Option<String> = None;

        for _ in 0..pages {
            if activities.len() >= requested {
                break;
            }
            // Build endpoint with optional time filter (WHOOP supports start/end parameters)
            let mut endpoint = format!("activity/workout?limit={page_size}");
            if let Some(after) = params.after {
                if let Some(dt) = chrono::DateTime::from_timestamp(after, 0) {
                    let _ = write!(endpoint, "&start={}", dt.format("%Y-%m-%dT%H:%M:%S%.3fZ"));
                }
            }
            if let Some(before) = params.before {
                if let Some(dt) = chrono::DateTime::from_timestamp(before, 0) {
                    let _ = write!(endpoint, "&end={}", dt.format("%Y-%m-%dT%H:%M:%S%.3fZ"));
                }
            }
            if let Some(token) = &next_token {
                let _ = write!(endpoint, "&nextToken={token}");
            }

            // WHOOP returns 404 for collection endpoints when no data exists in range
            let response: WhoopPaginatedResponse<WhoopWorkout> =
                match self.api_request(&endpoint).await {
                    Ok(resp) => resp,
                    Err(e) if e.to_string().contains("No data available from WHOOP") => {
                        info!("No WHOOP workout data in requested range, returning empty");
                        break;
                    }
                    Err(e) => return Err(e),
                };

            for workout in &response.records {
                match Self::convert_workout(workout) {
                    Ok(activity) => activities.push(activity),
                    Err(e) => {
                        warn!("Failed to convert WHOOP workout: {e}");
                    }
                }
            }

            // No token means WHOOP has nothing further in this window; an empty
            // page with a token would otherwise walk to the ceiling for nothing.
            next_token = response.next_token;
            if next_token.is_none() || response.records.is_empty() {
                break;
            }
        }

        activities.truncate(requested);
        Ok(activities)
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        let limit = params.limit.min(25);

        // Build endpoint with cursor-based parameters
        let mut endpoint = format!("activity/workout?limit={limit}");

        // If cursor provided, use it as the next_token
        if let Some(cursor) = &params.cursor {
            if let Some((_, token)) = cursor.decode() {
                let _ = write!(endpoint, "&nextToken={token}");
            }
        }

        // WHOOP returns 404 for collection endpoints when no data exists in range
        let response: WhoopPaginatedResponse<WhoopWorkout> = match self.api_request(&endpoint).await
        {
            Ok(resp) => resp,
            Err(e) if e.to_string().contains("No data available from WHOOP") => {
                return Ok(CursorPage::new(vec![], None, None, false));
            }
            Err(e) => return Err(e),
        };

        let mut activities = Vec::with_capacity(response.records.len());
        for workout in &response.records {
            match Self::convert_workout(workout) {
                Ok(activity) => activities.push(activity),
                Err(e) => {
                    warn!("Failed to convert WHOOP workout: {e}");
                }
            }
        }

        // Determine if there are more results
        let has_more = response.next_token.is_some();

        // Create next cursor from next_token
        let next_cursor = response.next_token.as_ref().map(|token| {
            // Use current time and token as cursor
            Cursor::new(Utc::now(), token)
        });

        // Create previous cursor from first activity
        let prev_cursor = if params.cursor.is_some() {
            activities
                .first()
                .map(|first| Cursor::new(first.start_date(), first.id()))
        } else {
            None
        };

        Ok(CursorPage::new(
            activities,
            next_cursor,
            prev_cursor,
            has_more,
        ))
    }

    #[instrument(
        skip(self),
        fields(provider = "whoop", api_call = "get_activity", activity_id = %id)
    )]
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        let endpoint = format!("activity/workout/{id}");
        let workout: WhoopWorkout = self.api_request(&endpoint).await?;
        Self::convert_workout(&workout)
    }

    #[instrument(skip(self), fields(provider = "whoop", api_call = "get_stats"))]
    async fn get_stats(&self) -> AppResult<Stats> {
        // WHOOP exposes no aggregate-stats endpoint, so derive the rollup from
        // the available workouts (WHOOP returns the most recent page) rather than
        // reporting synthetic zeros. Mirrors the Terra provider's
        // compute-from-activities approach.
        let activities = self
            .get_activities_with_params(&ActivityQueryParams::with_pagination(None, None))
            .await?;

        let total_activities = activities.len() as u64;
        let total_distance: f64 = activities
            .iter()
            .filter_map(Activity::distance_meters)
            .sum();
        let total_duration: u64 = activities.iter().map(Activity::duration_seconds).sum();
        let total_elevation_gain: f64 =
            activities.iter().filter_map(Activity::elevation_gain).sum();

        Ok(Stats {
            total_activities,
            total_distance,
            total_duration,
            total_elevation_gain,
            year_to_date: None,
        })
    }

    async fn get_personal_records(&self) -> AppResult<Vec<PersonalRecord>> {
        // WHOOP doesn't track personal records in the same way
        Ok(vec![])
    }
}

// ============================================================================
// Provider Factory
// ============================================================================

use super::core::ProviderFactory;

/// Factory for creating WHOOP provider instances
pub struct WhoopProviderFactory;

impl ProviderFactory for WhoopProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(WhoopProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[oauth_providers::WHOOP]
    }
}

// ============================================================================
// Owner id lookup
// ============================================================================

/// Read the WHOOP user id behind an access token.
///
/// WHOOP's token response carries no owner id, but its webhooks name the
/// athlete by that id and nothing else, so a stored token without it can
/// never be matched to a push event. The id is served at
/// `user/profile/basic`; the OAuth flow reads it right after the exchange and
/// the refresh path fills a stored token that still lacks it. Both go through
/// the registry's WHOOP provider — the same request path, circuit breaker and
/// `PIERRE_WHOOP_API_BASE_URL` seam as every other WHOOP call — rather than a
/// second client.
///
/// The access token is the only credential set: a bearer read of the profile
/// never refreshes, so no client id, secret, refresh token or expiry is
/// needed, and the provider instance is dropped afterwards.
///
/// # Errors
///
/// Returns the registry's error when WHOOP is not registered and the
/// provider's own error when the profile read fails (a rejected token, a
/// transport failure).
pub async fn owner_id_for_access_token(
    registry: &ProviderRegistry,
    access_token: &str,
) -> AppResult<String> {
    let provider = registry.create_provider(oauth_providers::WHOOP)?;
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Some(access_token.to_owned()),
            refresh_token: None,
            expires_at: None,
            scopes: Vec::new(),
        })
        .await?;
    Ok(provider.get_athlete().await?.id)
}
