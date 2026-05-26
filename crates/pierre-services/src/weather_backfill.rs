// ABOUTME: Lazy weather backfill for activities missing ambient temperature
// ABOUTME: Resolves temperature from (start_lat, start_lng, start_date) via dravr-meteo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Weather backfill orchestrator.
//!
//! Many providers don't surface ambient temperature on workouts —
//! sciotte (Strava Mirror scraper), Whoop, Fitbit, Terra all leave the
//! field empty. For activities that have GPS coordinates and a start
//! time, we can fill the gap post-hoc by hitting a historical weather
//! provider keyed on `(lat, lng, hour)`.
//!
//! This module fans out concurrent lookups (capped) against an
//! `Arc<dyn WeatherProvider>` and returns a side-table of
//! `activity_id -> temperature_celsius` that the response builder
//! merges into [`ActivitySummary`] and the inline activity-list
//! formatter — `Activity` itself is immutable, so we don't try to
//! mutate the provider's payload in place.
//!
//! Cache coherence is the responsibility of the wrapped provider
//! (typically `pierre_weather::CachedProvider` over the persistent
//! `weather_cache` table) — this module never decides cache TTL or
//! buckets.

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use futures_util::stream::{FuturesUnordered, StreamExt};
use pierre_core::http_client::api_client;
use pierre_core::models::Activity;
use pierre_weather::{WeatherProvider, WeatherQuery};
use serde::Deserialize;
use tracing::{debug, info, warn};

/// Default ceiling on concurrent weather lookups during a backfill pass.
///
/// Open-Meteo accepts ~10K requests/day; 20 in flight at once keeps a
/// list of 200 activities under a few seconds while staying well below
/// the per-second burst limit.
const DEFAULT_BACKFILL_CONCURRENCY: usize = 20;

/// Environment variable controlling the concurrency cap.
const BACKFILL_CONCURRENCY_ENV: &str = "WEATHER_BACKFILL_CONCURRENCY";

/// Environment toggle (`true` / `false`) gating the entire backfill pass.
const BACKFILL_ENABLED_ENV: &str = "WEATHER_BACKFILL_ENABLED";

/// Return whether the response-side weather backfill should run at all.
///
/// Defaults to `true`. Set `WEATHER_BACKFILL_ENABLED=false` to disable
/// (useful in tests or for cost containment if the upstream weather API
/// is rate-limited).
#[must_use]
pub fn is_enabled() -> bool {
    env::var(BACKFILL_ENABLED_ENV)
        .ok()
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(true)
}

/// Compute backfilled temperatures for activities missing ambient temp.
///
/// Two paths populate the candidate list:
/// 1. Activities with explicit `start_latitude` / `start_longitude` are
///    used as-is.
/// 2. Activities lacking GPS but carrying a `city` (e.g., Strava-Mirror
///    sciotte rows enriched from the dashboard feed) are geocoded via
///    Open-Meteo's free geocoding API. Each unique city resolves once
///    per backfill pass — typical users repeat 5-10 places across
///    hundreds of activities.
///
/// Returns a map of `activity_id -> temperature_celsius`. Activities
/// that already carry a temperature, lack any location signal, or whose
/// geocoding/weather lookup fails are silently omitted — partial fill
/// is the design intent.
pub async fn fill_activity_temperatures(
    activities: &[Activity],
    provider: Arc<dyn WeatherProvider>,
) -> HashMap<String, f32> {
    let concurrency = env::var(BACKFILL_CONCURRENCY_ENV)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_BACKFILL_CONCURRENCY);

    let total = activities.len();
    let missing_temp = activities
        .iter()
        .filter(|a| a.temperature().is_none())
        .count();

    let (candidates, from_gps, from_city) = collect_candidates(activities).await;

    if candidates.is_empty() {
        info!(
            total,
            missing_temp,
            candidates = 0,
            "weather backfill: no candidates (no GPS, no resolvable city)"
        );
        return HashMap::new();
    }

    info!(
        total,
        missing_temp,
        candidates = candidates.len(),
        from_gps,
        from_city,
        concurrency,
        "weather backfill: dispatching lookups"
    );

    let filled = run_lookup_pump(provider, candidates, concurrency).await;

    info!(filled = filled.len(), "weather backfill: completed lookups");

    filled
}

/// Concurrency-capped fan-out over weather lookups; held in a helper so
/// `fill_activity_temperatures` stays under the workspace cognitive
/// complexity ceiling.
async fn run_lookup_pump(
    provider: Arc<dyn WeatherProvider>,
    candidates: Vec<(&Activity, f64, f64)>,
    concurrency: usize,
) -> HashMap<String, f32> {
    let mut filled: HashMap<String, f32> = HashMap::with_capacity(candidates.len());
    let mut in_flight = FuturesUnordered::new();
    let mut iter = candidates.into_iter();

    while in_flight.len() < concurrency {
        if let Some((activity, lat, lng)) = iter.next() {
            in_flight.push(lookup_one(provider.clone(), activity, lat, lng));
        } else {
            break;
        }
    }

    while let Some(result) = in_flight.next().await {
        if let Some((id, temp)) = result {
            filled.insert(id, temp);
        }
        if let Some((activity, lat, lng)) = iter.next() {
            in_flight.push(lookup_one(provider.clone(), activity, lat, lng));
        }
    }

    filled
}

