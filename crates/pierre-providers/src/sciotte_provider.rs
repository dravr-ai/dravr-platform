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
use chrono::{NaiveDate, Utc};
use dravr_sciotte::models::{
    Activity as SciotteActivity, ActivityComment as SciotteComment, AuthSession, Lap as SciotteLap,
    RouteTrack as SciotteRouteTrack, Split as SciotteSplit, SportType as SciotteSportType,
};
use pierre_core::untrusted::fence_athlete_text;
use serde_json::{Map, Value};
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::core::{
    planned_workouts_unsupported, ActivityQueryParams, FitnessProvider, OAuth2Credentials,
    ProviderConfig, ProviderFactory,
};
use crate::errors::{AppError, AppResult, ErrorCode};
use crate::models::{
    activity::{Lap, Split},
    Activity, ActivityBuilder, ActivityComment, Athlete, Feel, PersonalRecord, PlannedWorkout,
    SportType, Stats, TimeSeriesData,
};
use crate::pagination::{CursorPage, PaginationParams};
use crate::sciotte_remote::{
    athlete_not_accessible, sciotte_refusal, AthleteId, AthleteProfile, RemoteActivityQuery,
    RemoteSciotteClient, ATHLETE_REQUIRED,
};
use crate::spi::{
    ProviderDescriptor, SciotteDescriptor, SciotteGarminDescriptor, SciotteTrainingPeaksDescriptor,
};
use crate::trainingpeaks_plan::planned_workout_from_trainingpeaks;
use crate::trainingpeaks_self_report::{feel_from_trainingpeaks, rpe_from_trainingpeaks};

/// Target fitness platform for the sciotte scraper
#[derive(Debug, Clone, Copy)]
pub enum SciotteTarget {
    /// Scrape activities from Strava (strava.com)
    Strava,
    /// Scrape activities from Garmin Connect (connect.garmin.com)
    Garmin,
    /// Scrape workouts from the TrainingPeaks calendar (app.trainingpeaks.com)
    TrainingPeaks,
}

impl SciotteTarget {
    /// Parse the API-level target string (e.g. "garmin", "strava",
    /// "trainingpeaks") into a
    /// [`SciotteTarget`]. Unknown values fall back to [`Self::Strava`] to
    /// preserve the long-standing default behaviour of hosted login.
    #[must_use]
    pub fn from_target_param(target: &str) -> Self {
        match target {
            "garmin" => Self::Garmin,
            "trainingpeaks" => Self::TrainingPeaks,
            _ => Self::Strava,
        }
    }

