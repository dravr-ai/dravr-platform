// ABOUTME: The four sciotte backends' descriptors: Strava, Garmin, TrainingPeaks and COROS read by signing in as the athlete
// ABOUTME: No OAuth, browser sessions; each names the service it reads, whose terms and field set its data carries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The mirror backends' descriptors, one per service the scraper reads.

use pierre_core::constants::oauth::providers::{COROS, GARMIN, STRAVA, TRAININGPEAKS};
use pierre_core::transport::TransportPolicy;

use super::{OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor};
use crate::field_support::{ActivityFieldSupport, ActivityFields as F, StreamChannels as S};
use crate::provider_terms;

/// The channels every scraped route carries: the track and its elevations.
/// A scraped route has no time axis and no sensor channel.
const ROUTE: S = S::LATLNG.union(S::ALTITUDE);

/// Sciotte web scraping provider descriptor
///
/// Sciotte uses browser-based web scraping via CDP to capture session cookies
/// and extract activity data from fitness platform HTML pages (Strava, Garmin, etc.).
/// It does not use OAuth — authentication happens via a streamed browser login session.
#[derive(Debug)]
pub struct SciotteDescriptor;

impl ProviderDescriptor for SciotteDescriptor {
    /// What the pinned client scrapes from Strava's pages (sciotte 2.0): a
    /// list row carries the athlete's description, the workout tag, the
    /// trainer and commute flags, the moving time and the average heart
    /// rate; the detail page adds the private note, the rest of the sensor
    /// summary, gear, device, laps, splits and the athlete's exertion, and
    /// the recorded streams on their time axis.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::DESCRIPTION
                .union(F::WORKOUT_TYPE)
                .union(F::TRAINER)
                .union(F::COMMUTE)
                .union(F::AVERAGE_HEART_RATE)
                .union(F::MOVING_TIME),
            detail_only: F::PRIVATE_NOTE
                .union(F::MAX_HEART_RATE)
                .union(F::AVERAGE_POWER)
                .union(F::AVERAGE_CADENCE)
                .union(F::CALORIES)
                .union(F::GEAR)
                .union(F::DEVICE)
                .union(F::LAPS)
                .union(F::SPLITS)
                .union(F::PERCEIVED_EXERTION),
            streams: S::all(),
        }
    }

    fn name(&self) -> &'static str {
        "sciotte"
    }

    fn display_name(&self) -> &'static str {
        "Strava"
    }

    /// The Strava backend: its stamp is `sciotte`, its data Strava's.
    fn origin(&self) -> Option<&'static str> {
        Some(STRAVA)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Sciotte uses browser login (not OAuth) and provides activity data
        ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::RECORDED_ACTIVITIES)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None // Sciotte uses browser-based session cookies, not OAuth
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None // Sciotte uses browser-based session cookies, not OAuth
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // A browser session, not an OAuth grant
    }

    fn api_base_url(&self) -> &'static str {
        // Sciotte runs in-process via dravr-sciotte — no external API
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[] // No OAuth scopes — browser session-based
    }

    /// This backend reads strava.com, so what it serves is Strava data.
    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::STRAVA_TRANSPORT
    }
}

/// Sciotte Garmin Connect web scraping provider descriptor
#[derive(Debug)]
pub struct SciotteGarminDescriptor;

impl ProviderDescriptor for SciotteGarminDescriptor {
    /// Garmin Connect's activity list carries the sensor summary; its detail
    /// adds the moving time, the device, laps, splits and the athlete's
    /// exertion.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::AVERAGE_HEART_RATE
                .union(F::MAX_HEART_RATE)
                .union(F::CALORIES)
                .union(F::AVERAGE_CADENCE)
                .union(F::AVERAGE_POWER)
                .union(F::NORMALIZED_POWER),
            detail_only: F::MOVING_TIME
                .union(F::DEVICE)
                .union(F::LAPS)
                .union(F::SPLITS)
                .union(F::PERCEIVED_EXERTION),
            streams: ROUTE,
        }
    }

    fn name(&self) -> &'static str {
        "sciotte_garmin"
    }

    fn display_name(&self) -> &'static str {
        "Garmin"
    }

    fn origin(&self) -> Option<&'static str> {
        Some(GARMIN)
    }

    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::GARMIN_TRANSPORT
    }

    /// Activities are scraped on demand; the night's sleep, resting heart
    /// rate, overnight HRV, stress, Body Battery, body metrics and `VO2max`
    /// are synced from the same session's daily summary by the health sync.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES
            .union(ProviderCapabilities::RECORDED_ACTIVITIES)
            .union(ProviderCapabilities::SLEEP_TRACKING)
            .union(ProviderCapabilities::RECOVERY_METRICS)
            .union(ProviderCapabilities::HEALTH_METRICS)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // A browser session, not an OAuth grant
    }

    fn api_base_url(&self) -> &'static str {
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Sciotte TrainingPeaks web scraping provider descriptor.
///
/// Activities and the calendar's planned workouts, and never
/// `CHEAP_ACTIVITY_DETAIL`: a TrainingPeaks workout's detail is a browser
/// round trip per workout on the scraper service, the same N+1 the Garmin and
/// Strava mirrors ration. The planned read is one scraper call per window.
#[derive(Debug)]
pub struct SciotteTrainingPeaksDescriptor;

