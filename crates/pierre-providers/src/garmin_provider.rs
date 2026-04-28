// ABOUTME: Garmin Connect API provider implementation using unified provider architecture
// ABOUTME: Handles OAuth2 PKCE authentication and fitness data fetching with proper error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::circuit_breaker::CircuitBreaker;
use super::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, TokenRefreshCallback,
};
use super::errors::provider::ProviderError;
use super::utils::{self, RetryConfig};
use crate::constants::oauth::GARMIN_DEFAULT_SCOPES;
use crate::constants::{api_provider_limits, oauth_providers};
use crate::errors::{AppError, AppResult};
use crate::http_client::shared_client;
use crate::models::{
    activity::{Lap, Split},
    Activity, ActivityBuilder, Athlete, PersonalRecord, SportType, Stats,
};
use crate::pagination::{CursorPage, PaginationParams};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

/// Garmin API response for athlete data
#[derive(Debug, Deserialize)]
struct GarminAthleteResponse {
    user_id: String,
    display_name: Option<String>,
    full_name: Option<String>,
    profile_image_url: Option<String>,
}

/// Garmin Connect API activity-type DTO. Returned at multiple nesting depths
/// (`activityTypeDTO` on detail; `activityType` on list rows).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct GarminActivityType {
    #[serde(rename = "typeKey")]
    type_key: Option<String>,
    #[serde(rename = "parentTypeId")]
    parent_type_id: Option<u32>,
    #[serde(rename = "typeId")]
    type_id: Option<u32>,
}

/// Garmin's `summaryDTO` — top-level summary fields nested inside the
/// detail response. Numeric durations are seconds (f64); HR/cadence values
/// are f32 because Garmin's API emits floating-point HR for partial-second
/// averages.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GarminSummary {
    distance: Option<f64>,
    duration: Option<f64>,
    moving_duration: Option<f64>,
    elapsed_duration: Option<f64>,
    elevation_gain: Option<f64>,
    elevation_loss: Option<f64>,
    average_speed: Option<f64>,
    max_speed: Option<f64>,
    average_moving_speed: Option<f64>,
    #[serde(rename = "averageHR")]
    average_hr: Option<f32>,
    #[serde(rename = "maxHR")]
    max_hr: Option<f32>,
    #[serde(rename = "minHR")]
    min_hr: Option<f32>,
    average_run_cadence: Option<f32>,
    average_biking_cadence: Option<f32>,
    average_power: Option<f32>,
    max_power: Option<f32>,
    normalized_power: Option<f32>,
    calories: Option<f64>,
    bmr_calories: Option<f64>,
    average_temperature: Option<f64>,
    max_temperature: Option<f64>,
    min_temperature: Option<f64>,
    avg_elevation: Option<f64>,
    min_elevation: Option<f64>,
    max_elevation: Option<f64>,
    start_latitude: Option<f64>,
    start_longitude: Option<f64>,
    end_latitude: Option<f64>,
    end_longitude: Option<f64>,
    start_time_gmt: Option<String>,
    start_time_local: Option<String>,
    training_effect: Option<f32>,
    anaerobic_training_effect: Option<f32>,
    activity_training_load: Option<f32>,
}

/// One lap from Garmin's `/activity-service/activity/{id}/laps` endpoint.
/// Field names are camelCase in the source JSON; `serde(rename_all)` maps
/// them. All metric fields are `Option` because Garmin omits them when the
/// recording device didn't capture the corresponding sensor.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GarminLap {
    lap_index: Option<u32>,
    message_index: Option<u32>,
    start_time_gmt: Option<String>,
    distance: Option<f64>,
    duration: Option<f64>,
    elapsed_duration: Option<f64>,
    moving_duration: Option<f64>,
    elevation_gain: Option<f64>,
    elevation_loss: Option<f64>,
    max_elevation: Option<f64>,
    min_elevation: Option<f64>,
    average_speed: Option<f64>,
    max_speed: Option<f64>,
    average_moving_speed: Option<f64>,
    max_vertical_speed: Option<f64>,
    #[serde(rename = "averageHR")]
    average_hr: Option<f32>,
    #[serde(rename = "maxHR")]
    max_hr: Option<f32>,
    average_running_cadence_in_steps_per_minute: Option<f32>,
    average_biking_cadence_in_rev_per_minute: Option<f32>,
    average_power: Option<f32>,
    max_power: Option<f32>,
    calories: Option<f64>,
    bmr_calories: Option<f64>,
    average_temperature: Option<f64>,
    max_temperature: Option<f64>,
    min_temperature: Option<f64>,
    start_latitude: Option<f64>,
    start_longitude: Option<f64>,
    end_latitude: Option<f64>,
    end_longitude: Option<f64>,
}

