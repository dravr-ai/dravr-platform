// ABOUTME: The athlete's unit system — metric or imperial — and the conversions every printed distance goes through
// ABOUTME: Metric unless the athlete chose imperial in Settings; nothing else (provider, locale) changes it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit system (carnet#835, carnet#1044).
//!
//! Every distance, elevation and pace Dravr prints — Home, the activity view,
//! the route's distance marks, the agent's replies — reads in one unit system
//! per athlete: metric, unless the athlete chose imperial in Settings. The
//! provider's own setting and the device locale are deliberately not read:
//! neither says which units the athlete wants to read in, and inferring
//! imperial from them puts miles in front of metric athletes.
//!
//! The data itself stays metric everywhere it is stored and computed; only
//! what an athlete reads is converted.

use serde::{Deserialize, Serialize};

/// Kilometres in a statute mile.
pub const KILOMETRES_PER_MILE: f64 = 1.609_344;

/// The unit system an athlete reads distances, elevation and pace in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitSystem {
    /// Kilometres, metres, pace per kilometre. Every athlete's default.
    #[default]
    Metric,
    /// Miles, feet, pace per mile.
    Imperial,
}

impl UnitSystem {
    /// The wire spelling: `metric` or `imperial`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metric => "metric",
            Self::Imperial => "imperial",
        }
    }

    /// Read the wire spelling back; anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "metric" => Some(Self::Metric),
            "imperial" => Some(Self::Imperial),
            _ => None,
        }
    }

    /// A distance in kilometres, counted in this system's distance unit:
    /// kilometres stay kilometres, imperial counts miles.
    #[must_use]
    pub fn distance_from_km(self, km: f64) -> f64 {
        match self {
            Self::Metric => km,
            Self::Imperial => km / KILOMETRES_PER_MILE,
        }
    }

    /// The symbol a distance in this system is printed with: `km` or `mi`.
    #[must_use]
    pub const fn distance_symbol(self) -> &'static str {
        match self {
            Self::Metric => "km",
            Self::Imperial => "mi",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_distance_reads_in_kilometres_or_miles() {
        assert!((UnitSystem::Metric.distance_from_km(41.5) - 41.5).abs() < 1e-12);
        assert!((UnitSystem::Imperial.distance_from_km(16.093_44) - 10.0).abs() < 1e-9);
        assert_eq!(UnitSystem::Metric.distance_symbol(), "km");
        assert_eq!(UnitSystem::Imperial.distance_symbol(), "mi");
    }

    #[test]
    fn wire_spellings_round_trip() {
        for system in [UnitSystem::Metric, UnitSystem::Imperial] {
            assert_eq!(UnitSystem::parse(system.as_str()), Some(system));
        }
        assert_eq!(UnitSystem::parse("automatic"), None);
        assert_eq!(UnitSystem::parse("km"), None);
    }
}
