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
use crate::models::refresh_due;
use crate::models::{Activity, ActivityBuilder, Athlete, SportType, Stats};
use crate::pagination::{Cursor, CursorPage, PaginationParams};
use crate::request_budget;
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
    /// Name of the WHOOP sport performed — the field WHOOP's v2 API requires;
    /// WHOOP documents `sport_id` as removed after 2025-09-01.
    sport_name: Option<String>,
    /// WHOOP's numeric sport id, read only when the name does not resolve
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
        // The signing app's budget, which admits each request before it is sent.
        let budget = request_budget::carried_by(&self.credentials).await;
        let result = utils::api_request_with_retry(
            &self.client,
            &url,
            &access_token,
            oauth_providers::WHOOP,
            &retry_config,
            budget.as_ref(),
            Self::no_data_for_period,
        )
        .await;

        // Record success/failure for circuit breaker
        match &result {
            Ok(_) => self.circuit_breaker.record_success(),
            Err(e) => self.circuit_breaker.record_error(e),
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

        let sport_name = workout
            .sport_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let sport = whoop_sport(sport_name, workout.sport_id);
        Ok(ActivityBuilder::new(
            workout.id.clone(),
            format!("WHOOP {}", sport.display_name()),
            sport,
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
        .sport_type_detail_opt(
            sport_name
                .map(str::to_owned)
                .or_else(|| workout.sport_id.map(|id| format!("whoop_sport_{id}"))),
        )
        .build())
    }
}

/// WHOOP's sports that have a Dravr sport of their own: id, WHOOP's name for
/// it, and the sport, as WHOOP's workout documentation lists them
/// (developer.whoop.com, "Workout" → sport ids). Every WHOOP sport absent
/// here keeps WHOOP's own name rather than a guessed one.
const WHOOP_SPORTS: &[(i32, &str, SportType)] = &[
    (-1, "Activity", SportType::Workout),
    (0, "Running", SportType::Run),
    (1, "Cycling", SportType::Ride),
    (17, "Basketball", SportType::Basketball),
    (18, "Rowing", SportType::Rowing),
    (22, "Golf", SportType::Golf),
    (29, "Skiing", SportType::AlpineSkiing),
    (30, "Soccer", SportType::Soccer),
    (33, "Swimming", SportType::Swim),
    (34, "Tennis", SportType::Tennis),
    (43, "Pilates", SportType::Pilates),
    (44, "Yoga", SportType::Yoga),
    (45, "Weightlifting", SportType::StrengthTraining),
    (47, "Cross Country Skiing", SportType::CrossCountrySkiing),
    (48, "Functional Fitness", SportType::Crossfit),
    (52, "Hiking/Rucking", SportType::Hike),
    (55, "Kayaking", SportType::Kayaking),
    (57, "Mountain Biking", SportType::MountainBike),
    (59, "Powerlifting", SportType::StrengthTraining),
    (60, "Rock Climbing", SportType::RockClimbing),
    (61, "Paddleboarding", SportType::Paddleboarding),
    (63, "Walking", SportType::Walk),
    (64, "Surfing", SportType::Surfing),
    (71, "Other", SportType::Workout),
    (86, "Skateboarding", SportType::Skateboarding),
    (91, "Snowboarding", SportType::Snowboarding),
    (97, "Spin", SportType::VirtualRide),
    (102, "Inline Skating", SportType::InlineSkating),
    (123, "Strength Trainer", SportType::StrengthTraining),
    (239, "Ice Skating", SportType::IceSkating),
    (264, "Kite Boarding", SportType::Kitesurfing),
];

/// The sport of a WHOOP workout: its `sport_name` when [`WHOOP_SPORTS`] knows
/// it, else its `sport_id`, else WHOOP's own name as an `Other` sport. A
/// workout that names no sport at all is a generic `Workout`.
fn whoop_sport(sport_name: Option<&str>, sport_id: Option<i32>) -> SportType {
    let by_name = sport_name.and_then(|name| {
        let key = sport_name_key(name);
        WHOOP_SPORTS
            .iter()
            .find(|(_, known, _)| sport_name_key(known) == key)
    });
    let known = by_name
        .or_else(|| sport_id.and_then(|id| WHOOP_SPORTS.iter().find(|(known, _, _)| *known == id)));
    match (known, sport_name, sport_id) {
        (Some((_, _, sport)), _, _) => sport.clone(),
        (None, Some(name), _) if !name.trim().is_empty() => {
            SportType::Other(name.trim().to_owned())
        }
        (None, _, Some(id)) => SportType::Other(format!("whoop_sport_{id}")),
        (None, _, None) => SportType::Workout,
    }
}

