// ABOUTME: How the coach reacts to a synced workout, as admin parameters resolved per athlete
// ABOUTME: Registers activity_landed.frequency and .digest and parses a stored value into its mode

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach's reaction to a synced workout as admin configuration (carnet#1486).
//!
//! JF, 2026-11-18: "make the mechanism configurable via ServerConfig,
//! pierre-cli per user".
//!
//! - `activity_landed.frequency`: `per_day` posts at most once per
//!   athlete-local day, on the first landing that passes the post's gates;
//!   `per_ride` posts once per landing, with no daily limit.
//! - `activity_landed.digest`: for an athlete with a coach thread whose
//!   persona holds notifications for a per-session digest, `replace_digest`
//!   sends the landing post instead of the digest, `keep_digest` sends both.
//!
//! Both are read for each landing, user → tenant → system-wide → default, so
//! `pierre-cli config set <key> <value> --user <email>` changes one athlete
//! from their next landing, with no restart.

use std::collections::HashMap;
use std::hash::BuildHasher;

use tracing::warn;

use crate::admin_definitions::ParameterDefinition;
use crate::admin_types::ConfigDataType;

/// Catalog category both parameters are grouped under.
pub const CATEGORY: &str = "activity_landed";

/// How often the coach reacts to a landing: `per_day` or `per_ride`.
pub const FREQUENCY_KEY: &str = "activity_landed.frequency";

/// Whether a landing post replaces the per-session digest or goes beside it.
pub const DIGEST_KEY: &str = "activity_landed.digest";

const PER_DAY: &str = "per_day";
const PER_RIDE: &str = "per_ride";
const REPLACE_DIGEST: &str = "replace_digest";
const KEEP_DIGEST: &str = "keep_digest";

/// How often the coach reacts to a workout a live sync landed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LandingFrequency {
    /// At most one reaction per athlete-local day.
    #[default]
    PerDay,
    /// One reaction per landing, with no daily limit.
    PerRide,
}

impl LandingFrequency {
    /// The mode a stored value names. An unset value, a JSON null or a word
    /// outside the options reads as the default (logged when it was set).
    #[must_use]
    pub fn from_value(value: Option<&serde_json::Value>) -> Self {
        match enum_word(FREQUENCY_KEY, value) {
            Some(PER_RIDE) => Self::PerRide,
            Some(PER_DAY) | None => Self::PerDay,
            Some(other) => {
                warn!(
                    key = FREQUENCY_KEY,
                    value = other,
                    "unknown value; using per_day"
                );
                Self::PerDay
            }
        }
    }
}

/// Whether a stored landing post replaces the per-session digest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LandingDigest {
    /// The post goes instead of the digest; held notifications return at the
    /// next landing that does not post.
    #[default]
    ReplaceDigest,
    /// The post and the digest both go.
    KeepDigest,
}

impl LandingDigest {
    /// The mode a stored value names. An unset value, a JSON null or a word
    /// outside the options reads as the default (logged when it was set).
    #[must_use]
    pub fn from_value(value: Option<&serde_json::Value>) -> Self {
        match enum_word(DIGEST_KEY, value) {
            Some(KEEP_DIGEST) => Self::KeepDigest,
            Some(REPLACE_DIGEST) | None => Self::ReplaceDigest,
            Some(other) => {
                warn!(
                    key = DIGEST_KEY,
                    value = other,
                    "unknown value; using replace_digest"
                );
                Self::ReplaceDigest
            }
        }
    }
}

/// The word `value` holds; `None` when it is unset or null, and the key is
/// logged when it holds something that is not a string.
fn enum_word<'v>(key: &str, value: Option<&'v serde_json::Value>) -> Option<&'v str> {
    match value? {
        serde_json::Value::Null => None,
        serde_json::Value::String(word) => Some(word.as_str()),
        other => {
            warn!(key, value = %other, "not a string; using the default");
            None
        }
    }
}

