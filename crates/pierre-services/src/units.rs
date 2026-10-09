// ABOUTME: One resolver for "does this athlete read kilometres or miles" — the REST surface and the agent's prompt both ask it
// ABOUTME: Reads the stored inputs and the profile locale, then applies pierre_core's resolve_units; also records a provider's setting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete unit resolution (carnet#835).
//!
//! [`pierre_core::models::resolve_units`] is the rule; this module feeds it.
//! The locale rung reads the locale the athlete's device last reported, which
//! carries a region (`en-US`), and falls back to the profile language
//! ([`crate::locale::resolve_user_locale`]) when no device has reported one —
//! a bare language, so that fallback reads metric.

use pierre_core::models::{resolve_units, Athlete, ResolvedUnits};
use pierre_database::repositories::{
    StoredUnitPreferences, UnitPreferencesRepository, UserRepository,
};
use tracing::warn;
use uuid::Uuid;

use crate::locale::resolve_user_locale;

/// What one athlete's units are, and the stored inputs that decided them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserUnits {
    /// The stored inputs.
    pub stored: StoredUnitPreferences,
    /// The effective system and the rung that decided it.
    pub resolved: ResolvedUnits,
}

/// Resolve `user_id`'s unit system.
///
/// `device_locale` is the locale the caller's device reports on this request,
/// when it sent one; it stands in for the stored one so the answer is right on
/// the request that first reports it. Never fails: an unreadable row reads as
/// nothing stored, so the athlete gets their locale's units rather than an
/// error on every page that prints a distance.
pub async fn resolve_user_units(
    users: &dyn UserRepository,
    unit_preferences: &dyn UnitPreferencesRepository,
    user_id: Uuid,
    device_locale: Option<&str>,
) -> UserUnits {
    let stored = match unit_preferences.get_unit_preferences(user_id).await {
        Ok(stored) => stored,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Unit preferences unreadable; resolving from locale");
            StoredUnitPreferences::default()
        }
    };
    let locale = match device_locale.or(stored.device_locale.as_deref()) {
        Some(locale) => locale.to_owned(),
        None => resolve_user_locale(users, user_id).await,
    };
    let resolved = resolve_units(stored.preference, stored.provider_units, &locale);
    UserUnits { stored, resolved }
}

/// Record the unit system a provider's athlete profile carries, if any.
///
/// A provider read is the only place this setting is seen, so a
/// failure to store it is logged and the read goes on: it costs the athlete
/// nothing but the provider rung until the next read.
pub async fn record_provider_units(
    unit_preferences: &dyn UnitPreferencesRepository,
    user_id: Uuid,
    athlete: &Athlete,
) {
    let Some(units) = athlete.preferred_units else {
        return;
    };
    if let Err(e) = unit_preferences
        .set_provider_units(user_id, &athlete.provider, units)
        .await
    {
        warn!(
            user_id = %user_id,
            provider = %athlete.provider,
            error = %e,
            "Could not record the provider's unit setting"
        );
    }
}