    /// Pierre provider name attached to OAuth/Sciotte rows for this target.
    #[must_use]
    pub const fn provider_name(self) -> &'static str {
        match self {
            Self::Strava => "sciotte",
            Self::Garmin => "sciotte_garmin",
            Self::TrainingPeaks => "sciotte_trainingpeaks",
        }
    }

    /// Inverse of [`Self::provider_name`]: the target for a Pierre backend
    /// name (`"sciotte"`, `"sciotte_garmin"`, `"sciotte_trainingpeaks"`).
    /// Unknown values fall back to
    /// [`Self::Strava`], mirroring [`Self::from_target_param`].
    #[must_use]
    pub fn from_backend_name(backend: &str) -> Self {
        match backend {
            "sciotte_garmin" => Self::Garmin,
            "sciotte_trainingpeaks" => Self::TrainingPeaks,
            _ => Self::Strava,
        }
    }

    /// Provider name the dravr-sciotte scraper service uses for this target
    /// (`"garmin"`, `"strava"`, `"trainingpeaks"`) — sent on remote login/import so the
    /// multi-provider service routes to the right scraper (ADR-021).
    #[must_use]
    pub const fn scraper_provider_name(self) -> &'static str {
        match self {
            Self::Strava => "strava",
            Self::Garmin => "garmin",
            Self::TrainingPeaks => "trainingpeaks",
        }
    }

    /// Whether the provider's own login form asks for a username rather than
    /// an email — TrainingPeaks' does, so a login form typed as `email`
    /// rejects the account before the scraper ever sees it.
    #[must_use]
    pub const fn signs_in_with_username(self) -> bool {
        matches!(self, Self::TrainingPeaks)
    }

    /// The brand the athlete knows this target by ("Strava", "Garmin",
    /// "TrainingPeaks"), read from the target's descriptor so a refusal and
    /// the connect card cannot name it differently.
    fn brand(self) -> &'static str {
        match self {
            Self::Strava => SciotteDescriptor.display_name(),
            Self::Garmin => SciotteGarminDescriptor.display_name(),
            Self::TrainingPeaks => SciotteTrainingPeaksDescriptor.display_name(),
        }
    }

    /// What a read of a coach account's own calendar on this target is told.
    ///
    /// A coach account keeps no calendar of its own, so the read has nothing
    /// to return and signing in again changes nothing. What reads an
    /// athlete's workouts through the account is a link the coach makes from
    /// a group they coach, which the athlete confirms, so the text points
    /// there. It names the brand the account signed in to, and reaches the
    /// reader and the model as written — from the provider when the scraper
    /// refuses the read, and from the read path before any scrape once the
    /// connection is known to be a coach account.
    #[must_use]
    pub fn coach_account_refusal(self) -> String {
        let brand = self.brand();
        format!(
            "This {brand} account is a coach account. {brand} keeps each athlete's plan and \
             training on that athlete's own calendar, and a coach account has none of its \
             own. To read an athlete's {brand} workouts, link each athlete from a group you \
             coach; the athlete confirms the link."
        )
    }
}

/// Longest name a `TrainingPeaks` profile or roster entry reaches a model
/// with. A person's name is well under it; the cap only bounds a field
/// someone filled with something else.
const NAME_FENCE_MAX_CHARS: usize = 80;

/// The username an athlete reads as when the provider gave no name at all.
const UNNAMED_ATHLETE: &str = "Sciotte User";

/// Details key marking an error as the failure of a delegated read's
/// borrowed session, read back by [`is_delegated_session_expired`].
const DELEGATED_DETAIL: &str = "delegated";

/// The error a delegated read answers when the session it goes through — the
/// coach's, not the reader's — is dead.
///
/// It is deliberately not [`AppError::provider_auth_required`]: that code
/// sends the reader through a re-login and lets a sweep flag the reader's own
/// connection, and neither fixes a session only the coach can renew. It is an
/// [`ErrorCode::ExternalAuthFailed`] naming the backend, with `delegated`
/// set in its details so a caller can tell it from any other.
#[must_use]
pub fn delegated_session_expired(backend: &str) -> AppError {
    let brand = SciotteTarget::from_backend_name(backend).brand();
    let mut error = AppError::new(
        ErrorCode::ExternalAuthFailed,
        format!(
            "These {brand} workouts are read through the coach's {brand} connection, \
             which has expired: the coach needs to reconnect {brand}. Nothing is wrong \
             with the athlete's own account."
        ),
    );
    let mut details = Map::new();
    details.insert("provider".to_owned(), Value::from(backend));
    details.insert(DELEGATED_DETAIL.to_owned(), Value::Bool(true));
    error.details = Some(Box::new(Value::Object(details)));
    error
}

/// Whether `error` is a delegated read's dead borrowed session
/// ([`delegated_session_expired`]).
#[must_use]
pub fn is_delegated_session_expired(error: &AppError) -> bool {
    matches!(error.code, ErrorCode::ExternalAuthFailed)
        && error
            .details
            .as_ref()
            .and_then(|details| details.get(DELEGATED_DETAIL))
            .and_then(Value::as_bool)
            == Some(true)
}

