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
}

impl FeatureKey {
    /// All known flags, in stable order for admin UI rendering.
    pub const ALL: &'static [Self] = &[
        Self::ApiTokens,
        Self::BillingHeader,
        Self::PersonaNotificationPolicy,
    ];

    /// Storage key (matches the `feature_key` column and the JSON field
    /// returned to the frontend).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApiTokens => "api_tokens",
            Self::BillingHeader => "billing_header",
            Self::PersonaNotificationPolicy => "persona_notification_policy",
        }
    }

    /// Compile-time fallback when no row exists at either tenant or user
    /// level. Keep defaults pessimistic (off) — flags are an opt-in surface.
    #[must_use]
    pub const fn default_enabled(self) -> bool {
        match self {
            Self::ApiTokens | Self::BillingHeader | Self::PersonaNotificationPolicy => false,
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
            other => Err(UnknownFeatureKey(other.to_owned())),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_round_trip_through_str() {
        for &key in FeatureKey::ALL {
            let s = key.as_str();
            assert_eq!(FeatureKey::from_str(s).unwrap(), key);
        }
    }

    #[test]
    fn all_constant_matches_variants() {
        // If a new variant is added without updating ALL, this test catches it.
        let mut from_all: Vec<&str> = FeatureKey::ALL.iter().map(|k| k.as_str()).collect();
        from_all.sort_unstable();
        let mut expected = vec![
            "api_tokens",
            "billing_header",
            "persona_notification_policy",
        ];
        expected.sort_unstable();
        assert_eq!(from_all, expected);
    }

    #[test]
    fn defaults_are_all_disabled() {
        for &key in FeatureKey::ALL {
            assert!(
                !key.default_enabled(),
                "{key} should default to disabled; new flags must opt in by admin"
            );
        }
    }

    #[test]
    fn unknown_key_errors() {
        let err = FeatureKey::from_str("does_not_exist").unwrap_err();
        assert_eq!(err.0, "does_not_exist");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(FeatureKey::ApiTokens.to_string(), "api_tokens");
        assert_eq!(FeatureKey::BillingHeader.to_string(), "billing_header");
        assert_eq!(
            FeatureKey::PersonaNotificationPolicy.to_string(),
            "persona_notification_policy"
        );
    }

    #[test]
    fn serde_uses_snake_case() {
        let json = serde_json::to_string(&FeatureKey::ApiTokens).unwrap();
        assert_eq!(json, "\"api_tokens\"");
        let back: FeatureKey = serde_json::from_str("\"billing_header\"").unwrap();
        assert_eq!(back, FeatureKey::BillingHeader);
    }
}
