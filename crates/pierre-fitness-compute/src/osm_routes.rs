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
use std::time::{Duration, Instant, SystemTime};
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

/// Metres in a kilometre — the unit OSM's `distance` tag is written in when
/// it names none.
const METERS_PER_KILOMETER: f64 = 1000.0;

/// Metres in a statute mile, for a `distance` tag written in miles.
const METERS_PER_MILE: f64 = 1609.344;

/// The units a `distance` tag may be written in, with their length in
/// metres. Ordered so a suffix is tried before any shorter suffix it ends in.
const DISTANCE_TAG_UNITS: &[(&str, f64)] = &[
    ("km", METERS_PER_KILOMETER),
    ("mi", METERS_PER_MILE),
    ("m", 1.0),
];

/// OSM stores a coordinate to seven decimal places; multiplying by this
/// turns one into the whole number two ways meeting at a node share.
const OSM_COORDINATE_SCALE: f64 = 1e7;

/// Walk steps one name's chain search may take before it settles for the
/// longest chain found. A name rarely has more than a few dozen fetched
/// segments, which an exhaustive search covers in hundreds of steps; the cap
/// is for a densely braided network, where the search grows exponentially
/// and a tool call cannot wait for it.
const MAX_CHAIN_SEARCH_STEPS: usize = 20_000;

/// How far from a target distance a route's usable distance may land, as a
/// share of the target, and still count as matching it. A tenth: a session
/// is planned to the kilometre and adjusted on the day by starting a few
/// hundred metres up the path, and a length summed along mapped geometry is
/// itself only good to a few percent.
const TARGET_TOLERANCE: f64 = 0.10;

/// Passes over a route that cover a target distance in one direction.
const SINGLE_PASS: u32 = 1;

/// Passes over a route that make one out-and-back.
const OUT_AND_BACK_PASSES: u32 = 2;

/// Process-wide Overpass result cache.
///
/// OSM route data is public and identical for every tenant, so one cache
/// serves them all. It lives outside [`RouteDiscoveryService`] because each
/// `discover_routes` tool call constructs a fresh service — a cache owned by
/// the service could never register a hit, and every agent turn would open a
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

/// Overall wall-clock budget across the whole mirror walk.
///
/// Each attempt is already bounded (`[timeout:25]` server-side under the
/// shared client's 30s request timeout), but three slow-hanging mirrors in a
/// row still cost ~90s — well past what a coaching turn tolerates. The budget
/// admits one fast-failing mirror plus one slow success (a healthy mirror
/// answers in 3-9s, a loaded one in ~25s) and then stops trying: no further
/// mirror is attempted once it is spent, and the accumulated failures come
/// back as the retryable error.
const OVERPASS_TOTAL_BUDGET: Duration = Duration::from_secs(45);

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
    /// Length in metres, when it could be established. See
    /// [`DiscoveredRoute::distance_source`] for what the figure measures.
    pub distance_meters: Option<f64>,
    /// Where `distance_meters` comes from; absent exactly when it is.
    pub distance_source: Option<DistanceSource>,
    /// Difficulty level (if available)
    pub difficulty: Option<String>,
    /// Data source (`OpenSkiMap`, Overpass, `OpenRouteService`)
    pub source: RouteSource,
    /// Latitude of the route start or center
    pub latitude: f64,
    /// Longitude of the route start or center
    pub longitude: f64,
    /// Straight-line distance from the search center, in metres. The agent
    /// quotes this to the athlete ("about 8 km from your door"), so it is
    /// measured rather than inferred from the coordinates by the model.
    pub distance_from_center_meters: f64,
    /// How the route serves the target distance the caller asked for. Absent
    /// when no target was given, or when the route's length is unknown.
    pub target_use: Option<TargetUse>,
}

