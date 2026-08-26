// ABOUTME: Route and trail discovery service using Overpass API for OpenStreetMap data
// ABOUTME: Discovers running, cycling, and ski routes near a given location
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::routes::haversine_meters_between;
use pierre_core::constants::project::user_agent;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::api_client as shared_client;
use pierre_core::http_client::SharedHttpClient;
use pierre_core::models::SportType;
use reqwest::header::USER_AGENT;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, SystemTime};
use tracing::{debug, warn};

/// Cache duration for route queries (24 hours)
const ROUTE_CACHE_DURATION_SECS: u64 = 86400;

/// Maximum number of routes to return per query
const MAX_ROUTES_PER_QUERY: usize = 20;

/// How many raw OSM elements to pull from Overpass before ranking locally.
///
/// Overpass's `out <n>` limit truncates server-side in element-id order, not
/// by relevance or proximity — so a budget the size of the caller-facing
/// result set hands back whichever ways happen to carry the lowest ids. In a
/// suburb that is a wall of sidewalks, and the named trail three kilometres
/// out never enters the response at all. Fetch a wide slice and let
/// [`rank_elements`] decide which [`MAX_ROUTES_PER_QUERY`] the athlete sees.
const OVERPASS_ELEMENT_BUDGET: usize = 300;

/// Upper bound on distinct cache keys held in [`ROUTE_CACHE`].
///
/// Keys are rounded to three decimal degrees (~110 m), so a busy tenant base
/// spread across a country still lands in the low hundreds. The cap keeps a
/// pathological caller from growing the map without bound.
const MAX_CACHE_ENTRIES: usize = 512;

/// Default search radius in meters for Overpass queries
const DEFAULT_SEARCH_RADIUS_METERS: u32 = 10_000;

/// Process-wide Overpass result cache.
///
/// OSM route data is public and identical for every tenant, so one cache
/// serves them all. It lives outside [`RouteDiscoveryService`] because each
/// `discover_routes` tool call constructs a fresh service — a cache owned by
/// the service could never register a hit, and every coach turn would open a
/// new round of requests against a shared free API that answers 502 under
/// load.
static ROUTE_CACHE: LazyLock<RwLock<HashMap<String, CachedRoutes>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

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
    /// Straight-line distance from the search center, in metres. The coach
    /// quotes this to the athlete ("about 8 km from your door"), so it is
    /// measured rather than inferred from the coordinates by the model.
    pub distance_from_center_meters: f64,
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
    client: &'static SharedHttpClient,
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
            overpass_mirrors: OVERPASS_MIRRORS.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    /// Discover named routes near a location for a given sport.
    ///
    /// Returns an empty list for sports with no land or snow route surface
    /// (swim, gym work): there is nothing in OSM to ground them in, and an
    /// empty list is the honest answer rather than an unrelated fallback.
    ///
    /// # Errors
    ///
    /// Returns an error when every Overpass mirror fails to answer.
    pub async fn discover_routes_for_sport(
        &self,
        sport: &SportType,
        latitude: f64,
        longitude: f64,
        radius_meters: Option<u32>,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let radius = radius_meters.unwrap_or(DEFAULT_SEARCH_RADIUS_METERS);
        let Some(query) = build_overpass_query(sport, latitude, longitude, radius) else {
            return Ok(Vec::new());
        };

        let cache_key = format!(
            "{}_{latitude:.3}_{longitude:.3}_{radius}",
            query_family(sport)
        );
        if let Some(cached) = get_cached(&cache_key) {
            return Ok(cached);
        }

        let routes = self
            .fetch_ranked(&query, sport, latitude, longitude)
            .await?;
        set_cached(cache_key, routes.clone());
        Ok(routes)
    }

    // ========================================================================
    // Overpass API integration
    // ========================================================================

    /// Try each configured Overpass mirror in order until one answers with a
    /// payload that parses, then rank it into the caller-facing route list.
    ///
    /// A mirror that answers 200 with an HTML error page counts as a failure
    /// and falls through to the next one — free Overpass instances do exactly
    /// that under load. If every mirror fails, the accumulated reasons come
    /// back as one error so the coach can say "retry" instead of fabricating.
    async fn fetch_ranked(
        &self,
        query: &str,
        sport: &SportType,
        center_lat: f64,
        center_lon: f64,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let mut failures: Vec<String> = Vec::with_capacity(self.overpass_mirrors.len());

        for mirror in &self.overpass_mirrors {
            let body = match self.try_mirror(mirror, query).await {
                Ok(body) => body,
                Err(reason) => {
                    failures.push(reason);
                    continue;
                }
            };
            match routes_from_overpass_json(&body, sport, center_lat, center_lon) {
                Ok(routes) => {
                    debug!(mirror, count = routes.len(), "Overpass mirror answered");
                    return Ok(routes);
                }
                Err(e) => {
                    warn!(mirror, error = %e, "Failed to parse Overpass response");
                    failures.push(format!("{mirror}: parse error: {e}"));
                }
            }
        }

        Err(AppError::internal(format!(
            "All Overpass mirrors failed: {}",
            failures.join(" | ")
        )))
    }

    /// Query a single Overpass mirror and return its raw response body.
    ///
    /// Returns `Ok(body)` on a successful status, or `Err(reason)` describing
    /// why this mirror failed so the caller can accumulate a diagnostic across
    /// the full mirror list before surfacing a single error to the LLM.
    async fn try_mirror(&self, mirror: &str, query: &str) -> Result<String, String> {
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

        response.text().await.map_err(|e| {
            warn!(mirror, error = %e, "Reading Overpass response body failed");
            format!("{mirror}: body read error: {e}")
        })
    }
}

