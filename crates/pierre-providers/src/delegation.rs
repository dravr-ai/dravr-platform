// ABOUTME: Reads on behalf of a coached athlete through a coach's own provider credential, for any coach platform
// ABOUTME: The factory capability that builds such a provider, the coach roster it reads, and the refusals a read answers
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
//!
//! A read of the athlete's settings has a third: `settings_not_shared`, the
//! platform keeping the athlete's settings from the coach while still
//! serving their training. It ends nothing.
//!
//! A coaching platform whose calendar takes writes also lets a coach write a
//! coached athlete's calendar through the same credential
//! ([`ProviderRegistry::create_delegated_calendar_writer`]). Whether this
//! coach may write this athlete's is decided before that provider is built;
//! a write the platform still refuses is `coach_write_refused`, which ends
//! nothing either: the coach may read the athlete without writing for them.

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

/// The [`DELEGATION_REFUSAL_DETAIL`] of an athlete's settings the platform
/// does not show the coach.
const SETTINGS_NOT_SHARED: &str = "settings_not_shared";

/// The [`DELEGATION_REFUSAL_DETAIL`] of a calendar write the platform refused
/// the coach.
#[cfg(feature = "provider-intervals-icu")]
const WRITE_REFUSED: &str = "coach_write_refused";

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

    /// A provider that reads `athlete_id` and writes their training calendar
    /// through the credential the caller then sets on it, the coach's: what
    /// a coach's write to a coached athlete's calendar goes through. Whether
    /// the coach may write it is decided before this is built.
    ///
    /// Refused unless the factory says how: a platform's coach credential
    /// writes nothing of an athlete's by default.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error on a platform whose coach credential
    /// writes no athlete's calendar, the [`Self::check_athlete_id`] refusal,
    /// or the error of building the provider.
    fn create_delegated_calendar_writer(
        &self,
        _config: ProviderConfig,
        _athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        Err(AppError::invalid_input(
            "This platform does not let a coach write a coached athlete's calendar",
        ))
    }

    /// A provider that reads `athlete_id` for their human coach through the
    /// credential the caller then sets on it, the coach's: what the coach's
    /// own read of a coached athlete's planned calendar goes through. Whether
    /// the coach may read it is decided before this is built.
    ///
    /// It reads what [`Self::create_delegated`] reads unless the factory
    /// says how a coach's read differs: a coach never sees less of the
    /// calendar than the athlete, and more only where the platform shows the
    /// coach what it hides from the athlete.
    ///
    /// # Errors
    ///
    /// Returns the [`Self::check_athlete_id`] refusal, or the error of
    /// building the provider.
    fn create_delegated_coach_reader(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        self.create_delegated(config, athlete_id)
    }
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

/// The refusal of a delegated read of an athlete's settings that the coach's
/// `brand` account is not shown, though the athlete still shares their
/// training with it.
///
/// It ends nothing: the athlete is still on the coach's roster, so it is
/// deliberately not [`athlete_off_roster`], which ends the link. Only the
/// Intervals.icu provider builds it, so a build without that provider has
/// none.
#[cfg(feature = "provider-intervals-icu")]
#[must_use]
pub(crate) fn settings_not_shared(brand: &str) -> AppError {
    let mut error = AppError::new(
        ErrorCode::PermissionDenied,
        format!("{brand} does not show this athlete's sport settings to the coach's account"),
    );
    let mut details = Map::new();
    details.insert(
        DELEGATION_REFUSAL_DETAIL.to_owned(),
        Value::from(SETTINGS_NOT_SHARED),
    );
    error.details = Some(Box::new(Value::Object(details)));
    error
}

/// Whether `error` is a delegated settings read the platform did not show the
/// coach (`settings_not_shared`).
#[must_use]
pub fn is_settings_not_shared(error: &AppError) -> bool {
    error
        .details
        .as_ref()
        .and_then(|details| details.get(DELEGATION_REFUSAL_DETAIL))
        .and_then(Value::as_str)
        == Some(SETTINGS_NOT_SHARED)
}

/// The refusal of a write to a coached athlete's calendar that the platform
/// turned down for the coach's `brand` account: the account reads the
/// athlete but may not write their calendar.
///
/// It ends nothing: the athlete still shares their training with the coach,
/// so it is deliberately not [`athlete_off_roster`], which ends the link.
/// Only the Intervals.icu provider builds it, so a build without that
/// provider has none.
#[cfg(feature = "provider-intervals-icu")]
#[must_use]
pub(crate) fn coach_write_refused(brand: &str) -> AppError {
    let mut error = AppError::new(
        ErrorCode::PermissionDenied,
        format!(
            "{brand} refused this calendar write: the coach's {brand} account reads this \
             athlete but may not write their calendar"
        ),
    );
    let mut details = Map::new();
    details.insert(
        DELEGATION_REFUSAL_DETAIL.to_owned(),
        Value::from(WRITE_REFUSED),
    );
    error.details = Some(Box::new(Value::Object(details)));
    error
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

    #[cfg(feature = "provider-intervals-icu")]
    #[test]
    fn settings_the_coach_is_not_shown_never_read_as_an_athlete_off_the_roster() {
        let refused = settings_not_shared("Intervals.icu");
        assert!(is_settings_not_shared(&refused));
        assert!(refused.message.contains("Intervals.icu"));
        // The athlete still shares their training: nothing may end the link.
        assert!(!is_athlete_off_roster(&refused));
        assert!(!is_coach_credential_expired(&refused));
        assert!(!is_settings_not_shared(&athlete_off_roster(
            "Intervals.icu"
        )));
        assert!(!is_settings_not_shared(&AppError::not_found("Activity")));
    }

    #[cfg(feature = "provider-intervals-icu")]
    #[test]
    fn a_refused_calendar_write_ends_nothing_and_names_the_platform() {
        let refused = coach_write_refused("Intervals.icu");
        assert!(refused.code == ErrorCode::PermissionDenied, "{refused:?}");
        assert!(refused.message.contains("Intervals.icu"), "{refused}");
        assert_eq!(
            refused
                .details
                .as_deref()
                .and_then(|details| details.get(DELEGATION_REFUSAL_DETAIL))
                .and_then(Value::as_str),
            Some(WRITE_REFUSED)
        );
        // The athlete still shares their training: the link must survive it.
        assert!(!is_athlete_off_roster(&refused));
        assert!(!is_coach_credential_expired(&refused));
        assert!(!is_settings_not_shared(&refused));
    }
}
