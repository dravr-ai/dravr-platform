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
use crate::activity_paging::pages_for;
use crate::constants::oauth::{
    GARMIN_API_BASE_URL, GARMIN_AUTH_URL, GARMIN_DEREGISTRATION_URL, GARMIN_TOKEN_URL,
};
use crate::constants::{api_provider_limits, oauth_providers};
use crate::errors::{AppError, AppResult};
use crate::http_client::{shared_client, SharedHttpClient};
use crate::models::refresh_due;
use crate::models::StatsCoverage;
use crate::models::{
    activity::{Lap, Split},
    Activity, ActivityBuilder, Athlete, SportType, Stats,
};
use crate::pagination::{CursorPage, PaginationParams};
use crate::request_budget;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use secrecy::ExposeSecret;
use serde::Deserialize;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument};

mod wire;

use wire::{
    GarminActivityResponse, GarminLap, GarminLapsResponse, GarminListRow, GarminSplit,
    GarminSplitsResponse, GarminStatsResponse, GarminUserIdResponse,
};

/// Garmin Connect provider implementation
pub struct GarminProvider {
    config: ProviderConfig,
    credentials: RwLock<Option<OAuth2Credentials>>,
    client: SharedHttpClient,
    circuit_breaker: CircuitBreaker,
    token_refresh_callback: OnceLock<TokenRefreshCallback>,
}