// ============================================================================
// Overpass query construction
//
// Keep the clause count down. Overpass re-evaluates the spatial `(around:...)`
// filter once per clause and that dominates the cost — measured around
// Prevost, a two-clause query answered in 3s, three clauses in 2-9s, and five
// in 24.9s against a 25s server timeout sitting under a 30s client timeout.
// Splitting a tag regex into one exact-match clause per value is the wrong
// instinct for the same reason: the eight-clause form of the running query
// took 24.7s where the three-clause regex form took 3.3s. Group tags into a
// regex, and add extra tag predicates to an existing clause rather than
// opening a new one — those are nearly free.
// ============================================================================

/// Build the Overpass query for a sport, or `None` when the sport has no
/// land or snow route surface to search.
///
/// Public so an operator can paste the exact query the coach ran into
/// overpass-turbo and see the same elements come back.
#[must_use]
pub fn build_overpass_query(
    sport: &SportType,
    latitude: f64,
    longitude: f64,
    radius: u32,
) -> Option<String> {
    match sport {
        SportType::Ride
        | SportType::EbikeRide
        | SportType::GravelRide
        | SportType::MountainBike => Some(build_cycling_query(latitude, longitude, radius)),
        SportType::Run | SportType::TrailRunning => {
            Some(build_running_query(latitude, longitude, radius))
        }
        SportType::Hike | SportType::Walk => Some(build_hiking_query(latitude, longitude, radius)),
        SportType::CrossCountrySkiing
        | SportType::AlpineSkiing
        | SportType::BackcountrySkiing
        | SportType::Snowshoe => Some(build_ski_query(latitude, longitude, radius)),
        _ => None,
    }
}

/// Cache family for a sport — every sport sharing a query shares its cached
/// results, so a trail run and a hike around the same point cost one lookup.
fn query_family(sport: &SportType) -> &'static str {
    match sport {
        SportType::Ride
        | SportType::EbikeRide
        | SportType::GravelRide
        | SportType::MountainBike => "cycling",
        SportType::Hike | SportType::Walk => "hiking",
        SportType::CrossCountrySkiing
        | SportType::AlpineSkiing
        | SportType::BackcountrySkiing
        | SportType::Snowshoe => "ski",
        _ => "running",
    }
}

/// Render the `(around:...)` filter shared by every clause of a query.
fn around(latitude: f64, longitude: f64, radius: u32) -> String {
    format!("(around:{radius},{latitude},{longitude})")
}

/// Build the running/trail-running Overpass query.
///
/// Every clause carries `["name"]`. That is not cosmetic: the tool's contract
/// is to hand the coach routes it can name to the athlete, and Overpass
/// truncates in element-id order, so admitting unnamed ways lets a city's
/// sidewalk mesh consume the whole response budget before a single named
/// trail is reached. `footway=sidewalk` and `footway=crossing` are excluded
/// for the same reason — they carry the abutting street's name and are not
/// routes. `path`/`track`/`bridleway` cover the trails that Québec mapping
/// puts outside `foot=designated`, and named `cycleway` picks up the linear
/// riverside parks that are runnable but tagged for bikes.
fn build_running_query(latitude: f64, longitude: f64, radius: u32) -> String {
    let a = around(latitude, longitude, radius);
    format!(
        r#"[out:json][timeout:25];
(
  relation["route"~"^(foot|hiking|running)$"]["name"]{a};
  way["highway"~"^(path|track|bridleway)$"]["name"]{a};
  way["highway"~"^(footway|cycleway)$"]["name"]["footway"!~"^(sidewalk|crossing)$"]{a};
);
out tags center {OVERPASS_ELEMENT_BUDGET};"#
    )
}

