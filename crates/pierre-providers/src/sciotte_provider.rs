// ABOUTME: Sciotte web scraping provider using dravr-sciotte as an in-process library
// ABOUTME: Launches headless Chrome for login, scrapes activities directly via ActivityScraper trait
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Sciotte Provider
//!
//! Implements `FitnessProvider` using `dravr-sciotte` as an in-process Cargo dependency.
//! Launches short-lived headless Chrome for credential-based login, then scrapes activity
//! data directly from fitness platform HTML pages (Strava, Garmin Connect, etc.).
//!
//! No HTTP sidecar needed — the scraper runs in Pierre's process.

use async_trait::async_trait;
use dravr_sciotte::cache::CachedScraper;
use dravr_sciotte::config::{CacheConfig, ScraperConfig};
use dravr_sciotte::models::{
    Activity as SciotteActivity, ActivityParams, AuthSession, Lap as SciotteLap,
    Split as SciotteSplit, SportType as SciotteSportType,
};
use dravr_sciotte::provider::ProviderConfig as SciotteProviderConfig;
use dravr_sciotte::scraper::ChromeScraper;
use dravr_sciotte::ActivityScraper;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use crate::errors::{AppError, AppResult};
use crate::models::{
    activity::{Lap, Split},
    Activity, ActivityBuilder, Athlete, PersonalRecord, SportType, Stats,
};
use crate::pagination::{CursorPage, PaginationParams};

/// Target fitness platform for the sciotte scraper
#[derive(Debug, Clone, Copy)]
pub enum SciotteTarget {
    /// Scrape activities from Strava (strava.com)
    Strava,
    /// Scrape activities from Garmin Connect (connect.garmin.com)
    Garmin,
}

/// Sciotte provider that uses the dravr-sciotte library directly (in-process)
pub struct SciotteProvider {
    config: ProviderConfig,
    scraper: Arc<CachedScraper<ChromeScraper>>,
    session: RwLock<Option<AuthSession>>,
    provider_name: &'static str,
}

impl SciotteProvider {
    fn new(config: ProviderConfig, target: SciotteTarget) -> Self {
        let scraper_config = ScraperConfig::default();
        let provider_config = match target {
            SciotteTarget::Strava => SciotteProviderConfig::strava_default(),
            SciotteTarget::Garmin => SciotteProviderConfig::garmin_default(),
        };
        let provider_name = match target {
            SciotteTarget::Strava => "sciotte",
            SciotteTarget::Garmin => "sciotte_garmin",
        };
        let chrome_scraper = ChromeScraper::new(scraper_config, provider_config);
        let cached = CachedScraper::new(chrome_scraper, &CacheConfig::default());

        info!(target = ?target, "Sciotte provider initialized (in-process)");

        Self {
            config,
            scraper: Arc::new(cached),
            session: RwLock::new(None),
            provider_name,
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
        let session = self.session.read().await;
        match session.as_ref() {
            Some(s) => self.scraper.is_authenticated(s).await,
            None => false,
        }
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        // Sciotte sessions don't refresh — user must re-login when expired
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        let session = self.session.read().await;
        let session = session
            .as_ref()
            .ok_or_else(|| AppError::invalid_input("No sciotte session — please connect first"))?;

        let profile =
            self.scraper.get_athlete(session).await.map_err(|e| {
                AppError::internal(format!("Failed to scrape athlete profile: {e}"))
            })?;

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
            .ok_or_else(|| AppError::invalid_input("No sciotte session — please connect first"))?;

        let limit = params.limit.unwrap_or(20);
        // Trait contract mirrors Strava/Fitbit: return list-page summary only.
        // Detail-page enrichment (HR streams, laps, segments) is an N+1 roundtrip
        // through the headless browser and unacceptable on interactive paths like
        // chat orchestration and group snapshots. Callers that need full detail
        // must invoke the Sciotte scraper directly with `enrich_details: true`.
        let sciotte_params = ActivityParams {
            limit: Some(limit as u32),
            enrich_details: false,
            ..Default::default()
        };

        debug!(limit, "Fetching activities from sciotte (in-process)");

        let sciotte_activities = self
            .scraper
            .get_activities(session, &sciotte_params)
            .await
            .map_err(|e| AppError::internal(format!("Sciotte scraping failed: {e}")))?;

        let activities: Vec<Activity> = sciotte_activities.iter().map(convert_activity).collect();

        info!(count = activities.len(), "Activities fetched from sciotte");
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
            .ok_or_else(|| AppError::invalid_input("No sciotte session — please connect first"))?;

        let sciotte_activity = self
            .scraper
            .get_activity(session, id)
            .await
            .map_err(|e| AppError::internal(format!("Sciotte activity fetch failed: {e}")))?;

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
        })
    }

    async fn get_personal_records(&self) -> AppResult<Vec<PersonalRecord>> {
        Ok(vec![])
    }

    async fn disconnect(&self) -> AppResult<()> {
        *self.session.write().await = None;
        info!("Sciotte provider disconnected");
        Ok(())
    }
}

/// Factory for creating `SciotteProvider` instances
/// Factory for Strava — Sciotte provider
pub struct SciotteProviderFactory;

impl ProviderFactory for SciotteProviderFactory {
    fn create(&self, config: ProviderConfig) -> Box<dyn FitnessProvider> {
        Box::new(SciotteProvider::new(config, SciotteTarget::Strava))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte"]
    }
}

/// Factory for Garmin Connect — Sciotte provider
pub struct SciotteGarminProviderFactory;

impl ProviderFactory for SciotteGarminProviderFactory {
    fn create(&self, config: ProviderConfig) -> Box<dyn FitnessProvider> {
        Box::new(SciotteProvider::new(config, SciotteTarget::Garmin))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte_garmin"]
    }
}