/// Register both landing parameters.
pub fn register_activity_landed<S: BuildHasher>(
    defs: &mut HashMap<String, ParameterDefinition, S>,
) {
    defs.insert(
        FREQUENCY_KEY.to_owned(),
        ParameterDefinition {
            key: FREQUENCY_KEY.to_owned(),
            display_name: "Coach Landing Post Frequency".to_owned(),
            description: "per_day: at most one coach post per athlete-local day, on the first \
                landing that passes the gates. per_ride: one post per landing, with no daily \
                limit. A sync that brings several new workouts at once is one landing, about the \
                newest. Resolved per athlete: user, then tenant, then system-wide."
                .to_owned(),
            category: CATEGORY.to_owned(),
            data_type: ConfigDataType::Enum,
            default_value: serde_json::json!(PER_DAY),
            valid_range: None,
            enum_options: Some(vec![PER_DAY.to_owned(), PER_RIDE.to_owned()]),
            units: None,
            scientific_basis: None,
            env: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
    defs.insert(
        DIGEST_KEY.to_owned(),
        ParameterDefinition {
            key: DIGEST_KEY.to_owned(),
            display_name: "Coach Landing Post and Digest".to_owned(),
            description: "For an athlete with a coach thread whose persona holds notifications \
                for a per_session digest: replace_digest sends the landing post instead of the \
                digest, and the held notifications return at the next landing that does not \
                post. keep_digest sends both. An athlete with no coach thread always gets the \
                digest."
                .to_owned(),
            category: CATEGORY.to_owned(),
            data_type: ConfigDataType::Enum,
            default_value: serde_json::json!(REPLACE_DIGEST),
            valid_range: None,
            enum_options: Some(vec![REPLACE_DIGEST.to_owned(), KEEP_DIGEST.to_owned()]),
            units: None,
            scientific_basis: None,
            env: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_null_or_unknown_value_reads_as_the_default() {
        for value in [
            None,
            Some(serde_json::Value::Null),
            Some(serde_json::json!("hourly")),
            Some(serde_json::json!(3)),
        ] {
            assert_eq!(
                LandingFrequency::from_value(value.as_ref()),
                LandingFrequency::PerDay,
                "bug: {value:?} must read as per_day"
            );
            assert_eq!(
                LandingDigest::from_value(value.as_ref()),
                LandingDigest::ReplaceDigest,
                "bug: {value:?} must read as replace_digest"
            );
        }
    }

    #[test]
    fn each_option_reads_as_its_mode() {
        assert_eq!(
            LandingFrequency::from_value(Some(&serde_json::json!("per_ride"))),
            LandingFrequency::PerRide,
            "bug: per_ride must read as PerRide"
        );
        assert_eq!(
            LandingFrequency::from_value(Some(&serde_json::json!("per_day"))),
            LandingFrequency::PerDay,
            "bug: per_day must read as PerDay"
        );
        assert_eq!(
            LandingDigest::from_value(Some(&serde_json::json!("keep_digest"))),
            LandingDigest::KeepDigest,
            "bug: keep_digest must read as KeepDigest"
        );
        assert_eq!(
            LandingDigest::from_value(Some(&serde_json::json!("replace_digest"))),
            LandingDigest::ReplaceDigest,
            "bug: replace_digest must read as ReplaceDigest"
        );
    }

    #[test]
    fn both_parameters_register_as_runtime_enums_with_two_options() {
        let mut defs = HashMap::new();
        register_activity_landed(&mut defs);
        for (key, default, options) in [
            (FREQUENCY_KEY, PER_DAY, [PER_DAY, PER_RIDE]),
            (DIGEST_KEY, REPLACE_DIGEST, [REPLACE_DIGEST, KEEP_DIGEST]),
        ] {
            let def = &defs[key];
            assert_eq!(
                def.key, key,
                "bug: {key} must be keyed by its full dotted name"
            );
            assert_eq!(def.category, CATEGORY, "bug: {key} must sit in {CATEGORY}");
            assert_eq!(
                def.data_type,
                ConfigDataType::Enum,
                "bug: {key} must be an enum, so a write outside its options is refused"
            );
            assert_eq!(
                def.default_value,
                serde_json::json!(default),
                "bug: {key} must default to {default}"
            );
            assert_eq!(
                def.enum_options.as_deref(),
                Some(&options.map(str::to_owned)[..]),
                "bug: {key} must offer exactly its two options"
            );
            assert!(
                def.is_runtime_configurable && !def.requires_restart,
                "bug: {key} must change without a restart"
            );
        }
    }
}