/// Sciotte provider — a thin session-holder over the dedicated scraper service.
///
/// Routes every scrape to the dedicated dravr-sciotte service ([[ADR-021]]).
/// Since the Phase 4 cutover it holds no in-process Chrome; it keeps the
/// platform-held [`AuthSession`] and forwards it to the service on each fetch.
///
/// A provider built by [`Self::delegated`] reads one athlete of a
/// `TrainingPeaks` coach account through that coach's session: every read
/// names the athlete, and nothing outside that athlete's calendar is reachable
/// through it.
pub struct SciotteProvider {
    config: ProviderConfig,
    session: RwLock<Option<AuthSession>>,
    provider_name: &'static str,
    /// Whose calendar a delegated provider reads, by the provider's athlete
    /// id; `None` reads the signed-in account's own.
    subject: Option<AthleteId>,
    /// Whether the last list scrape read the list's head, as the service
    /// reported it. Starts `true`: nothing has been fetched, so nothing is
    /// known to be missing.
    head_complete: AtomicBool,
}

impl SciotteProvider {
    fn new(config: ProviderConfig, target: SciotteTarget) -> Self {
        let provider_name = target.provider_name();
        info!(target = ?target, "Sciotte provider initialized (remote service)");
        Self {
            config,
            session: RwLock::new(None),
            provider_name,
            subject: None,
            head_complete: AtomicBool::new(true),
        }
    }

    /// A `TrainingPeaks` provider that reads `athlete`'s calendar through the
    /// session it is given, which is a coach account's.
    ///
    /// `TrainingPeaks` is the one target built this way: a coach account there
    /// has no calendar of its own and reads each athlete on its roster by that
    /// athlete's id, so the target is fixed rather than taken as an argument.
    #[must_use]
    pub fn delegated(config: ProviderConfig, athlete: AthleteId) -> Self {
        info!("Sciotte provider initialized for a coached TrainingPeaks athlete (remote service)");
        Self {
            subject: Some(athlete),
            ..Self::new(config, SciotteTarget::TrainingPeaks)
        }
    }

    /// Build the `provider_auth_required` error used for the "no session at
    /// all" branch, so the chat pipeline can mint a hosted-login URL and
    /// short-circuit the LLM with an actionable reply.
    fn auth_required_no_session(&self) -> AppError {
        AppError::provider_auth_required(self.provider_name)
    }

    /// Clone the authenticated session and release the lock before returning.
    ///
    /// Every caller follows this with a remote round trip — `import_session`
    /// plus a scrape on the dedicated service — bounded only by the 330s client
    /// timeout. `tokio::sync::RwLock` is write-preferring with a FIFO queue, so
    /// a read guard held across that call would park the `authenticate` writer
    /// at the head of the queue and every later reader behind the writer: this
    /// provider's session lock would be dead for the life of the process,
    /// surfacing as hung requests and 504s rather than a crash.
    ///
    /// Nothing contends today, but only because [`SciotteProviderFactory`]
    /// builds a fresh `SciotteProvider` per call, so the `Arc<RwLock<_>>` has a
    /// single owner. That safety is a property of the provider's *lifetime*
    /// rather than of the locking — caching or sharing a provider to skip
    /// re-authentication would reintroduce the stall from a different file,
    /// with the lock code untouched and no static analysis able to see it.
    /// Snapshotting here makes the hold time independent of that decision.
    async fn session_snapshot(&self) -> AppResult<AuthSession> {
        let guard = self.session.read().await;
        Ok(guard
            .as_ref()
            .ok_or_else(|| self.auth_required_no_session())?
            .clone())
    }