/// Build the cycling Overpass query (road, gravel, and mountain bike).
///
/// `highway=track` is what gravel rides are made of and `highway=path` is where
/// singletrack lives — neither is reachable through `cycleway`/`bicycle=designated`
/// alone, which is why a gravel-heavy region used to come back as a list of
/// downtown streets. Ways explicitly closed to bikes are filtered out on the
/// same clause; extra tag predicates are nearly free, unlike extra clauses.
fn build_cycling_query(latitude: f64, longitude: f64, radius: u32) -> String {
    let a = around(latitude, longitude, radius);
    format!(
        r#"[out:json][timeout:25];
(
  relation["route"~"^(bicycle|mtb)$"]["name"]{a};
  way["highway"~"^(cycleway|track|path)$"]["name"]["bicycle"!~"^(no|dismount)$"]{a};
  way["bicycle"="designated"]["name"]{a};
);
out tags center {OVERPASS_ELEMENT_BUDGET};"#
    )
}

/// Build the hiking/walking Overpass query.
///
/// `sac_scale` is an alpine difficulty tag that almost nothing in eastern
/// North America sets, so requiring it returned nothing across whole regions.
/// Named paths, tracks and non-sidewalk footways carry the trails instead.
fn build_hiking_query(latitude: f64, longitude: f64, radius: u32) -> String {
    let a = around(latitude, longitude, radius);
    format!(
        r#"[out:json][timeout:25];
(
  relation["route"~"^(hiking|foot)$"]["name"]{a};
  way["highway"~"^(path|track|bridleway)$"]["name"]{a};
  way["highway"="footway"]["name"]["footway"!~"^(sidewalk|crossing)$"]{a};
);
out tags center {OVERPASS_ELEMENT_BUDGET};"#
    )
}

/// Build the ski/snowshoe Overpass query against OSM piste data — the same
/// source `OpenSkiMap` renders.
fn build_ski_query(latitude: f64, longitude: f64, radius: u32) -> String {
    let a = around(latitude, longitude, radius);
    format!(
        r#"[out:json][timeout:25];
(
  relation["route"~"^(ski|piste)$"]["name"]{a};
  way["piste:type"~"^(downhill|nordic|skitour)$"]["name"]{a};
);
out tags center {OVERPASS_ELEMENT_BUDGET};"#
    )
}

// ============================================================================
// Ranking
// ============================================================================

/// Rank class for an OSM element, lowest first.
///
/// A signed itinerary relation five kilometres out is a better answer than a
/// named park connector across the street, so class outranks proximity;
/// within a class the nearest wins.
fn rank_class(element_type: &str, tags: &HashMap<String, String>) -> u8 {
    if element_type == "relation" && tags.contains_key("route") {
        return 0;
    }
    let is_trail = tags.contains_key("piste:type")
        || tags.contains_key("mtb:scale")
        || tags.contains_key("sac_scale")
        || matches!(
            tags.get("highway").map(String::as_str),
            Some("path" | "track" | "bridleway")
        );
    if is_trail {
        1
    } else {
        2
    }
}

/// Label an element with the route type the athlete should picture.
///
/// Ski elements carry their own answer in `piste:type`. Land elements take
/// the type implied by the sport, narrowed to [`RouteType::MultiUse`] when the
/// way is explicitly shared between feet and wheels — a runner should know a
/// "trail" is also a bike path before showing up on it.
fn classify(sport: &SportType, tags: &HashMap<String, String>) -> RouteType {
    match sport {
        SportType::CrossCountrySkiing
        | SportType::AlpineSkiing
        | SportType::BackcountrySkiing
        | SportType::Snowshoe => {
            if tags.get("piste:type").map(String::as_str) == Some("downhill") {
                RouteType::DownhillSki
            } else {
                RouteType::CrossCountrySki
            }
        }
        _ => {
            let shared = tags.get("foot").map(String::as_str) == Some("designated")
                && tags.get("bicycle").map(String::as_str) == Some("designated");
            if shared {
                return RouteType::MultiUse;
            }
            match sport {
                SportType::Ride
                | SportType::EbikeRide
                | SportType::GravelRide
                | SportType::MountainBike => RouteType::Cycling,
                SportType::Hike | SportType::Walk => RouteType::Hiking,
                _ => RouteType::Running,
            }
        }
    }
}