impl GarminProvider {
    /// Create a new Garmin provider with default configuration
    #[must_use]
    pub fn new() -> Self {
        let config = ProviderConfig {
            name: oauth_providers::GARMIN.to_owned(),
            auth_url: GARMIN_AUTH_URL.to_owned(),
            token_url: GARMIN_TOKEN_URL.to_owned(),
            api_base_url: GARMIN_API_BASE_URL.to_owned(),
            revoke_url: Some(GARMIN_DEREGISTRATION_URL.to_owned()),
            // Garmin's scope is fixed server-side; its authorization takes none.
            default_scopes: Vec::new(),
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

        self.refresh_token_if_needed().await?;

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

        // The signing app's budget, which admits each request before it is sent.
        let budget = request_budget::carried_by(&self.credentials).await;
        let result = utils::api_request_with_retry(
            &self.client,
            &url,
            access_token.expose_secret(),
            oauth_providers::GARMIN,
            &retry_config,
            budget.as_ref(),
            utils::no_vendor_error,
        )
        .await;

        // Record success/failure for circuit breaker
        match &result {
            Ok(_) => self.circuit_breaker.record_success(),
            Err(e) => self.circuit_breaker.record_error(e),
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

        let start_date =
            garmin_start_date(summary.start_time_gmt.as_deref(), "summaryDTO.startTimeGMT")?;

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
        .normalized_power_opt(summary.normalized_power.map(utils::conversions::f32_to_u32))
        .calories_opt(summary.calories.map(utils::conversions::f64_to_u32))
        .temperature_opt(
            summary
                .average_temperature
                .map(utils::conversions::f64_to_f32),
        ))
    }

    fn convert_garmin_activity(activity: GarminActivityResponse) -> AppResult<Activity> {
        Ok(Self::garmin_activity_builder(activity)?.build())
    }

    /// Convert one flat list row into the canonical `Activity`.
    fn convert_garmin_list_row(row: GarminListRow) -> AppResult<Activity> {
        let type_key = row
            .activity_type
            .type_key
            .as_deref()
            .or(row.activity_type.parent_type_key.as_deref())
            .unwrap_or("unknown");
        let start_date = garmin_start_date(row.start_time_gmt.as_deref(), "startTimeGMT")?;

        Ok(ActivityBuilder::new(
            row.activity_id.to_string(),
            row.activity_name.unwrap_or_else(|| "Untitled".to_owned()),
            Self::parse_sport_type(type_key),
            start_date,
            row.duration.map_or(0, utils::conversions::f64_to_u64),
            oauth_providers::GARMIN,
        )
        .distance_meters_opt(row.distance)
        .elevation_gain_opt(row.elevation_gain)
        .average_speed_opt(row.average_speed)
        .max_speed_opt(row.max_speed)
        .average_heart_rate_opt(row.average_hr.map(utils::conversions::f32_to_u32))
        .max_heart_rate_opt(row.max_hr.map(utils::conversions::f32_to_u32))
        .average_cadence_opt(
            row.average_running_cadence_in_steps_per_minute
                .or(row.average_biking_cadence_in_rev_per_minute)
                .map(utils::conversions::f32_to_u32),
        )
        .average_power_opt(
            row.avg_power
                .or(row.average_power)
                .map(utils::conversions::f32_to_u32),
        )
        .max_power_opt(row.max_power.map(utils::conversions::f32_to_u32))
        .normalized_power_opt(row.norm_power.map(utils::conversions::f32_to_u32))
        .calories_opt(row.calories.map(utils::conversions::f64_to_u32))
        .build())
    }
}

/// The activity start from Garmin's `startTimeGMT`, which arrives as SQL-style
/// "YYYY-MM-DD HH:MM:SS.f" or RFC3339 depending on the surface. `field` names
/// the key in the error so a response missing it is diagnosable.
fn garmin_start_date(raw: Option<&str>, field: &str) -> AppResult<DateTime<Utc>> {
    let raw = raw.ok_or_else(|| AppError::internal(format!("Garmin activity missing {field}")))?;
    parse_garmin_timestamp(raw)
        .map_err(|e| AppError::internal(format!("Failed to parse activity start date: {e}")))
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
/// would only add noise to agent reasoning.
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
                creds.expires_at.is_some_and(refresh_due)
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

        // Garmin's refresh grant, as its descriptor declares it: the client
        // credentials in the form body and no other field. Garmin rotates the
        // refresh token on every refresh, so the new one is what the
        // write-back callback stores.
        let mut new_credentials = utils::refresh_oauth_token(
            &self.client,
            &utils::RefreshRequest::form_fields(
                oauth_providers::GARMIN,
                &self.config.token_url,
                &credentials.client_id,
                credentials.client_secret.expose_secret(),
                refresh_token.expose_secret(),
            ),
        )
        .await?;

        // Preserve original scopes
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

    /// The Garmin user behind the access token.
    ///
    /// The Health API serves the user's id at `GET user/id` and nothing else
    /// about them (no name, no picture), so the athlete is the id alone. It is
    /// what Garmin's push notifications name the user by, and what the OAuth
    /// callback stores as the connection's owner id.
    #[instrument(skip(self), fields(provider = "garmin", api_call = "get_athlete"))]
    async fn get_athlete(&self) -> AppResult<Athlete> {
        let user: GarminUserIdResponse = self.api_request("user/id").await?;

        Ok(Athlete {
            id: user.user_id,
            username: String::new(),
            firstname: None,
            lastname: None,
            profile_picture: None,
            provider: oauth_providers::GARMIN.to_owned(),
            sex: None,
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
            coverage: StatsCoverage::Lifetime,
            year_to_date: None,
        })
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

        let garmin_activities: Vec<GarminListRow> = self.api_request(&endpoint).await?;
        info!(
            "Received {} activities from single page",
            garmin_activities.len()
        );

        let mut activities = Vec::new();
        for activity in garmin_activities {
            activities.push(Self::convert_garmin_list_row(activity)?);
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
        garmin_activities: Vec<GarminListRow>,
        limit: usize,
    ) -> AppResult<()> {
        for activity in garmin_activities {
            if activities.len() >= limit {
                break;
            }
            activities.push(Self::convert_garmin_list_row(activity)?);
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

        let garmin_activities = self.api_request::<Vec<GarminListRow>>(&endpoint).await?;

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
        // Clamped to the ceiling every provider shares; previously this walk was
        // bounded only by whatever limit the caller passed in.
        let pages_needed = pages_for(total_limit, activities_per_page);

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
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(GarminProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &[oauth_providers::GARMIN]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A list row in the flat shape Garmin Connect serves and dravr-sciotte's
    /// `providers/garmin.toml` list extractor reads from live traffic: the type
    /// under `activityType`, the metrics at the top level, `startTimeGMT` in
    /// Garmin's SQL-style spelling, and the average power as `avgPower`.
    /// `maxPower` and `normPower` are the names public Garmin Connect clients
    /// read, not ones live traffic was seen to carry (LIMITATION on
    /// `GarminSummary::normalized_power`).
    fn list_row() -> serde_json::Value {
        json!({
            "activityId": 20_260_427_001_u64,
            "activityName": "Tempo ride",
            "activityType": { "typeKey": "cycling", "parentTypeKey": "cycling" },
            "startTimeGMT": "2026-04-27 14:25:46.0",
            "startTimeLocal": "2026-04-27 10:25:46",
            "distance": 42_000.0,
            "duration": 5_400.0,
            "elevationGain": 310.0,
            "averageSpeed": 7.8,
            "maxSpeed": 14.2,
            "averageHR": 148.0,
            "maxHR": 176.0,
            "calories": 980.0,
            "averageBikingCadenceInRevPerMinute": 88.0,
            "avgPower": 214.0,
            "maxPower": 640.0,
            "normPower": 231.0
        })
    }

    #[test]
    fn a_flat_list_row_converts_with_its_type_start_and_power() {
        let row: GarminListRow = serde_json::from_value(list_row()).unwrap();
        let activity = GarminProvider::convert_garmin_list_row(row).unwrap();

        assert_eq!(activity.id(), "20260427001");
        assert_eq!(activity.name(), "Tempo ride");
        assert_eq!(*activity.sport_type(), SportType::Ride);
        assert_eq!(
            activity.start_date().to_rfc3339(),
            "2026-04-27T14:25:46+00:00"
        );
        assert_eq!(activity.duration_seconds(), 5_400);
        assert_eq!(activity.average_heart_rate(), Some(148));
        assert_eq!(activity.average_cadence(), Some(88));
        assert_eq!(activity.average_power(), Some(214));
        assert_eq!(activity.max_power(), Some(640));
        assert_eq!(activity.normalized_power(), Some(231));
    }

    #[test]
    fn a_list_row_without_a_power_meter_states_no_power() {
        let mut value = list_row();
        let row_map = value.as_object_mut().unwrap();
        for key in ["avgPower", "maxPower", "normPower"] {
            row_map.remove(key);
        }
        let row: GarminListRow = serde_json::from_value(value).unwrap();
        let activity = GarminProvider::convert_garmin_list_row(row).unwrap();

        assert_eq!(activity.average_power(), None);
        assert_eq!(activity.max_power(), None);
        assert_eq!(activity.normalized_power(), None);
    }

    #[test]
    fn a_list_row_falls_back_to_the_parent_type_and_the_averagepower_spelling() {
        let mut value = list_row();
        value["activityType"] = json!({ "parentTypeKey": "running" });
        value["avgPower"] = serde_json::Value::Null;
        value["averagePower"] = json!(230.0);
        let row: GarminListRow = serde_json::from_value(value).unwrap();
        let activity = GarminProvider::convert_garmin_list_row(row).unwrap();

        assert_eq!(*activity.sport_type(), SportType::Run);
        assert_eq!(activity.average_power(), Some(230));
    }

    #[test]
    fn a_list_row_without_a_start_time_is_refused_naming_the_key() {
        let mut value = list_row();
        value.as_object_mut().unwrap().remove("startTimeGMT");
        let row: GarminListRow = serde_json::from_value(value).unwrap();
        let err = GarminProvider::convert_garmin_list_row(row).unwrap_err();
        assert!(err.to_string().contains("startTimeGMT"), "{err}");
    }

    /// The detail response nests the type as `activityTypeDTO` and the metrics
    /// under `summaryDTO`, where `startTimeGMT` is spelled with a capital GMT:
    /// `rename_all = "camelCase"` alone expects `startTimeGmt`, which Garmin
    /// never sends, so every detail conversion used to fail.
    ///
    /// `normalizedPower` is the key public Garmin Connect clients read; no live
    /// capture confirms it (see the LIMITATION on the field).
    fn detail() -> serde_json::Value {
        json!({
            "activityId": 20_260_427_001_u64,
            "activityName": "Tempo ride",
            "activityTypeDTO": { "typeKey": "cycling" },
            "summaryDTO": {
                "startTimeGMT": "2026-04-27 14:25:46.0",
                "duration": 5_400.0,
                "averageHR": 148.0,
                "averagePower": 214.0,
                "maxPower": 640.0,
                "normalizedPower": 231.0
            }
        })
    }

    #[test]
    fn a_detail_response_reads_its_start_time_and_keeps_every_power_figure() {
        let response: GarminActivityResponse = serde_json::from_value(detail()).unwrap();
        let activity = GarminProvider::convert_garmin_activity(response).unwrap();

        assert_eq!(
            activity.start_date().to_rfc3339(),
            "2026-04-27T14:25:46+00:00"
        );
        assert_eq!(activity.average_power(), Some(214));
        assert_eq!(activity.max_power(), Some(640));
        assert_eq!(activity.normalized_power(), Some(231));
    }

    #[test]
    fn a_detail_without_normalized_power_leaves_it_unset() {
        let mut value = detail();
        value["summaryDTO"]
            .as_object_mut()
            .unwrap()
            .remove("normalizedPower");
        let response: GarminActivityResponse = serde_json::from_value(value).unwrap();
        let activity = GarminProvider::convert_garmin_activity(response).unwrap();

        assert_eq!(activity.normalized_power(), None);
        assert_eq!(activity.average_power(), Some(214));
    }
}