    /// Re-tag a remote-service error that must name this backend.
    ///
    /// The remote client does not know which backend owns the session, so
    /// two of its errors are finished here:
    ///
    /// - An auth-shaped error carries only the generic `sciotte` slug, and
    ///   the reconnect link minted downstream branches on the backend name to
    ///   pick the hosted-login target. Without the re-tag, a dead
    ///   `sciotte_garmin` session would send the athlete to a Strava login.
    /// - The coach-account refusal ([`ATHLETE_REQUIRED`]) reaches the athlete
    ///   as written, so it is re-worded with the brand they signed in to —
    ///   "this TrainingPeaks account is a coach account" — keeping its code
    ///   and its refusal marker. It is never an auth error: a coach account
    ///   signed in again is still a coach account.
    ///
    /// On a delegated provider the auth-shaped error becomes
    /// [`delegated_session_expired`] instead: the dead session is the coach's,
    /// and a reconnect prompt to the reader could not renew it.
    fn tag_remote_auth(&self, e: AppError) -> AppError {
        if e.provider_auth_required_provider().is_some() {
            if self.subject.is_some() {
                return delegated_session_expired(self.provider_name);
            }
            return AppError::provider_auth_required(self.provider_name);
        }
        if sciotte_refusal(&e) == Some(ATHLETE_REQUIRED) {
            return AppError {
                message: SciotteTarget::from_backend_name(self.provider_name)
                    .coach_account_refusal(),
                ..e
            };
        }
        e
    }

    /// Refuse, before any remote call, a detail id outside the delegated
    /// athlete's calendar.
    ///
    /// `TrainingPeaks` addresses a workout detail as `athleteId:workoutId`, and
    /// the scraper reads whichever athlete that names under the session it is
    /// handed. A coach session can read every athlete on the roster, so
    /// without this a reader holding a link to one athlete could read another
    /// athlete's workout by editing the prefix. The refusal is a plain
    /// not-found: the id names nothing this reader may see. A provider reading
    /// its own account takes any id, as before.
    fn require_detail_in_scope(&self, id: &str) -> AppResult<()> {
        let Some(athlete) = &self.subject else {
            return Ok(());
        };
        let in_scope = id
            .strip_prefix(athlete.as_str())
            .and_then(|rest| rest.strip_prefix(':'))
            .is_some_and(|workout| {
                !workout.is_empty() && workout.bytes().all(|b| b.is_ascii_digit())
            });
        if in_scope {
            Ok(())
        } else {
            Err(AppError::not_found("Activity"))
        }
    }
}

/// A profile or roster name from `target`, as a model may read it.
///
/// `TrainingPeaks` names are fenced as data (decision 4 of the coach roster
/// work: every `TrainingPeaks` free text reaching a model is): the name a coach
/// sees for an athlete, or the account holder's own, is whatever was typed
/// into that account. Strava and Garmin names cross as before. `None` when
/// there is no name, or nothing left of it once whitespace is folded.
fn profile_name(raw: Option<String>, target: SciotteTarget) -> Option<String> {
    match target {
        SciotteTarget::TrainingPeaks => raw
            .as_deref()
            .and_then(|name| fence_athlete_text(name, NAME_FENCE_MAX_CHARS)),
        SciotteTarget::Strava | SciotteTarget::Garmin => raw,
    }
}

/// The signed-in account's own profile as an [`Athlete`].
fn own_athlete(profile: AthleteProfile, target: SciotteTarget) -> Athlete {
    Athlete {
        id: "sciotte".to_owned(),
        username: profile_name(profile.display_name, target)
            .unwrap_or_else(|| UNNAMED_ATHLETE.to_owned()),
        firstname: profile_name(profile.firstname, target),
        lastname: profile_name(profile.lastname, target),
        profile_picture: profile.profile_picture_url,
        provider: "sciotte".to_owned(),
    }
}

/// The athlete a delegated provider reads, as the coach's roster lists them.
///
/// The roster is the authority on who the coach may read: an athlete it no
/// longer lists is the [`athlete_not_accessible`] refusal, the answer the
/// scraper gives a read that names one. The roster carries an id and a name
/// only, so the other profile fields stay empty.
fn roster_athlete(profile: AthleteProfile, athlete: &AthleteId) -> AppResult<Athlete> {
    let entry = profile
        .coached_athletes
        .into_iter()
        .find(|coached| coached.id == athlete.as_str())
        .ok_or_else(athlete_not_accessible)?;
    Ok(Athlete {
        id: athlete.as_str().to_owned(),
        username: profile_name(entry.display_name, SciotteTarget::TrainingPeaks)
            .unwrap_or_else(|| UNNAMED_ATHLETE.to_owned()),
        firstname: None,
        lastname: None,
        profile_picture: None,
        provider: "sciotte".to_owned(),
    })
}