/// What a route's `distance_meters` measures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DistanceSource {
    /// Measured along the longest chain of fetched ways that carry the name
    /// and join end to end. Same-named ways that do not connect to that
    /// chain are left out, so the figure is a stretch that exists on the
    /// ground and a lower bound on the trail: it can continue past the
    /// search, or through segments one query did not return.
    MappedGeometry,
    /// The length the mapper declared in the element's `distance` tag. The
    /// only length a route relation has — the query returns no geometry for
    /// one — and the fallback for a way without geometry. It is the whole
    /// route, however little of it lies inside the search.
    OsmTag,
}

/// How a route covers a target distance.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetFit {
    /// One direction covers the target: the route is as long as it, or
    /// short of it by no more than a tenth.
    SinglePass,
    /// Shorter than that, but out and back covers the target to within the
    /// same tenth.
    OutAndBack,
    /// Shorter still: it has to be run or ridden more than twice.
    Repeats,
}

/// A route measured against the target distance a caller asked for.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetUse {
    /// The shape of the session on this route.
    pub fit: TargetFit,
    /// How many times the route is covered: 1 for a single pass, 2 for an
    /// out-and-back, more for repeats — the fewest passes that reach the
    /// target to within a tenth.
    pub passes: u32,
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
    /// With a `target_distance_meters`, the routes whose usable distance lands
    /// closest to it lead the list — see [`select_routes`]; without one the
    /// order is curated itineraries, then trails, then connectors, nearest
    /// first within each.
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
        target_distance_meters: Option<f64>,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let radius = radius_meters.unwrap_or(DEFAULT_SEARCH_RADIUS_METERS);
        let Some(query) = build_overpass_query(sport, latitude, longitude, radius) else {
            return Ok(Vec::new());
        };

        let cache_key = format!(
            "{}_{latitude:.3}_{longitude:.3}_{radius}",
            query_family(sport)
        );
        // The cache holds every candidate the query produced, not the
        // twenty a caller sees: which twenty depends on the target distance,
        // and one Overpass round trip has to serve every target asked of it.
        if let Some(cached) = get_cached(&cache_key) {
            return Ok(select_routes(cached, target_distance_meters));
        }

        let candidates = self
            .fetch_candidates(&query, sport, latitude, longitude)
            .await?;
        set_cached(cache_key, candidates.clone());
        Ok(select_routes(candidates, target_distance_meters))
    }

    // ========================================================================
    // Overpass API integration
    // ========================================================================

    /// Try each configured Overpass mirror in order until one answers with a
    /// payload that parses, then rank it into the full candidate list.
    ///
    /// A mirror that answers 200 with an HTML error page counts as a failure
    /// and falls through to the next one — free Overpass instances do exactly
    /// that under load. If every mirror fails, the accumulated reasons come
    /// back as one error so the agent can say "retry" instead of fabricating.
    async fn fetch_candidates(
        &self,
        query: &str,
        sport: &SportType,
        center_lat: f64,
        center_lon: f64,
    ) -> AppResult<Vec<DiscoveredRoute>> {
        let mut failures: Vec<String> = Vec::with_capacity(self.overpass_mirrors.len());
        let started = Instant::now();

        for mirror in &self.overpass_mirrors {
            if started.elapsed() >= OVERPASS_TOTAL_BUDGET {
                failures.push(format!(
                    "budget exhausted after {:.0?}; remaining mirrors not tried",
                    started.elapsed()
                ));
                break;
            }
            let body = match self.try_mirror(mirror, query).await {
                Ok(body) => body,
                Err(reason) => {
                    failures.push(reason);
                    continue;
                }
            };
            match candidates_from_overpass_json(&body, sport, center_lat, center_lon) {
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
//
// Every query ends in `out tags geom`: `geom` adds each way's node
// coordinates and every element's bounding box to the same response. A
// relation gets its bounding box and nothing else — at `tags` verbosity
// Overpass leaves its members out, so a relation has no geometry here and is
// never measured. The ways' coordinates are what a length is summed from,
// and it costs no clause and no second request — only a larger body: measured
// around Prevost on 2026-10-01, 300 named ways came back as 560 kB carrying
// 9,823 points, where the same elements with a center point alone are about
// 70 kB. `geom` replaces `center` in the output, so the element's position
// is the middle of its bounding box — the same point `center` reports.
// ============================================================================

/// Build the Overpass query for a sport, or `None` when the sport has no
/// land or snow route surface to search.
///
/// Public so an operator can paste the exact query the agent ran into
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
/// is to hand the agent routes it can name to the athlete, and Overpass
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
out tags geom {OVERPASS_ELEMENT_BUDGET};"#
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
out tags geom {OVERPASS_ELEMENT_BUDGET};"#
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
out tags geom {OVERPASS_ELEMENT_BUDGET};"#
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
out tags geom {OVERPASS_ELEMENT_BUDGET};"#
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
/// Named elements only, deduplicated by name, each with the length its
/// geometry measures, capped at 20. Without a target distance the order is
/// rank class then distance from the search center; with one, see
/// [`select_routes`]. This is the whole of the selection logic —
/// [`RouteDiscoveryService`] adds only the HTTP round trip and the cache
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
    target_distance_meters: Option<f64>,
) -> AppResult<Vec<DiscoveredRoute>> {
    let candidates = candidates_from_overpass_json(body, sport, center_lat, center_lon)?;
    Ok(select_routes(candidates, target_distance_meters))
}

/// Every named route in an Overpass payload, deduplicated and in the default
/// order, before the caller-facing cap.
fn candidates_from_overpass_json(
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

/// Length in metres along a line of points, summed great-circle leg by leg.
///
/// A gap in the line (Overpass writes `null` for a node it clipped away)
/// ends one run of legs and starts the next, so no leg is drawn across it.
fn polyline_length_meters(points: &[Option<OverpassPoint>]) -> f64 {
    points
        .windows(2)
        .filter_map(|pair| match (&pair[0], &pair[1]) {
            (Some(from), Some(to)) => {
                Some(haversine_meters_between(from.lat, from.lon, to.lat, to.lon))
            }
            _ => None,
        })
        .sum()
}

/// Where a way's line starts and ends, as a comparable key.
///
/// Two ways that meet share a node, and a node has one pair of coordinates,
/// so equal keys mean the ways connect end to end.
type EndPoint = (i64, i64);

/// One fetched way as a link in a chain: its two ends and its length.
#[derive(Debug, Clone, Copy)]
struct WaySegment {
    start: EndPoint,
    end: EndPoint,
    meters: f64,
}

/// A point as an [`EndPoint`], at the precision OSM stores coordinates in.
fn end_point(point: &OverpassPoint) -> EndPoint {
    // A coordinate times 1e7 is at most 1.8e9 in magnitude: it fits.
    #[allow(clippy::cast_possible_truncation)]
    let key = |degrees: f64| (degrees * OSM_COORDINATE_SCALE).round() as i64;
    (key(point.lat), key(point.lon))
}

/// A way's geometry as a chain link, or `None` when the response carried no
/// line for it or the line has no length.
fn way_segment(geometry: Option<&[Option<OverpassPoint>]>) -> Option<WaySegment> {
    let points = geometry?;
    let start = points.iter().flatten().next()?;
    let end = points.iter().flatten().next_back()?;
    let meters = polyline_length_meters(points);
    (meters > 0.0).then(|| WaySegment {
        start: end_point(start),
        end: end_point(end),
        meters,
    })
}

/// Length of the longest chain among ways that share a name.
///
/// A chain is a run of ways joined end to end that passes through no end
/// point twice. Ways that merely share a name — a second trail of the same
/// name across town, the opposite carriageway of a divided path — are not
/// joined to it, so they are never added in; of two ways between the same
/// pair of ends only one is taken. Every chain is a stretch that exists on
/// the ground, which is what makes the result a lower bound on the trail.
///
/// The search is exhaustive up to [`MAX_CHAIN_SEARCH_STEPS`]; past that it
/// returns the longest chain it has found, which is still a real one.
fn longest_chain_meters(segments: &[WaySegment]) -> Option<f64> {
    let mut search = ChainSearch {
        segments,
        steps_left: MAX_CHAIN_SEARCH_STEPS,
        best: 0.0,
    };
    // Start from ends that only one way touches first: on an unbranched trail
    // those are its two extremities, and the first walk is already the answer.
    let mut touches: HashMap<EndPoint, usize> = HashMap::new();
    for segment in segments {
        *touches.entry(segment.start).or_insert(0) += 1;
        *touches.entry(segment.end).or_insert(0) += 1;
    }
    let mut starts: Vec<EndPoint> = touches.keys().copied().collect();
    starts.sort_by_key(|point| (touches.get(point).copied().unwrap_or(0), *point));
    for start in starts {
        let mut visited = vec![start];
        search.extend(start, 0.0, &mut visited);
    }
    (search.best > 0.0).then_some(search.best)
}

/// The state of one [`longest_chain_meters`] search.
struct ChainSearch<'a> {
    segments: &'a [WaySegment],
    steps_left: usize,
    best: f64,
}

impl ChainSearch<'_> {
    /// Walk on from `at`, having covered `meters` through `visited`.
    fn extend(&mut self, at: EndPoint, meters: f64, visited: &mut Vec<EndPoint>) {
        self.best = self.best.max(meters);
        for index in 0..self.segments.len() {
            let segment = self.segments[index];
            let next = if segment.start == at {
                segment.end
            } else if segment.end == at {
                segment.start
            } else {
                continue;
            };
            if next == at {
                // A way that closes on itself is a chain of its own.
                self.best = self.best.max(meters + segment.meters);
                continue;
            }
            if visited.contains(&next) {
                continue;
            }
            if self.steps_left == 0 {
                return;
            }
            self.steps_left -= 1;
            visited.push(next);
            self.extend(next, meters + segment.meters, visited);
            visited.pop();
        }
    }
}

/// Read OSM's `distance` tag into metres.
///
/// The tag is kilometres unless it names a unit (`"12"`, `"12.5 km"`,
/// `"7 mi"`, `"800 m"`), with a comma accepted as the decimal mark. A value
/// that is not a positive number in one of those units is no length at all.
fn parse_distance_tag_meters(raw: &str) -> Option<f64> {
    let normalized = raw.trim().to_lowercase().replace(',', ".");
    // Longest suffix first: "km" also ends in "m".
    let (figure, unit_meters) = DISTANCE_TAG_UNITS
        .iter()
        .find_map(|(suffix, meters)| Some((normalized.strip_suffix(suffix)?, *meters)))
        .unwrap_or((normalized.as_str(), METERS_PER_KILOMETER));
    let value = figure.trim().parse::<f64>().ok()?;
    (value.is_finite() && value > 0.0).then_some(value * unit_meters)
}

/// One named element on its way through ranking.
struct RankedElement {
    class: u8,
    is_relation: bool,
    /// A way's line as a chain link; `None` for a relation, and for a way
    /// the response carried no geometry for.
    segment: Option<WaySegment>,
    /// What its `distance` tag declares.
    tagged_meters: Option<f64>,
    route: DiscoveredRoute,
}

fn rank_elements(
    elements: Vec<OverpassElement>,
    sport: &SportType,
    center_lat: f64,
    center_lon: f64,
) -> Vec<DiscoveredRoute> {
    let source = source_for(sport);
    let mut scored: Vec<RankedElement> = elements
        .into_iter()
        .filter_map(|el| {
            let is_relation = el.element_type == "relation";
            let segment = if is_relation {
                None
            } else {
                way_segment(el.geometry.as_deref())
            };
            let tags = el.tags?;
            // `ref` carries the trail number when a route has no name — a
            // usable label. An element with neither is not something the
            // agent can point an athlete at, so it is dropped rather than
            // padded out with an "Unnamed ..." placeholder.
            let name = tags.get("name").or_else(|| tags.get("ref"))?.clone();
            let (lat, lon) = el
                .center
                .map(|c| (c.lat, c.lon))
                .or_else(|| el.bounds.map(|b| b.center()))
                .or_else(|| Some((el.lat?, el.lon?)))?;
            let distance = haversine_meters_between(center_lat, center_lon, lat, lon);

            Some(RankedElement {
                class: rank_class(&el.element_type, &tags),
                is_relation,
                segment,
                tagged_meters: tags
                    .get("distance")
                    .and_then(|raw| parse_distance_tag_meters(raw)),
                route: DiscoveredRoute {
                    name,
                    route_type: classify(sport, &tags),
                    distance_meters: None,
                    distance_source: None,
                    difficulty: tags
                        .get("piste:difficulty")
                        .or_else(|| tags.get("sac_scale"))
                        .or_else(|| tags.get("mtb:scale"))
                        .cloned(),
                    source: source.clone(),
                    latitude: lat,
                    longitude: lon,
                    distance_from_center_meters: distance,
                    target_use: None,
                },
            })
        })
        .collect();

    scored.sort_by(|a, b| {
        a.class.cmp(&b.class).then(
            a.route
                .distance_from_center_meters
                .total_cmp(&b.route.distance_from_center_meters),
        )
    });

    // OSM splits a long trail into many ways that all share one name, so
    // gather the links by name before the duplicates are dropped.
    let mut segments_by_name: HashMap<String, Vec<WaySegment>> = HashMap::new();
    for element in &scored {
        if let Some(segment) = element.segment {
            segments_by_name
                .entry(element.route.name.to_lowercase())
                .or_default()
                .push(segment);
        }
    }

    // Deduplicate after sorting — the surviving copy is the best-ranked,
    // nearest element carrying the name.
    let mut seen: Vec<String> = Vec::with_capacity(scored.len());
    let mut routes = Vec::with_capacity(scored.len());
    for element in scored {
        let key = element.route.name.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        // A relation's length is what its mapper declared, or unknown: the
        // response carries no geometry for it, and the same-named ways that
        // happened to fall inside the search are a fragment of it, not it.
        // A way is measured along its longest chain, and falls back to its
        // own tag only when there is no geometry to measure.
        let measured = if element.is_relation {
            None
        } else {
            segments_by_name
                .get(&key)
                .and_then(|segments| longest_chain_meters(segments))
        };
        let mut route = element.route;
        if let Some(meters) = measured {
            route.distance_meters = Some(meters);
            route.distance_source = Some(DistanceSource::MappedGeometry);
        } else if let Some(meters) = element.tagged_meters {
            route.distance_meters = Some(meters);
            route.distance_source = Some(DistanceSource::OsmTag);
        }
        seen.push(key);
        routes.push(route);
    }
    routes
}

/// How a route of `length_meters` is used for a session of `target_meters`,
/// or `None` when either is not a positive finite length.
///
/// The passes are the fewest whose total reaches the target to within
/// [`TARGET_TOLERANCE`]: a route a little short of the target still holds
/// the session in one pass, and one a little short of half of it in an
/// out-and-back.
fn target_use(length_meters: f64, target_meters: f64) -> Option<TargetUse> {
    let usable = length_meters.is_finite()
        && target_meters.is_finite()
        && length_meters > 0.0
        && target_meters > 0.0;
    if !usable {
        return None;
    }
    // The fewest passes whose total reaches the target, give or take the
    // tolerance: a length is positive here, so the division is safe.
    let ratio = (target_meters * (1.0 - TARGET_TOLERANCE) / length_meters).ceil();
    // One too large for the counter saturates; one below 1 is a single pass.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let passes = if ratio >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        ratio as u32
    }
    .max(SINGLE_PASS);
    let fit = if passes == SINGLE_PASS {
        TargetFit::SinglePass
    } else if passes == OUT_AND_BACK_PASSES {
        TargetFit::OutAndBack
    } else {
        TargetFit::Repeats
    };
    Some(TargetUse { fit, passes })
}

