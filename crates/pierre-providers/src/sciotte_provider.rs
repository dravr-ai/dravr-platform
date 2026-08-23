// ABOUTME: Sciotte provider — forwards every scrape to the dedicated dravr-sciotte service over HTTP
// ABOUTME: Holds the platform's AuthSession and calls the remote scraper; no in-process Chrome (ADR-021)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Sciotte Provider
//!
//! Implements `FitnessProvider` by delegating to the dedicated `dravr-sciotte`
//! scraper service over HTTP (ADR-021). Since the Phase 4 cutover the platform
//! runs no headless Chrome: this provider holds the platform-held [`AuthSession`]
//! and forwards it to the service on each fetch, which scrapes the fitness
//! platform (Strava, Garmin Connect, etc.) and returns activities.

use async_trait::async_trait;
use chrono::Utc;
use dravr_sciotte::models::{
    Activity as SciotteActivity, AuthSession, Lap as SciotteLap, Split as SciotteSplit,
    SportType as SciotteSportType,
};
use std::env;
use tokio::sync::RwLock;
use tracing::info;

use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use crate::errors::{AppError, AppResult};
use crate::models::{
    activity::{Lap, Split},
    Activity, ActivityBuilder, Athlete, PersonalRecord, SportType, Stats,
};
use crate::pagination::{CursorPage, PaginationParams};
use crate::sciotte_remote::{RemoteActivityQuery, RemoteSciotteClient};

/// Target fitness platform for the sciotte scraper
#[derive(Debug, Clone, Copy)]
pub enum SciotteTarget {
    /// Scrape activities from Strava (strava.com)
    Strava,
    /// Scrape activities from Garmin Connect (connect.garmin.com)
    Garmin,
}

impl SciotteTarget {
    /// Parse the API-level target string (e.g. "garmin", "strava") into a
    /// [`SciotteTarget`]. Unknown values fall back to [`Self::Strava`] to
    /// preserve the long-standing default behaviour of hosted login.
    #[must_use]
    pub fn from_target_param(target: &str) -> Self {
        match target {
            "garmin" => Self::Garmin,
            _ => Self::Strava,
        }
    }

    /// Pierre provider name attached to OAuth/Sciotte rows for this target.
    #[must_use]
    pub const fn provider_name(self) -> &'static str {
        match self {
            Self::Strava => "sciotte",
            Self::Garmin => "sciotte_garmin",
        }
    }

    /// Inverse of [`Self::provider_name`]: the target for a Pierre backend
    /// name (`"sciotte"`, `"sciotte_garmin"`). Unknown values fall back to
    /// [`Self::Strava`], mirroring [`Self::from_target_param`].
    #[must_use]
    pub fn from_backend_name(backend: &str) -> Self {
        match backend {
            "sciotte_garmin" => Self::Garmin,
            _ => Self::Strava,
        }
    }

    /// Provider name the dravr-sciotte scraper service uses for this target
    /// (`"garmin"`, `"strava"`) — sent on remote login/import so the
    /// multi-provider service routes to the right scraper (ADR-021).
    #[must_use]
    pub const fn scraper_provider_name(self) -> &'static str {
        match self {
            Self::Strava => "strava",
            Self::Garmin => "garmin",
        }
    }
}

/// Sciotte provider — a thin session-holder over the dedicated scraper service.
///
/// Routes every scrape to the dedicated dravr-sciotte service ([[ADR-021]]).
/// Since the Phase 4 cutover it holds no in-process Chrome; it keeps the
/// platform-held [`AuthSession`] and forwards it to the service on each fetch.
pub struct SciotteProvider {
    config: ProviderConfig,
    session: RwLock<Option<AuthSession>>,
    provider_name: &'static str,
}

impl SciotteProvider {
    fn new(config: ProviderConfig, target: SciotteTarget) -> Self {
        let provider_name = target.provider_name();
        info!(target = ?target, "Sciotte provider initialized (remote service)");
        Self {
            config,
            session: RwLock::new(None),
            provider_name,
        }
    }

    /// Build the `provider_auth_required` error used for the "no session at
    /// all" branch, so the chat pipeline can mint a hosted-login URL and
    /// short-circuit the LLM with an actionable reply.
    fn auth_required_no_session(&self) -> AppError {
        AppError::provider_auth_required(self.provider_name)
    }

