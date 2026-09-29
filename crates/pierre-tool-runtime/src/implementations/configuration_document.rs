// ABOUTME: The athlete's saved training configuration: an applied profile template plus their own overrides
// ABOUTME: One reading of the stored document for get_ and update_user_configuration, and the keys it refuses

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How the two configuration tools read and write the stored document.
//!
//! A configuration is a profile template from [`ProfileTemplates`] — its
//! values applied as the base parameters — with the athlete's own overrides
//! layered on top. The document records the template by its profile name
//! (`active_profile`) and the overrides; everything else in it is derived from
//! those two on every write and every read, so the template's values reported
//! are always the template's, never a stale copy.
//!
//! Physiological measurements are not configuration: they live on the
//! physiological profile `set_physiology` writes, the one place every
//! training-load computation reads them. [`refuse_measurements`] turns them
//! away so a number saved here can never sit beside, and disagree with, the
//! one training load uses.

use std::collections::BTreeMap;

use chrono::Utc;
use pierre_config::constants::configuration_system::AVAILABLE_PARAMETERS_COUNT;
use pierre_core::config::profiles::{ConfigProfile, ProfileTemplates};
use pierre_core::errors::{AppError, AppResult};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::implementations::configuration_output::{
    ConfigurationDocument, UserConfigurationResult,
};

/// Override keys that belong to the athlete's physiological profile, saved
/// with `set_physiology` rather than here: its twelve fields, then the other
/// spellings a caller reaches for.
pub const MEASUREMENT_KEYS: [&str; 17] = [
    "ftp_watts",
    "threshold_pace_sec_per_km",
    "max_hr",
    "resting_hr",
    "threshold_hr",
    "lactate_threshold_percentage",
    "vo2_max",
    "weight",
    "age",
    "fitness_level",
    "primary_sport",
    "training_experience_years",
    "ftp",
    "lactate_threshold_hr",
    "lactate_threshold",
    "weight_kg",
    "threshold_pace",
];

/// Compose the document for `profile` and `overrides`: the template's values
/// as the base parameters, the overrides on top.
fn compose(
    profile: ConfigProfile,
    overrides: Map<String, Value>,
    last_modified: String,
) -> ConfigurationDocument {
    let profile_parameters: BTreeMap<String, f64> = profile.get_adjustments().into_iter().collect();
    let mut effective_parameters: Map<String, Value> = profile_parameters
        .iter()
        .map(|(key, value)| (key.clone(), json!(value)))
        .collect();
    effective_parameters.extend(overrides.clone());
    ConfigurationDocument {
        profile,
        profile_parameters,
        session_overrides: overrides,
        effective_parameters,
        last_modified,
    }
}

/// The template a saved document names, or the default one.
///
/// A saved `active_profile` that no template answers to recorded a name
/// without applying anything, so the default template is the one in effect.
fn saved_profile(saved: &Value) -> ConfigProfile {
    saved
        .get("active_profile")
        .and_then(Value::as_str)
        .and_then(ProfileTemplates::get)
        .unwrap_or_default()
}

/// The overrides a saved document carries.
fn saved_overrides(saved: &Value) -> Map<String, Value> {
    saved
        .get("session_overrides")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Read a stored configuration document; `None` when the athlete has saved
/// none, which is the default template with no overrides.
#[must_use]
pub fn read_document(stored: Option<&Value>) -> ConfigurationDocument {
    stored.map_or_else(
        || compose(ConfigProfile::Default, Map::new(), Utc::now().to_rfc3339()),
        |saved| {
            let last_modified = saved
                .get("last_modified")
                .and_then(Value::as_str)
                .map_or_else(|| Utc::now().to_rfc3339(), str::to_owned);
            compose(saved_profile(saved), saved_overrides(saved), last_modified)
        },
    )
}

/// Build `get_user_configuration`'s answer from the stored document.
#[must_use]
pub fn configuration_payload(user_id: &Uuid, stored: Option<&Value>) -> UserConfigurationResult {
    let configuration = read_document(stored);
    UserConfigurationResult {
        user_id: user_id.to_string(),
        active_profile: configuration.active_profile(),
        configuration,
        available_parameters: AVAILABLE_PARAMETERS_COUNT,
    }
}

/// Refuse overrides that belong to the physiological profile, naming each
/// and where it is saved.
///
/// # Errors
/// Returns an invalid-input error when `changes` names any of
/// [`MEASUREMENT_KEYS`].
pub fn refuse_measurements(changes: Option<&Map<String, Value>>) -> AppResult<()> {
    let refused: Vec<&str> = changes
        .into_iter()
        .flat_map(Map::keys)
        .map(String::as_str)
        .filter(|key| MEASUREMENT_KEYS.contains(key))
        .collect();
    if refused.is_empty() {
        return Ok(());
    }
    Err(AppError::invalid_input(format!(
        "{} {} to the athlete's physiological profile, not the configuration: save {} with \
         set_physiology, the one place training load, zones and the recovery tools read it",
        refused.join(", "),
        if refused.len() == 1 {
            "belongs"
        } else {
            "belong"
        },
        if refused.len() == 1 { "it" } else { "them" },
    )))
}

/// Resolve the template a call names, refusing a name no template answers to.
///
/// # Errors
/// Returns an invalid-input error listing the templates when `name` matches
/// none of them.
pub fn resolve_template(name: &str) -> AppResult<ConfigProfile> {
    ProfileTemplates::get(name).ok_or_else(|| {
        let names: Vec<String> = ProfileTemplates::all()
            .into_iter()
            .map(|(listed, profile)| format!("{listed} ({})", profile.name()))
            .collect();
        AppError::invalid_input(format!(
            "no configuration profile is called '{name}'; the profiles are {}",
            names.join(", ")
        ))
    })
}

/// The document `update_user_configuration` stores.
///
/// It applies the template named by `profile`, else the saved one, and merges
/// `changes` into the saved overrides: a null removes a key, a key not named
/// keeps its value. Returns the stored JSON and how many things the call changed: one per
/// override set or removed, plus one when a template was named.
///
/// # Errors
/// Returns an invalid-input error for a measurement override or an unknown
/// template name.
pub fn updated_document(
    saved: Option<&Value>,
    profile: Option<&str>,
    changes: Option<&Map<String, Value>>,
) -> AppResult<(Value, usize)> {
    refuse_measurements(changes)?;
    let template = match profile {
        Some(name) => resolve_template(name)?,
        None => saved.map(saved_profile).unwrap_or_default(),
    };
    let mut overrides = saved.map(saved_overrides).unwrap_or_default();
    for (key, value) in changes.into_iter().flatten() {
        if value.is_null() {
            overrides.remove(key);
        } else {
            overrides.insert(key.clone(), value.clone());
        }
    }
    let document = compose(template, overrides, Utc::now().to_rfc3339());
    let stored = json!({
        "active_profile": document.active_profile(),
        "profile": document.profile,
        "profile_parameters": document.profile_parameters,
        "session_overrides": document.session_overrides,
        "effective_parameters": document.effective_parameters,
        "applied_overrides": document.session_overrides.len(),
        "last_modified": document.last_modified,
    });
    let change_count = changes.map_or(0, Map::len) + usize::from(profile.is_some());
    Ok((stored, change_count))
}
