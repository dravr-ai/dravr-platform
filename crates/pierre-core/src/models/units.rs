// ABOUTME: The athlete's unit system — metric or imperial — and the one rule that resolves it
// ABOUTME: An explicit choice wins, then the connected provider's own setting, then the device locale (US English reads imperial)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit preference (carnet#835).
//!
//! Every distance, elevation and pace Dravr prints — Home, the activity view,
//! the route's distance marks, the agent's replies — reads in one unit system
//! per athlete. Which one is decided here and nowhere else:
//!
//! 1. the athlete's explicit choice in Settings, when they made one;
//! 2. otherwise the connected provider's own setting, where a provider
//!    exposes one (Strava's athlete `measurement_preference`);
//! 3. otherwise the locale of the athlete's device: US English reads
//!    imperial, every other locale metric.
//!
//! The data itself stays metric everywhere it is stored and computed; only
//! what an athlete reads is converted.

use serde::{Deserialize, Serialize};

/// Kilometres in a statute mile.
pub const KILOMETRES_PER_MILE: f64 = 1.609_344;

/// The unit system an athlete reads distances, elevation and pace in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitSystem {
    /// Kilometres, metres, pace per kilometre.
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

    /// The system Strava's athlete `measurement_preference` names: `feet`
    /// reads imperial, `meters` metric. Any other value says nothing.
    #[must_use]
    pub fn from_strava_measurement_preference(value: &str) -> Option<Self> {
        match value {
            "feet" => Some(Self::Imperial),
            "meters" => Some(Self::Metric),
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

    /// The system a device locale reads: US English (`en-US`, `en_US`) is
    /// imperial, every other locale — a bare `en` included — metric.
    #[must_use]
    pub fn for_locale(locale: &str) -> Self {
        let mut parts = locale.split(['-', '_']);
        let language = parts.next().unwrap_or_default();
        let us_english = language.eq_ignore_ascii_case("en")
            && parts.any(|part| part.len() == 2 && part.eq_ignore_ascii_case("us"));
        if us_english {
            Self::Imperial
        } else {
            Self::Metric
        }
    }
}

/// What the athlete chose in Settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitPreference {
    /// No choice: the provider's setting, else the device locale, decides.
    #[default]
    Automatic,
    /// Always metric.
    Metric,
    /// Always imperial.
    Imperial,
}

impl UnitPreference {
    /// The wire spelling: `automatic`, `metric` or `imperial`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Metric => "metric",
            Self::Imperial => "imperial",
        }
    }

    /// Read the wire spelling back; anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "automatic" => Some(Self::Automatic),
            "metric" => Some(Self::Metric),
            "imperial" => Some(Self::Imperial),
            _ => None,
        }
    }

    /// The system an explicit choice pins, or `None` for `Automatic`.
    #[must_use]
    pub const fn explicit(self) -> Option<UnitSystem> {
        match self {
            Self::Automatic => None,
            Self::Metric => Some(UnitSystem::Metric),
            Self::Imperial => Some(UnitSystem::Imperial),
        }
    }
}

/// Which rung of the resolution decided the athlete's unit system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitSource {
    /// The athlete's explicit choice in Settings.
    Override,
    /// The connected provider's own setting.
    Provider,
    /// The device locale.
    Locale,
}

/// The athlete's effective unit system and what decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedUnits {
    /// The system everything the athlete reads is written in.
    pub system: UnitSystem,
    /// Which rung decided it.
    pub source: UnitSource,
}

/// Resolve the athlete's unit system: an explicit choice wins, then the
/// connected provider's setting, then `locale` (see [`UnitSystem::for_locale`]).
#[must_use]
pub fn resolve_units(
    preference: UnitPreference,
    provider: Option<UnitSystem>,
    locale: &str,
) -> ResolvedUnits {
    if let Some(system) = preference.explicit() {
        return ResolvedUnits {
            system,
            source: UnitSource::Override,
        };
    }
    if let Some(system) = provider {
        return ResolvedUnits {
            system,
            source: UnitSource::Provider,
        };
    }
    ResolvedUnits {
        system: UnitSystem::for_locale(locale),
        source: UnitSource::Locale,
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
    fn an_explicit_choice_wins_over_the_provider_and_the_locale() {
        let resolved = resolve_units(UnitPreference::Metric, Some(UnitSystem::Imperial), "en-US");
        assert_eq!(resolved.system, UnitSystem::Metric);
        assert_eq!(resolved.source, UnitSource::Override);

        let resolved = resolve_units(UnitPreference::Imperial, Some(UnitSystem::Metric), "fr");
        assert_eq!(resolved.system, UnitSystem::Imperial);
        assert_eq!(resolved.source, UnitSource::Override);
    }

    #[test]
    fn automatic_follows_the_provider_before_the_locale() {
        let resolved = resolve_units(UnitPreference::Automatic, Some(UnitSystem::Imperial), "fr");
        assert_eq!(resolved.system, UnitSystem::Imperial);
        assert_eq!(resolved.source, UnitSource::Provider);

        let resolved = resolve_units(UnitPreference::Automatic, Some(UnitSystem::Metric), "en-US");
        assert_eq!(resolved.system, UnitSystem::Metric);
        assert_eq!(resolved.source, UnitSource::Provider);
    }

    #[test]
    fn automatic_without_a_provider_setting_follows_the_locale() {
        for (locale, system) in [
            ("en-US", UnitSystem::Imperial),
            ("en_US", UnitSystem::Imperial),
            ("EN-us", UnitSystem::Imperial),
            ("en-Latn-US", UnitSystem::Imperial),
            ("en", UnitSystem::Metric),
            ("en-GB", UnitSystem::Metric),
            ("en-CA", UnitSystem::Metric),
            ("es-US", UnitSystem::Metric),
            ("fr", UnitSystem::Metric),
            ("", UnitSystem::Metric),
        ] {
            let resolved = resolve_units(UnitPreference::Automatic, None, locale);
            assert_eq!(resolved.system, system, "locale {locale:?}");
            assert_eq!(resolved.source, UnitSource::Locale);
        }
    }

    #[test]
    fn strava_measurement_preference_maps_feet_and_meters() {
        assert_eq!(
            UnitSystem::from_strava_measurement_preference("feet"),
            Some(UnitSystem::Imperial)
        );
        assert_eq!(
            UnitSystem::from_strava_measurement_preference("meters"),
            Some(UnitSystem::Metric)
        );
        assert_eq!(UnitSystem::from_strava_measurement_preference(""), None);
        assert_eq!(
            UnitSystem::from_strava_measurement_preference("miles"),
            None
        );
    }

    #[test]
    fn wire_spellings_round_trip() {
        for preference in [
            UnitPreference::Automatic,
            UnitPreference::Metric,
            UnitPreference::Imperial,
        ] {
            assert_eq!(UnitPreference::parse(preference.as_str()), Some(preference));
        }
        for system in [UnitSystem::Metric, UnitSystem::Imperial] {
            assert_eq!(UnitSystem::parse(system.as_str()), Some(system));
        }
        assert_eq!(UnitPreference::parse("km"), None);
        assert_eq!(UnitSystem::parse("automatic"), None);
    }
}
