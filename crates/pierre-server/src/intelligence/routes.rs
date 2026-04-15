// ABOUTME: Route and trail discovery service using Overpass API for OpenStreetMap data
// ABOUTME: Discovers running, cycling, and ski routes near a given location
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::constants::project::user_agent;
use crate::errors::{AppError, AppResult};
use crate::models::SportType;
use crate::utils::http_client::shared_client;
use reqwest::header::USER_AGENT;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use tracing::{debug, warn};

/// Cache duration for route queries (24 hours)
const ROUTE_CACHE_DURATION_SECS: u64 = 86400;

/// Maximum number of routes to return per query
const MAX_ROUTES_PER_QUERY: usize = 20;

/// Default search radius in meters for Overpass queries
const DEFAULT_SEARCH_RADIUS_METERS: u32 = 10_000;

/// Public Overpass API mirrors, tried in order until one answers successfully.
///
/// The primary endpoint (`overpass-api.de`) regularly returns 503/504 during
/// peak hours because it's a shared free service. Production cannot depend on
/// a single public Overpass instance, so we fall through to community mirrors
/// published on the OSM wiki until one succeeds. If every mirror fails we
/// surface a transient-error variant so the MCP tool can tell the LLM to
/// retry rather than fabricate.
const OVERPASS_MIRRORS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
];

// ============================================================================
// Public types
// ============================================================================

/// A discovered route or trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredRoute {
    /// Route name (from OSM data)
    pub name: String,
    /// Type of route (cycling, hiking, ski, etc.)
    pub route_type: RouteType,
    /// Approximate distance in meters (if available)
    pub distance_meters: Option<f64>,
    /// Difficulty level (if available)
    pub difficulty: Option<String>,
    /// Data source (`OpenSkiMap`, Overpass, `OpenRouteService`)
    pub source: RouteSource,
    /// Latitude of the route start or center
    pub latitude: f64,
    /// Longitude of the route start or center
    pub longitude: f64,
}

/// Type of route
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteType {
    /// Cycling route (road, path, or cycleway)
    Cycling,
    /// Running or jogging path
    Running,
    /// Hiking trail
    Hiking,
    /// Cross-country ski trail
    CrossCountrySki,
    /// Downhill ski run
    DownhillSki,
    /// Snowshoe trail
    Snowshoe,
    /// Multi-use trail
    MultiUse,
}

/// Source of route data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteSource {
    /// `OpenSkiMap` — ski trail data from OSM
    OpenSkiMap,
    /// Overpass API — general OSM query engine
    Overpass,
    /// `OpenRouteService` — route generation
    OpenRouteService,
}

// ============================================================================
// Route discovery service
// ============================================================================

/// Service for discovering routes and trails near a location
pub struct RouteDiscoveryService {
    client: &'static Client,
    cache: HashMap<String, CachedRoutes>,
    overpass_mirrors: Vec<String>,
}

#[derive(Debug)]
struct CachedRoutes {
    routes: Vec<DiscoveredRoute>,
    cached_at: SystemTime,
}

impl RouteDiscoveryService {
    /// Create a route discovery service with default Overpass mirrors.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self {
            client: shared_client(),
            cache: HashMap::new(),
            overpass_mirrors: OVERPASS_MIRRORS.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    /// Discover cycling routes near a location using Overpass API
    ///
    /// # Errors
    ///
    /// Returns an error if the Overpass API request fails or returns an error status.
    pub async fn discover_cycling_routes(
        &mut self,
        latitude: f64,
        longitude: f64,
        radius_meters: Option<u32>,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let radius = radius_meters.unwrap_or(DEFAULT_SEARCH_RADIUS_METERS);
        let cache_key = format!("cycling_{latitude:.3}_{longitude:.3}_{radius}");

        if let Some(cached) = self.get_cached(&cache_key) {
            return Ok(cached);
        }

        let query = format!(
            r#"[out:json][timeout:25];
(
  way["highway"="cycleway"](around:{radius},{latitude},{longitude});
  way["bicycle"="designated"](around:{radius},{latitude},{longitude});
  relation["route"="bicycle"](around:{radius},{latitude},{longitude});
);
out tags center {MAX_ROUTES_PER_QUERY};"#,
        );

        let routes = self.query_overpass(&query, RouteType::Cycling).await?;
        self.set_cached(cache_key, routes.clone());
        Ok(routes)
    }

    /// Discover running/jogging paths near a location using Overpass API
    ///
    /// # Errors
    ///
    /// Returns an error if the Overpass API request fails or returns an error status.
    pub async fn discover_running_routes(
        &mut self,
        latitude: f64,
        longitude: f64,
        radius_meters: Option<u32>,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let radius = radius_meters.unwrap_or(DEFAULT_SEARCH_RADIUS_METERS);
        let cache_key = format!("running_{latitude:.3}_{longitude:.3}_{radius}");

        if let Some(cached) = self.get_cached(&cache_key) {
            return Ok(cached);
        }

        let query = format!(
            r#"[out:json][timeout:25];
(
  way["highway"="path"]["foot"="designated"](around:{radius},{latitude},{longitude});
  way["highway"="footway"](around:{radius},{latitude},{longitude});
  relation["route"="foot"](around:{radius},{latitude},{longitude});
  relation["route"="running"](around:{radius},{latitude},{longitude});
);
out tags center {MAX_ROUTES_PER_QUERY};"#,
        );

        let routes = self.query_overpass(&query, RouteType::Running).await?;
        self.set_cached(cache_key, routes.clone());
        Ok(routes)
    }

