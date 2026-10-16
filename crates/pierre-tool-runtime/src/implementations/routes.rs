// ABOUTME: MCP tool for discovering real running, cycling, hiking, and ski routes from OSM data
// ABOUTME: Wraps RouteDiscoveryService and stores each mapped line under a route_ref a map or compose_route reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::future::join_all;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use pierre_cache::Cache;
use pierre_fitness_compute::geocoding::{free_text_query, place_geocoder};
use pierre_fitness_compute::osm_routes::{
    DiscoveredRoute, DistanceSource, RouteDiscoveryService, TargetFit,
};
use pierre_fitness_compute::route_compose;

use super::compose_route::ComposeRouteTool;
use crate::context::ToolExecutionContext;
use crate::conversions::{answers_with, ok_typed, tool_definition, tool_result_to_response};
use crate::route_store::{self, StoredRoute};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_meteo::{GeocodeError, Geocoder, Place};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{resolve_sport_type, SportType, TenantId};
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// What `discover_routes` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DiscoverRoutesResult {
    /// The sport the search was for, as its serde name.
    pub sport_type: String,
    /// Where the search was centred, after resolving whatever the caller
    /// named — a place, or coordinates.
    pub center: RouteSearchCenter,
    /// How far out the search reached, in metres.
    pub radius_meters: u32,
    /// The session distance the routes were ranked against, in metres. Absent
    /// when the caller named none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_distance_meters: Option<f64>,
    /// How many routes came back.
    pub count: usize,
    /// The routes: nearest first within each class, or, when a target
    /// distance was given, the ones whose usable distance lands closest to
    /// it first.
    pub routes: Vec<DiscoveredRouteEntry>,
}

/// Where a route search was centred.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RouteSearchCenter {
    /// Latitude of the centre.
    pub latitude: f64,
    /// Longitude of the centre.
    pub longitude: f64,
    /// The resolved place name. Absent when the caller gave coordinates, so
    /// there was no name to resolve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// One route as `discover_routes` reports it.
///
/// Grounded in OpenStreetMap, which is why so much is optional: a way tagged
/// by a volunteer may carry a name and nothing else.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DiscoveredRouteEntry {
    /// The route's name in OSM.
    pub name: String,
    /// What kind of route it is, lowercased: cycling, hiking, ski.
    pub route_type: String,
    /// Length in metres, rounded; absent when it is unknown — a route
    /// relation with no declared length, or a way with neither geometry nor
    /// one.
    pub distance_meters: Option<f64>,
    /// What `distance_meters` measures. `mapped_geometry`: the longest run
    /// of fetched ways carrying this name that join end to end — a stretch
    /// that exists on the ground and a lower bound on the trail, which can
    /// continue past it. `osm_tag`: the length the mapper declared for the
    /// whole route, however little of it lies inside the search radius.
    pub distance_source: Option<String>,
    /// How the route covers the target distance: `single_pass` (as long as
    /// the target, or short of it by no more than a tenth), `out_and_back`
    /// (out and back reaches it to within that tenth), `repeats` (shorter
    /// still). Present only when a target was given and the route's length
    /// is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_fit: Option<String>,
    /// How many times the route is covered — 1, 2 for an out-and-back, more
    /// for repeats: the fewest passes that reach the target to within a
    /// tenth. Present with `target_fit`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passes_for_target: Option<u32>,
    /// Difficulty as the source grades it; absent when ungraded.
    pub difficulty: Option<String>,
    /// Which dataset it came from, lowercased.
    pub source: String,
    /// Latitude of the route start or centre.
    pub latitude: f64,
    /// Longitude of the route start or centre.
    pub longitude: f64,
    /// How far the route is from the search centre, in metres, rounded.
    pub distance_from_center_meters: f64,
    /// The handle this route's mapped line is kept under for 24 hours: `rt_`
    /// and 16 hex digits. Present when the line is mapped and stored, which
    /// is what lets the route be drawn: name it as `route_ref` in a
    /// `dravr-viz` route block whose `source_tool` is `discover_routes`, or
    /// pass it to `compose_route` to join this stretch with others into one
    /// itinerary. Absent for a route with no mapped line (`osm_tag` or no
    /// length), which no map can draw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_ref: Option<String>,
    /// The `route_ref`s of the other routes in this answer whose mapped line
    /// physically meets this one's: an end of either within 30 m of the
    /// other. A `compose_route` itinerary joins only stretches that meet, so
    /// chain stretches along these links, in running order. Absent when the
    /// route meets none of the others.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connects_to: Vec<String>,
}