/// Wrapper for the `/activity/{id}/laps` endpoint response.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GarminLapsResponse {
    activity_id: Option<u64>,
    #[serde(rename = "lapDTOs")]
    lap_dtos: Vec<GarminLap>,
}

/// One split from `/activity/{id}/splits` (typically per-km or per-mile slices).
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
struct GarminSplit {
    message_index: Option<u32>,
    start_time_gmt: Option<String>,
    distance: Option<f64>,
    duration: Option<f64>,
    moving_duration: Option<f64>,
    elevation_gain: Option<f64>,
    elevation_loss: Option<f64>,
    average_speed: Option<f64>,
    max_speed: Option<f64>,
    #[serde(rename = "averageHR")]
    average_hr: Option<f32>,
    #[serde(rename = "maxHR")]
    max_hr: Option<f32>,
    average_power: Option<f32>,
    calories: Option<f64>,
    start_latitude: Option<f64>,
    start_longitude: Option<f64>,
}

/// Wrapper for the `/activity/{id}/splits` endpoint response. Garmin returns
/// a top-level `lapDTOs` array (sic — they reuse the field name) for splits.
#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GarminSplitsResponse {
    #[serde(rename = "lapDTOs")]
    splits: Vec<GarminSplit>,
}

/// Top-level Garmin API response for `/activity-service/activity/{id}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GarminActivityResponse {
    #[serde(rename = "activityId")]
    activity_id: u64,
    #[serde(default, rename = "activityName")]
    activity_name: Option<String>,
    #[serde(default, rename = "activityTypeDTO")]
    activity_type_dto: GarminActivityType,
    #[serde(default, rename = "summaryDTO")]
    summary_dto: GarminSummary,
}

/// Garmin API response for summary stats
#[derive(Debug, Deserialize)]
struct GarminStatsResponse {
    #[serde(rename = "totalActivities")]
    activities_count: Option<u64>,
    #[serde(rename = "totalDistance")]
    distance: Option<f64>,
    #[serde(rename = "totalDuration")]
    duration: Option<f64>,
    #[serde(rename = "totalElevationGain")]
    elevation_gain: Option<f64>,
}

/// Garmin Connect provider implementation
pub struct GarminProvider {
    config: ProviderConfig,
    credentials: RwLock<Option<OAuth2Credentials>>,
    client: Client,
    circuit_breaker: CircuitBreaker,
    token_refresh_callback: OnceLock<TokenRefreshCallback>,
}

impl GarminProvider {
    /// Create a new Garmin provider with default configuration
    #[must_use]
    pub fn new() -> Self {
        let config = ProviderConfig {
            name: oauth_providers::GARMIN.to_owned(),
            auth_url: "https://connect.garmin.com/oauthConfirm".to_owned(),
            token_url: "https://connectapi.garmin.com/oauth-service/oauth/access_token".to_owned(),
            api_base_url: "https://apis.garmin.com/wellness-api/rest".to_owned(),
            revoke_url: Some("https://connectapi.garmin.com/oauth-service/oauth/revoke".to_owned()),
            default_scopes: GARMIN_DEFAULT_SCOPES
                .split(',')
                .map(str::to_owned)
                .collect(),
        };

        Self {
            circuit_breaker: CircuitBreaker::new(oauth_providers::GARMIN),
            config,
            credentials: RwLock::new(None),
            // Clone Arc<Client> from shared singleton - cheap reference counting operation
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
            // Clone Arc<Client> from shared singleton - cheap reference counting operation
            client: shared_client().clone(),
            token_refresh_callback: OnceLock::new(),
        }
    }

