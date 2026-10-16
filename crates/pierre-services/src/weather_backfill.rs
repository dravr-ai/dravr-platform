// ABOUTME: Lazy weather backfill for activities missing ambient temperature
// ABOUTME: Resolves temperature from (start_lat, start_lng, start_date) via dravr-meteo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Weather backfill orchestrator.
//!
//! Many providers don't surface ambient temperature on workouts —
//! sciotte (Strava Mirror scraper), Whoop, Terra all leave the field
//! empty. For activities that have GPS coordinates and a start
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
//! (typically `dravr_meteo::CachedProvider` over the persistent
//! `weather_cache` table) — this module never decides cache TTL or
//! buckets.

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use dravr_meteo::{Geocoder, PlacePreference, PlaceQuery, WeatherProvider, WeatherQuery};
use futures_util::stream::{FuturesUnordered, StreamExt};
use pierre_core::models::Activity;
use pierre_fitness_compute::geocoding::place_geocoder;
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
/// `(lat, lng)` pair through the platform's one place geocoder — the one
/// `discover_routes` and `get_weather_forecast` resolve through
/// (carnet#920). The gazetteer searches the city name alone, and the
/// activity's region and country choose among same-named places. Best-effort:
/// locations that fail to resolve simply don't appear in the returned map.
async fn geocode_unique_cities(activities: &[&Activity]) -> HashMap<String, (f64, f64)> {
    let geocoder = match place_geocoder() {
        Ok(geocoder) => geocoder,
        Err(e) => {
            warn!(error = %e, "geocode: no place geocoder; city-only activities keep no weather");
            return HashMap::new();
        }
    };
    let unique = unique_locations(activities);
    let mut resolved = HashMap::with_capacity(unique.len());
    for (key, region, country) in unique {
        if key.is_empty() {
            continue;
        }
        if let Some(coords) = resolve_city(geocoder, &key, region, country).await {
            resolved.insert(key, coords);
        }
    }
    resolved
}

/// The coordinates of the city a location key names, chosen among same-named
/// places by the activity's region and country; `None` when it resolves to
/// nothing.
async fn resolve_city(
    geocoder: &dyn Geocoder,
    key: &str,
    region: Option<String>,
    country: Option<String>,
) -> Option<(f64, f64)> {
    let city = key.split(',').next().unwrap_or(key).trim();
    match geocoder.resolve(&city_query(city, region, country)).await {
        Ok(resolution) => {
            let coords = (resolution.place.latitude, resolution.place.longitude);
            debug!(query = %key, city, lat = coords.0, lng = coords.1, "geocode: hit");
            Some(coords)
        }
        Err(e) => {
            debug!(query = %key, city, error = %e, "geocode: no result");
            None
        }
    }
}

/// Each distinct location signal among `activities` once: its cache key, and
/// the region and country it was recorded with.
fn unique_locations(activities: &[&Activity]) -> Vec<(String, Option<String>, Option<String>)> {
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
    unique
}

/// The gazetteer query for an activity's city, preferring the region and
/// country the activity recorded among places that share the city's name.
fn city_query(city: &str, region: Option<String>, country: Option<String>) -> PlaceQuery {
    PlaceQuery {
        name: city.to_owned(),
        country_code: None,
        prefer: PlacePreference {
            admin1: region,
            country,
        },
    }
}
