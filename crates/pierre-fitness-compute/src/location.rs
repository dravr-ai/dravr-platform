// ABOUTME: Location and geographic intelligence for activity analysis and environmental context
// ABOUTME: Resolves place names to coordinates through Nominatim behind a process-wide cache
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// NOTE: All remaining `.clone()` calls in this file are Safe - they are necessary for:
// - HTTP client Arc sharing for geocoding requests
// - Cache key and data ownership transfers for async operations
use pierre_core::constants::project::user_agent;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::api_client as shared_client;
use pierre_core::http_client::SharedHttpClient;
use reqwest::header::USER_AGENT;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, SystemTime};
use tracing::{debug, info, instrument, warn};

/// How long a geocoding answer stays usable. Place names do not move, so a day
/// is conservative.
const GEOCODE_CACHE_DURATION: Duration = Duration::from_hours(24);

/// Upper bound on entries in the geocoding cache, past which the map is
/// swept of expired entries and, failing that, dropped wholesale.
const MAX_GEOCODE_CACHE_ENTRIES: usize = 2048;

/// Process-wide forward-geocode cache (place name → coordinates).
///
/// Nominatim answers are public and identical for every tenant, so one cache
/// serves them all. It lives outside [`LocationService`] because each tool call
/// constructs a fresh service — a cache owned by the service could never
/// register a hit, and Nominatim's usage policy is one request per second with
/// real bans behind it.
static FORWARD_CACHE: LazyLock<RwLock<HashMap<String, ForwardCacheEntry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Cached forward-geocode result for a place-name query.
#[derive(Debug, Clone)]
struct ForwardCacheEntry {
    latitude: f64,
    longitude: f64,
    display_name: String,
    timestamp: SystemTime,
}

/// Resolved place-name → coordinates answer returned to callers of
/// [`LocationService::forward_geocode`]. Exposes the canonical display
/// name so the MCP tool can echo it back to the LLM for grounding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardGeocodeResult {
    /// Latitude in decimal degrees (WGS84)
    pub latitude: f64,
    /// Longitude in decimal degrees (WGS84)
    pub longitude: f64,
    /// Canonical display name as Nominatim returned it — e.g.
    /// "Prévost, MRC La Rivière-du-Nord, Laurentides, Québec, Canada"
    pub display_name: String,
}

/// Nominatim `/search` response element. Nominatim returns lat/lon as
/// STRINGS in this endpoint.
#[derive(Debug, Clone, Deserialize)]
struct NominatimSearchResult {
    lat: String,
    lon: String,
    display_name: String,
}

/// Service for geocoding and location data enrichment
pub struct LocationService {
    client: &'static SharedHttpClient,
    base_url: String,
    enabled: bool,
}

impl LocationService {
    /// Create a new location service with default configuration
    #[must_use]
    pub fn new() -> Self {
        Self::with_config("https://nominatim.openstreetmap.org".into(), true)
    }

    /// Creates a location service with custom configuration
    #[must_use]
    pub fn with_config(base_url: String, enabled: bool) -> Self {
        Self {
            client: shared_client(),
            base_url,
            enabled,
        }
    }