    /// Re-tag an auth-shaped remote-service error with this backend's
    /// provider name.
    ///
    /// The remote client only knows the generic `sciotte` slug — it can't
    /// tell which backend (`sciotte` vs `sciotte_garmin`) owns the session,
    /// and the reconnect link minted downstream branches on that name to
    /// pick the hosted-login target. Without the re-tag, a dead
    /// `sciotte_garmin` session would send the athlete to a Strava login.
    fn tag_remote_auth(&self, e: AppError) -> AppError {
        if e.provider_auth_required_provider().is_some() {
            AppError::provider_auth_required(self.provider_name)
        } else {
            e
        }
    }
}

/// Direct sciotte → cageux `SportType` conversion. Both enums share variant
/// names (sciotte mirrors cageux's canonical set), so a 1:1 match is
/// bulletproof; the previous round-trip via `display_name()` →
/// `from_internal_string()` was lossy because `display_name` returns
/// human-readable Title-Case-with-spaces ("Cross-Country Skiing") while
/// `from_internal_string` expects the `snake_case` serde form
/// ("`cross_country_skiing`"), so every non-trivial variant fell through to
/// `Other(<display_name>)` and broke filter / serialization.
fn convert_sport_type(s: &SciotteSportType) -> SportType {
    match s {
        SciotteSportType::Run => SportType::Run,
        SciotteSportType::Ride => SportType::Ride,
        SciotteSportType::Swim => SportType::Swim,
        SciotteSportType::Walk => SportType::Walk,
        SciotteSportType::Hike => SportType::Hike,
        SciotteSportType::VirtualRide => SportType::VirtualRide,
        SciotteSportType::VirtualRun => SportType::VirtualRun,
        SciotteSportType::Workout => SportType::Workout,
        SciotteSportType::Yoga => SportType::Yoga,
        SciotteSportType::EbikeRide => SportType::EbikeRide,
        SciotteSportType::MountainBike => SportType::MountainBike,
        SciotteSportType::GravelRide => SportType::GravelRide,
        SciotteSportType::CrossCountrySkiing => SportType::CrossCountrySkiing,
        SciotteSportType::AlpineSkiing => SportType::AlpineSkiing,
        SciotteSportType::Snowboarding => SportType::Snowboarding,
        SciotteSportType::Snowshoe => SportType::Snowshoe,
        SciotteSportType::IceSkating => SportType::IceSkating,
        SciotteSportType::BackcountrySkiing => SportType::BackcountrySkiing,
        SciotteSportType::Kayaking => SportType::Kayaking,
        SciotteSportType::Canoeing => SportType::Canoeing,
        SciotteSportType::Rowing => SportType::Rowing,
        SciotteSportType::Paddleboarding => SportType::Paddleboarding,
        SciotteSportType::Surfing => SportType::Surfing,
        SciotteSportType::Kitesurfing => SportType::Kitesurfing,
        SciotteSportType::StrengthTraining => SportType::StrengthTraining,
        SciotteSportType::Crossfit => SportType::Crossfit,
        SciotteSportType::Pilates => SportType::Pilates,
        SciotteSportType::RockClimbing => SportType::RockClimbing,
        SciotteSportType::TrailRunning => SportType::TrailRunning,
        SciotteSportType::Soccer => SportType::Soccer,
        SciotteSportType::Basketball => SportType::Basketball,
        SciotteSportType::Tennis => SportType::Tennis,
        SciotteSportType::Golf => SportType::Golf,
        SciotteSportType::Skateboarding => SportType::Skateboarding,
        SciotteSportType::InlineSkating => SportType::InlineSkating,
        SciotteSportType::Other(s) => SportType::Other(s.clone()),
    }
}