/// Where a route stands against a target distance — the whole ranking rule.
///
/// The usable distance is the route's length times its passes, and the
/// mismatch is how far that lands from the target, as a share of the target.
/// Lowest key first:
///
/// 1. **Near matches** — one pass or one out-and-back whose usable distance
///    is within [`TARGET_TOLERANCE`] of the target. A single pass leads an
///    out-and-back; among equals the smaller mismatch leads. A route slightly
///    short of the target is a near match like one slightly long.
/// 2. **Other single passes and out-and-backs**, by mismatch alone — so a
///    route a little over the target leads one many times its length, and
///    neither leads a near match.
/// 3. **Repeats**, fewest passes first, then by mismatch.
/// 4. **Unknown length** — nothing can be said about it.
///
/// Ties keep the default order (rank class, then distance from the centre).
fn target_rank(route: &DiscoveredRoute, target_meters: f64) -> (u8, u32, f64) {
    const NEAR_MATCH: u8 = 0;
    const OTHER_DIRECT: u8 = 1;
    const REPEATS: u8 = 2;
    const UNKNOWN: u8 = 3;

    let (Some(length), Some(usage)) = (route.distance_meters, route.target_use) else {
        return (UNKNOWN, 0, 0.0);
    };
    // `target_use` is `Some` only for a positive finite target, so the
    // division is safe.
    let mismatch = length
        .mul_add(f64::from(usage.passes), -target_meters)
        .abs()
        / target_meters;
    if usage.passes > OUT_AND_BACK_PASSES {
        (REPEATS, usage.passes, mismatch)
    } else if mismatch <= TARGET_TOLERANCE {
        (NEAR_MATCH, usage.passes, mismatch)
    } else {
        (OTHER_DIRECT, 0, mismatch)
    }
}

