// ABOUTME: External API provider configuration types for fitness platforms
// ABOUTME: Handles Strava, Garmin API settings and external service configurations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::api_provider_limits::{garmin, strava};
use pierre_core::constants::oauth;
use pierre_core::constants::rate_limits::{STRAVA_RATE_LIMIT_15MIN, STRAVA_RATE_LIMIT_DAILY};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::env;

/// External API services configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExternalServicesConfig {
    /// Weather service configuration
    pub weather: WeatherServiceConfig,
    /// Strava API configuration
    pub strava_api: StravaApiConfig,
    /// Garmin API configuration
    pub garmin_api: GarminApiConfig,
    /// The dravr-sciotte scraper service every sciotte call goes through
    pub sciotte: SciotteServiceConfig,
    /// Meta's Graph API base (`META_GRAPH_API_BASE`), where a `WhatsApp`
    /// link asks the number it dials; `None` is the version dravr-canot's
    /// `WhatsApp` transport sends through.
    pub meta_graph_api_base: Option<String>,
}

impl ExternalServicesConfig {
    /// Load external services configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            weather: WeatherServiceConfig::from_env(),
            strava_api: StravaApiConfig::from_env(),
            garmin_api: GarminApiConfig::from_env(),
            sciotte: SciotteServiceConfig::from_env(),
            meta_graph_api_base: env::var("META_GRAPH_API_BASE")
                .ok()
                .filter(|base| !base.is_empty()),
        }
    }
}

/// Where the dravr-sciotte scraper service is, read once at startup.
///
/// The server builds its one sciotte client from this; a `None` URL means no
/// scraper service is configured.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SciotteServiceConfig {
    /// The service's base URL (`DRAVR_SCIOTTE_REMOTE_URL`); `None` when the
    /// variable is unset, empty or only slashes.
    pub remote_url: Option<String>,
    /// The audience identity tokens to the service are addressed to
    /// (`DRAVR_SCIOTTE_AUDIENCE`); `None` when unset or empty. A loopback
    /// service needs none.
    pub audience: Option<String>,
}

impl SciotteServiceConfig {
    /// Load the scraper service's address from the environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            remote_url: env::var("DRAVR_SCIOTTE_REMOTE_URL")
                .ok()
                .filter(|url| !url.trim_end_matches('/').is_empty()),
            audience: env::var("DRAVR_SCIOTTE_AUDIENCE")
                .ok()
                .filter(|audience| !audience.is_empty()),
        }
    }
}

/// Weather API service configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WeatherServiceConfig {
    /// `OpenWeather` API key; Debug prints a redaction and serialization omits it
    #[serde(skip_serializing)]
    pub api_key: Option<SecretString>,
    /// Weather service base URL
    pub base_url: String,
    /// Enable weather service
    pub enabled: bool,
}

impl WeatherServiceConfig {
    /// Load weather service configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            api_key: env::var("OPENWEATHER_API_KEY").ok().map(SecretString::from),
            base_url: env_var_or(
                "OPENWEATHER_BASE_URL",
                "https://api.openweathermap.org/data/2.5",
            ),
            enabled: env_var_or("WEATHER_SERVICE_ENABLED", "true")
                .parse()
                .unwrap_or(true),
        }
    }
}

/// Strava API configuration for OAuth and data fetching
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StravaApiConfig {
    /// Strava API base URL
    pub base_url: String,
    /// Strava auth URL
    pub auth_url: String,
    /// Strava token URL
    pub token_url: String,
    /// Strava token-revocation URL.
    ///
    /// The June-2026 `/oauth/revoke` endpoint (HTTP Basic auth with the
    /// client credentials, `token` form param) — the legacy
    /// `/oauth/deauthorize` retires 2027-06-01, so nothing may point at it.
    pub revoke_url: String,
    /// Default activities per page when fetching
    pub default_activities_per_page: usize,
    /// Maximum activities per API request
    pub max_activities_per_request: usize,
    /// Rate limit for 15-minute window
    pub rate_limit_15min: u32,
    /// Rate limit for daily window
    pub rate_limit_daily: u32,
}