/// Upper bound on the Overpass `around:` radius to keep response sizes and
/// Overpass query runtime within reasonable limits. Overpass itself will
/// accept larger radii but response times explode past this value.
const MAX_RADIUS_METERS: u32 = 50_000;

/// Minimum meaningful search radius — smaller than 500 m rarely contains
/// more than a handful of ways, and the LLM gets a better grounding signal
/// with at least a small town-sized area.
const MIN_RADIUS_METERS: u32 = 500;

/// Default radius when the caller omits the parameter.
const DEFAULT_RADIUS_METERS: u32 = 10_000;

/// Shortest session a target distance may name: below this a "route" is a
/// lap of a block, and every mapped way already covers it.
const MIN_TARGET_DISTANCE_METERS: f64 = 500.0;

/// Longest session a target distance may name — past the longest single-day
/// rides (a 400 km brevet), a figure is a typo or a unit mix-up rather than a
/// session.
const MAX_TARGET_DISTANCE_METERS: f64 = 500_000.0;

fn discover_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        // Hits Overpass API (external OSM infrastructure)
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Tool for discovering real routes and trails near a location from OSM data.
///
/// The LLM calls this instead of fabricating terrain. When the user asks
/// "propose a 10km run in Prévost, QC", the agent looks up the approximate
/// coordinates of Prévost and invokes this tool to get a list of named
/// real trails within the search radius.
pub struct DiscoverRoutesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DiscoverRoutesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "place".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Place name to resolve via Nominatim forward geocoding. Prefer this \
                     over latitude/longitude — pass natural strings like 'Prévost, QC', \
                     'Saint-Alexis-des-Monts', or 'Mont-Tremblat'. When set, the \
                     latitude/longitude parameters are ignored."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "latitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Latitude of the search center, in decimal degrees. Only used when \
                     `place` is NOT set. Example: 45.87 for Prévost, Québec."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "longitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Longitude of the search center, in decimal degrees. Only used when \
                     `place` is NOT set. Example: -74.08 for Prévost, Québec."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "sport_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "What kind of route to look for. One of: 'run', 'trail_running', \
                     'ride', 'mountain_bike', 'gravel_ride', 'ebike_ride', 'hike', 'walk', \
                     'cross_country_skiing', 'alpine_skiing', 'backcountry_skiing', \
                     'snowshoe'. Defaults to 'run'; a name that is no sport is refused."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "radius_meters".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Search radius around the resolved center, in meters. Clamped \
                     between 500 and 50000. Defaults to 10000 (10 km)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "target_distance_meters".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Distance of the session the route is for, in meters (15000 for a \
                     15 km run), between 500 and 500000. When set, routes whose length \
                     lands within a tenth of it — in one pass or one out-and-back — lead \
                     the list, and each route says whether it covers the distance in a \
                     single pass, an out-and-back, or repeats. Omit it to get the nearest \
                     routes regardless of length."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties.into_iter().collect()),
            // `place` XOR (`latitude`+`longitude`) is enforced at runtime in
            // execute() — JsonSchema has no "one-of" construct that maps cleanly
            // through our existing PropertySchema abstraction, so the LLM sees
            // the params as all-optional and the error message when neither is
            // set explains the requirement.
            required: None,
            ..Default::default()
        };
        answers_with::<DiscoverRoutesResult>(tool_definition(
            "discover_routes",
            "Discover real named running, cycling, hiking, or ski routes near a location, \
             grounded in OpenStreetMap data via the Overpass API. Pass either a place name \
             via `place` (preferred — e.g. \"Prévost, QC\" or \"Saint-Alexis-des-Monts\") or \
             explicit `latitude`/`longitude` coordinates. Place names are forward-geocoded \
             via Nominatim before the Overpass query so the LLM never has to guess \
             coordinates. Use this whenever the user asks you to propose, suggest, or find \
             a route, trail, or outdoor session in a specific area. Never invent street \
             names, trail names, or terrain you have not verified via this tool. Returns up \
             to 20 named routes, nearest first, each with coordinates and a measured \
             `distance_from_center_meters` — quote that distance rather than estimating one \
             from the coordinates. Signed itineraries and trails are listed ahead of paved \
             connectors. `distance_meters` says what it measures in `distance_source`: \
             `mapped_geometry` is the longest connected run of fetched ways carrying the \
             name — a lower bound, since the trail can continue past it; `osm_tag` is the \
             length declared for the whole route, of which only part may lie inside the \
             radius. A signed route with no declared length has none. When the session \
             has a distance, pass it as `target_distance_meters`: routes within a tenth \
             of it, in one pass or one out-and-back, are listed first, and each carries \
             `target_fit` (`single_pass`, `out_and_back` or `repeats`) and \
             `passes_for_target` — tell \
             the athlete plainly when a route needs an out-and-back or repeats. Elevation, \
             surface and how flat or uninterrupted a route is are NOT returned: never \
             describe a route as flat, hilly or suited to intervals on the strength of \
             this tool. An empty list means OSM has no named route in that radius: widen \
             `radius_meters` or say so plainly, never substitute a name you did not read \
             here. For ski queries this reads OSM piste:type data (same source as \
             OpenSkiMap). A route whose line is mapped carries a `route_ref`: to show the \
             athlete that stretch, name it in a `dravr-viz` route block with `source_tool` \
             \"discover_routes\" — never write coordinates. It also carries `connects_to`, the \
             `route_ref`s of the other routes here that it physically meets. To propose an \
             itinerary over several stretches, pass their refs in running order to \
             `compose_route`, each meeting the next, and quote the distance it returns.",
            schema,
            Some(discover_annotations()),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let (tenant_id, user_id) = route_owner(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let resolved = match resolve_center(place_geocoder()?, &args).await {
                Ok(center) => center,
                Err(err) => return Ok(ToolResult::error(err)),
            };

            // An omitted sport searches running routes; a name that resolves to
            // no sport is refused rather than searched as running.
            let sport = match args.get("sport_type").and_then(Value::as_str) {
                None => SportType::Run,
                Some(raw) => parse_sport_type(raw).ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "sport_type '{raw}' is not a sport name; use one of run, trail_running, \
                         ride, mountain_bike, gravel_ride, ebike_ride, hike, walk, \
                         cross_country_skiing, alpine_skiing, backcountry_skiing, snowshoe"
                    ))
                })?,
            };

            // A target outside the domain is refused, not clamped: a clamped
            // target would rank routes against a distance nobody asked for.
            let target_distance_meters = parse_target_distance(&args)?;

            // Clamp radius into the allowed window without surfacing the clamp to
            // the caller — the LLM should not have to know the magic numbers, and
            // clamp-silently keeps the tool robust against sloppy inputs.
            let radius = args
                .get("radius_meters")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .map_or(DEFAULT_RADIUS_METERS, |v| {
                    v.clamp(MIN_RADIUS_METERS, MAX_RADIUS_METERS)
                });

            info!(
                latitude = resolved.latitude,
                longitude = resolved.longitude,
                sport = %sport_label(&sport),
                radius,
                display_name = %resolved.display_name.as_deref().unwrap_or("(raw coordinates)"),
                "discover_routes: querying Overpass for real OSM routes"
            );

            let service = RouteDiscoveryService::with_defaults();
            let routes = service
                .discover_routes_for_sport(
                    &sport,
                    resolved.latitude,
                    resolved.longitude,
                    Some(radius),
                    target_distance_meters,
                )
                .await?;

            let drawable = store_drawable_routes(state.cache(), tenant_id, user_id, &routes).await;
            let count = routes.len();
            let response = DiscoverRoutesResult {
                sport_type: sport_label(&sport).to_owned(),
                center: RouteSearchCenter {
                    latitude: resolved.latitude,
                    longitude: resolved.longitude,
                    display_name: resolved.display_name,
                },
                radius_meters: radius,
                target_distance_meters,
                count,
                routes: routes
                    .iter()
                    .zip(drawable)
                    .map(|(route, drawable)| discovered_route_to_json(route, drawable))
                    .collect(),
            };

            info!(count, "discover_routes: returning OSM-grounded routes");
            ok_typed("discover_routes", response)
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Resolved query center for a `discover_routes` call.
///
/// Carries the gazetteer's name for the place when the caller passed a
/// `place` string so we can echo the canonical resolution back to the LLM —
/// the agent then has something it can safely quote to the user (e.g.
/// "I found running trails near Prévost, Quebec, Canada").
struct ResolvedCenter {
    latitude: f64,
    longitude: f64,
    display_name: Option<String>,
}

