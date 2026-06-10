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
use dravr_sciotte::error::ScraperError;
use dravr_sciotte::models::{
    Activity as SciotteActivity, ActivityParams, AuthSession, Lap as SciotteLap,
    Split as SciotteSplit, SportType as SciotteSportType,
};
use dravr_sciotte::provider::ProviderConfig as SciotteProviderConfig;
use dravr_sciotte::scraper::ChromeScraper;
use dravr_sciotte::ActivityScraper;
use embacle::types::LlmProvider;
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, PoisonError};
use tokio::sync::{Mutex as TokioMutex, OwnedMutexGuard, RwLock};
use tracing::{debug, info, warn};

use crate::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use crate::errors::{AppError, AppResult};
use crate::models::{
    activity::{Lap, Split},
    Activity, ActivityBuilder, Athlete, PersonalRecord, SportType, Stats,
};
use crate::pagination::{CursorPage, PaginationParams};
use crate::sciotte_limiter::{global_limiter, LimiterError, ScrapePermit};

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

    /// Build a fresh [`CachedScraper`] configured for this target. Used by
    /// the hosted-login route to drive the headless-Chrome flow and by
    /// [`SciotteProvider::new`] to back the runtime fetch path — both must
    /// use the same scraper configuration so a cached login from one path
    /// stays usable by the other.
    #[must_use]
    pub fn build_scraper(self) -> CachedScraper<ChromeScraper> {
        let provider_config = match self {
            Self::Strava => SciotteProviderConfig::strava_default(),
            Self::Garmin => SciotteProviderConfig::garmin_default(),
        };
        let chrome_scraper = ChromeScraper::new(ScraperConfig::default(), provider_config);
        CachedScraper::new(chrome_scraper, &CacheConfig::default())
    }

    /// Build a scraper with an LLM provider attached for vision-based login.
    ///
    /// Identical to [`Self::build_scraper`] but injects `llm` via
    /// `ChromeScraper::with_llm`, enabling the vision login path. Under the
    /// `Hybrid` `DRAVR_SCIOTTE_LOGIN_MODE` the selector path still runs first
    /// and vision only fires when selectors fail; under `Vision` mode every
    /// login uses the LLM. The login route supplies the shared Copilot-headless
    /// provider so a Strava DOM change degrades to screenshot reasoning instead
    /// of a hard failure.
    #[must_use]
    pub fn build_scraper_with_llm(self, llm: Arc<dyn LlmProvider>) -> CachedScraper<ChromeScraper> {
        let provider_config = match self {
            Self::Strava => SciotteProviderConfig::strava_default(),
            Self::Garmin => SciotteProviderConfig::garmin_default(),
        };
        let chrome_scraper = ChromeScraper::new(ScraperConfig::default(), provider_config)
            .with_llm(Arc::new(EmbacleVisionAdapter(llm)));
        CachedScraper::new(chrome_scraper, &CacheConfig::default())
    }
}

/// Adapts an embacle [`LlmProvider`] to sciotte's `VisionModel` trait so the
/// vision-login path can use the platform's Copilot-headless provider without
/// sciotte itself depending on embacle. The embacle `ChatRequest` build lives
/// here, on the consumer side of the boundary.
struct EmbacleVisionAdapter(Arc<dyn LlmProvider>);

#[async_trait::async_trait]
impl dravr_sciotte::VisionModel for EmbacleVisionAdapter {
    async fn analyze_screenshot(
        &self,
        prompt: &str,
        screenshot_png_b64: &str,
    ) -> Result<String, dravr_sciotte::VisionModelError> {
        use embacle::types::{ChatMessage, ChatRequest, ImagePart};

        let image = ImagePart::new(screenshot_png_b64.to_owned(), "image/png")
            .map_err(|e| format!("invalid image part: {e}"))?;
        let message = ChatMessage::user_with_images(prompt.to_owned(), vec![image]);
        let request = ChatRequest {
            messages: vec![message],
            model: None,
            temperature: Some(0.0),
            max_tokens: Some(4096),
            stream: false,
            tools: None,
            tool_choice: None,
            top_p: None,
            stop: None,
            response_format: None,
            turn_id: None,
            mcp_servers: Vec::new(),
        };
        let response = self
            .0
            .complete(&request)
            .await
            .map_err(|e| format!("vision LLM call failed: {e}"))?;
        Ok(response.content)
    }
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
        let cached = target.build_scraper();
        let provider_name = target.provider_name();

        info!(target = ?target, "Sciotte provider initialized (in-process)");

        Self {
            config,
            scraper: Arc::new(cached),
            session: RwLock::new(None),
            provider_name,
        }
    }

    /// Translate a `dravr_sciotte::ScraperError` into an `AppError`. The
    /// `SessionExpired` variant maps to `provider_auth_required` so the chat
    /// pipeline can mint a hosted-login URL and short-circuit the LLM with an
    /// actionable reply; everything else stays an internal error tagged with
    /// the call-site context.
    fn map_scraper_error(&self, err: &ScraperError, context: &str) -> AppError {
        if matches!(err, ScraperError::SessionExpired { .. }) {
            return AppError::provider_auth_required(self.provider_name);
        }
        AppError::internal(format!("{context}: {err}"))
    }

    /// Build the same `provider_auth_required` error the runtime emits when a
    /// scrape lands on the SSO page, used for the "no session at all" branch.
    fn auth_required_no_session(&self) -> AppError {
        AppError::provider_auth_required(self.provider_name)
    }
}

