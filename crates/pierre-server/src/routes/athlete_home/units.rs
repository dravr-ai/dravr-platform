// ABOUTME: GET and PUT /api/me/units — the unit system the athlete reads distances in, and the Settings choice behind it
// ABOUTME: Resolved in one place: the explicit choice, then the provider's own setting, then the device locale (carnet#835)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's units.
//!
//! - `GET /api/me/units?device_locale=en-US` — the effective unit system,
//!   which rung decided it, and the stored inputs. `device_locale` is the
//!   locale the calling device reports; it decides the locale rung of this
//!   answer without being stored, so a read never writes.
//! - `PUT /api/me/units` — store the athlete's Settings choice
//!   (`automatic`, `metric` or `imperial`), the locale their device reports,
//!   or both, answering as `GET` does. Each is stored only when sent, so a
//!   device reporting its locale never rewrites the choice made on another.
//!   The agent reads the stored locale, so a client stores it whenever it
//!   differs from what is stored. A body naming neither is refused.
//!
//! Stored server-side, keyed by the user alone, so the web, the phone and the
//! agent's replies agree whichever device the choice was made on.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{UnitPreference, UnitSource, UnitSystem};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::units::{resolve_user_units, UserUnits};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The longest locale tag stored: BCP 47's own practical bound.
const MAX_LOCALE_TAG_LEN: usize = 35;

/// Body of `GET` and `PUT /api/me/units`. Every key is always present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitsBody {
    /// The athlete's choice in Settings.
    pub preference: UnitPreference,
    /// The system everything the athlete reads is written in.
    pub units: UnitSystem,
    /// Which rung decided [`Self::units`].
    pub source: UnitSource,
    /// The provider whose own unit setting is stored, or null.
    pub provider: Option<String>,
    /// That provider's setting, or null when none was read.
    pub provider_units: Option<UnitSystem>,
    /// The locale tag the athlete's device last stored, or null.
    pub device_locale: Option<String>,
}

impl From<UserUnits> for UnitsBody {
    fn from(units: UserUnits) -> Self {
        Self {
            preference: units.stored.preference,
            units: units.resolved.system,
            source: units.resolved.source,
            provider: units.stored.provider,
            provider_units: units.stored.provider_units,
            device_locale: units.stored.device_locale,
        }
    }
}

/// Query of `GET /api/me/units`.
#[derive(Debug, Deserialize)]
pub struct UnitsQuery {
    /// The locale the calling device reports (`en-US`).
    pub device_locale: Option<String>,
}

/// Body of `PUT /api/me/units`. At least one key is present.
#[derive(Debug, Deserialize)]
pub struct UnitsUpdate {
    /// The athlete's choice in Settings, stored when sent.
    pub preference: Option<UnitPreference>,
    /// The locale the athlete's device reports, stored for the agent.
    pub device_locale: Option<String>,
}

/// A locale tag as a device reports it: letters, digits, `-` and `_`, at most
/// [`MAX_LOCALE_TAG_LEN`] long. Anything else is refused rather than stored.
fn checked_locale(tag: Option<String>) -> AppResult<Option<String>> {
    let Some(tag) = tag else { return Ok(None) };
    let tag = tag.trim().to_owned();
    let well_formed = !tag.is_empty()
        && tag.len() <= MAX_LOCALE_TAG_LEN
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if well_formed {
        Ok(Some(tag))
    } else {
        Err(AppError::invalid_input("device_locale is not a locale tag"))
    }
}

async fn answer(
    resources: &ServerContext,
    user_id: Uuid,
    device_locale: Option<&str>,
) -> UnitsBody {
    let repos = &resources.common.repos;
    resolve_user_units(
        repos.users.as_ref(),
        repos.unit_preferences.as_ref(),
        user_id,
        device_locale,
    )
    .await
    .into()
}

/// `GET /api/me/units`.
pub(super) async fn get_units(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<UnitsQuery>,
) -> AppResult<Json<UnitsBody>> {
    let device_locale = checked_locale(query.device_locale)?;
    Ok(Json(
        answer(&resources, auth.user_id, device_locale.as_deref()).await,
    ))
}

/// `PUT /api/me/units`.
pub(super) async fn put_units(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(update): Json<UnitsUpdate>,
) -> AppResult<Json<UnitsBody>> {
    let device_locale = checked_locale(update.device_locale)?;
    if update.preference.is_none() && device_locale.is_none() {
        return Err(AppError::invalid_input(
            "a units update names a preference, a device_locale or both",
        ));
    }
    let repos = &resources.common.repos;
    if let Some(preference) = update.preference {
        repos
            .unit_preferences
            .set_unit_preference(auth.user_id, preference)
            .await?;
    }
    if let Some(device_locale) = &device_locale {
        repos
            .unit_preferences
            .set_device_locale(auth.user_id, device_locale)
            .await?;
    }
    Ok(Json(answer(&resources, auth.user_id, None).await))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_locale_tag_is_accepted_trimmed() {
        assert_eq!(
            checked_locale(Some(" en-US ".to_owned())).unwrap(),
            Some("en-US".to_owned())
        );
        assert_eq!(
            checked_locale(Some("pt_BR".to_owned())).unwrap(),
            Some("pt_BR".to_owned())
        );
        assert_eq!(checked_locale(None).unwrap(), None);
    }

    #[test]
    fn anything_but_a_locale_tag_is_refused() {
        for tag in [
            "",
            "en US",
            "en-US;drop",
            &"x".repeat(MAX_LOCALE_TAG_LEN + 1),
        ] {
            assert!(checked_locale(Some(tag.to_owned())).is_err(), "{tag:?}");
        }
    }
}