    /// Variant of [`api_request`] that returns `None` on any error instead of
    /// propagating. Used by detail-mode fanout where a missing sibling
    /// endpoint (e.g. `/laps` for swim activities) shouldn't fail the whole
    /// operation — we just lose enrichment for that one slice of data.
    async fn api_request_optional<T>(&self, endpoint: &str) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        match self.api_request::<T>(endpoint).await {
            Ok(value) => Some(value),
            Err(e) => {
                debug!(endpoint, error = %e, "Optional Garmin API call failed; continuing without it");
                None
            }
        }
    }

    /// Make authenticated API request with rate limit handling and circuit breaker protection
    /// Uses shared retry logic with exponential backoff for 429 errors
    async fn api_request<T>(&self, endpoint: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        // Check circuit breaker before making request
        if !self.circuit_breaker.is_allowed() {
            let err = ProviderError::CircuitBreakerOpen {
                provider: oauth_providers::GARMIN.to_owned(),
                retry_after_secs: 30,
            };
            return Err(AppError::external_service("Garmin", err.to_string()));
        }

        // Clone access token to avoid holding lock across await
        let access_token = {
            let guard = self.credentials.read().await;
            let credentials = guard.as_ref().ok_or_else(|| {
                AppError::internal("No credentials available for Garmin API request")
            })?;

            let token = credentials
                .access_token
                .clone() // Safe: String ownership needed for async request
                .ok_or_else(|| AppError::internal("No access token available"))?;
            drop(guard); // Release lock immediately after cloning
            token
        };

        let url = format!(
            "{}/{}",
            self.config.api_base_url,
            endpoint.trim_start_matches('/')
        );

        let retry_config = RetryConfig {
            max_retries: 3,
            initial_backoff_ms: 1000,
            retryable_status_codes: vec![StatusCode::TOO_MANY_REQUESTS],
            estimated_block_duration_secs:
                api_provider_limits::garmin::ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS,
        };

        let result = utils::api_request_with_retry(
            &self.client,
            &url,
            &access_token,
            "Garmin",
            &retry_config,
        )
        .await;

        // Record success/failure for circuit breaker
        match &result {
            Ok(_) => self.circuit_breaker.record_success(),
            Err(_) => self.circuit_breaker.record_failure(),
        }

        result
    }

    /// Convert Garmin activity type to our `SportType` enum
    /// Based on Garmin Connect activity types from `activity_types.properties`
    /// Source: <https://connect.garmin.com/modern/main/js/properties/activity_types/activity_types.properties>
    fn parse_sport_type(garmin_type: &str) -> SportType {
        match garmin_type.to_lowercase().as_str() {
            // Running variants
            "running" | "run" | "track" | "track_running" => SportType::Run,
            "trail_running" | "trail_run" => SportType::TrailRunning,
            "treadmill" | "treadmill_running" => SportType::VirtualRun,

            // Cycling variants
            "cycling" | "bike" | "biking" | "road_cycling" | "road" | "cyclocross" | "cx" => {
                SportType::Ride
            }
            "mountain_biking" | "mountain_bike_ride" | "mountain" => SportType::MountainBike,
            "indoor_cycling" | "spin" | "virtual_ride" => SportType::VirtualRide,
            "gravel_cycling" | "gravel_ride" => SportType::GravelRide,
            "ebike" | "e_bike_ride" | "e_bike" => SportType::EbikeRide,

            // Swimming variants
            "swimming"
            | "swim"
            | "open_water_swimming"
            | "open_water"
            | "pool_swimming"
            | "lap_swimming" => SportType::Swim,

            // Walking and hiking
            "walking" | "walk" | "casual_walking" => SportType::Walk,
            "hiking" | "hike" => SportType::Hike,

            // Winter sports
            "resort_skiing" | "alpine_skiing" | "downhill_skiing" => SportType::AlpineSkiing,
            "backcountry_skiing_snowboarding" | "backcountry_skiing" => {
                SportType::BackcountrySkiing
            }
            "cross_country_skiing" | "xc_skiing" | "nordic_skiing" => SportType::CrossCountrySkiing,
            "snowboarding" | "snowboard" => SportType::Snowboarding,
            "snowshoeing" | "snowshoe" => SportType::Snowshoe,
            "ice_skating" | "skating" => SportType::IceSkating,

            // Water sports
            "kayaking" | "kayak" => SportType::Kayaking,
            "canoeing" | "canoe" => SportType::Canoeing,
            "rowing" | "row" | "indoor_rowing" => SportType::Rowing,
            "stand_up_paddleboarding" | "sup" | "paddleboarding" => SportType::Paddleboarding,
            "surfing" | "surf" => SportType::Surfing,
            "kitesurfing" | "kiteboarding" => SportType::Kitesurfing,

            // Strength and fitness
            "strength_training" | "weight_training" | "weights" => SportType::StrengthTraining,
            "crossfit" | "cross_fit" => SportType::Crossfit,
            "pilates" => SportType::Pilates,
            "yoga" => SportType::Yoga,

            // Climbing and adventure
            "rock_climbing" | "climbing" | "bouldering" => SportType::RockClimbing,

            // Team sports
            "soccer" | "football" => SportType::Soccer,
            "basketball" => SportType::Basketball,
            "tennis" => SportType::Tennis,
            "golf" => SportType::Golf,

            // Alternative transport
            "skateboarding" | "skateboard" => SportType::Skateboarding,
            "inline_skating" | "roller_skating" => SportType::InlineSkating,

            // Generic/other - combined cardio, fitness equipment, and generic workout types
            "cardio" | "cardio_training" | "elliptical" | "fitness_equipment"
            | "stair_climbing" | "stepper" | "other" | "generic" | "workout" | "training" => {
                SportType::Workout
            }

            // Unmapped types fall through to Other variant
            _ => SportType::Other(garmin_type.to_owned()),
        }
    }

    /// Convert Garmin activity response to internal Activity model
    /// Build an `ActivityBuilder` from a Garmin response without finalizing it.
    /// Shared by the summary-path conversion and the detailed-path conversion —
    /// the detailed path layers laps/splits before [`ActivityBuilder::build`].
    fn garmin_activity_builder(activity: GarminActivityResponse) -> AppResult<ActivityBuilder> {
        let summary = activity.summary_dto;
        let activity_type_key = activity
            .activity_type_dto
            .type_key
            .as_deref()
            .unwrap_or("unknown");

        // Garmin returns timestamps as SQL-style "YYYY-MM-DD HH:MM:SS.f" or
        // RFC3339 depending on the field. Normalize both shapes.
        let start_time_str = summary
            .start_time_gmt
            .as_deref()
            .ok_or_else(|| AppError::internal("Garmin activity missing summaryDTO.startTimeGMT"))?;
        let start_date = parse_garmin_timestamp(start_time_str)
            .map_err(|e| AppError::internal(format!("Failed to parse activity start date: {e}")))?;

        Ok(ActivityBuilder::new(
            activity.activity_id.to_string(),
            activity
                .activity_name
                .unwrap_or_else(|| "Untitled".to_owned()),
            Self::parse_sport_type(activity_type_key),
            start_date,
            summary.duration.map_or(0, utils::conversions::f64_to_u64),
            oauth_providers::GARMIN,
        )
        .distance_meters_opt(summary.distance)
        .elevation_gain_opt(summary.elevation_gain)
        .average_speed_opt(summary.average_speed)
        .max_speed_opt(summary.max_speed)
        .average_heart_rate_opt(summary.average_hr.map(utils::conversions::f32_to_u32))
        .max_heart_rate_opt(summary.max_hr.map(utils::conversions::f32_to_u32))
        .average_cadence_opt(
            summary
                .average_run_cadence
                .or(summary.average_biking_cadence)
                .map(utils::conversions::f32_to_u32),
        )
        .average_power_opt(summary.average_power.map(utils::conversions::f32_to_u32))
        .max_power_opt(summary.max_power.map(utils::conversions::f32_to_u32))
        .calories_opt(summary.calories.map(utils::conversions::f64_to_u32)))
    }

    fn convert_garmin_activity(activity: GarminActivityResponse) -> AppResult<Activity> {
        Ok(Self::garmin_activity_builder(activity)?.build())
    }
}