impl ProviderDescriptor for SciotteTrainingPeaksDescriptor {
    /// A TrainingPeaks workout carries its summary, the athlete's exertion
    /// and the athlete's and coach's comments on every row. The workout's
    /// description is the coach's prescription, never mapped as the
    /// athlete's.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::AVERAGE_HEART_RATE
                .union(F::MAX_HEART_RATE)
                .union(F::CALORIES)
                .union(F::AVERAGE_CADENCE)
                .union(F::AVERAGE_POWER)
                .union(F::NORMALIZED_POWER)
                .union(F::PERCEIVED_EXERTION)
                .union(F::COMMENTS),
            detail_only: F::empty(),
            streams: S::empty(),
        }
    }

    fn name(&self) -> &'static str {
        "sciotte_trainingpeaks"
    }

    fn display_name(&self) -> &'static str {
        "TrainingPeaks"
    }

    /// The planned workouts are stamped `trainingpeaks`
    /// ([`crate::trainingpeaks_plan`]), as is every scraped activity's source.
    fn origin(&self) -> Option<&'static str> {
        Some(TRAININGPEAKS)
    }

    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::TRAININGPEAKS_TRANSPORT
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // A coach account reads its athletes through the coach's scraper
        // session (`SciotteTrainingPeaksProviderFactory::delegated_reads`).
        ProviderCapabilities::ACTIVITIES
            .union(ProviderCapabilities::RECORDED_ACTIVITIES)
            .union(ProviderCapabilities::PLANNED_WORKOUTS)
            .union(ProviderCapabilities::COACH_ROSTER)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // A browser session, not an OAuth grant
    }

    fn api_base_url(&self) -> &'static str {
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Sciotte COROS Training Hub web scraping provider descriptor
#[derive(Debug)]
pub struct SciotteCorosDescriptor;

impl ProviderDescriptor for SciotteCorosDescriptor {
    /// The Training Hub's list carries the summary and the device; its detail
    /// adds the maximum heart rate, the moving time, laps and the athlete's
    /// note.
    fn activity_fields(&self) -> ActivityFieldSupport {
        ActivityFieldSupport {
            list: F::AVERAGE_HEART_RATE
                .union(F::CALORIES)
                .union(F::AVERAGE_CADENCE)
                .union(F::AVERAGE_POWER)
                .union(F::NORMALIZED_POWER)
                .union(F::DEVICE),
            detail_only: F::MAX_HEART_RATE
                .union(F::MOVING_TIME)
                .union(F::LAPS)
                .union(F::COMMENTS),
            streams: ROUTE,
        }
    }

    fn name(&self) -> &'static str {
        "sciotte_coros"
    }

    fn display_name(&self) -> &'static str {
        "COROS"
    }

    /// `coros`, which no compiled descriptor carries in production: the
    /// partner-API provider is feature-gated off (registre#509).
    fn origin(&self) -> Option<&'static str> {
        Some(COROS)
    }

    fn transport_policy(&self) -> TransportPolicy {
        provider_terms::COROS_TRANSPORT
    }

    /// Activities are scraped on demand; the resting heart rate, sleep HRV
    /// and `VO2max` are synced from the Training Hub's daily analysis by the
    /// health sync. The Training Hub carries no sleep sessions.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES
            .union(ProviderCapabilities::RECORDED_ACTIVITIES)
            .union(ProviderCapabilities::RECOVERY_METRICS)
            .union(ProviderCapabilities::HEALTH_METRICS)
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None // A browser session, not an OAuth grant
    }

    fn api_base_url(&self) -> &'static str {
        ""
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}
