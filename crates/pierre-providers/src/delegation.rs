// ABOUTME: Reads on behalf of a coached athlete through a coach's own provider credential, for any coach platform
// ABOUTME: The factory capability that builds such a provider, the coach roster it reads, and the two refusals a read answers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Delegated reads
//!
//! A coaching platform (`TrainingPeaks`, Intervals.icu) lets a coach's own
//! account read the athletes who share with that coach. A group member a
//! coach linked, and who confirmed the link, is read that way: through the
//! coach's stored credential, naming the member's athlete id on every read.
//!
//! A provider that can be read this way says so in its descriptor, which
//! declares [`ProviderCapabilities::COACH_ROSTER`], and its factory's
//! [`DelegatedReads`] capability builds a provider fixed to one athlete from
//! the coach's credential, an OAuth grant or an API key alike
//! ([`ProviderRegistry::create_delegated_provider`]). Such a provider reads
//! nothing outside that athlete, and its failures take one of two shapes a
//! caller acts on:
//!
//! - [`coach_credential_expired`]: the coach's credential no longer works.
//!   Only the coach can renew it, so it is not the reader's reconnect error.
//! - [`athlete_off_roster`]: the platform no longer lets the coach read the
//!   athlete, so the link through the coach ends.

#[cfg(feature = "provider-sciotte")]
use dravr_sciotte::wire::ATHLETE_NOT_ACCESSIBLE;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::{FitnessProvider, ProviderConfig};
use crate::errors::{AppError, AppResult, ErrorCode};
use crate::models::RosterAthlete;
#[cfg(doc)]
use crate::registry::ProviderRegistry;
#[cfg(feature = "provider-sciotte")]
use crate::sciotte_error::sciotte_refusal;
#[cfg(doc)]
use crate::spi::ProviderCapabilities;

/// Details key marking an error as the failure of the coach's credential
/// behind a delegated read, read back by [`is_coach_credential_expired`].
const DELEGATED_DETAIL: &str = "delegated";

/// Details key carrying a delegated read's refusal, read back by
/// [`is_athlete_off_roster`].
const DELEGATION_REFUSAL_DETAIL: &str = "delegation_refusal";

/// The [`DELEGATION_REFUSAL_DETAIL`] of an athlete the coach may no longer
/// read.
const OFF_ROSTER: &str = "athlete_not_accessible";

/// The athletes a coach account can read, as its platform lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoachRoster {
    /// The coach account's own email on the platform, when it shares one:
    /// what binds the account to the coach's Dravr account.
    pub account_email: Option<String>,
    /// The athletes the account coaches, the account itself left out.
    pub athletes: Vec<RosterAthlete>,
}

/// The capability of a provider factory whose provider reads one coached
/// athlete through a coach's own credential.
///
/// A factory offers it through
/// [`ProviderFactory::delegated_reads`](crate::core::ProviderFactory::delegated_reads),
/// and its descriptor declares [`ProviderCapabilities::COACH_ROSTER`]; a
/// provider without both cannot be read on behalf of anyone.
pub trait DelegatedReads: Send + Sync {
    /// Refuse an athlete id the platform could not have issued, before it
    /// reaches a URL or a stored link.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error naming why the id is refused.
    fn check_athlete_id(&self, athlete_id: &str) -> AppResult<()>;

    /// A provider that reads `athlete_id` through the credential the caller
    /// then sets on it, which is the coach's. Every read it serves names
    /// that athlete, and a detail read of anything outside that athlete is
    /// refused as not found.
    ///
    /// # Errors
    ///
    /// Returns the [`Self::check_athlete_id`] refusal, or the error of
    /// building the provider.
    fn create_delegated(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>>;
}

/// The error a delegated read answers when the credential it goes through —
/// the coach's, not the reader's — no longer works.
///
/// It is deliberately not [`AppError::provider_auth_required`]: that code
/// sends the reader through a reconnect and lets a sweep flag the reader's
/// own connection, and neither fixes a credential only the coach can renew.
/// It is an [`ErrorCode::ExternalAuthFailed`] naming `backend`, with
/// `delegated` set in its details so a caller can tell it from any other.
/// `brand` is the platform as the athlete knows it.
#[must_use]
pub fn coach_credential_expired(backend: &str, brand: &str) -> AppError {
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

/// Whether `error` is a delegated read's dead coach credential
/// ([`coach_credential_expired`]).
#[must_use]
pub fn is_coach_credential_expired(error: &AppError) -> bool {
    matches!(error.code, ErrorCode::ExternalAuthFailed)
        && error
            .details
            .as_ref()
            .and_then(|details| details.get(DELEGATED_DETAIL))
            .and_then(Value::as_bool)
            == Some(true)
}

/// The refusal of a delegated read naming an athlete the coach's `brand`
/// account may no longer read: the platform no longer lists them with the
/// coach.
#[must_use]
pub fn athlete_off_roster(brand: &str) -> AppError {
    let mut error = AppError::new(
        ErrorCode::PermissionDenied,
        format!("That athlete no longer shares their {brand} account with this coach"),
    );
    let mut details = Map::new();
    details.insert(
        DELEGATION_REFUSAL_DETAIL.to_owned(),
        Value::from(OFF_ROSTER),
    );
    error.details = Some(Box::new(Value::Object(details)));
    error
}

/// Whether `error` says the coach may no longer read the athlete a delegated
/// read named: [`athlete_off_roster`], or the scraper's `athlete_not_accessible`
/// refusal of a `TrainingPeaks` read.
#[must_use]
pub fn is_athlete_off_roster(error: &AppError) -> bool {
    let marked = error
        .details
        .as_ref()
        .and_then(|details| details.get(DELEGATION_REFUSAL_DETAIL))
        .and_then(Value::as_str)
        == Some(OFF_ROSTER);
    marked || scraper_off_roster(error)
}

/// Whether the scraper refused the athlete a `TrainingPeaks` read named.
#[cfg(feature = "provider-sciotte")]
fn scraper_off_roster(error: &AppError) -> bool {
    sciotte_refusal(error) == Some(ATHLETE_NOT_ACCESSIBLE)
}

/// A build without the scraper has no scraper refusal to read.
#[cfg(not(feature = "provider-sciotte"))]
const fn scraper_off_roster(_error: &AppError) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_coach_credential_failure_is_told_apart_from_every_other_auth_error() {
        let expired = coach_credential_expired("intervals_icu", "Intervals.icu");
        assert!(is_coach_credential_expired(&expired));
        assert!(expired.message.contains("Intervals.icu"));
        assert!(!is_coach_credential_expired(
            &AppError::provider_auth_required("intervals_icu")
        ));
        assert!(!is_coach_credential_expired(&AppError::new(
            ErrorCode::ExternalAuthFailed,
            "a different auth failure"
        )));
    }

    #[test]
    fn an_off_roster_refusal_is_recognised_and_nothing_else_is() {
        assert!(is_athlete_off_roster(&athlete_off_roster("Intervals.icu")));
        assert!(!is_athlete_off_roster(&AppError::not_found("Activity")));
        assert!(!is_athlete_off_roster(&coach_credential_expired(
            "intervals_icu",
            "Intervals.icu"
        )));
    }
}
