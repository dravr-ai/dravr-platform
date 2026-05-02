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
//! (typically `dravr_meteo::CachedProvider` over the persistent
//! `weather_cache` table) — this module never decides cache TTL or
//! buckets.

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use dravr_meteo::{WeatherProvider, WeatherQuery};
use futures_util::stream::{FuturesUnordered, StreamExt};
use pierre_core::models::Activity;
use tracing::{info, warn};

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

/// Compute backfilled temperatures for activities missing ambient temp
/// but carrying GPS coordinates.
///
/// Returns a map of `activity_id -> temperature_celsius`. Activities
/// that already carry a temperature, lack coordinates, or whose lookup
/// fails are silently omitted — partial fill is the design intent.
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
    let candidates: Vec<(&Activity, f64, f64)> = activities
        .iter()
        .filter(|a| a.temperature().is_none())
        .filter_map(|a| {
            let lat = a.start_latitude()?;
            let lng = a.start_longitude()?;
            Some((a, lat, lng))
        })
        .collect();

    if candidates.is_empty() {
        info!(
            total,
            missing_temp,
            candidates = 0,
            "weather backfill: no candidates (activities without temperature lacked GPS coordinates)"
        );
        return HashMap::new();
    }

    info!(
        total,
        missing_temp,
        candidates = candidates.len(),
        concurrency,
        "weather backfill: dispatching lookups"
    );

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

    info!(filled = filled.len(), "weather backfill: completed lookups");

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