impl StravaApiConfig {
    /// Load Strava API configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            base_url: env_var_or("STRAVA_API_BASE", "https://www.strava.com/api/v3"),
            auth_url: env_var_or("STRAVA_AUTH_URL", "https://www.strava.com/oauth/authorize"),
            token_url: env_var_or("STRAVA_TOKEN_URL", "https://www.strava.com/oauth/token"),
            revoke_url: env_var_or("STRAVA_REVOKE_URL", "https://www.strava.com/oauth/revoke"),
            default_activities_per_page: env::var("STRAVA_DEFAULT_ACTIVITIES_PER_PAGE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(strava::DEFAULT_ACTIVITIES_PER_PAGE),
            max_activities_per_request: env::var("STRAVA_MAX_ACTIVITIES_PER_REQUEST")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(strava::MAX_ACTIVITIES_PER_REQUEST),
            rate_limit_15min: env::var("STRAVA_RATE_LIMIT_15MIN")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(STRAVA_RATE_LIMIT_15MIN),
            rate_limit_daily: env::var("STRAVA_RATE_LIMIT_DAILY")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(STRAVA_RATE_LIMIT_DAILY),
        }
    }
}

/// Garmin API configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GarminApiConfig {
    /// Garmin API base URL
    pub base_url: String,
    /// Garmin auth URL
    pub auth_url: String,
    /// Garmin token URL
    pub token_url: String,
    /// Garmin Health API user-deregistration URL.
    ///
    /// Garmin has no token-revocation endpoint; withdrawing consent is a
    /// `DELETE` on the user's registration, authenticated with the user's
    /// access token as `Bearer`.
    pub revoke_url: String,
    /// Default activities per page when fetching
    pub default_activities_per_page: usize,
    /// Maximum activities per API request
    pub max_activities_per_request: usize,
    /// Recommended maximum requests per hour per user
    pub recommended_max_requests_per_hour: usize,
    /// Recommended minimum interval between login attempts (seconds)
    pub recommended_min_login_interval_secs: u64,
    /// Estimated rate limit block duration (seconds)
    pub estimated_rate_limit_block_duration_secs: u64,
}

impl GarminApiConfig {
    /// Load Garmin API configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            base_url: env_var_or("GARMIN_API_BASE", oauth::GARMIN_API_BASE_URL),
            auth_url: env_var_or("GARMIN_AUTH_URL", oauth::GARMIN_AUTH_URL),
            token_url: env_var_or("GARMIN_TOKEN_URL", oauth::GARMIN_TOKEN_URL),
            revoke_url: env_var_or("GARMIN_REVOKE_URL", oauth::GARMIN_DEREGISTRATION_URL),
            default_activities_per_page: env::var("GARMIN_DEFAULT_ACTIVITIES_PER_PAGE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(garmin::DEFAULT_ACTIVITIES_PER_PAGE),
            max_activities_per_request: env::var("GARMIN_MAX_ACTIVITIES_PER_REQUEST")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(garmin::MAX_ACTIVITIES_PER_REQUEST),
            recommended_max_requests_per_hour: env::var("GARMIN_MAX_REQUESTS_PER_HOUR")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(garmin::RECOMMENDED_MAX_REQUESTS_PER_HOUR),
            recommended_min_login_interval_secs: env::var("GARMIN_MIN_LOGIN_INTERVAL_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(garmin::RECOMMENDED_MIN_LOGIN_INTERVAL_SECS),
            estimated_rate_limit_block_duration_secs: env::var(
                "GARMIN_RATE_LIMIT_BLOCK_DURATION_SECS",
            )
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(garmin::ESTIMATED_RATE_LIMIT_BLOCK_DURATION_SECS),
        }
    }
}

/// Get environment variable or default value
fn env_var_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}