/// The name a resolved place is quoted by: its own name, region and country.
fn display_name(place: &Place) -> String {
    [
        Some(place.name.as_str()),
        place.admin1.as_deref(),
        place.country.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}

/// Resolve the search center from the tool arguments.
///
/// Resolution priority:
/// 1. If `place` is a non-empty string, resolve it through `geocoder` — the
///    platform's one place geocoder, the one `get_weather_forecast` resolves
///    through, so one place name lands on one place in both tools
///    (carnet#920). The text before the first comma is the name; what follows
///    chooses among same-named places. Forward geocoding is the preferred
///    path — the LLM should not have to guess coordinates.
/// 2. Otherwise, require both `latitude` and `longitude` numeric params
///    within valid WGS84 ranges.
///
/// Returns an LLM-facing JSON error payload when neither path works, so
/// the agent gets a structured message it can act on instead of a panic.
async fn resolve_center(geocoder: &dyn Geocoder, args: &Value) -> Result<ResolvedCenter, Value> {
    if let Some((place, query)) = args
        .get("place")
        .and_then(Value::as_str)
        .map(str::trim)
        .and_then(|p| free_text_query(p, None).map(|query| (p, query)))
    {
        return match geocoder.resolve(&query).await {
            Ok(resolution) => Ok(ResolvedCenter {
                latitude: resolution.place.latitude,
                longitude: resolution.place.longitude,
                display_name: Some(display_name(&resolution.place)),
            }),
            Err(GeocodeError::NotFound { .. }) => {
                warn!(
                    place,
                    "discover_routes: the gazetteer has no place by that name"
                );
                Err(json!({
                    "error": format!("no place matched '{place}'. Try the place's own name, optionally followed by its region and country (\"Prévost, Quebec, Canada\"), or pass latitude/longitude directly."),
                }))
            }
            Err(e) => {
                warn!(error = %e, place, "forward geocoding failed");
                Err(json!({
                    "error": format!("forward geocoding failed for '{place}': {e}. You can retry with explicit latitude/longitude parameters."),
                }))
            }
        };
    }

    let latitude = match args.get("latitude").and_then(Value::as_f64) {
        Some(lat) if (-90.0..=90.0).contains(&lat) => lat,
        _ => {
            return Err(json!({
                "error": "Provide either a 'place' string (preferred) or both 'latitude' (between -90 and 90) and 'longitude'.",
            }));
        }
    };
    let longitude = match args.get("longitude").and_then(Value::as_f64) {
        Some(lon) if (-180.0..=180.0).contains(&lon) => lon,
        _ => {
            return Err(json!({
                "error": "Provide either a 'place' string (preferred) or both 'latitude' and 'longitude' (between -180 and 180).",
            }));
        }
    };
    Ok(ResolvedCenter {
        latitude,
        longitude,
        display_name: None,
    })
}

/// Whose route store a call writes and reads: the caller's tenant and
/// athlete, keyed the way every cache-backed tool keys them. A call that
/// carries no tenant files under the nil tenant, which is still the caller's
/// own slot, since the athlete is part of the key.
pub(super) fn route_owner(state: &Arc<dyn ToolRuntime>, ctx: &ToolContext) -> (TenantId, Uuid) {
    let context = ToolExecutionContext::from_tronc(state, ctx);
    (
        TenantId::from_uuid(context.tenant_id.unwrap_or_else(Uuid::nil)),
        context.user_id,
    )
}

/// A stored route's reference and the line it names.
type MappedLine<'a> = (&'a str, &'a [(f64, f64)]);

/// How one route of an answer can be drawn and joined.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DrawableRoute {
    /// The reference its mapped line is stored under; `None` when it has no
    /// line, or the line could not be stored.
    route_ref: Option<String>,
    /// The references of the other routes in the answer that it meets.
    connects_to: Vec<String>,
}

