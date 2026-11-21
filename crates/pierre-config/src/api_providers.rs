// ABOUTME: External API provider configuration types for fitness platforms
// ABOUTME: Handles Strava, Garmin API settings and external service configurations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::api_provider_limits::{garmin, strava};
use pierre_core::constants::rate_limits::{STRAVA_RATE_LIMIT_15MIN, STRAVA_RATE_LIMIT_DAILY};
use serde::{Deserialize, Serialize};
use std::env;

/// External API services configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExternalServicesConfig {
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
    /// The origin of the Overpass instance route discovery queries
    /// (`OVERPASS_API_BASE`), a scheme and authority; `None` is the public
    /// instances, `overpass-api.de` first. Nothing in a deployment sets it:
    /// a test configuration names a refused or loopback origin, so no route
    /// search leaves the machine.
    pub overpass_api_base: Option<String>,
}

impl ExternalServicesConfig {
    /// Load external services configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            strava_api: StravaApiConfig::from_env(),
            garmin_api: GarminApiConfig::from_env(),
            sciotte: SciotteServiceConfig::from_env(),
            meta_graph_api_base: env::var("META_GRAPH_API_BASE")
                .ok()
                .filter(|base| !base.is_empty()),
            overpass_api_base: env::var("OVERPASS_API_BASE")
                .ok()
                .filter(|base| !base.trim_end_matches('/').is_empty()),
        }
    }
}

/// Where the dravr-sciotte scraper services are, read once at startup.
///
/// The server builds its sciotte clients from this; no read URL means no
/// scraper service is configured.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SciotteServiceConfig {
    /// The read services' base URLs, in order (`DRAVR_SCIOTTE_REMOTE_URL`, a
    /// comma-separated list); empty when the variable is unset or names none.
    ///
    /// Each read service runs one instance, and every call naming a session
    /// goes to the one its id picks, so the session meets one cookie jar
    /// (carnet#1504). The order is the services' identity: every API instance
    /// reads the same list. One URL is the single read service of local
    /// development and the test doubles.
    pub read_urls: Vec<String>,
    /// The audience identity tokens to the service are addressed to
    /// (`DRAVR_SCIOTTE_AUDIENCE`); `None` when unset or empty. A loopback
    /// service needs none.
    pub audience: Option<String>,
    /// The base URL of the service that runs sign-ins
    /// (`DRAVR_SCIOTTE_LOGIN_URL`); `None` when unset, empty or only slashes.
    ///
    /// A sign-in parks its browser in one instance's memory while the athlete
    /// fetches a code, so logins go to a service capped at one instance while
    /// reads go to `read_urls` (carnet#1504). It takes the same audience.
    /// `None` sends sign-ins to the first read service, the one-service shape
    /// local development and the test doubles run; so does a URL naming a
    /// read service itself.
    pub login_url: Option<String>,
}

impl SciotteServiceConfig {
    /// Load the scraper services' addresses from the environment
    #[must_use]
    pub fn from_env() -> Self {
        let read_urls =
            env::var("DRAVR_SCIOTTE_REMOTE_URL").map_or_default(|list| read_urls_of(&list));
        // A read service named again is that service, not a sign-in service
        // of its own: sign-ins would otherwise drop the session the reads use.
        let login_url = env::var("DRAVR_SCIOTTE_LOGIN_URL")
            .ok()
            .filter(|url| !url.trim_end_matches('/').is_empty())
            .filter(|url| {
                !read_urls
                    .iter()
                    .any(|read| read.trim_end_matches('/') == url.trim_end_matches('/'))
            });
        Self {
            read_urls,
            audience: env::var("DRAVR_SCIOTTE_AUDIENCE")
                .ok()
                .filter(|audience| !audience.is_empty()),
            login_url,
        }
    }
}

/// The read service URLs `list` names, in order: its comma-separated
/// entries, trimmed, without the empty or slash-only ones.
fn read_urls_of(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|url| !url.trim_end_matches('/').is_empty())
        .map(str::to_owned)
        .collect()
}

/// Strava request budgets and page sizes.
///
/// Strava's endpoints are not here: the provider registry holds them, with
/// the deployment's `PIERRE_STRAVA_*_URL` overrides applied
/// (`OAuthConfig::endpoints`), and every Strava call — the API, token
/// exchanges, revocation — goes where it says.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StravaApiConfig {
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

/// Garmin page sizes and request pacing.
///
/// Garmin's endpoints are not here: the provider registry holds them, with
/// the deployment's `PIERRE_GARMIN_*_URL` overrides applied
/// (`OAuthConfig::endpoints`), and every Garmin call — the API, token
/// exchanges, deregistration, the push callback's origin check — goes where
/// it says.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GarminApiConfig {
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
