// ABOUTME: The per-athlete sciotte session-read caps as admin parameters, one per cap per tier
// ABOUTME: Registers sciotte_reads.<cap>.<tier> with the compiled SessionReadCaps as defaults (carnet#1661)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The session-read caps as admin configuration (carnet#1661).
//!
//! JF, 2026-11-19: "We must be able to change those caps using pierre-cli."
//!
//! Every read of an athlete's signed-in session at Strava, Garmin, COROS or
//! `TrainingPeaks` (sciotte) is counted per athlete and per backend. Each
//! number of [`SessionReadCaps`] is one parameter per tier,
//! `sciotte_reads.<cap>.<tier>`, so an override of one tier never flattens
//! the others:
//!
//! ```text
//! pierre-cli config show --category sciotte_reads
//! pierre-cli config set sciotte_reads.asked_daily.professional 400 --reason "…"
//! pierre-cli config set sciotte_reads.background_15min.starter 20 --user athlete@example.com
//! ```
//!
//! A value is resolved per athlete, user then system-wide, and reaches every
//! instance within a minute, with no restart. A `--user` override is read
//! only when that user's tier is the key's tier.

use std::collections::HashMap;
use std::hash::BuildHasher;

use pierre_core::models::{SessionReadCap, SessionReadCaps, UserTier, SESSION_READ_CAPS_CATEGORY};

use crate::admin_definitions::ParameterDefinition;
use crate::admin_types::{ConfigDataType, ParameterRange};

/// Every tier a cap has a parameter for.
const TIERS: [UserTier; 3] = [
    UserTier::Starter,
    UserTier::Professional,
    UserTier::Enterprise,
];

/// The highest value a daily cap accepts.
const MAX_DAILY: u32 = 100_000;

/// The highest value a 15-minute cap accepts.
const MAX_BURST: u32 = 10_000;

/// The highest value `cap` accepts: a day's or a burst's.
#[must_use]
const fn max_of(cap: SessionReadCap) -> u32 {
    match cap {
        SessionReadCap::AskedDaily
        | SessionReadCap::BackgroundDaily
        | SessionReadCap::WellnessFirstFillDaily => MAX_DAILY,
        SessionReadCap::Asked15Min | SessionReadCap::Background15Min => MAX_BURST,
    }
}

/// The display title and meaning of `cap`.
const fn describe(cap: SessionReadCap) -> (&'static str, &'static str) {
    match cap {
        SessionReadCap::AskedDaily => (
            "Asked Reads per Day",
            "Reads of one athlete's signed-in session at one provider that the athlete is \
             waiting on (a tool call, an app page), per UTC day.",
        ),
        SessionReadCap::Asked15Min => (
            "Asked Reads per 15 Minutes",
            "Reads of one athlete's signed-in session at one provider that the athlete is \
             waiting on, per 15-minute window (:00, :15, :30, :45 UTC).",
        ),
        SessionReadCap::BackgroundDaily => (
            "Background Reads per Day",
            "Reads of one athlete's signed-in session at one provider that nobody is waiting \
             on (walks, sweeps, health sync, coach posts), per UTC day.",
        ),
        SessionReadCap::Background15Min => (
            "Background Reads per 15 Minutes",
            "Reads of one athlete's signed-in session at one provider that nobody is waiting \
             on, per 15-minute window; a first wellness copy's reads count here too.",
        ),
        SessionReadCap::WellnessFirstFillDaily => (
            "First Wellness Copy Reads per Day",
            "Days of a Garmin or COROS athlete's first wellness copy read per UTC day, counted \
             in place of the background day; once the copy is complete only the background \
             caps apply.",
        ),
    }
}

/// The parameter holding `cap` for `tier`, defaulting to the compiled cap.
fn definition(cap: SessionReadCap, tier: &UserTier) -> ParameterDefinition {
    let (title, meaning) = describe(cap);
    ParameterDefinition {
        key: cap.config_key(tier),
        display_name: format!("Session {title} ({})", tier.display_name()),
        description: format!(
            "{meaning} For {} athletes, at each of Strava, Garmin, COROS and TrainingPeaks \
             read through the athlete's session. Resolved per athlete: user, then \
             system-wide; a tenant row is not read. 0 refuses every such read.",
            tier.display_name()
        ),
        category: SESSION_READ_CAPS_CATEGORY.to_owned(),
        data_type: ConfigDataType::Integer,
        default_value: serde_json::json!(cap.of(&SessionReadCaps::for_tier(tier))),
        valid_range: Some(ParameterRange {
            min: serde_json::json!(0),
            max: serde_json::json!(max_of(cap)),
            step: Some(1.0),
        }),
        enum_options: None,
        units: Some("reads".to_owned()),
        scientific_basis: None,
        env: None,
        is_runtime_configurable: true,
        requires_restart: false,
    }
}

/// Register every `sciotte_reads.<cap>.<tier>` parameter.
pub fn register_sciotte_reads<S: BuildHasher>(defs: &mut HashMap<String, ParameterDefinition, S>) {
    for tier in &TIERS {
        for cap in SessionReadCap::ALL {
            let def = definition(cap, tier);
            defs.insert(def.key.clone(), def);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered() -> HashMap<String, ParameterDefinition> {
        let mut defs = HashMap::new();
        register_sciotte_reads(&mut defs);
        defs
    }

    #[test]
    fn every_cap_of_every_tier_is_one_parameter_defaulting_to_the_compiled_cap() {
        let defs = registered();
        assert_eq!(
            defs.len(),
            TIERS.len() * SessionReadCap::ALL.len(),
            "bug: a cap of a tier is missing or registered twice"
        );
        for tier in &TIERS {
            let compiled = SessionReadCaps::for_tier(tier);
            for cap in SessionReadCap::ALL {
                let key = cap.config_key(tier);
                let def = defs
                    .get(&key)
                    .unwrap_or_else(|| panic!("bug: {key} is not registered"));
                assert_eq!(
                    def.category, SESSION_READ_CAPS_CATEGORY,
                    "bug: {key} is outside {SESSION_READ_CAPS_CATEGORY}"
                );
                assert_eq!(
                    def.default_value,
                    serde_json::json!(cap.of(&compiled)),
                    "bug: {key} does not default to the compiled cap"
                );
                assert_eq!(
                    def.data_type,
                    ConfigDataType::Integer,
                    "bug: {key} must be an integer, so a non-number is refused"
                );
                assert!(
                    def.is_runtime_configurable && !def.requires_restart && def.env.is_none(),
                    "bug: {key} must change at runtime, with no restart and no env pin"
                );
            }
        }
    }

    #[test]
    fn a_cap_accepts_zero_up_to_its_windows_ceiling() {
        let defs = registered();
        for tier in &TIERS {
            for cap in SessionReadCap::ALL {
                let key = cap.config_key(tier);
                let range = defs[&key]
                    .valid_range
                    .as_ref()
                    .unwrap_or_else(|| panic!("bug: {key} is not range-bounded"));
                assert_eq!(
                    range.min,
                    serde_json::json!(0),
                    "bug: {key} cannot be set to 0, the kill switch"
                );
                assert_eq!(
                    range.max,
                    serde_json::json!(max_of(cap)),
                    "bug: {key} is not bounded by its window's ceiling"
                );
                assert!(
                    cap.of(&SessionReadCaps::for_tier(tier)) <= max_of(cap),
                    "bug: {key}'s default is above its own ceiling"
                );
            }
        }
    }
}
