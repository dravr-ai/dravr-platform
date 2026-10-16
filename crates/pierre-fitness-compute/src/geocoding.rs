// ABOUTME: The one place-name geocoder the platform resolves places through: dravr-meteo's Open-Meteo gazetteer behind its bounded cache
// ABOUTME: Shared by route discovery, the weather forecast tool and the weather backfill, so one name resolves to one place
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Place-name resolution for every platform surface that takes a place.
//!
//! `discover_routes`, `get_weather_forecast` and the weather backfill used to
//! resolve names through three clients — Nominatim, dravr-meteo's
//! Open-Meteo geocoder and a hand-rolled Open-Meteo call — so one place could
//! land on different coordinates in one turn (carnet#920). They all resolve
//! through [`place_geocoder`] now: one gazetteer, one bounded cache, one
//! disambiguation rule (dravr-meteo's [`PlacePreference`]).

use std::sync::LazyLock;

use dravr_meteo::{
    CacheLimits, CachedGeocoder, ClientBuildError, OpenMeteoGeocoder, PlacePreference, PlaceQuery,
};
use pierre_core::errors::{AppError, AppResult};

/// The geocoder every caller shares: Open-Meteo's gazetteer behind the
/// bounded cache dravr-meteo sizes for place lookups
/// ([`CacheLimits::GEOCODE`]). Only resolutions are cached; an unknown name
/// reaches the gazetteer again next time. Built once; a client that could not
/// be built is reported to every caller rather than replaced by one without a
/// deadline.
static GEOCODER: LazyLock<Result<CachedGeocoder<OpenMeteoGeocoder>, ClientBuildError>> =
    LazyLock::new(|| {
        OpenMeteoGeocoder::new().map(|geocoder| CachedGeocoder::new(geocoder, CacheLimits::GEOCODE))
    });

/// The process-wide place geocoder.
///
/// # Errors
///
/// An external-service error when the geocoder's HTTP client could not be
/// built.
pub fn place_geocoder() -> AppResult<&'static CachedGeocoder<OpenMeteoGeocoder>> {
    GEOCODER
        .as_ref()
        .map_err(|e| AppError::external_service("Open-Meteo geocoding", e.to_string()))
}

/// A free-text place such as `"Prévost, Québec"` or
/// `"Paris, Texas, United States"` as a gazetteer query.
///
/// The gazetteer searches names alone, so the text before the first comma is
/// the name and what follows only chooses among same-named places: with three
/// or more parts the second is the region and the last the country; with two,
/// the second may be either, so it is preferred as both. A preference never
/// narrows the search — an unmatched one (an abbreviation such as `"QC"`, which
/// no gazetteer writes) leaves the top-ranked place. `None` when the text names
/// nothing.
#[must_use]
pub fn free_text_query(text: &str, country_code: Option<String>) -> Option<PlaceQuery> {
    let mut parts = text
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let name = parts.next()?.to_owned();
    let rest: Vec<&str> = parts.collect();
    let prefer = match rest.as_slice() {
        [] => PlacePreference::default(),
        [either] => PlacePreference {
            admin1: Some((*either).to_owned()),
            country: Some((*either).to_owned()),
        },
        [region, .., country] => PlacePreference {
            admin1: Some((*region).to_owned()),
            country: Some((*country).to_owned()),
        },
    };
    Some(PlaceQuery {
        name,
        country_code,
        prefer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_prefers_nothing() {
        let query = free_text_query("  Chamonix ", None).unwrap();
        assert_eq!(query.name, "Chamonix");
        assert_eq!(query.prefer, PlacePreference::default());
        assert_eq!(query.country_code, None);
    }

    #[test]
    fn a_second_part_is_preferred_as_region_or_country() {
        let query = free_text_query("Prévost, Québec", Some("CA".to_owned())).unwrap();
        assert_eq!(query.name, "Prévost");
        assert_eq!(query.prefer.admin1.as_deref(), Some("Québec"));
        assert_eq!(query.prefer.country.as_deref(), Some("Québec"));
        assert_eq!(query.country_code.as_deref(), Some("CA"));
    }

    #[test]
    fn three_parts_name_the_region_then_the_country() {
        let query = free_text_query("Paris, Texas, , United States", None).unwrap();
        assert_eq!(query.name, "Paris");
        assert_eq!(query.prefer.admin1.as_deref(), Some("Texas"));
        assert_eq!(query.prefer.country.as_deref(), Some("United States"));
    }

    #[test]
    fn text_with_no_name_is_no_query() {
        assert!(free_text_query(" , ,", None).is_none());
        assert!(free_text_query("", None).is_none());
    }
}
