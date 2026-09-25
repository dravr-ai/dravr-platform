// ABOUTME: Runtime feature flag registry — known keys + compile-time defaults
// ABOUTME: Resolution is per-user override > tenant default > FeatureKey::default_enabled()
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Feature Flags
//!
//! Compile-time-known feature keys with per-key default values. The storage
//! layer (`pierre-database::FeatureFlagsRepository`) holds tenant defaults
//! and per-user overrides; this module is the single source of truth for
//! which keys exist and what their fallback value is when nothing is set.
//!
//! ## Adding a flag
//!
//! 1. Add a variant to [`FeatureKey`].
//! 2. Wire the variant into [`FeatureKey::ALL`], [`FeatureKey::as_str`],
//!    [`FeatureKey::from_str`], [`FeatureKey::default_enabled`], and
//!    [`FeatureKey::description`].
//! 3. Read it on the frontend from `GET /api/me/features` as
//!    `flags[FeatureKey::as_str()]`.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::str::FromStr;

/// Known runtime feature flags.
///
/// Every variant must appear in [`FeatureKey::ALL`] for the admin UI to
/// surface it and for `/api/me/features` to return a value for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKey {
    /// User-facing API Tokens settings tab (create personal MCP tokens for
    /// Claude Desktop / Cursor). Disabled by default while the surface is
    /// being stabilized; admins flip on per-tenant or per-user.
    ApiTokens,
    /// "Current Plan" / upgrade card on the Billing (now "Usage") screen.
    /// Disabled by default until paid plans go live; admins flip on once a
    /// tenant is wired to a billing provider.
    BillingHeader,
    /// Arm persona notification-policy enforcement: a push whose tier falls
    /// above the user's persona floor is persisted for the weekly digest
    /// instead of delivered. Disabled by default — the gate runs in shadow
    /// mode (verdict logs only) until an operator arms it per tenant or user.
    PersonaNotificationPolicy,
    /// State the notice a provider requires before connecting — the exposure
    /// notice of a provider read through the account's own signed-in session
    /// (`TrainingPeaks`, COROS), WHOOP's owner authorization — and refuse the
    /// connect until it is accepted; health sync keeps no WHOOP record for an
    /// account it arms that has not accepted WHOOP's. Disabled by default so a
    /// demo account connects without it; admins arm it per tenant or per user
    /// for the athletes they onboard.
    ProviderExposureNotice,
}

impl FeatureKey {
    /// All known flags, in stable order for admin UI rendering.
    pub const ALL: &'static [Self] = &[
        Self::ApiTokens,
        Self::BillingHeader,
        Self::PersonaNotificationPolicy,
        Self::ProviderExposureNotice,
    ];

    /// Storage key (matches the `feature_key` column and the JSON field
    /// returned to the frontend).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApiTokens => "api_tokens",
            Self::BillingHeader => "billing_header",
            Self::PersonaNotificationPolicy => "persona_notification_policy",
            Self::ProviderExposureNotice => "provider_exposure_notice",
        }
    }

    /// Compile-time fallback when no row exists at either tenant or user
    /// level. Keep defaults pessimistic (off) — flags are an opt-in surface.
    #[must_use]
    pub const fn default_enabled(self) -> bool {
        match self {
            Self::ApiTokens
            | Self::BillingHeader
            | Self::PersonaNotificationPolicy
            | Self::ProviderExposureNotice => false,
        }
    }

    /// Human-readable description shown in the admin UI next to the toggle.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::ApiTokens => {
                "Show the API Tokens tab in user Settings (personal MCP tokens for Claude Desktop / Cursor)."
            }
            Self::BillingHeader => {
                "Show the Current Plan / upgrade card on the Usage screen."
            }
            Self::PersonaNotificationPolicy => {
                "Enforce the persona push-tier floor (gated pushes are persisted and rolled into the weekly digest instead of delivered)."
            }
            Self::ProviderExposureNotice => {
                "Ask for the provider notice before a TrainingPeaks or COROS login or a WHOOP connect, refuse the connect until it is accepted, and keep no WHOOP record for an account that has not accepted it."
            }
        }
    }
}

impl fmt::Display for FeatureKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by [`FeatureKey::from_str`] when the input does not match
/// any known flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFeatureKey(pub String);

impl fmt::Display for UnknownFeatureKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown feature key '{}'", self.0)
    }
}

impl Error for UnknownFeatureKey {}

impl FromStr for FeatureKey {
    type Err = UnknownFeatureKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "api_tokens" => Ok(Self::ApiTokens),
            "billing_header" => Ok(Self::BillingHeader),
            "persona_notification_policy" => Ok(Self::PersonaNotificationPolicy),
            "provider_exposure_notice" => Ok(Self::ProviderExposureNotice),
            other => Err(UnknownFeatureKey(other.to_owned())),
        }
    }
}