/// Parse a Garmin timestamp string. Garmin emits two formats across their
/// API surface:
/// - SQL datetime: `"2026-04-27 16:25:47.0"` (assumed UTC, no offset suffix)
/// - RFC3339: `"2026-04-27T16:25:47.0Z"`
///
/// Returns a UTC `DateTime`. Returns an error string if neither shape parses.
fn parse_garmin_timestamp(s: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    let trimmed = s.trim_end_matches(".0");
    let formats = [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
    ];
    for fmt in &formats {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(trimmed, fmt) {
            return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
        }
    }
    Err(format!("Unrecognized Garmin timestamp format: {s}"))
}

/// Translate Garmin's camelCase `lapDTO` into cageux's `snake_case` [`Lap`].
/// Falls back to the slot index when Garmin omits `lapIndex`. Garmin's
/// running cadence is in steps/min; biking cadence in revs/min — we surface
/// whichever is present. Returns `None` when distance or duration are absent
/// — a lap without those two anchors cannot be located on the timeline and
/// would only add noise to coach reasoning.
fn convert_garmin_lap(slot: usize, l: &GarminLap) -> Option<Lap> {
    let distance_meters = l.distance?;
    let elapsed_time_seconds = utils::conversions::f64_to_u64(l.duration?);
    Some(Lap {
        id: None,
        index: l
            .lap_index
            .unwrap_or_else(|| u32::try_from(slot + 1).unwrap_or(u32::MAX)),
        distance_meters,
        elapsed_time_seconds,
        moving_time_seconds: l.moving_duration.map(utils::conversions::f64_to_u64),
        elevation_gain_meters: l.elevation_gain,
        average_speed_mps: l.average_speed,
        max_speed_mps: l.max_speed,
        average_heart_rate: l.average_hr.map(utils::conversions::f32_to_u32),
        max_heart_rate: l.max_hr.map(utils::conversions::f32_to_u32),
        average_cadence: l
            .average_running_cadence_in_steps_per_minute
            .or(l.average_biking_cadence_in_rev_per_minute)
            .map(utils::conversions::f32_to_u32),
        average_power: l.average_power.map(utils::conversions::f32_to_u32),
    })
}