/// Store the mapped line of every route that has one, and say which of them
/// meet, one entry per route in the order given.
///
/// A line the store refuses is logged and left without a reference rather
/// than failing the search: the athlete still gets the routes, only that one
/// cannot be drawn or composed. A route without a reference is in no
/// `connects_to`, since there would be nothing to compose it by.
async fn store_drawable_routes(
    cache: &Cache,
    tenant_id: TenantId,
    user_id: Uuid,
    routes: &[DiscoveredRoute],
) -> Vec<DrawableRoute> {
    let refs = join_all(
        routes
            .iter()
            .map(|route| store_line(cache, tenant_id, user_id, route)),
    )
    .await;
    let lines: Vec<Option<MappedLine<'_>>> = routes
        .iter()
        .zip(&refs)
        .map(|(route, route_ref)| Some((route_ref.as_deref()?, route.geometry.as_deref()?)))
        .collect();

    let mut drawable: Vec<DrawableRoute> = refs
        .iter()
        .map(|route_ref| DrawableRoute {
            route_ref: route_ref.clone(),
            connects_to: Vec::new(),
        })
        .collect();
    for (i, first) in lines.iter().enumerate() {
        let Some((first_ref, first_line)) = *first else {
            continue;
        };
        for (j, second) in lines.iter().enumerate().skip(i + 1) {
            let Some((second_ref, second_line)) = *second else {
                continue;
            };
            // One verdict per pair, written both ways, so the links are
            // symmetric by construction. Two routes drawn along the same line
            // share a reference, and a route does not connect to itself.
            if first_ref == second_ref || !route_compose::meets(first_line, second_line) {
                continue;
            }
            link(&mut drawable[i].connects_to, second_ref);
            link(&mut drawable[j].connects_to, first_ref);
        }
    }
    drawable
}

