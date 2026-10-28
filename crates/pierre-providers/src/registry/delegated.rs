// ABOUTME: ProviderRegistry's delegated-provider builders: one coached athlete read, or written, through a coach's own credential
// ABOUTME: Which providers a coach account reads athletes through, and the athlete-id check every link takes before it is stored
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};

use super::ProviderRegistry;
use crate::backend_resolver;
use crate::core::{FitnessProvider, OAuth2Credentials, ProviderConfig};
use crate::delegation::DelegatedReads;
#[cfg(doc)]
use crate::spi::ProviderCapabilities;

impl ProviderRegistry {
    /// Refuse an athlete id `provider_name` could not have issued, before it
    /// is stored on a link or reaches a URL.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for a provider that cannot be read on
    /// anyone's behalf, or for an id the provider refuses.
    pub fn check_delegated_athlete(&self, provider_name: &str, athlete_id: &str) -> AppResult<()> {
        self.delegated_reads(provider_name)?
            .check_athlete_id(athlete_id)
    }

    /// Create a provider that reads one coached athlete, `athlete_id`,
    /// through `coach_credentials`: the coach account's own stored
    /// credential, an OAuth grant or an API key alike
    /// ([`CredentialKind`](crate::core::CredentialKind)).
    ///
    /// Only a provider whose descriptor declares
    /// [`ProviderCapabilities::COACH_ROSTER`] is built this way. A coaching
    /// platform's coach account reads each athlete who shares with it by that
    /// athlete's id; the provider built names `athlete_id` on every read and
    /// refuses a detail id outside that athlete. The athlete id is checked
    /// before the provider is built, so a refused id reaches no URL.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for a provider that cannot be read on
    /// anyone's behalf, for an athlete id it refuses, or when the provider
    /// has no default configuration; or the provider's refusal of the
    /// credentials.
    pub async fn create_delegated_provider(
        &self,
        provider_name: &str,
        athlete_id: &str,
        coach_credentials: OAuth2Credentials,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        let reads = self.delegated_reads(provider_name)?;
        let provider = reads.create_delegated(self.delegated_config(provider_name)?, athlete_id)?;
        provider.set_credentials(coach_credentials).await?;
        Ok(provider)
    }

    /// Create a provider that reads one coached athlete, `athlete_id`, for
    /// their human coach through `coach_credentials`, the coach account's own
    /// stored credential: what the coach's own read of a coached athlete's
    /// planned calendar goes through.
    ///
    /// Built as [`Self::create_delegated_provider`] builds a member's read,
    /// with the coach as its reader
    /// ([`DelegatedReads::create_delegated_coach_reader`]): on a platform that
    /// hides events from the athlete, the coach sees them, labelled. Whether
    /// this coach may read this athlete's calendar — the link and whose it
    /// is — is the caller's to decide first.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for a provider that cannot be read on
    /// anyone's behalf, for an athlete id it refuses, or when the provider
    /// has no default configuration; or the provider's refusal of the
    /// credentials.
    pub async fn create_delegated_coach_reader(
        &self,
        provider_name: &str,
        athlete_id: &str,
        coach_credentials: OAuth2Credentials,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        let provider = self
            .delegated_reads(provider_name)?
            .create_delegated_coach_reader(self.delegated_config(provider_name)?, athlete_id)?;
        provider.set_credentials(coach_credentials).await?;
        Ok(provider)
    }

    /// Create a provider that reads one coached athlete, `athlete_id`, and
    /// writes their training calendar through `coach_credentials`, the
    /// coach account's own stored credential: what a coach's write to a
    /// coached athlete's calendar goes through.
    ///
    /// Only a provider whose descriptor declares both
    /// [`ProviderCapabilities::COACH_ROSTER`] and
    /// [`ProviderCapabilities::CALENDAR_WRITE`]
    /// ([`ProviderCapabilities::supports_coach_calendar_write`]) is built this
    /// way. Whether this coach may write this athlete's calendar — the link,
    /// the athlete's consent, the coach's access on the platform — is the
    /// caller's to decide first. The provider built names `athlete_id` on
    /// every call, as a delegated reader does.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error for a provider whose coach credential
    /// writes no athlete's calendar, for an athlete id it refuses, or when the
    /// provider has no default configuration; or the provider's refusal of
    /// the credentials.
    pub async fn create_delegated_calendar_writer(
        &self,
        provider_name: &str,
        athlete_id: &str,
        coach_credentials: OAuth2Credentials,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        let writes = self
            .get_capabilities(provider_name)
            .is_some_and(|caps| caps.supports_coach_calendar_write());
        if !writes {
            return Err(AppError::invalid_input(format!(
                "{provider_name} does not let a coach write a coached athlete's calendar"
            )));
        }
        let provider = self
            .delegated_reads(provider_name)?
            .create_delegated_calendar_writer(self.delegated_config(provider_name)?, athlete_id)?;
        provider.set_credentials(coach_credentials).await?;
        Ok(provider)
    }

    /// The default configuration a provider built for a coached athlete
    /// starts from.
    fn delegated_config(&self, provider_name: &str) -> AppResult<ProviderConfig> {
        self.default_configs
            .get(provider_name)
            .cloned()
            .ok_or_else(|| {
                AppError::invalid_input(format!(
                    "No default configuration for provider: {provider_name}"
                ))
            })
    }

    /// Every provider a coach account reads its athletes through (those whose
    /// descriptor declares [`ProviderCapabilities::COACH_ROSTER`]) by
    /// registered name, sorted so every call lists them in one order.
    #[must_use]
    pub fn coach_roster_providers(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .descriptors
            .iter()
            .filter(|(_, d)| d.capabilities().supports_coach_roster())
            .map(|(name, _)| *name)
            .collect();
        names.sort_unstable();
        names
    }

    /// The delegated-read capability of `provider_name`: its descriptor
    /// declares [`ProviderCapabilities::COACH_ROSTER`], and its factory says
    /// how to build the provider fixed to one athlete.
    fn delegated_reads(&self, provider_name: &str) -> AppResult<&dyn DelegatedReads> {
        let declared = self
            .get_capabilities(provider_name)
            .is_some_and(|caps| caps.supports_coach_roster());
        if !declared {
            // A tool quotes this refusal to the model: name the provider as
            // the athlete knows it, never a mirror backend.
            return Err(AppError::invalid_input(format!(
                "{} cannot be read on behalf of a coached athlete",
                backend_resolver::user_facing_name(provider_name)
            )));
        }
        self.factories
            .get(provider_name)
            .and_then(|factory| factory.delegated_reads())
            .ok_or_else(|| {
                AppError::internal(format!(
                    "{provider_name} declares a coach roster but its factory builds no delegated reads"
                ))
            })
    }
}