/// Translate Garmin's camelCase split shape into cageux's [`Split`]. Garmin
/// emits per-km/per-mile splits via the `/splits` endpoint (which uses the
/// `lapDTOs` JSON key — sic — for the array). Returns `None` for splits
/// missing distance or duration.
fn convert_garmin_split(slot: usize, s: &GarminSplit) -> Option<Split> {
    let distance_meters = s.distance?;
    let elapsed_time_seconds = utils::conversions::f64_to_u64(s.duration?);
    Some(Split {
        index: s
            .message_index
            .unwrap_or_else(|| u32::try_from(slot + 1).unwrap_or(u32::MAX)),
        distance_meters,
        elapsed_time_seconds,
        moving_time_seconds: s.moving_duration.map(utils::conversions::f64_to_u64),
        elevation_difference_meters: s.elevation_gain,
        average_speed_mps: s.average_speed,
        average_heart_rate: s.average_hr.map(utils::conversions::f32_to_u32),
        pace_zone: None,
    })
}

impl Default for GarminProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FitnessProvider for GarminProvider {
    fn name(&self) -> &'static str {
        oauth_providers::GARMIN
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        info!("Setting Garmin credentials");
        *self.credentials.write().await = Some(credentials);
        Ok(())
    }

    fn set_token_refresh_callback(&self, callback: TokenRefreshCallback) {
        let _ = self.token_refresh_callback.set(callback);
    }

    async fn is_authenticated(&self) -> bool {
        if let Some(creds) = self.credentials.read().await.as_ref() {
            if creds.access_token.is_some() {
                // Check if token is expired
                if let Some(expires_at) = creds.expires_at {
                    return Utc::now() < expires_at;
                }
                return true;
            }
        }
        false
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
                return Err(AppError::internal("No credentials available"));
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

        info!("Refreshing Garmin access token");

        let mut new_credentials = utils::refresh_oauth_token(
            &self.client,
            &self.config.token_url,
            &credentials.client_id,
            &credentials.client_secret,
            &refresh_token,
            "Garmin",
        )
        .await?;

        // Preserve original scopes
        new_credentials.scopes = credentials.scopes;

        *self.credentials.write().await = Some(new_credentials.clone());

        // Persist refreshed token to database via callback
        if let Some(callback) = self.token_refresh_callback.get() {
            callback(new_credentials).await;
        }

        Ok(())
    }

    #[instrument(skip(self), fields(provider = "garmin", api_call = "get_athlete"))]
    async fn get_athlete(&self) -> AppResult<Athlete> {
        // Source: https://github.com/cyberjunky/python-garminconnect
        // Endpoint: /userprofile-service/userprofile/profile
        let garmin_athlete: GarminAthleteResponse = self
            .api_request("userprofile-service/userprofile/profile")
            .await?;

        Ok(Athlete {
            id: garmin_athlete.user_id,
            // Use as_deref() to borrow rather than clone the String
            username: garmin_athlete
                .display_name
                .as_deref()
                .unwrap_or_default()
                .to_owned(),
            firstname: garmin_athlete.full_name,
            lastname: None,
            profile_picture: garmin_athlete.profile_image_url,
            provider: oauth_providers::GARMIN.to_owned(),
        })
    }

    #[instrument(
        skip(self, params),
        fields(
            provider = "garmin",
            api_call = "get_activities",
            limit = ?params.limit,
            offset = ?params.offset,
        )
    )]
    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        let requested_limit = params
            .limit
            .unwrap_or(api_provider_limits::garmin::DEFAULT_ACTIVITIES_PER_PAGE);
        let start_offset = params.offset.unwrap_or(0);

        info!(
            "Starting get_activities - requested_limit: {}, offset: {}, before: {:?}, after: {:?}",
            requested_limit, start_offset, params.before, params.after
        );

        // Garmin API doesn't support before/after params directly, they're for client-side filtering
        if requested_limit <= api_provider_limits::garmin::MAX_ACTIVITIES_PER_REQUEST {
            return self
                .get_activities_single_page(requested_limit, start_offset)
                .await;
        }

        self.get_activities_multi_page(requested_limit, start_offset)
            .await
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        // Garmin API uses numeric pagination - delegate to offset-based approach
        let query_params = ActivityQueryParams::with_pagination(Some(params.limit), None);
        let activities = self.get_activities_with_params(&query_params).await?;
        Ok(CursorPage::new(activities, None, None, false))
    }

    #[instrument(
        skip(self),
        fields(provider = "garmin", api_call = "get_activity", activity_id = %id)
    )]
    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        // Source: https://github.com/cyberjunky/python-garminconnect
        // Endpoint: /activity-service/activity/{activity_id}
        let endpoint = format!("activity-service/activity/{id}");
        let garmin_activity: GarminActivityResponse = self.api_request(&endpoint).await?;
        Self::convert_garmin_activity(garmin_activity)
    }

    /// Detail-mode fetch: main activity + sibling `/laps` and `/splits`
    /// endpoints folded into the resulting `Activity`.
    ///
    /// Garmin Connect splits the per-activity surface across three endpoints
    /// in the `/app/` redesign. The list endpoint and the bare detail endpoint
    /// give summary metrics; per-lap and per-split arrays live on the sibling
    /// paths. We fan out 3 GETs, tolerating per-endpoint failures so a missing
    /// laps response (e.g. swim activities) doesn't kill the whole fetch.
    async fn get_activity_detailed(&self, id: &str) -> AppResult<Activity> {
        let main_endpoint = format!("activity-service/activity/{id}");
        let laps_endpoint = format!("activity-service/activity/{id}/laps");
        let splits_endpoint = format!("activity-service/activity/{id}/splits");

        let main: GarminActivityResponse = self.api_request(&main_endpoint).await?;
        let laps_result: Option<GarminLapsResponse> =
            self.api_request_optional(&laps_endpoint).await;
        let splits_result: Option<GarminSplitsResponse> =
            self.api_request_optional(&splits_endpoint).await;

        let laps = laps_result.and_then(|laps| {
            let converted: Vec<Lap> = laps
                .lap_dtos
                .iter()
                .enumerate()
                .filter_map(|(i, l)| convert_garmin_lap(i, l))
                .collect();
            (!converted.is_empty()).then_some(converted)
        });

        let splits = splits_result.and_then(|splits| {
            let converted: Vec<Split> = splits
                .splits
                .iter()
                .enumerate()
                .filter_map(|(i, s)| convert_garmin_split(i, s))
                .collect();
            (!converted.is_empty()).then_some(converted)
        });

        Ok(Self::garmin_activity_builder(main)?
            .laps_opt(laps)
            .splits_opt(splits)
            .build())
    }

    #[instrument(skip(self), fields(provider = "garmin", api_call = "get_stats"))]
    async fn get_stats(&self) -> AppResult<Stats> {
        // Source: https://github.com/cyberjunky/python-garminconnect
        // Using aggregate stats endpoint which provides all-time totals
        // Alternative endpoint /usersummary-service/usersummary/daily/{display_name}?calendarDate={date}
        // provides daily summaries but requires display_name and specific date parameters
        let stats: GarminStatsResponse = self
            .api_request("usersummary-service/stats/aggregate")
            .await?;

        let total_activities = stats.activities_count.unwrap_or_else(|| {
            debug!("Garmin API returned None for activities_count - defaulting to 0");
            0
        });

        let total_distance = stats.distance.unwrap_or_else(|| {
            debug!("Garmin API returned None for distance - defaulting to 0.0");
            0.0
        });

        let total_duration = stats.duration.map_or_else(
            || {
                debug!("Garmin API returned None for duration - defaulting to 0");
                0
            },
            utils::conversions::f64_to_u64,
        );

        let total_elevation_gain = stats.elevation_gain.unwrap_or_else(|| {
            debug!("Garmin API returned None for elevation_gain - defaulting to 0.0");
            0.0
        });

        Ok(Stats {
            total_activities,
            total_distance,
            total_duration,
            total_elevation_gain,
        })
    }

    async fn get_personal_records(&self) -> AppResult<Vec<PersonalRecord>> {
        // Garmin Connect does not expose a dedicated personal records endpoint
        // Personal records would need to be computed from activity history analysis
        // or extracted from the athlete profile if available in future API updates
        Ok(vec![])
    }

    async fn disconnect(&self) -> AppResult<()> {
        // Clone access token and revoke URL to avoid holding lock across await
        let (access_token_opt, revoke_url_opt) = {
            let guard = self.credentials.read().await;
            guard.as_ref().map_or((None, None), |credentials| {
                (
                    credentials.access_token.clone(), // Safe: String ownership for revoke request
                    self.config.revoke_url.clone(),   // Safe: String ownership for revoke request
                )
            })
        };

        if let (Some(access_token), Some(revoke_url)) = (access_token_opt, revoke_url_opt) {
            self.client
                .post(&revoke_url)
                .form(&[("token", access_token.as_str())])
                .send()
                .await
                .inspect_err(|e| {
                    warn!(
                        error = ?e,
                        "Failed to revoke Garmin access token - continuing with credential cleanup"
                    );
                })
                .ok();
            info!("Attempted to revoke Garmin access token");
        }

        // Clear credentials regardless of revoke success
        *self.credentials.write().await = None;
        Ok(())
    }
}