/// Add `route_ref` to a `connects_to` list once.
fn link(connects_to: &mut Vec<String>, route_ref: &str) {
    if !connects_to.iter().any(|known| known == route_ref) {
        connects_to.push(route_ref.to_owned());
    }
}

/// Store one route's mapped line, returning the reference it is read back by;
/// `None` when it has no line, or when the store refuses it.
async fn store_line(
    cache: &Cache,
    tenant_id: TenantId,
    user_id: Uuid,
    route: &DiscoveredRoute,
) -> Option<String> {
    let line = route.geometry.as_ref()?;
    let stored = StoredRoute {
        name: route.name.clone(),
        coordinates: line.clone(),
        out_and_back: false,
    };
    match route_store::store_route(cache, tenant_id, user_id, &stored).await {
        Ok(route_ref) => Some(route_ref),
        Err(error) => {
            warn!(
                route = %route.name,
                error = %error,
                "discover_routes: the route's line could not be stored; it is listed without a route_ref"
            );
            None
        }
    }
}

/// Serialize a [`DiscoveredRoute`] into the LLM-facing JSON shape, with the
/// handles its line is drawn and composed by.
///
/// Kept tight — the LLM doesn't need the enum's `snake_case` wrapper and
/// benefits from a flat structure when it wants to read a specific field.
fn discovered_route_to_json(
    route: &DiscoveredRoute,
    drawable: DrawableRoute,
) -> DiscoveredRouteEntry {
    DiscoveredRouteEntry {
        name: route.name.clone(),
        route_type: format!("{:?}", route.route_type).to_lowercase(),
        distance_meters: route.distance_meters.map(f64::round),
        distance_source: route.distance_source.map(|source| {
            match source {
                DistanceSource::MappedGeometry => "mapped_geometry",
                DistanceSource::OsmTag => "osm_tag",
            }
            .to_owned()
        }),
        target_fit: route.target_use.map(|usage| {
            match usage.fit {
                TargetFit::SinglePass => "single_pass",
                TargetFit::OutAndBack => "out_and_back",
                TargetFit::Repeats => "repeats",
            }
            .to_owned()
        }),
        passes_for_target: route.target_use.map(|usage| usage.passes),
        difficulty: route.difficulty.clone(),
        source: format!("{:?}", route.source).to_lowercase(),
        latitude: route.latitude,
        longitude: route.longitude,
        distance_from_center_meters: route.distance_from_center_meters.round(),
        route_ref: drawable.route_ref,
        connects_to: drawable.connects_to,
    }
}