    /// Discover hiking trails near a location using Overpass API
    ///
    /// # Errors
    ///
    /// Returns an error if the Overpass API request fails or returns an error status.
    pub async fn discover_hiking_trails(
        &mut self,
        latitude: f64,
        longitude: f64,
        radius_meters: Option<u32>,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let radius = radius_meters.unwrap_or(DEFAULT_SEARCH_RADIUS_METERS);
        let cache_key = format!("hiking_{latitude:.3}_{longitude:.3}_{radius}");

        if let Some(cached) = self.get_cached(&cache_key) {
            return Ok(cached);
        }

        let query = format!(
            r#"[out:json][timeout:25];
(
  relation["route"="hiking"](around:{radius},{latitude},{longitude});
  way["highway"="path"]["sac_scale"](around:{radius},{latitude},{longitude});
);
out tags center {MAX_ROUTES_PER_QUERY};"#,
        );

        let routes = self.query_overpass(&query, RouteType::Hiking).await?;
        self.set_cached(cache_key, routes.clone());
        Ok(routes)
    }

    /// Discover ski trails near a location using OSM piste data
    ///
    /// # Errors
    ///
    /// Returns an error if the Overpass API request fails or returns an error status.
    pub async fn discover_ski_trails(
        &mut self,
        latitude: f64,
        longitude: f64,
        radius_meters: Option<u32>,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let radius = radius_meters.unwrap_or(DEFAULT_SEARCH_RADIUS_METERS);
        let cache_key = format!("ski_{latitude:.3}_{longitude:.3}_{radius}");

        if let Some(cached) = self.get_cached(&cache_key) {
            return Ok(cached);
        }

        // Query OSM piste data directly (same data source as OpenSkiMap)
        let query = format!(
            r#"[out:json][timeout:25];
(
  way["piste:type"="downhill"](around:{radius},{latitude},{longitude});
  way["piste:type"="nordic"](around:{radius},{latitude},{longitude});
  way["piste:type"="skitour"](around:{radius},{latitude},{longitude});
  relation["route"="ski"](around:{radius},{latitude},{longitude});
  relation["route"="piste"](around:{radius},{latitude},{longitude});
);
out tags center {MAX_ROUTES_PER_QUERY};"#,
        );

        let routes = self.query_overpass_ski(&query).await?;
        self.set_cached(cache_key, routes.clone());
        Ok(routes)
    }

    /// Discover routes for any sport type
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying Overpass API request fails.
    pub async fn discover_routes_for_sport(
        &mut self,
        sport: &SportType,
        latitude: f64,
        longitude: f64,
        radius_meters: Option<u32>,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        match sport {
            SportType::Ride
            | SportType::EbikeRide
            | SportType::GravelRide
            | SportType::MountainBike => {
                self.discover_cycling_routes(latitude, longitude, radius_meters)
                    .await
            }
            SportType::Run | SportType::TrailRunning => {
                self.discover_running_routes(latitude, longitude, radius_meters)
                    .await
            }
            SportType::Hike | SportType::Walk => {
                self.discover_hiking_trails(latitude, longitude, radius_meters)
                    .await
            }
            SportType::CrossCountrySkiing
            | SportType::AlpineSkiing
            | SportType::BackcountrySkiing
            | SportType::Snowshoe => {
                self.discover_ski_trails(latitude, longitude, radius_meters)
                    .await
            }
            _ => Ok(vec![]),
        }
    }

    // ========================================================================
    // Overpass API integration
    // ========================================================================

    /// Try each configured Overpass mirror in order until one answers with
    /// a successful response, then parse the JSON and return the elements.
    ///
    /// If every mirror fails (transient 5xx, network errors, parse errors),
    /// returns the accumulated failure reasons so callers can surface a
    /// single clear error instead of a cryptic "unknown server issue".
    async fn fetch_elements(&self, query: &str) -> AppResult<Vec<OverpassElement>> {
        let mut failures: Vec<String> = Vec::with_capacity(self.overpass_mirrors.len());

        for mirror in &self.overpass_mirrors {
            match self.try_mirror(mirror, query).await {
                Ok(elements) => return Ok(elements),
                Err(reason) => failures.push(reason),
            }
        }

        Err(AppError::internal(format!(
            "All Overpass mirrors failed: {}",
            failures.join(" | ")
        )))
    }

