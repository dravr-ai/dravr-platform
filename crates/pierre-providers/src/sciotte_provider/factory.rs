// ABOUTME: The server's one sciotte client and the per-platform factories that share it
// ABOUTME: Each factory builds a SciotteProvider aimed at one scraped platform (ADR-021)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Building the sciotte client once at startup, and the [`ProviderFactory`]
//! for each platform the scraper service reads — Strava, Garmin Connect,
//! TrainingPeaks and COROS Training Hub.

use std::fmt;
use std::str::FromStr;

use dravr_sciotte::client::{ClientError, ConfigError, SciotteClient};
use dravr_sciotte::models::AthleteId;
use tracing::{error, warn};

use super::{SciotteProvider, SciotteTarget};
use crate::core::{FitnessProvider, ProviderConfig, ProviderFactory};
use crate::delegation::DelegatedReads;
use crate::errors::{AppError, AppResult};

/// The server's one sciotte client, built once at startup.
///
/// Built from the service's `remote_url` and `audience` as the server's
/// configuration read them at startup (`DRAVR_SCIOTTE_REMOTE_URL`,
/// `DRAVR_SCIOTTE_AUDIENCE`), and handed to
/// [`ProviderRegistry::with_sciotte_client`](crate::registry::ProviderRegistry::with_sciotte_client),
/// which shares it with every sciotte call. `None` when no service is
/// configured, and — logged once here — when the configured one cannot be
/// built, including a service off loopback with no audience, which would
/// refuse every request: every call to the service then reports it
/// unconfigured.
#[must_use]
pub fn startup_client(remote_url: Option<&str>, audience: Option<&str>) -> Option<SciotteClient> {
    let base_url = remote_url?;
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

/// Factory for Strava — Sciotte provider
///
/// Every factory holds a clone of the server's one [`SciotteClient`] (`None`
/// when no scraper service is configured), so each provider it builds shares
/// that client's connection pool and identity-token cache.
pub struct SciotteProviderFactory {
    client: Option<SciotteClient>,
}

impl fmt::Debug for SciotteProviderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteProviderFactory")
            .finish_non_exhaustive()
    }
}

impl SciotteProviderFactory {
    /// A factory whose providers call the scraper service through `client`.
    #[must_use]
    pub const fn new(client: Option<SciotteClient>) -> Self {
        Self { client }
    }
}

impl ProviderFactory for SciotteProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::Strava,
            self.client.clone(),
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte"]
    }
}

/// Factory for Garmin Connect — Sciotte provider
pub struct SciotteGarminProviderFactory {
    client: Option<SciotteClient>,
}

impl fmt::Debug for SciotteGarminProviderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteGarminProviderFactory")
            .finish_non_exhaustive()
    }
}

impl SciotteGarminProviderFactory {
    /// A factory whose providers call the scraper service through `client`.
    #[must_use]
    pub const fn new(client: Option<SciotteClient>) -> Self {
        Self { client }
    }
}

impl ProviderFactory for SciotteGarminProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::Garmin,
            self.client.clone(),
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte_garmin"]
    }
}

/// Factory for TrainingPeaks — Sciotte provider
pub struct SciotteTrainingPeaksProviderFactory {
    client: Option<SciotteClient>,
}

impl fmt::Debug for SciotteTrainingPeaksProviderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteTrainingPeaksProviderFactory")
            .finish_non_exhaustive()
    }
}

impl SciotteTrainingPeaksProviderFactory {
    /// A factory whose providers call the scraper service through `client`.
    #[must_use]
    pub const fn new(client: Option<SciotteClient>) -> Self {
        Self { client }
    }
}

impl ProviderFactory for SciotteTrainingPeaksProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::TrainingPeaks,
            self.client.clone(),
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
            self.client.clone(),
        )))
    }
}

/// Factory for COROS Training Hub — Sciotte provider
pub struct SciotteCorosProviderFactory {
    client: Option<SciotteClient>,
}

impl fmt::Debug for SciotteCorosProviderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SciotteCorosProviderFactory")
            .finish_non_exhaustive()
    }
}

impl SciotteCorosProviderFactory {
    /// A factory whose providers call the scraper service through `client`.
    #[must_use]
    pub const fn new(client: Option<SciotteClient>) -> Self {
        Self { client }
    }
}

impl ProviderFactory for SciotteCorosProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(SciotteProvider::new(
            config,
            SciotteTarget::Coros,
            self.client.clone(),
        )))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["sciotte_coros"]
    }
}