/// Read the optional `target_distance_meters` argument.
///
/// Absent or `null` means no target. Anything else has to be a number of
/// metres inside the domain; a string, a negative or an absurd figure is
/// refused with the parameter named, so the caller can correct it.
fn parse_target_distance(args: &Value) -> AppResult<Option<f64>> {
    let Some(raw) = args.get("target_distance_meters").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    match raw.as_f64() {
        Some(meters)
            if (MIN_TARGET_DISTANCE_METERS..=MAX_TARGET_DISTANCE_METERS).contains(&meters) =>
        {
            Ok(Some(meters))
        }
        _ => Err(AppError::invalid_input(format!(
            "target_distance_meters must be a number of meters between \
             {MIN_TARGET_DISTANCE_METERS} and {MAX_TARGET_DISTANCE_METERS} (15000 for a 15 km \
             session), got {raw}"
        ))),
    }
}

/// Parse the LLM-supplied sport string into the typed [`SportType`].
///
/// Accepts both `snake_case` (matching the [`SportType`] serde representation)
/// and a handful of common aliases so the LLM's tool-call args don't have to
/// match the enum exactly. Unknown values fall back to [`SportType::Run`]
/// in the caller — we return `None` here so the caller can apply its default.
/// Thin wrapper around [`pierre_core::models::resolve_sport_type`] —
/// the canonical resolver covers route-relevant variants plus EN/FR
/// aliases, so this tool reuses it instead of duplicating a partial map.
fn parse_sport_type(raw: &str) -> Option<SportType> {
    resolve_sport_type(raw)
}

/// Human-readable label for a [`SportType`], used in log lines and in the
/// tool's JSON response to give the LLM back a canonical value it can reason
/// over without having to know the internal enum shape.
fn sport_label(sport: &SportType) -> &'static str {
    match sport {
        SportType::Run => "run",
        SportType::TrailRunning => "trail_running",
        SportType::Ride => "ride",
        SportType::MountainBike => "mountain_bike",
        SportType::GravelRide => "gravel_ride",
        SportType::EbikeRide => "ebike_ride",
        SportType::Hike => "hike",
        SportType::Walk => "walk",
        SportType::CrossCountrySkiing => "cross_country_skiing",
        SportType::AlpineSkiing => "alpine_skiing",
        SportType::BackcountrySkiing => "backcountry_skiing",
        SportType::Snowshoe => "snowshoe",
        _ => "unsupported",
    }
}

// ============================================================================
// Factory
// ============================================================================

