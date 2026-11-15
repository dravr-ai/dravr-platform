// ABOUTME: Provider data retention as admin parameters — one default for every provider plus per-provider days
// ABOUTME: Registers both keys and parses the per-provider map, so the write path and the sweep read one grammar

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider data retention as admin configuration (carnet#1482, D8/D9).
//!
//! The platform keeps one retention system: a default every provider takes
//! ([`DEFAULT_DAYS_KEY`]) and a per-provider override ([`PROVIDER_DAYS_KEY`]),
//! both set with `pierre-cli config set` and read system-wide by the one
//! retention sweep. Nothing in code declares a retention: both start unset,
//! so nothing expires for anyone (JF, 2026-11-10, D9).
//!
//! - `activity_retention.default_days`: an integer; `0` keeps every
//!   provider's data until the athlete deletes it or disconnects.
//! - `activity_retention.provider_days`: a JSON object from provider key to
//!   days, held as its text, e.g. `{"strava":30}`. A provider it names takes
//!   its days over the default, `0` keeping that provider's data forever; a
//!   provider it does not name takes the default.
//!
//! The days count from when a copy was last read from the provider, so a
//! copy every sync reads again never expires. An athlete's own exports are
//! never expired, whatever the policy says.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

use crate::admin_definitions::ParameterDefinition;
use crate::admin_types::{ConfigDataType, ParameterRange};

/// Catalog category both parameters are grouped under.
pub const CATEGORY: &str = "activity_retention";

/// Days every provider's data is kept; `0` keeps it with no expiry.
pub const DEFAULT_DAYS_KEY: &str = "activity_retention.default_days";

/// Days per provider key, overriding the default for the providers it names.
pub const PROVIDER_DAYS_KEY: &str = "activity_retention.provider_days";

/// Longest retention a value may name: ten years. Past it, a value is a
/// typo for "no expiry", which `0` says.
pub const MAX_RETENTION_DAYS: u64 = 3_650;

/// Register both retention parameters.
pub fn register_activity_retention<S: BuildHasher>(
    defs: &mut HashMap<String, ParameterDefinition, S>,
) {
    defs.insert(
        DEFAULT_DAYS_KEY.to_owned(),
        ParameterDefinition {
            key: DEFAULT_DAYS_KEY.to_owned(),
            display_name: "Provider Data Retention (Default)".to_owned(),
            description: "Days a provider's activities and wellness are kept after they were \
                last read from the provider, for every provider activity_retention.provider_days \
                does not name. 0 keeps them with no expiry. An athlete's own exports never \
                expire. Read system-wide only."
                .to_owned(),
            category: CATEGORY.to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0),
                max: serde_json::json!(MAX_RETENTION_DAYS),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("days".to_owned()),
            scientific_basis: None,
            env: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
    defs.insert(
        PROVIDER_DAYS_KEY.to_owned(),
        ParameterDefinition {
            key: PROVIDER_DAYS_KEY.to_owned(),
            display_name: "Provider Data Retention (Per Provider)".to_owned(),
            description: "A JSON object from provider key to days, e.g. {\"strava\":30}: each \
                provider it names keeps its data that many days after the last read instead of \
                the default, 0 meaning no expiry for that provider. A provider it does not name \
                takes activity_retention.default_days. Read system-wide only."
                .to_owned(),
            category: CATEGORY.to_owned(),
            data_type: ConfigDataType::String,
            default_value: serde_json::json!("{}"),
            valid_range: None,
            enum_options: None,
            units: Some("days".to_owned()),
            scientific_basis: None,
            env: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// `text` as the per-provider days it names.
///
/// # Errors
///
/// Why `text` is not a JSON object of lowercase provider keys
/// (`[a-z0-9_]+`) to whole days in `0..=MAX_RETENTION_DAYS`.
pub fn parse_provider_days(text: &str) -> Result<BTreeMap<String, u64>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let serde_json::Value::Object(entries) = value else {
        return Err("expected a JSON object from provider key to days".to_owned());
    };
    let mut days = BTreeMap::new();
    for (provider, value) in entries {
        if provider.is_empty()
            || !provider
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(format!(
                "`{provider}` is not a provider key (lowercase letters, digits, `_`)"
            ));
        }
        let count = value
            .as_u64()
            .filter(|count| *count <= MAX_RETENTION_DAYS)
            .ok_or_else(|| {
                format!(
                    "{provider}: expected whole days from 0 to {MAX_RETENTION_DAYS}, got {value}"
                )
            })?;
        days.insert(provider, count);
    }
    Ok(days)
}

/// Why `value` cannot be stored under `key`, when `key` is a retention
/// parameter whose text has a grammar of its own; `None` otherwise.
#[must_use]
pub fn shape_error(key: &str, value: &serde_json::Value) -> Option<String> {
    if key != PROVIDER_DAYS_KEY {
        return None;
    }
    let text = value.as_str()?;
    parse_provider_days(text).err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_days_read_whole_days_per_lowercase_key() {
        let days = parse_provider_days(r#"{"strava":30,"nolio":7,"garmin":0}"#).unwrap();
        assert_eq!(days.get("strava"), Some(&30), "strava's days are read");
        assert_eq!(days.get("nolio"), Some(&7), "nolio's days are read");
        assert_eq!(days.get("garmin"), Some(&0), "a zero is kept as zero days");
        assert!(
            parse_provider_days("{}").unwrap().is_empty(),
            "an empty map names no provider"
        );
    }

    #[test]
    fn provider_days_refuse_what_is_not_a_map_of_days() {
        for text in [
            "30",
            "[]",
            "not json",
            r#"{"Strava":30}"#,
            r#"{"":30}"#,
            r#"{"strava":-1}"#,
            r#"{"strava":1.5}"#,
            r#"{"strava":"30"}"#,
            r#"{"strava":3651}"#,
        ] {
            assert!(parse_provider_days(text).is_err(), "{text} was accepted");
        }
    }

    #[test]
    fn only_the_provider_map_has_a_shape_of_its_own() {
        assert!(
            shape_error(PROVIDER_DAYS_KEY, &serde_json::json!("{\"strava\":30}")).is_none(),
            "a map of days is a valid shape"
        );
        assert!(
            shape_error(PROVIDER_DAYS_KEY, &serde_json::json!("strava=30")).is_some(),
            "text that is not a map is refused"
        );
        assert!(
            shape_error("anything.else", &serde_json::json!("strava=30")).is_none(),
            "another key has no shape check here"
        );
    }

    #[test]
    fn both_parameters_start_with_no_expiry() {
        let mut defs = HashMap::new();
        register_activity_retention(&mut defs);
        assert_eq!(
            defs[DEFAULT_DAYS_KEY].default_value,
            serde_json::json!(0),
            "the default starts with no expiry"
        );
        let map = defs[PROVIDER_DAYS_KEY].default_value.as_str().unwrap();
        assert!(
            parse_provider_days(map).unwrap().is_empty(),
            "the provider map starts empty"
        );
    }
}