async fn lookup_one(
    provider: Arc<dyn WeatherProvider>,
    activity: &Activity,
    latitude: f64,
    longitude: f64,
) -> Option<(String, f32)> {
    let query = WeatherQuery {
        latitude,
        longitude,
        timestamp: activity.start_date(),
    };

    match provider.weather_at(query).await {
        Ok(sample) => Some((activity.id().to_owned(), sample.temperature_celsius)),
        Err(e) => {
            warn!(
                activity_id = %activity.id(),
                error = %e,
                "weather backfill: lookup failed"
            );
            None
        }
    }
}

/// Walk the activity list once, splitting into GPS-ready candidates and
/// city-only candidates. Geocode any city-only group via Open-Meteo,
/// then merge back into the candidate list. Returns
/// `(candidates, from_gps_count, from_city_count)` for telemetry.
async fn collect_candidates(activities: &[Activity]) -> (Vec<(&Activity, f64, f64)>, usize, usize) {
    let mut candidates: Vec<(&Activity, f64, f64)> = Vec::new();
    let mut needs_geocode: Vec<&Activity> = Vec::new();
    let mut from_gps = 0usize;

    for a in activities.iter().filter(|a| a.temperature().is_none()) {
        if let (Some(lat), Some(lng)) = (a.start_latitude(), a.start_longitude()) {
            candidates.push((a, lat, lng));
            from_gps += 1;
        } else if a.city().is_some() {
            needs_geocode.push(a);
        }
    }

    let mut from_city = 0usize;
    if !needs_geocode.is_empty() {
        let geocoded = geocode_unique_cities(&needs_geocode).await;
        for a in needs_geocode {
            let key = location_cache_key(a);
            if let Some(&(lat, lng)) = geocoded.get(&key) {
                candidates.push((a, lat, lng));
                from_city += 1;
            }
        }
    }

    (candidates, from_gps, from_city)
}

/// Stable cache key for an activity's location signal — collapses
/// city/region/country into a single normalized string so two activities
/// at the same place share a single geocode lookup.
fn location_cache_key(a: &Activity) -> String {
    let mut key = a.city().unwrap_or_default().trim().to_owned();
    if let Some(region) = a.region().map(str::trim).filter(|s| !s.is_empty()) {
        key.push_str(", ");
        key.push_str(region);
    }
    if let Some(country) = a.country().map(str::trim).filter(|s| !s.is_empty()) {
        key.push_str(", ");
        key.push_str(country);
    }
    key
}

/// Resolve every unique location-signal referenced by `activities` to a
/// `(lat, lng)` pair via Open-Meteo's free geocoding API. We query the
/// city name alone (the API rejects "City, Region" as a single query)
/// and use the activity's `region` to disambiguate when multiple
/// continents share a city name. Best-effort: locations that fail to
/// resolve simply don't appear in the returned map.
async fn geocode_unique_cities(activities: &[&Activity]) -> HashMap<String, (f64, f64)> {
    let mut unique: Vec<(String, Option<String>, Option<String>)> = activities
        .iter()
        .map(|a| {
            (
                location_cache_key(a),
                a.region()
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty()),
                a.country()
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty()),
            )
        })
        .collect();
    unique.sort_by(|a, b| a.0.cmp(&b.0));
    unique.dedup_by(|a, b| a.0 == b.0);

    let mut resolved = HashMap::with_capacity(unique.len());
    for (key, region, country) in unique {
        if key.is_empty() {
            continue;
        }
        let city = key.split(',').next().unwrap_or(&key).trim();
        match geocode_via_open_meteo(city, region.as_deref(), country.as_deref()).await {
            Some(coords) => {
                debug!(query = %key, city, lat = coords.0, lng = coords.1, "geocode: hit");
                resolved.insert(key, coords);
            }
            None => {
                debug!(query = %key, city, "geocode: no result");
            }
        }
    }
    resolved
}

/// Geocoding response shape from
/// `https://geocoding-api.open-meteo.com/v1/search`. Open-Meteo returns
/// results sorted by population + query relevance, plus `admin1` (state
/// or province) and `country` so we can disambiguate same-named cities.
#[derive(Debug, Deserialize)]
struct GeocodeResponse {
    #[serde(default)]
    results: Vec<GeocodeHit>,
}

#[derive(Debug, Deserialize)]
struct GeocodeHit {
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    admin1: Option<String>,
    #[serde(default)]
    country: Option<String>,
}

/// Hit Open-Meteo's geocoding API with just the city name; if the
/// caller supplied a region or country, prefer the hit whose `admin1` /
/// `country` matches. Free, no API key required, ~10K requests/day per
/// IP — well above any single user's activity volume.
async fn geocode_via_open_meteo(
    city: &str,
    expected_region: Option<&str>,
    expected_country: Option<&str>,
) -> Option<(f64, f64)> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=10&language=en&format=json",
        urlencoding::encode(city)
    );
    let response = api_client().get(&url).send().await.ok()?;
    if !response.status().is_success() {
        warn!(city, status = %response.status(), "geocode: non-2xx");
        return None;
    }
    let body: GeocodeResponse = response.json().await.ok()?;
    if body.results.is_empty() {
        return None;
    }
    let matches_expected = |hit: &GeocodeHit| -> bool {
        if let Some(region) = expected_region {
            if hit
                .admin1
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(region))
            {
                return true;
            }
        }
        if let Some(country) = expected_country {
            if hit
                .country
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case(country))
            {
                return true;
            }
        }
        false
    };

    let best = body
        .results
        .iter()
        .find(|h| matches_expected(h))
        .or_else(|| body.results.first())?;
    Some((best.latitude, best.longitude))
}