/// Create the route tools for registration: discovery, and the composition
/// that joins discovered stretches into one itinerary.
#[must_use]
pub fn create_route_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(DiscoverRoutesTool), Box::new(ComposeRouteTool)]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(DiscoverRoutesTool => UNTRUSTED_OUTPUT);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use dravr_meteo::{
        CacheLimits, CachedGeocoder, DummyWeatherProvider, Geocoded, PlacePreference, PlaceQuery,
    };
    use pierre_cache::CacheConfig;
    use pierre_fitness_compute::osm_routes::{RouteSource, RouteType};

    use super::super::weather_forecast::forecast;

    /// A gazetteer holding two places named Paris, counting its lookups and
    /// keeping the last query it was asked. Both are shared (`Arc`) so the
    /// test reads them while the geocoder sits inside its cache.
    #[derive(Default, Clone)]
    struct TwoParises {
        lookups: Arc<AtomicUsize>,
        last: Arc<Mutex<Option<PlaceQuery>>>,
    }

    #[async_trait]
    impl Geocoder for TwoParises {
        async fn resolve(&self, query: &PlaceQuery) -> Result<Geocoded, GeocodeError> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().unwrap() = Some(query.clone());
            let france = Place {
                name: "Paris".to_owned(),
                admin1: Some("Île-de-France".to_owned()),
                country: Some("France".to_owned()),
                country_code: Some("FR".to_owned()),
                latitude: 48.853_41,
                longitude: 2.3488,
            };
            let texas = Place {
                name: "Paris".to_owned(),
                admin1: Some("Texas".to_owned()),
                country: Some("United States".to_owned()),
                country_code: Some("US".to_owned()),
                latitude: 33.660_94,
                longitude: -95.555_51,
            };
            let place = if query
                .prefer
                .fit(texas.admin1.as_deref(), texas.country.as_deref())
                > query
                    .prefer
                    .fit(france.admin1.as_deref(), france.country.as_deref())
            {
                texas
            } else {
                france
            };
            Ok(Geocoded { place, matches: 2 })
        }

        fn name(&self) -> &'static str {
            "two-parises"
        }
    }

    // carnet#920: one place string lands on one place in discover_routes and
    // get_weather_forecast, and the second tool reads it from the cache.
    #[tokio::test]
    async fn routes_and_the_forecast_resolve_one_place_through_one_cache() {
        let inner = TwoParises::default();
        let geocoder = CachedGeocoder::new(inner.clone(), CacheLimits::GEOCODE);
        let args = json!({ "place": "Paris, Texas" });

        let center = resolve_center(&geocoder, &args).await.unwrap();
        assert!((center.latitude - 33.660_94).abs() < 1e-9);
        assert!((center.longitude + 95.555_51).abs() < 1e-9);
        assert_eq!(
            center.display_name.as_deref(),
            Some("Paris, Texas, United States")
        );

        let weather_args = json!({
            "place": "Paris, Texas",
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        let weather = forecast(&DummyWeatherProvider::temperate(), &geocoder, &weather_args)
            .await
            .unwrap();
        assert!(!weather.is_error, "{:?}", weather.content);
        assert_eq!(weather.content["latitude"], json!(center.latitude));
        assert_eq!(weather.content["longitude"], json!(center.longitude));

        assert_eq!(
            inner.lookups.load(Ordering::SeqCst),
            1,
            "the second tool's lookup is a cache hit"
        );
        let asked = inner.last.lock().unwrap().clone().unwrap();
        assert_eq!(asked.name, "Paris", "the gazetteer is asked the name alone");
        assert_eq!(
            asked.prefer,
            PlacePreference {
                admin1: Some("Texas".to_owned()),
                country: Some("Texas".to_owned()),
            }
        );
    }

    /// Degrees of latitude in a kilometre, near enough for test lines.
    const DEGREES_PER_KM: f64 = 1.0 / 111.195;

    async fn memory_cache() -> Cache {
        Cache::new(CacheConfig {
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
        .await
        .unwrap()
    }

    /// A route as discovery reports one: measured along `geometry` when it
    /// has one, by its OSM tag otherwise.
    fn route(name: &str, geometry: Option<Vec<(f64, f64)>>) -> DiscoveredRoute {
        let (distance_meters, distance_source) =
            geometry
                .as_ref()
                .map_or((Some(12_000.0), Some(DistanceSource::OsmTag)), |line| {
                    let meters = line
                        .windows(2)
                        .map(|pair| (pair[1].0 - pair[0].0).abs() / DEGREES_PER_KM * 1_000.0)
                        .sum();
                    (Some(meters), Some(DistanceSource::MappedGeometry))
                });
        DiscoveredRoute {
            name: name.to_owned(),
            route_type: RouteType::Running,
            distance_meters,
            distance_source,
            difficulty: None,
            source: RouteSource::Overpass,
            latitude: 46.0,
            longitude: -72.0,
            distance_from_center_meters: 500.0,
            target_use: None,
            geometry,
        }
    }

    /// A line due north along `longitude` from `south`, `km` long, a node
    /// every 100 m.
    fn northward(longitude: f64, south: f64, km: f64) -> Vec<(f64, f64)> {
        let nodes = (km * 10.0).round() as usize;
        (0..=nodes)
            .map(|index| {
                (
                    (index as f64 * 0.1).mul_add(DEGREES_PER_KM, south),
                    longitude,
                )
            })
            .collect()
    }

    /// Two trails end to end, one across town and a relation with no line.
    fn answer() -> Vec<DiscoveredRoute> {
        let riverside = northward(-72.0, 46.0, 1.0);
        let rail_trail = northward(-72.0, 46.0 + DEGREES_PER_KM, 0.5);
        vec![
            route("Sentier de la Rivière", Some(riverside)),
            route("Route Verte", None),
            route("Piste du Parc", Some(northward(-72.5, 46.5, 2.0))),
            route("Ancienne voie ferrée", Some(rail_trail)),
        ]
    }

    /// Every mapped line is stored under the reference it is listed with and
    /// reads back whole; a relation has no line and no reference.
    #[tokio::test]
    async fn a_mapped_route_is_stored_under_the_ref_it_is_listed_with() {
        let cache = memory_cache().await;
        let (tenant_id, user_id) = (TenantId::generate(), Uuid::new_v4());
        let routes = answer();
        let drawable = store_drawable_routes(&cache, tenant_id, user_id, &routes).await;

        assert_eq!(drawable.len(), routes.len());
        for (route, drawable) in routes.iter().zip(&drawable) {
            let Some(line) = &route.geometry else {
                assert_eq!(drawable, &DrawableRoute::default(), "{}", route.name);
                continue;
            };
            let route_ref = drawable
                .route_ref
                .as_deref()
                .expect("a mapped line is stored");
            assert_eq!(route_ref, route_compose::route_ref(line, false));
            let stored = route_store::load_route(&cache, tenant_id, user_id, route_ref)
                .await
                .unwrap()
                .expect("the line reads back by its ref");
            assert_eq!(
                stored,
                StoredRoute {
                    name: route.name.clone(),
                    coordinates: line.clone(),
                    out_and_back: false,
                }
            );
        }
    }

    /// Stretches that meet list each other, both ways; a stretch across town
    /// and a relation with no line are in no list and list nothing.
    #[tokio::test]
    async fn connects_to_links_the_stretches_that_meet_both_ways() {
        let cache = memory_cache().await;
        let routes = answer();
        let drawable =
            store_drawable_routes(&cache, TenantId::generate(), Uuid::new_v4(), &routes).await;
        let ref_of = |index: usize| drawable[index].route_ref.clone().unwrap();

        assert_eq!(drawable[0].connects_to, vec![ref_of(3)]);
        assert_eq!(drawable[3].connects_to, vec![ref_of(0)]);
        assert!(drawable[1].connects_to.is_empty());
        assert!(drawable[2].connects_to.is_empty());
    }

    /// A line the store refuses leaves that route without a reference, and
    /// out of every `connects_to`, while the rest of the answer is stored.
    #[tokio::test]
    async fn a_line_the_store_refuses_does_not_fail_the_answer() {
        let cache = memory_cache().await;
        let mut routes = answer();
        // One point is no line: the store refuses it.
        routes[3].geometry = Some(vec![(46.0 + DEGREES_PER_KM, -72.0)]);
        let drawable =
            store_drawable_routes(&cache, TenantId::generate(), Uuid::new_v4(), &routes).await;

        assert_eq!(drawable[3], DrawableRoute::default());
        assert!(drawable[0].route_ref.is_some());
        assert!(drawable[0].connects_to.is_empty());
    }

    /// The entry says nothing of a reference or links it does not have, and
    /// carries both when it does.
    #[test]
    fn the_entry_carries_its_ref_and_links_only_when_it_has_them() {
        let routes = answer();
        let bare = serde_json::to_value(discovered_route_to_json(
            &routes[1],
            DrawableRoute::default(),
        ))
        .unwrap();
        assert!(bare.get("route_ref").is_none(), "{bare:#}");
        assert!(bare.get("connects_to").is_none(), "{bare:#}");

        let linked = serde_json::to_value(discovered_route_to_json(
            &routes[0],
            DrawableRoute {
                route_ref: Some("rt_3f9a0c41d27b8e65".to_owned()),
                connects_to: vec!["rt_c07e5d2a9b146f38".to_owned()],
            },
        ))
        .unwrap();
        assert_eq!(linked["route_ref"], "rt_3f9a0c41d27b8e65");
        assert_eq!(linked["connects_to"][0], "rt_c07e5d2a9b146f38");
    }
}