    /// Query a single Overpass mirror and return its parsed elements.
    ///
    /// Returns `Ok(elements)` on success, or `Err(reason)` describing why
    /// this mirror failed so the caller can accumulate a diagnostic across
    /// the full mirror list before surfacing a single error to the LLM.
    async fn try_mirror(&self, mirror: &str, query: &str) -> Result<Vec<OverpassElement>, String> {
        debug!(mirror, "Querying Overpass mirror");

        let response = self
            .client
            .post(mirror)
            .header(USER_AGENT, user_agent())
            .form(&[("data", query)])
            .send()
            .await
            .map_err(|e| {
                warn!(mirror, error = %e, "Overpass mirror network error");
                format!("{mirror}: network error: {e}")
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            // Keep the body in the per-mirror diagnostic but truncate so
            // three mirrors' worth of HTML error pages don't flood logs.
            let truncated: String = body.chars().take(200).collect();
            warn!(mirror, %status, "Overpass mirror returned error");
            return Err(format!("{mirror}: HTTP {status}: {truncated}"));
        }

        let data = response.json::<OverpassResponse>().await.map_err(|e| {
            warn!(mirror, error = %e, "Failed to parse Overpass response");
            format!("{mirror}: parse error: {e}")
        })?;

        debug!(
            mirror,
            element_count = data.elements.len(),
            "Overpass mirror answered successfully"
        );
        Ok(data.elements)
    }

    async fn query_overpass(
        &self,
        query: &str,
        route_type: RouteType,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let elements = self.fetch_elements(query).await?;

        let routes = elements
            .into_iter()
            .filter_map(|el| {
                let tags = el.tags?;
                let name = tags
                    .get("name")
                    .or_else(|| tags.get("ref"))
                    .cloned()
                    .unwrap_or_else(|| format!("Unnamed {}", route_type_label(&route_type)));

                let (lat, lon) = el
                    .center
                    .map(|c| (c.lat, c.lon))
                    .or_else(|| Some((el.lat?, el.lon?)))?;

                Some(DiscoveredRoute {
                    name,
                    route_type: route_type.clone(),
                    distance_meters: tags.get("distance").and_then(|d| d.parse::<f64>().ok()),
                    difficulty: tags.get("sac_scale").cloned(),
                    source: RouteSource::Overpass,
                    latitude: lat,
                    longitude: lon,
                })
            })
            .take(MAX_ROUTES_PER_QUERY)
            .collect();

        Ok(routes)
    }

    async fn query_overpass_ski(&self, query: &str) -> AppResult<Vec<DiscoveredRoute>> {
        let elements = self.fetch_elements(query).await?;

        let routes = elements
            .into_iter()
            .filter_map(|el| {
                let tags = el.tags?;
                let name = tags
                    .get("name")
                    .or_else(|| tags.get("ref"))
                    .cloned()
                    .unwrap_or_else(|| "Unnamed ski trail".to_owned());

                let piste_type = tags.get("piste:type").cloned().unwrap_or_default();
                let route_type = if piste_type == "downhill" {
                    RouteType::DownhillSki
                } else {
                    RouteType::CrossCountrySki
                };

                let (lat, lon) = el
                    .center
                    .map(|c| (c.lat, c.lon))
                    .or_else(|| Some((el.lat?, el.lon?)))?;

                Some(DiscoveredRoute {
                    name,
                    route_type,
                    distance_meters: tags.get("distance").and_then(|d| d.parse::<f64>().ok()),
                    difficulty: tags.get("piste:difficulty").cloned(),
                    source: RouteSource::OpenSkiMap,
                    latitude: lat,
                    longitude: lon,
                })
            })
            .take(MAX_ROUTES_PER_QUERY)
            .collect();

        Ok(routes)
    }

    // ========================================================================
    // Cache management
    // ========================================================================

    fn get_cached(&self, key: &str) -> Option<Vec<DiscoveredRoute>> {
        self.cache.get(key).and_then(|entry| {
            let elapsed = entry
                .cached_at
                .elapsed()
                .unwrap_or(Duration::from_secs(ROUTE_CACHE_DURATION_SECS + 1));
            if elapsed < Duration::from_secs(ROUTE_CACHE_DURATION_SECS) {
                debug!(key, "Route cache hit");
                // Clone is required — cached data is shared across multiple callers
                Some(entry.routes.clone())
            } else {
                None
            }
        })
    }

    fn set_cached(&mut self, key: String, routes: Vec<DiscoveredRoute>) {
        self.cache.insert(
            key,
            CachedRoutes {
                routes,
                cached_at: SystemTime::now(),
            },
        );
    }
}

// ============================================================================
// Overpass API response types
// ============================================================================

#[derive(Debug, Deserialize)]
struct OverpassResponse {
    elements: Vec<OverpassElement>,
}

#[derive(Debug, Deserialize)]
struct OverpassElement {
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
    #[serde(default)]
    center: Option<OverpassCenter>,
    #[serde(default)]
    tags: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct OverpassCenter {
    lat: f64,
    lon: f64,
}

#[must_use]
fn route_type_label(rt: &RouteType) -> &'static str {
    match rt {
        RouteType::Cycling => "cycling route",
        RouteType::Running => "running path",
        RouteType::Hiking => "hiking trail",
        RouteType::CrossCountrySki => "XC ski trail",
        RouteType::DownhillSki => "ski run",
        RouteType::Snowshoe => "snowshoe trail",
        RouteType::MultiUse => "trail",
    }
}