/// Cut a candidate list down to the routes a caller sees.
///
/// `candidates` arrive in the default order — rank class, then distance from
/// the search center. Without a target that order stands and the first 20 are
/// returned.
///
/// With a target, each route of known length is marked with how it covers
/// the distance ([`TargetUse`]) and the list is re-ordered by
/// [`target_rank`] before the cut, so a near match is never lost to it.
#[must_use]
pub fn select_routes(
    mut candidates: Vec<DiscoveredRoute>,
    target_distance_meters: Option<f64>,
) -> Vec<DiscoveredRoute> {
    if let Some(target) = target_distance_meters {
        for route in &mut candidates {
            route.target_use = route
                .distance_meters
                .and_then(|length| target_use(length, target));
        }
        // `sort_by` is stable, so equal keys keep the default order.
        candidates.sort_by(|a, b| {
            let (left, right) = (target_rank(a, target), target_rank(b, target));
            left.0
                .cmp(&right.0)
                .then(left.1.cmp(&right.1))
                .then(left.2.total_cmp(&right.2))
        });
    }
    candidates.truncate(MAX_ROUTES_PER_QUERY);
    candidates
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
    /// The element's bounding box, present when the query asked for `geom`.
    #[serde(default)]
    bounds: Option<OverpassBounds>,
    /// A way's line of nodes, present when the query asked for `geom`.
    #[serde(default)]
    geometry: Option<Vec<Option<OverpassPoint>>>,
}

#[derive(Debug, Deserialize)]
struct OverpassBounds {
    minlat: f64,
    minlon: f64,
    maxlat: f64,
    maxlon: f64,
}

impl OverpassBounds {
    /// The middle of the box — the point Overpass reports as `center`.
    fn center(&self) -> (f64, f64) {
        (
            f64::midpoint(self.minlat, self.maxlat),
            f64::midpoint(self.minlon, self.maxlon),
        )
    }
}

#[derive(Debug, Deserialize)]
struct OverpassPoint {
    lat: f64,
    lon: f64,
}

#[derive(Debug, Deserialize)]
struct OverpassCenter {
    lat: f64,
    lon: f64,
}