impl GarminProvider {
    /// Fetch activities using single API call
    async fn get_activities_single_page(
        &self,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<Activity>> {
        // Source: https://github.com/cyberjunky/python-garminconnect
        // Endpoint: /activitylist-service/activities/search/activities?start={offset}&limit={limit}
        let endpoint = format!(
            "activitylist-service/activities/search/activities?start={offset}&limit={limit}"
        );

        info!("Single page request - endpoint: {}", endpoint);

        let garmin_activities: Vec<GarminActivityResponse> = self.api_request(&endpoint).await?;
        info!(
            "Received {} activities from single page",
            garmin_activities.len()
        );

        let mut activities = Vec::new();
        for activity in garmin_activities {
            activities.push(Self::convert_garmin_activity(activity)?);
        }

        Ok(activities)
    }

    /// Fetch activities using multiple API calls
    /// Build the endpoint URL for a page of activities
    fn build_activities_endpoint(offset: usize, limit: usize) -> String {
        format!("activitylist-service/activities/search/activities?start={offset}&limit={limit}")
    }

    /// Convert and add Garmin activities to the collection
    fn add_converted_activities(
        activities: &mut Vec<Activity>,
        garmin_activities: Vec<GarminActivityResponse>,
        limit: usize,
    ) -> AppResult<()> {
        for activity in garmin_activities {
            if activities.len() >= limit {
                break;
            }
            activities.push(Self::convert_garmin_activity(activity)?);
        }
        Ok(())
    }

    /// Check if pagination should stop based on current state
    fn should_stop_pagination(
        activities_count: usize,
        page_index: usize,
        activities_per_page: usize,
        total_limit: usize,
    ) -> bool {
        if activities_count >= total_limit {
            return true;
        }

        let expected_count = (page_index + 1) * activities_per_page.min(total_limit);
        if activities_count < expected_count {
            info!(
                "Reached end of activities - got {} total, breaking early",
                activities_count
            );
            return true;
        }

        false
    }

    /// Fetch a single page of activities and add to collection
    async fn fetch_activities_page(
        &self,
        all_activities: &mut Vec<Activity>,
        page_index: usize,
        pages_needed: usize,
        start_offset: usize,
        activities_per_page: usize,
        total_limit: usize,
    ) -> AppResult<bool> {
        let remaining = total_limit - all_activities.len();
        let current_page_limit = remaining.min(activities_per_page);
        let current_offset = start_offset + (page_index * activities_per_page);

        let endpoint = Self::build_activities_endpoint(current_offset, current_page_limit);
        info!(
            "Fetching page {} of {} - endpoint: {}",
            page_index + 1,
            pages_needed,
            endpoint
        );

        let garmin_activities = self
            .api_request::<Vec<GarminActivityResponse>>(&endpoint)
            .await?;

        info!(
            "Page {} returned {} activities",
            page_index + 1,
            garmin_activities.len()
        );

        Self::add_converted_activities(all_activities, garmin_activities, total_limit)?;

        Ok(Self::should_stop_pagination(
            all_activities.len(),
            page_index,
            activities_per_page,
            total_limit,
        ))
    }

    async fn get_activities_multi_page(
        &self,
        total_limit: usize,
        start_offset: usize,
    ) -> AppResult<Vec<Activity>> {
        let mut all_activities = Vec::with_capacity(total_limit);
        let activities_per_page = api_provider_limits::garmin::MAX_ACTIVITIES_PER_REQUEST;
        let pages_needed = total_limit.div_ceil(activities_per_page);

        info!(
            "Multi-page request - total_limit: {}, pages_needed: {}, start_offset: {}",
            total_limit, pages_needed, start_offset
        );

        for page_index in 0..pages_needed {
            let should_stop = self
                .fetch_activities_page(
                    &mut all_activities,
                    page_index,
                    pages_needed,
                    start_offset,
                    activities_per_page,
                    total_limit,
                )
                .await?;

            if should_stop {
                break;
            }
        }

        info!(
            "Multi-page fetch completed - requested: {}, retrieved: {}",
            total_limit,
            all_activities.len()
        );

        Ok(all_activities)
    }
}

// ============================================================================
// Provider Factory
// ============================================================================

use super::core::ProviderFactory;

/// Factory for creating Garmin provider instances
pub struct GarminProviderFactory;

impl ProviderFactory for GarminProviderFactory {
    fn create(&self, config: ProviderConfig) -> Box<dyn FitnessProvider> {
        Box::new(GarminProvider::with_config(config))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[oauth_providers::GARMIN]
    }
}