/// Convert a sciotte `Activity` to a Pierre `Activity`
fn convert_activity(sciotte: &SciotteActivity) -> Activity {
    let sport_type = convert_sport_type(&sciotte.sport_type);

    let splits = sciotte
        .splits
        .as_ref()
        .map(|v| v.iter().map(convert_split).collect());
    let laps = sciotte
        .laps
        .as_ref()
        .map(|v| v.iter().map(convert_lap).collect());

    ActivityBuilder::new(
        &sciotte.id,
        &sciotte.name,
        sport_type,
        sciotte.start_date,
        sciotte.duration_seconds,
        "sciotte",
    )
    .distance_meters_opt(sciotte.distance_meters)
    .elevation_gain_opt(sciotte.elevation_gain)
    .average_heart_rate_opt(sciotte.average_heart_rate)
    .max_heart_rate_opt(sciotte.max_heart_rate)
    .average_speed_opt(sciotte.average_speed)
    .max_speed_opt(sciotte.max_speed)
    .calories_opt(sciotte.calories)
    .average_power_opt(sciotte.average_power)
    .max_power_opt(sciotte.max_power)
    .average_cadence_opt(sciotte.average_cadence)
    .suffer_score_opt(sciotte.suffer_score)
    .temperature_opt(sciotte.temperature)
    .humidity_opt(sciotte.humidity)
    .wind_speed_opt(sciotte.wind_speed)
    .city_opt(sciotte.city.clone())
    .region_opt(sciotte.region.clone())
    .country_opt(sciotte.country.clone())
    .start_latitude_opt(sciotte.start_latitude)
    .start_longitude_opt(sciotte.start_longitude)
    .splits_opt(splits)
    .laps_opt(laps)
    .build()
}

/// Translate sciotte's [`SciotteSplit`] into cageux's [`Split`] — same
/// field set, re-emitted under cageux's canonical names so the chat
/// pipeline treats OAuth-Strava splits and scraper-Strava splits
/// identically.
fn convert_split(s: &SciotteSplit) -> Split {
    Split {
        index: s.index,
        distance_meters: s.distance_meters,
        elapsed_time_seconds: s.elapsed_time_seconds,
        moving_time_seconds: s.moving_time_seconds,
        elevation_difference_meters: s.elevation_difference_meters,
        average_speed_mps: s.average_speed_mps,
        average_heart_rate: s.average_heart_rate,
        pace_zone: s.pace_zone,
    }
}

/// Translate sciotte's [`SciotteLap`] into cageux's [`Lap`].
fn convert_lap(l: &SciotteLap) -> Lap {
    Lap {
        id: l.id.clone(),
        index: l.index,
        distance_meters: l.distance_meters,
        elapsed_time_seconds: l.elapsed_time_seconds,
        moving_time_seconds: l.moving_time_seconds,
        elevation_gain_meters: l.elevation_gain_meters,
        average_speed_mps: l.average_speed_mps,
        max_speed_mps: l.max_speed_mps,
        average_heart_rate: l.average_heart_rate,
        max_heart_rate: l.max_heart_rate,
        average_cadence: l.average_cadence,
        average_power: l.average_power,
    }
}

#[async_trait]
impl FitnessProvider for SciotteProvider {
    fn name(&self) -> &'static str {
        self.provider_name
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    /// Restore a session from stored cookies (passed as serialized JSON in `access_token`)
    async fn set_credentials(&self, credentials: OAuth2Credentials) -> AppResult<()> {
        let session_json = credentials
            .access_token
            .ok_or_else(|| AppError::invalid_input("Missing session data for sciotte provider"))?;
        if session_json.is_empty() {
            return Err(AppError::invalid_input(
                "Empty session data for sciotte provider",
            ));
        }

        let session: AuthSession = serde_json::from_str(&session_json).map_err(|e| {
            AppError::internal(format!("Failed to deserialize sciotte session: {e}"))
        })?;

        *self.session.write().await = Some(session);
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        // A held, non-expired session means "connected". Without an in-pod
        // scraper (Phase 4 cutover) we can't probe cookies, but a stored expiry
        // already in the past is certainly dead — report it honestly so callers
        // never treat an expired session as live. A `None` expiry is "unknown,
        // assume usable"; the dedicated service re-auths on the next scrape
        // import if the cookies turn out stale (ADR-021).
        self.session
            .read()
            .await
            .as_ref()
            .is_some_and(|s| s.expires_at.is_none_or(|exp| exp > Utc::now()))
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        // Sciotte sessions don't refresh — user must re-login when expired
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        let session = self.session.read().await;
        let session = session
            .as_ref()
            .ok_or_else(|| self.auth_required_no_session())?;

        // ADR-021: scrape on the dedicated service (there is no in-process
        // fallback since the Phase 4 cutover). Import the platform-held session
        // — re-hydrates the service after a scale-to-zero / redeploy — then fetch.
        let remote = RemoteSciotteClient::require_from_env()?;
        remote
            .import_session(
                session,
                SciotteTarget::from_backend_name(self.provider_name).scraper_provider_name(),
            )
            .await?;
        let profile = remote
            .get_athlete(&session.session_id)
            .await
            .map_err(|e| self.tag_remote_auth(e))?;
        let display_name = profile
            .display_name
            .clone()
            .unwrap_or_else(|| "Sciotte User".to_owned());
        Ok(Athlete {
            id: "sciotte".to_owned(),
            username: display_name,
            firstname: profile.firstname,
            lastname: profile.lastname,
            profile_picture: profile.profile_picture_url,
            provider: "sciotte".to_owned(),
        })
    }

    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        let session = self.session.read().await;
        let session = session
            .as_ref()
            .ok_or_else(|| self.auth_required_no_session())?;