/// Direct sciotte → cageux `SportType` conversion. Both enums share variant
/// names (sciotte mirrors cageux's canonical set), so a 1:1 match is
/// bulletproof; the previous round-trip via `display_name()` →
/// `from_internal_string()` was lossy because `display_name` returns
/// human-readable Title-Case-with-spaces ("Cross-Country Skiing") while
/// `from_internal_string` expects the `snake_case` serde form
/// ("`cross_country_skiing`"), so every non-trivial variant fell through to
/// `Other(<display_name>)` and broke filter / serialization.
pub(crate) fn convert_sport_type(s: &SciotteSportType) -> SportType {
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

/// Convert a sciotte `Activity` scraped from `target` to a Pierre `Activity`.
///
/// The athlete's self-report crosses in the platform's scales (see
/// [`self_report`]), and the comments cross entry for entry in the order
/// sciotte gives them — the notes a provider keeps in a single field first,
/// then its timestamped thread, oldest first — whichever platform they were
/// scraped from.
///
/// The activity carries no description, on purpose. Sciotte's `Activity`
/// has none to give, and the text `TrainingPeaks` keeps in a workout's
/// description is the coach's prescription for the session: it belongs to
/// the planned workout, not to the athlete's account of the activity that
/// completed it.
fn convert_activity(sciotte: &SciotteActivity, target: SciotteTarget) -> Activity {
    let sport_type = convert_sport_type(&sciotte.sport_type);
    let (perceived_exertion, feel) = self_report(sciotte, target);
    let comments = (!sciotte.comments.is_empty())
        .then(|| sciotte.comments.iter().map(convert_comment).collect());

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
    .normalized_power_opt(sciotte.normalized_power)
    .average_cadence_opt(sciotte.average_cadence)
    .suffer_score_opt(sciotte.suffer_score)
    .training_stress_score_opt(sciotte.training_stress_score)
    .intensity_factor_opt(sciotte.intensity_factor)
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
    .time_series_data_opt(sciotte.route.as_ref().and_then(route_to_time_series))
    .perceived_exertion_opt(perceived_exertion)
    .feel_opt(feel)
    .comments_opt(comments)
    .build()
}

/// The athlete's self-report on a scraped row as `(perceived exertion, feel)`
/// in the platform's scales: a 1–10 CR-10 rating and a named [`Feel`].
///
/// Only `TrainingPeaks` states either as a rating, and its encodings are read
/// in [`crate::trainingpeaks_self_report`]. Strava's scraped exertion is the
/// label its slider shows ("Moderate", "Hard") — a band of the scale, not a
/// rating on it — which the numeric field cannot hold without inventing a
/// number, and neither the Strava nor the Garmin extract carries a feel rank.
fn self_report(sciotte: &SciotteActivity, target: SciotteTarget) -> (Option<f32>, Option<Feel>) {
    match target {
        SciotteTarget::TrainingPeaks => (
            sciotte
                .perceived_exertion
                .as_deref()
                .and_then(rpe_from_trainingpeaks),
            sciotte.feel.and_then(feel_from_trainingpeaks),
        ),
        SciotteTarget::Strava | SciotteTarget::Garmin => (None, None),
    }
}

/// Translate one of sciotte's [`SciotteComment`]s into cageux's
/// [`ActivityComment`]. The two mirror each other field for field; an author
/// the provider only names by role (`coach`, `athlete`) stays that word.
fn convert_comment(comment: &SciotteComment) -> ActivityComment {
    ActivityComment {
        author: comment.author.clone(),
        text: comment.text.clone(),
        created_at: comment.created_at,
    }
}

/// Fold sciotte's GPS [`SciotteRouteTrack`] into cageux's [`TimeSeriesData`],
/// the channel every route consumer reads — `export_routes` and the endurance
/// route endpoint hydrate their geometry from `gps_coordinates` + `altitude`.
///
/// A track is the only stream a scrape carries, so the numeric channels the
/// API providers fill from device streams stay absent here. `timestamps` hold
/// sample indices: the scraped track has no time axis, and indices are the
/// same fallback the Strava and intervals.icu conversions use for a stream set
/// whose provider omits one. A track without coordinates yields `None` rather
/// than an empty series.
fn route_to_time_series(route: &SciotteRouteTrack) -> Option<TimeSeriesData> {
    if route.coordinates.is_empty() {
        return None;
    }

    // cageux's altitude channel is f32. Elevation in metres spans roughly
    // -430 (Dead Sea) to 8849 (Everest), well inside f32's exactly-representable
    // integer range, so narrowing costs sub-millimetre precision that no
    // barometric or GPS altimeter resolves.
    let altitude = match route.altitudes_meters.as_ref() {
        Some(meters) if meters.len() == route.coordinates.len() => {
            Some(meters.iter().map(|m| *m as f32).collect())
        }
        // An elevation series of a different length is not index-aligned with
        // the track, and a caller reading the pair position by position would
        // attribute each reading to the wrong point. Dropping it reports the
        // track's elevation as unknown, which is what a misaligned series means.
        Some(meters) => {
            warn!(
                coordinates = route.coordinates.len(),
                altitudes = meters.len(),
                "sciotte route elevation series is not index-aligned with its track; dropping it"
            );
            None
        }
        None => None,
    };

    Some(TimeSeriesData {
        timestamps: (0..route.coordinates.len())
            .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
            .collect(),
        heart_rate: None,
        power: None,
        cadence: None,
        speed: None,
        altitude,
        temperature: None,
        gps_coordinates: Some(route.coordinates.clone()),
    })
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

    /// The signed-in account's profile, or — on a delegated provider — the
    /// athlete it reads, as the coach's roster lists them.
    async fn get_athlete(&self) -> AppResult<Athlete> {
        let session = &self.session_snapshot().await?;

        // ADR-021: scrape on the dedicated service (there is no in-process
        // fallback since the Phase 4 cutover). Import the platform-held session
        // — re-hydrates the service after a scale-to-zero / redeploy — then fetch.
        let target = SciotteTarget::from_backend_name(self.provider_name);
        let remote = RemoteSciotteClient::require_from_env()?;
        remote
            .import_session(session, target.scraper_provider_name())
            .await?;
        let profile = remote
            .get_athlete(&session.session_id)
            .await
            .map_err(|e| self.tag_remote_auth(e))?;
        match &self.subject {
            Some(athlete) => roster_athlete(profile, athlete),
            None => Ok(own_athlete(profile, target)),
        }
    }

    async fn get_activities_with_params(
        &self,
        params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        let session = &self.session_snapshot().await?;

        let limit = params.limit.unwrap_or(20);
        // Detail-page enrichment (HR streams, laps, segments — and the real UTC
        // start_date, absent from the date-only list page) is an N+1 roundtrip
        // through the headless browser, so it stays OFF by default to keep
        // interactive paths (chat, group snapshots) fast. It's opt-in per
        // deployment via PIERRE_SCIOTTE_ENRICH_DETAILS=true (dev leaves it off)
        // when correct start times matter more than the latency; the scraper
        // bounds the pass by its own per-request ceiling.
        let enrich_details =
            env::var("PIERRE_SCIOTTE_ENRICH_DETAILS").is_ok_and(|v| v == "true" || v == "1");

        // ADR-021: fetch on the dedicated service (no in-process fallback since
        // the Phase 4 cutover). Import the platform-held session — re-hydrates
        // the service after a scale-to-zero / redeploy — then scrape over HTTP;
        // convert_activity keeps the returned shape identical for every caller.
        // before/after pass through as epoch seconds so the scrape bounds the
        // fetch by date, matching the API providers (Strava/Whoop).
        let target = SciotteTarget::from_backend_name(self.provider_name);
        let remote = RemoteSciotteClient::require_from_env()?;
        // A delegated provider names its athlete. Otherwise no athlete is
        // named: the platform reads the signed-in account's own activities,
        // and a TrainingPeaks coach account, which has none, answers the
        // coach-account refusal `tag_remote_auth` words.
        let query = RemoteActivityQuery {
            limit: Some(limit as u32),
            after_epoch: params.after,
            before_epoch: params.before,
            sport_type: None,
            enrich_details,
            athlete: self.subject.clone(),
        };
        remote
            .import_session(session, target.scraper_provider_name())
            .await?;
        let list = remote
            .get_activities(&session.session_id, &query)
            .await
            .map_err(|e| self.tag_remote_auth(e))?;
        self.head_complete
            .store(list.head_complete, Ordering::Relaxed);
        let activities: Vec<Activity> = list
            .activities
            .iter()
            .map(|activity| convert_activity(activity, target))
            .collect();
        if list.head_complete {
            info!(
                count = activities.len(),
                "Sciotte scrape completed (remote service)"
            );
        } else {
            warn!(
                count = activities.len(),
                "Sciotte scrape completed without the list head: the fresh-head fetch failed, \
                 so the newest activities may be missing from this capture"
            );
        }
        Ok(activities)
    }

    fn head_complete(&self) -> bool {
        self.head_complete.load(Ordering::Relaxed)
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
        self.require_detail_in_scope(id)?;
        let session = &self.session_snapshot().await?;

        // ADR-021: fetch the single activity's detail on the dedicated service.
        let target = SciotteTarget::from_backend_name(self.provider_name);
        let remote = RemoteSciotteClient::require_from_env()?;
        remote
            .import_session(session, target.scraper_provider_name())
            .await?;
        let sciotte_activity = remote
            .get_activity(&session.session_id, id)
            .await
            .map_err(|e| self.tag_remote_auth(e))?;
        Ok(convert_activity(&sciotte_activity, target))
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

    /// Only the TrainingPeaks mirror reads a planned calendar — the one
    /// sciotte target whose descriptor declares `PLANNED_WORKOUTS`. The
    /// Strava and Garmin mirrors share this type and answer the same refusal
    /// every provider without the capability does.
    async fn list_planned_workouts(
        &self,
        after: NaiveDate,
        before: NaiveDate,
    ) -> AppResult<Vec<PlannedWorkout>> {
        let target = SciotteTarget::from_backend_name(self.provider_name);
        if !matches!(target, SciotteTarget::TrainingPeaks) {
            return Err(planned_workouts_unsupported(self.provider_name));
        }
        let session = &self.session_snapshot().await?;

        // ADR-021: read on the dedicated service, after importing the
        // platform-held session, exactly as the activity list does, and for
        // the same athlete: a delegated provider's, or else the signed-in
        // account's own.
        let remote = RemoteSciotteClient::require_from_env()?;
        remote
            .import_session(session, target.scraper_provider_name())
            .await?;
        let planned = remote
            .get_planned_workouts(&session.session_id, after, before, self.subject.as_ref())
            .await
            .map_err(|e| self.tag_remote_auth(e))?;
        info!(
            count = planned.len(),
            %after,
            %before,
            "Sciotte planned-workout read completed (remote service)"
        );
        Ok(planned
            .iter()
            .map(planned_workout_from_trainingpeaks)
            .collect())
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

/// Factory for TrainingPeaks — Sciotte provider
pub struct SciotteTrainingPeaksProviderFactory;

impl ProviderFactory for SciotteTrainingPeaksProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::TrainingPeaks,
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte_trainingpeaks"]
    }
}