/// Serializes Chrome profile access, keyed by `AuthSession.session_id`.
///
/// sciotte derives the on-disk Chrome profile directory from the session id,
/// and `launch_browser` documents that the *caller* must serialize concurrent
/// launches against the same id — Chrome holds a `SingletonLock` on its
/// profile directory and a second launch aborts with "Failed to create
/// SingletonLock: File exists". Each `SciotteProvider` instance launches its
/// own Chrome, so two overlapping scrapes for the same user (e.g. a background
/// activity-cache revalidation racing an LLM `get_activities` call) crash
/// instantly without this serialization.
///
/// Production code shares one registry via [`Self::global`]; tests construct
/// isolated instances. Map entries are never evicted — the map is bounded by
/// the number of distinct sciotte sessions seen by the process.
pub struct ProfileLockRegistry {
    locks: StdMutex<HashMap<String, Arc<TokioMutex<()>>>>,
}

impl ProfileLockRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            locks: StdMutex::new(HashMap::new()),
        }
    }

    /// The process-global registry shared by every `SciotteProvider` instance.
    pub fn global() -> &'static Self {
        static GLOBAL: LazyLock<ProfileLockRegistry> = LazyLock::new(ProfileLockRegistry::new);
        &GLOBAL
    }

    /// Await exclusive use of a session's Chrome profile directory.
    ///
    /// The returned guard must be held for the entire scrape (browser launch
    /// through close) so a concurrent scrape for the same session waits for the
    /// profile to free up instead of colliding on Chrome's `SingletonLock`.
    /// Distinct session ids never block each other.
    pub async fn acquire(&self, session_id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            // Recover from poisoning: the inner map stays consistent even if a
            // holder panicked, and refusing all future scrapes would be worse.
            let mut map = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(map.entry(session_id.to_owned()).or_default())
        };
        lock.lock_owned().await
    }
}

impl Default for ProfileLockRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Acquire a permit from the process-global Sciotte limiter so every
/// Chrome-backed scrape on the data-fetch path counts against the same pod
/// concurrency budget the login routes use. Returns `None` when no limiter is
/// registered — a unit test may construct the provider without the server
/// bootstrap that calls `set_global_limiter` — in which case the scrape
/// proceeds ungated rather than failing.
async fn acquire_scrape_permit() -> AppResult<Option<ScrapePermit>> {
    match global_limiter() {
        Some(limiter) => limiter
            .acquire()
            .await
            .map(Some)
            .map_err(|err| limiter_rejection(&err)),
        None => Ok(None),
    }
}

/// Map a backpressure rejection to a sanitized `ResourceUnavailable` error so
/// the client sees a generic "temporarily unavailable" message, never the
/// internal Chrome/queue details.
fn limiter_rejection(err: &LimiterError) -> AppError {
    warn!(
        reason = %err,
        retry_after_secs = err.retry_after_secs(),
        "Sciotte scrape shed by backpressure limiter"
    );
    AppError::resource_unavailable(format!("Sciotte scrape shed by limiter: {err}"))
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
            .ok_or_else(|| self.auth_required_no_session())?;

        // Profile lock before the global permit: waiting for the profile to
        // free up must not pin a scarce Chrome-concurrency slot.
        let _profile_lock = ProfileLockRegistry::global()
            .acquire(&session.session_id)
            .await;
        let _permit = acquire_scrape_permit().await?;
        let profile = self
            .scraper
            .get_athlete(session)
            .await
            .map_err(|e| self.map_scraper_error(&e, "Failed to scrape athlete profile"))?;

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
        let sciotte_params = ActivityParams {
            limit: Some(limit as u32),
            enrich_details,
            ..Default::default()
        };

        debug!(limit, "Fetching activities from sciotte (in-process)");

        // Profile lock before the global permit: waiting for the profile to
        // free up must not pin a scarce Chrome-concurrency slot.
        let _profile_lock = ProfileLockRegistry::global()
            .acquire(&session.session_id)
            .await;
        let _permit = acquire_scrape_permit().await?;
        let sciotte_activities = self
            .scraper
            .get_activities(session, &sciotte_params)
            .await
            .map_err(|e| self.map_scraper_error(&e, "Sciotte scraping failed"))?;

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
            .ok_or_else(|| self.auth_required_no_session())?;

        // Profile lock before the global permit: waiting for the profile to
        // free up must not pin a scarce Chrome-concurrency slot.
        let _profile_lock = ProfileLockRegistry::global()
            .acquire(&session.session_id)
            .await;
        let _permit = acquire_scrape_permit().await?;
        let sciotte_activity = self
            .scraper
            .get_activity(session, id)
            .await
            .map_err(|e| self.map_scraper_error(&e, "Sciotte activity fetch failed"))?;

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
