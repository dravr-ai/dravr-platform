// ABOUTME: The refusal of a provider-context read by a provider that serves none, and the test that tells it from a failed read
// ABOUTME: Context is the provider's own zones, gear, records, best efforts, weight and paged search; absent is never an empty answer

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider context reads (carnet#1622).
//!
//! `FitnessProvider`'s context reads default to [`unsupported`]: a provider
//! that keeps no zones, gear, records or weight of its own says so, naming
//! itself the way the athlete knows it. An empty answer would read as an
//! athlete with no gear, so the default is never `Ok`. A caller tells that
//! refusal from a read that failed with [`is_unsupported`].

use crate::backend_resolver::{is_mirror_backend, user_facing_name};
use crate::errors::{AppError, ErrorCode};
use pierre_core::constants::oauth_providers;

/// The details key marking an [`unsupported`] refusal.
const UNSUPPORTED_DETAIL: &str = "provider_context_unsupported";

/// The refusal of a context read (`what`: "activity zones", "gear") by a
/// provider that serves none.
///
/// Names the provider as the athlete knows it, never a backend slug
/// (`sciotte_garmin`). It is the caller's misuse of a provider without the
/// read, not a failure of the provider: nothing should retry or reconnect.
#[must_use]
pub fn unsupported(provider: &str, what: &str) -> AppError {
    let mut error = AppError::new(
        ErrorCode::InvalidInput,
        format!("{} does not expose {what}", user_facing_name(provider)),
    );
    let mut details = serde_json::Map::new();
    details.insert(UNSUPPORTED_DETAIL.to_owned(), serde_json::Value::Bool(true));
    error.details = Some(Box::new(serde_json::Value::Object(details)));
    error
}

/// Whether `backend` serves provider context reads at all.
///
/// Decided from its name before any I/O: only the Strava mirror does. A
/// caller skips authenticating and reading the gear ledger for every other backend, whose
/// provider would answer [`unsupported`].
#[must_use]
pub fn serves_context(backend: &str) -> bool {
    is_mirror_backend(backend) && user_facing_name(backend) == oauth_providers::STRAVA
}

/// Whether `error` is the refusal of a provider that serves no such context
/// ([`unsupported`]), never a read that failed.
#[must_use]
pub fn is_unsupported(error: &AppError) -> bool {
    error.code == ErrorCode::InvalidInput
        && error
            .details
            .as_ref()
            .is_some_and(|details| details.get(UNSUPPORTED_DETAIL).is_some())
}

#[cfg(test)]
mod tests {
    use super::{is_unsupported, serves_context, unsupported};
    use crate::errors::{AppError, ErrorCode};

    #[test]
    fn only_the_strava_mirror_serves_context() {
        assert!(
            serves_context("sciotte"),
            "bug: the Strava mirror is not asked for context"
        );
        for backend in [
            "sciotte_garmin",
            "sciotte_trainingpeaks",
            "strava",
            "garmin",
            "whoop",
        ] {
            assert!(
                !serves_context(backend),
                "bug: {backend} is asked for context"
            );
        }
    }

    #[test]
    fn the_refusal_names_the_service_and_is_recognised() {
        let refusal = unsupported("sciotte_garmin", "activity zones");
        assert!(
            refusal.message.contains("does not expose activity zones")
                && !refusal.message.contains("sciotte"),
            "bug: the refusal leaks the backend slug or loses what was asked: {refusal}"
        );
        assert!(
            is_unsupported(&refusal),
            "bug: the refusal is not recognised as one"
        );
    }

    #[test]
    fn a_failed_read_is_not_the_refusal() {
        for error in [
            AppError::invalid_input("bad id"),
            AppError::new(ErrorCode::ExternalServiceUnavailable, "down"),
        ] {
            assert!(
                !is_unsupported(&error),
                "bug: a failed read passed for an unsupported provider: {error}"
            );
        }
    }
}