        let limit = params.limit.unwrap_or(20);
        // Detail-page enrichment (HR streams, laps, segments — and the real UTC
        // start_date, absent from the date-only list page) is an N+1 roundtrip
        // through the headless browser, so it stays OFF by default to keep
        // interactive paths (chat, group snapshots) fast. It's opt-in per
        // deployment via PIERRE_SCIOTTE_ENRICH_DETAILS=true (dev sets it) when
        // correct start times matter more than the extra latency on the bounded
        // recent set this fetch returns.
        let enrich_details =
            env::var("PIERRE_SCIOTTE_ENRICH_DETAILS").is_ok_and(|v| v == "true" || v == "1");

        // ADR-021: fetch on the dedicated service (no in-process fallback since
        // the Phase 4 cutover). Import the platform-held session — re-hydrates
        // the service after a scale-to-zero / redeploy — then scrape over HTTP;
        // convert_activity keeps the returned shape identical for every caller.
        // before/after pass through as epoch seconds so the scrape bounds the
        // fetch by date, matching the API providers (Strava/Whoop).
        let remote = RemoteSciotteClient::require_from_env()?;
        let query = RemoteActivityQuery {
            limit: Some(limit as u32),
            after_epoch: params.after,
            before_epoch: params.before,
            sport_type: None,
            enrich_details,
        };
        remote
            .import_session(
                session,
                SciotteTarget::from_backend_name(self.provider_name).scraper_provider_name(),
            )
            .await?;
        let sciotte_activities = remote
            .get_activities(&session.session_id, &query)
            .await
            .map_err(|e| self.tag_remote_auth(e))?;
        let activities: Vec<Activity> = sciotte_activities.iter().map(convert_activity).collect();
        info!(
            count = activities.len(),
            "Sciotte scrape completed (remote service)"
        );
        Ok(activities)
    }

    async fn get_activities_cursor(
        &self,
        params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        let query_params = ActivityQueryParams::with_pagination(Some(params.limit), None);
        let activities = self.get_activities_with_params(&query_params).await?;
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
        let session = self.session.read().await;
        let session = session
            .as_ref()
            .ok_or_else(|| self.auth_required_no_session())?;

        // ADR-021: fetch the single activity's detail on the dedicated service.
        let remote = RemoteSciotteClient::require_from_env()?;
        remote
            .import_session(
                session,
                SciotteTarget::from_backend_name(self.provider_name).scraper_provider_name(),
            )
            .await?;
        let sciotte_activity = remote
            .get_activity(&session.session_id, id)
            .await
            .map_err(|e| self.tag_remote_auth(e))?;
        Ok(convert_activity(&sciotte_activity))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        let activities = self
            .get_activities_with_params(&ActivityQueryParams::with_pagination(Some(100), None))
            .await?;

        let total_distance: f64 = activities
            .iter()
            .filter_map(Activity::distance_meters)
            .sum();
        let total_duration: u64 = activities.iter().map(Activity::duration_seconds).sum();
        let total_elevation: f64 = activities.iter().filter_map(Activity::elevation_gain).sum();

        Ok(Stats {
            total_activities: activities.len() as u64,
            total_distance,
            total_duration,
            total_elevation_gain: total_elevation,
            year_to_date: None,
        })
    }

    async fn get_personal_records(&self) -> AppResult<Vec<PersonalRecord>> {
        Ok(vec![])
    }
}

/// Factory for creating `SciotteProvider` instances
/// Factory for Strava — Sciotte provider
pub struct SciotteProviderFactory;

impl ProviderFactory for SciotteProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::Strava,
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte"]
    }
}

/// Factory for Garmin Connect — Sciotte provider
pub struct SciotteGarminProviderFactory;

impl ProviderFactory for SciotteGarminProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::Garmin,
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte_garmin"]
    }
}