/// A sport name reduced to its lowercase letters and digits, so WHOOP's
/// `Hiking/Rucking`, `hiking-rucking` and `hiking_rucking` read as one sport.
fn sport_name_key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
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
                creds.expires_at.is_some_and(refresh_due)
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

        let mut new_credentials = utils::refresh_oauth_token(
            &self.client,
            &utils::RefreshRequest::whoop(
                &self.config.token_url,
                &credentials.client_id,
                &credentials.client_secret,
                &refresh_token,
            ),
        )
        .await?;
        // The rotated refresh token is kept when WHOOP sends one; scopes are
        // not re-issued by the exchange, so carry the stored set across.
        new_credentials.refresh_token = new_credentials.refresh_token.or(Some(refresh_token));
        new_credentials.scopes = credentials.scopes;
        // A refresh keeps the app that signs the token, so its budget carries over.
        new_credentials.request_budget = credentials.request_budget;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn workout(sport_name: Option<&str>, sport_id: Option<i32>) -> WhoopWorkout {
        WhoopWorkout {
            id: "whoop-1".to_owned(),
            start: "2026-10-07T14:00:00.000Z".to_owned(),
            end: "2026-10-07T15:43:32.000Z".to_owned(),
            sport_name: sport_name.map(str::to_owned),
            sport_id,
            score: None,
        }
    }

    #[test]
    fn whoop_documented_ids_map_to_their_own_sports() {
        // 2026-10-07: a ride arrived as "WHOOP run" and a trail run as "WHOOP
        // workout" because 0 and 1 were read as Workout and Run. WHOOP's table
        // has 0 = Running, 1 = Cycling, 33 = Swimming, 44 = Yoga, 63 = Walking.
        assert_eq!(whoop_sport(None, Some(0)), SportType::Run);
        assert_eq!(whoop_sport(None, Some(1)), SportType::Ride);
        assert_eq!(whoop_sport(None, Some(33)), SportType::Swim);
        assert_eq!(whoop_sport(None, Some(44)), SportType::Yoga);
        assert_eq!(whoop_sport(None, Some(63)), SportType::Walk);
        assert_eq!(whoop_sport(None, Some(-1)), SportType::Workout);
    }

    #[test]
    fn whoop_sport_name_wins_over_the_id_in_any_spelling() {
        assert_eq!(whoop_sport(Some("cycling"), Some(0)), SportType::Ride);
        assert_eq!(whoop_sport(Some("hiking-rucking"), None), SportType::Hike);
        assert_eq!(
            whoop_sport(Some("Cross Country Skiing"), None),
            SportType::CrossCountrySkiing
        );
        assert_eq!(
            whoop_sport(Some("functional_fitness"), None),
            SportType::Crossfit
        );
    }

    #[test]
    fn whoop_unknown_sport_keeps_whoop_name_and_unnamed_is_a_workout() {
        assert_eq!(
            whoop_sport(Some("hiit"), Some(96)),
            SportType::Other("hiit".to_owned())
        );
        // A name the table does not know still falls back to a known id.
        assert_eq!(whoop_sport(Some("road cycling"), Some(1)), SportType::Ride);
        assert_eq!(
            whoop_sport(None, Some(16)),
            SportType::Other("whoop_sport_16".to_owned())
        );
        assert_eq!(whoop_sport(None, None), SportType::Workout);
    }

    #[test]
    fn whoop_workout_is_named_and_detailed_after_its_sport() {
        let activity = WhoopProvider::convert_workout(&workout(Some("cycling"), Some(1))).unwrap();
        assert_eq!(activity.sport_type(), &SportType::Ride);
        assert_eq!(activity.name(), "WHOOP bike ride");
        assert_eq!(activity.sport_type_detail(), Some("cycling"));
        assert_eq!(activity.duration_seconds(), 6_212);
    }

    #[test]
    fn whoop_blank_sport_name_reads_as_absent() {
        let activity = WhoopProvider::convert_workout(&workout(Some("  "), Some(1))).unwrap();
        assert_eq!(activity.sport_type(), &SportType::Ride);
        assert_eq!(activity.sport_type_detail(), Some("whoop_sport_1"));
    }
}