    /// Resolve a place name to WGS84 coordinates via Nominatim `/search`.
    ///
    /// Used by the MCP route-discovery tool so the LLM can pass natural
    /// place strings like `"Prévost, QC"` or `"Saint-Alexis-des-Monts"`
    /// instead of having to know coordinates. Results are cached for 24h
    /// per lowercased query string so repeated calls from a single chat
    /// session don't hammer the free Nominatim endpoint.
    ///
    /// # Errors
    ///
    /// - Returns `AppError::not_found` when Nominatim returns zero matches
    ///   for the query.
    /// - Returns `AppError::external_service` when Nominatim is unreachable,
    ///   returns a non-success status, or returns a response that can't be
    ///   parsed (including non-numeric `lat`/`lon` strings).
    #[instrument(
        skip(self),
        fields(
            service = "nominatim",
            api_call = "forward_geocode",
            query = %query,
        )
    )]
    pub async fn forward_geocode(&self, query: &str) -> AppResult<ForwardGeocodeResult> {
        if !self.enabled {
            return Err(AppError::external_service(
                "Nominatim",
                "Location service is disabled; cannot forward-geocode",
            ));
        }

        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Err(AppError::invalid_input("place query must not be empty"));
        }

        let cache_key = trimmed.to_lowercase();

        if let Some(hit) = Self::check_forward_cache(&cache_key) {
            return Ok(ForwardGeocodeResult {
                latitude: hit.latitude,
                longitude: hit.longitude,
                display_name: hit.display_name,
            });
        }

        let encoded = urlencoding::encode(trimmed);
        let url = format!(
            "{}/search?q={encoded}&format=json&limit=1&addressdetails=0",
            self.base_url
        );
        info!(url = %url, "forward_geocode: dispatching Nominatim request");

        let response = self
            .client
            .get(&url)
            .header(USER_AGENT, user_agent())
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, url = %url, "forward_geocode: HTTP send failed");
                AppError::external_service(
                    "Nominatim",
                    format!("Forward geocoding request failed: {e}"),
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            warn!(
                status = %status,
                body_len = body.len(),
                body_preview = %body.chars().take(200).collect::<String>(),
                "forward_geocode: Nominatim returned non-success status"
            );
            return Err(AppError::external_service(
                "Nominatim",
                format!("Forward geocoding API returned status: {status}"),
            ));
        }

        let body_text = response.text().await.map_err(|e| {
            warn!(error = %e, "forward_geocode: reading response body failed");
            AppError::external_service(
                "Nominatim",
                format!("Failed to read forward geocoding response: {e}"),
            )
        })?;
        info!(
            body_len = body_text.len(),
            body_preview = %body_text.chars().take(200).collect::<String>(),
            "forward_geocode: Nominatim responded"
        );
        let results: Vec<NominatimSearchResult> =
            serde_json::from_str(&body_text).map_err(|e| {
                warn!(error = %e, body_len = body_text.len(), "forward_geocode: JSON parse failed");
                AppError::external_service(
                    "Nominatim",
                    format!("Failed to parse forward geocoding response: {e}"),
                )
            })?;

        let first = results.into_iter().next().ok_or_else(|| {
            warn!(
                query = %trimmed,
                "forward_geocode: Nominatim returned zero matches"
            );
            AppError::not_found(format!("no geocoding match for place '{trimmed}'"))
        })?;

        let latitude = first.lat.parse::<f64>().map_err(|e| {
            AppError::external_service(
                "Nominatim",
                format!("invalid latitude '{}' in response: {e}", first.lat),
            )
        })?;
        let longitude = first.lon.parse::<f64>().map_err(|e| {
            AppError::external_service(
                "Nominatim",
                format!("invalid longitude '{}' in response: {e}", first.lon),
            )
        })?;

        debug!(
            query = %trimmed,
            latitude,
            longitude,
            display_name = %first.display_name,
            "forward_geocode resolved"
        );

        if let Ok(mut cache) = FORWARD_CACHE.write() {
            evict_if_full(&mut cache, |entry| entry.timestamp);
            cache.insert(
                cache_key,
                ForwardCacheEntry {
                    latitude,
                    longitude,
                    display_name: first.display_name.clone(),
                    timestamp: SystemTime::now(),
                },
            );
        }

        Ok(ForwardGeocodeResult {
            latitude,
            longitude,
            display_name: first.display_name,
        })
    }

    /// Return a fresh forward-geocode cache entry, or `None` on miss/expired.
    fn check_forward_cache(cache_key: &str) -> Option<ForwardCacheEntry> {
        let cache = FORWARD_CACHE.read().ok()?;
        let entry = cache.get(cache_key)?;
        if entry.timestamp.elapsed().unwrap_or(Duration::from_secs(0)) < GEOCODE_CACHE_DURATION {
            debug!(cache_key, "forward_geocode cache hit");
            // Clone is required — cached entry is shared across callers
            return Some(entry.clone());
        }
        debug!(cache_key, "forward_geocode cache entry expired");
        None
    }
}

impl Default for LocationService {
    fn default() -> Self {
        Self::new()
    }
}

/// Keep a geocoding cache under [`MAX_GEOCODE_CACHE_ENTRIES`].
///
/// Sweeps expired entries first; if every entry is still live, drops the map
/// rather than growing past the cap. Geocoding answers are cheap to re-fetch
/// and unbounded residency in a long-lived server process is not.
fn evict_if_full<V>(cache: &mut HashMap<String, V>, stamp: impl Fn(&V) -> SystemTime) {
    if cache.len() < MAX_GEOCODE_CACHE_ENTRIES {
        return;
    }
    cache.retain(|_, entry| {
        stamp(entry)
            .elapsed()
            .is_ok_and(|elapsed| elapsed < GEOCODE_CACHE_DURATION)
    });
    if cache.len() >= MAX_GEOCODE_CACHE_ENTRIES {
        cache.clear();
    }
}
