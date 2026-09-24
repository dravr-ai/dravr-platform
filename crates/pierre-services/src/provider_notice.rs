// ABOUTME: The exposure-notice version a user must have accepted before a provider login, or none
// ABOUTME: Combines the provider's notice version with the provider_exposure_notice feature flag

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which exposure notice is in force for one account.
//!
//! `TrainingPeaks` and COROS are read through the athlete's own signed-in
//! session, which their terms forbid for third parties, so each carries a
//! notice version ([`provider_terms_version`]). Whether an account is asked
//! for it is the `provider_exposure_notice` feature flag: off by default, so a
//! demo account connects without it, and armed per tenant or per user for the
//! athletes an operator onboards. Every surface that asks for the notice, and
//! the login that refuses without it, reads this one answer.

use pierre_core::constants::oauth_providers::provider_terms_version;
use pierre_core::feature_flags::FeatureKey;
use pierre_database::RepositoryRegistry;
use tracing::debug;
use uuid::Uuid;

/// The notice version `user_id` must accept before connecting `backend`.
///
/// `None` when no notice applies: the backend carries none, or the
/// `provider_exposure_notice` flag is off for this account.
///
/// A flag read that fails resolves to the flag's compile default (off), as
/// every feature flag does.
pub async fn notice_in_force(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
    backend: &str,
) -> Option<&'static str> {
    let version = provider_terms_version(backend)?;
    let armed = match repos
        .feature_flags
        .resolve_for_user(tenant_id, user_id)
        .await
    {
        Ok(flags) => flags
            .get(&FeatureKey::ProviderExposureNotice)
            .copied()
            .unwrap_or_else(|| FeatureKey::ProviderExposureNotice.default_enabled()),
        Err(e) => {
            debug!(%user_id, error = %e, "exposure notice flag unreadable; compile default applies");
            FeatureKey::ProviderExposureNotice.default_enabled()
        }
    };
    armed.then_some(version)
}