/// Where a sport's results come from — ski queries read OSM piste data, the
/// same layer `OpenSkiMap` renders; everything else is a plain Overpass query.
fn source_for(sport: &SportType) -> RouteSource {
    match sport {
        SportType::CrossCountrySkiing
        | SportType::AlpineSkiing
        | SportType::BackcountrySkiing
        | SportType::Snowshoe => RouteSource::OpenSkiMap,
        _ => RouteSource::Overpass,
    }
}

/// Parse an Overpass JSON payload into the ranked, caller-facing route list.
///
/// Named elements only, deduplicated by name, ordered by rank class then
/// distance from the search center, capped at 20. This is the whole of the
/// selection logic — [`RouteDiscoveryService`] adds only the HTTP round trip
/// around it, so a captured payload exercises exactly what production runs.
///
/// # Errors
///
/// Returns an error when the body is not a parseable Overpass JSON response —
/// a free mirror answering 200 with an HTML error page lands here.
pub fn routes_from_overpass_json(
    body: &str,
    sport: &SportType,
    center_lat: f64,
    center_lon: f64,
) -> AppResult<Vec<DiscoveredRoute>> {
    let parsed: OverpassResponse = serde_json::from_str(body)
        .map_err(|e| AppError::internal(format!("Overpass response is not valid JSON: {e}")))?;
    Ok(rank_elements(
        parsed.elements,
        sport,
        center_lat,
        center_lon,
    ))
}

fn rank_elements(
    elements: Vec<OverpassElement>,
    sport: &SportType,
    center_lat: f64,
    center_lon: f64,
) -> Vec<DiscoveredRoute> {
    let source = source_for(sport);
    let mut scored: Vec<(u8, f64, DiscoveredRoute)> = elements
        .into_iter()
        .filter_map(|el| {
            let tags = el.tags?;
            // `ref` carries the trail number when a route has no name — a
            // usable label. An element with neither is not something the
            // coach can point an athlete at, so it is dropped rather than
            // padded out with an "Unnamed ..." placeholder.
            let name = tags.get("name").or_else(|| tags.get("ref"))?.clone();
            let (lat, lon) = el
                .center
                .map(|c| (c.lat, c.lon))
                .or_else(|| Some((el.lat?, el.lon?)))?;
            let distance = haversine_meters_between(center_lat, center_lon, lat, lon);
            let class = rank_class(&el.element_type, &tags);

            Some((
                class,
                distance,
                DiscoveredRoute {
                    name,
                    route_type: classify(sport, &tags),
                    distance_meters: tags.get("distance").and_then(|d| d.parse::<f64>().ok()),
                    difficulty: tags
                        .get("piste:difficulty")
                        .or_else(|| tags.get("sac_scale"))
                        .or_else(|| tags.get("mtb:scale"))
                        .cloned(),
                    source: source.clone(),
                    latitude: lat,
                    longitude: lon,
                    distance_from_center_meters: distance,
                },
            ))
        })
        .collect();

    scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));

    // OSM splits a long trail into many ways that all share one name, so
    // deduplicate after sorting — the surviving copy is the nearest segment.
    let mut seen: Vec<String> = Vec::with_capacity(MAX_ROUTES_PER_QUERY);
    let mut routes = Vec::with_capacity(MAX_ROUTES_PER_QUERY);
    for (_, _, route) in scored {
        let key = route.name.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        routes.push(route);
        if routes.len() == MAX_ROUTES_PER_QUERY {
            break;
        }
    }
    routes
}

// ============================================================================
// Cache management
// ============================================================================

fn get_cached(key: &str) -> Option<Vec<DiscoveredRoute>> {
    let cache = ROUTE_CACHE.read().ok()?;
    let entry = cache.get(key)?;
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
}

fn set_cached(key: String, routes: Vec<DiscoveredRoute>) {
    let Ok(mut cache) = ROUTE_CACHE.write() else {
        return;
    };
    if cache.len() >= MAX_CACHE_ENTRIES {
        let ttl = Duration::from_secs(ROUTE_CACHE_DURATION_SECS);
        cache.retain(|_, entry| entry.cached_at.elapsed().is_ok_and(|elapsed| elapsed < ttl));
        // Every entry is still live — drop the whole map rather than grow
        // past the cap. Route lookups are cheap to rebuild; unbounded
        // residency in a long-lived server process is not.
        if cache.len() >= MAX_CACHE_ENTRIES {
            cache.clear();
        }
    }
    cache.insert(
        key,
        CachedRoutes {
            routes,
            cached_at: SystemTime::now(),
        },
    );
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
    /// `"way"` or `"relation"` — drives the rank class, since a route
    /// relation is a curated itinerary and a way is a single segment.
    #[serde(rename = "type", default)]
    element_type: String,
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
