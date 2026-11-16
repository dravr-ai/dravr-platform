// ABOUTME: The sciotte clients a server builds at startup and the per-platform factories that share them
// ABOUTME: Each factory builds a SciotteProvider aimed at one scraped platform (ADR-021)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Building the sciotte clients once at startup, and the [`ProviderFactory`]
//! for each platform the scraper service reads — Strava, Garmin Connect,
//! TrainingPeaks and COROS Training Hub.

use std::fmt;
use std::str::FromStr;

use dravr_sciotte::client::{ClientError, ConfigError, SciotteClient};
use dravr_sciotte::models::AthleteId;
use tracing::{error, warn};

use super::{SciotteProvider, SciotteReads, SciotteTarget};
use crate::core::{FitnessProvider, ProviderConfig, ProviderFactory};
use crate::delegation::DelegatedReads;
use crate::errors::{AppError, AppResult};

/// One sciotte service's client, built once at startup from its `base_url`
/// and the shared `audience` (`DRAVR_SCIOTTE_AUDIENCE`).
///
/// `None` — logged once here — when the client cannot be built, including a
/// service off loopback with no audience, which would refuse every request:
/// the caller then reports the service unconfigured
/// ([`startup_reads`](super::startup_reads)).
#[must_use]
pub(super) fn startup_client(base_url: &str, audience: Option<&str>) -> Option<SciotteClient> {
    match SciotteClient::new(base_url, audience) {
        Ok(client) => Some(client),
        Err(ClientError::Config(ConfigError::AudienceRequired { .. })) => {
            warn!(
                "The sciotte service is configured without an audience; disabling the \
                 sciotte client rather than sending requests the service will refuse"
            );
            None
        }
        Err(e) => {
            error!(
                error = %e,
                "The sciotte client could not be built; scraper calls will report the service unconfigured"
            );
            None
        }
    }
}

/// The client sign-ins reach the scraper service through, as the server's
/// configuration built it at startup.
///
/// A sign-in parks its browser in one instance's memory while the athlete
/// fetches a code, so a deployment runs sign-ins on a service capped at one
/// instance, apart from the read service (carnet#1504).
#[derive(Clone)]
pub enum SciotteLoginClient {
    /// No sign-in service is configured (`DRAVR_SCIOTTE_LOGIN_URL` unset):
    /// sign-ins go to the read service, the one-service shape of local
    /// development and the test doubles.
    Unconfigured,
    /// The sign-in service's own client.
    Built(SciotteClient),
    /// A sign-in service is configured but its client could not be built.
    /// Sign-ins are refused rather than sent to the read service, which may
    /// run more than one instance and lose the parked browser.
    Unbuildable,
}

impl fmt::Debug for SciotteLoginClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unconfigured => "SciotteLoginClient::Unconfigured",
            Self::Built(_) => "SciotteLoginClient::Built",
            Self::Unbuildable => "SciotteLoginClient::Unbuildable",
        })
    }
}

/// The sign-in client for `login_url`, built with the read service's
/// `audience`, which the sign-in service shares.
///
/// A configured URL whose client cannot be built (an off-loopback service with
/// no audience included) is logged once here, naming the sign-in service and
/// the cause, and refuses every sign-in.
#[must_use]
pub fn startup_login_client(login_url: Option<&str>, audience: Option<&str>) -> SciotteLoginClient {
    let Some(base_url) = login_url else {
        return SciotteLoginClient::Unconfigured;
    };
    match SciotteClient::new(base_url, audience) {
        Ok(client) => SciotteLoginClient::Built(client),
        Err(e) => {
            error!(
                error = %e,
                "DRAVR_SCIOTTE_LOGIN_URL is set but its client could not be built; \
                 sign-ins are refused until it is fixed"
            );
            SciotteLoginClient::Unbuildable
        }
    }
}

/// Factory for Strava — Sciotte provider
///
/// Every factory holds a clone of the server's read services ([`SciotteReads`], `None`
/// when no scraper service is configured), so each provider it builds shares
/// those clients' connection pools and identity-token caches.
pub struct SciotteProviderFactory {
    reads: Option<SciotteReads>,
}

impl fmt::Debug for SciotteProviderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteProviderFactory")
            .finish_non_exhaustive()
    }
}

impl SciotteProviderFactory {
    /// A factory whose providers call the scraper services through `reads`.
    #[must_use]
    pub const fn new(reads: Option<SciotteReads>) -> Self {
        Self { reads }
    }
}

impl ProviderFactory for SciotteProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::Strava,
            self.reads.clone(),
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte"]
    }
}

/// Factory for Garmin Connect — Sciotte provider
pub struct SciotteGarminProviderFactory {
    reads: Option<SciotteReads>,
}

impl fmt::Debug for SciotteGarminProviderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteGarminProviderFactory")
            .finish_non_exhaustive()
    }
}

impl SciotteGarminProviderFactory {
    /// A factory whose providers call the scraper services through `reads`.
    #[must_use]
    pub const fn new(reads: Option<SciotteReads>) -> Self {
        Self { reads }
    }
}

impl ProviderFactory for SciotteGarminProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::Garmin,
            self.reads.clone(),
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte_garmin"]
    }
}

/// Factory for TrainingPeaks — Sciotte provider
pub struct SciotteTrainingPeaksProviderFactory {
    reads: Option<SciotteReads>,
}

impl fmt::Debug for SciotteTrainingPeaksProviderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteTrainingPeaksProviderFactory")
            .finish_non_exhaustive()
    }
}

impl SciotteTrainingPeaksProviderFactory {
    /// A factory whose providers call the scraper services through `reads`.
    #[must_use]
    pub const fn new(reads: Option<SciotteReads>) -> Self {
        Self { reads }
    }
}

impl ProviderFactory for SciotteTrainingPeaksProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::TrainingPeaks,
            self.reads.clone(),
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte_trainingpeaks"]
    }

    // A coach account there has no calendar of its own and reads each athlete
    // on its roster by that athlete's id.
    fn delegated_reads(&self) -> Option<&dyn DelegatedReads> {
        Some(self)
    }
}

impl DelegatedReads for SciotteTrainingPeaksProviderFactory {
    fn check_athlete_id(&self, athlete_id: &str) -> AppResult<()> {
        AthleteId::from_str(athlete_id)
            .map(drop)
            .map_err(|e| AppError::invalid_input(e.to_string()))
    }

    fn create_delegated(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        let athlete =
            AthleteId::from_str(athlete_id).map_err(|e| AppError::invalid_input(e.to_string()))?;
        Ok(Box::new(SciotteProvider::delegated(
            config,
            athlete,
            self.reads.clone(),
        )))
    }
}

/// Factory for COROS Training Hub — Sciotte provider
pub struct SciotteCorosProviderFactory {
    reads: Option<SciotteReads>,
}

impl fmt::Debug for SciotteCorosProviderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteCorosProviderFactory")
            .finish_non_exhaustive()
    }
}

impl SciotteCorosProviderFactory {
    /// A factory whose providers call the scraper services through `reads`.
    #[must_use]
    pub const fn new(reads: Option<SciotteReads>) -> Self {
        Self { reads }
    }
}

impl ProviderFactory for SciotteCorosProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::Coros,
            self.reads.clone(),
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte_coros"]
    }
}
