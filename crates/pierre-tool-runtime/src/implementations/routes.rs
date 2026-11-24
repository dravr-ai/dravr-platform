// ABOUTME: MCP tool for discovering routes: the athlete's own loops, then real running, cycling, hiking and ski routes from OSM
// ABOUTME: Wraps RouteDiscoveryService and the loops search, storing each line under a route_ref a map or compose_route reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::annotations::{annotations, Effect, Reach};
use crate::read_subject::DataClass;
use crate::tool_data::ToolData;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::future::{self, join_all};
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use pierre_cache::Cache;
use pierre_fitness_compute::geocoding::free_text_query;
use pierre_fitness_compute::osm_routes::{
    DiscoveredRoute, DistanceSource, RouteDiscoveryService, TargetFit,
};
use pierre_fitness_compute::route_compose;
use pierre_services::athlete_loops::{Effort, LoopSearch};

use super::compose_route::ComposeRouteTool;
use super::weather_forecast::default_place;
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
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

mod own_loops;

/// What a model reads after a route search, so a session planned on one of
/// the routes carries its weather and daylight (carnet#1665).
const FORECAST_NEXT_STEP: &str = "For a session planned on one of these routes, call \
     get_weather_forecast for this place and the session's hour, and give its conditions and \
     sunset with the route.";

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
    /// The routes. The athlete's own loops (`source` `your_rides`) come
    /// first, best for the asked `effort`; then OpenStreetMap's, nearest
    /// first within each class, or, when a target distance was given, the
    /// ones whose usable distance lands closest to it first.
    pub routes: Vec<DiscoveredRouteEntry>,
    /// Why no OpenStreetMap route is listed: no public Overpass server
    /// answered within the search's few seconds, and the athlete's own loops
    /// answer alone. Absent when OpenStreetMap answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub osm_unavailable: Option<String>,
    /// What to read before planning a session on one of the routes.
    pub next_step: String,
}

/// Where a route search was centred.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RouteSearchCenter {
    /// Latitude of the centre.
    pub latitude: f64,
    /// Longitude of the centre.
    pub longitude: f64,
    /// The resolved place name, or the name of the athlete's training place
    /// searched when no location was given. Absent when the caller gave
    /// coordinates, or that place has no name to show.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// One route as `discover_routes` reports it: one of the athlete's own loops,
/// or a route from OpenStreetMap.
///
/// An OpenStreetMap route is why so much is optional: a way tagged by a
/// volunteer may carry a name and nothing else.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DiscoveredRouteEntry {
    /// The route's name in OSM; for an own loop, the ride's name.
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
    /// `recorded`: the distance the athlete's own ride of the loop recorded.
    pub distance_source: Option<String>,
    /// How the route covers the target distance: `single_pass` (as long as
    /// the target, or short of it by no more than a tenth), `out_and_back`
    /// (out and back reaches it to within that tenth), `repeats` (shorter
    /// still). Present only when a target was given and an
    /// OpenStreetMap route's length is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_fit: Option<String>,
    /// How many times the route is covered — 1, 2 for an out-and-back, more
    /// for repeats: the fewest passes that reach the target to within a
    /// tenth. Present with `target_fit`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passes_for_target: Option<u32>,
    /// Difficulty as the source grades it; absent when ungraded.
    pub difficulty: Option<String>,
    /// Which dataset it came from, lowercased: `your_rides` for one of the
    /// athlete's own loops.
    pub source: String,
    /// Latitude of the route start or centre; an own loop's is where its
    /// privacy-trimmed line starts, never the spot the ride left from.
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
    /// An own loop's ride: the activity it was recorded as. Only an own loop
    /// carries it; a map draws the loop by its `route_ref`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<String>,
    /// The day an own loop was ridden, `YYYY-MM-DD` on the athlete's clock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// The name of the athlete's place an own loop starts from, when it has
    /// one that may be shown here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_name: Option<String>,
    /// An own loop's recorded climb, in metres, rounded; absent when none
    /// was recorded, and for every OpenStreetMap route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_gain_meters: Option<f64>,
    /// How hard an own loop is among the athlete's rides of the sport at
    /// that place: `easy` (the shortest, flattest third), `moderate` or
    /// `hard`. Absent where they have too few rides to say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_effort: Option<Effort>,
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

/// Tool for discovering real routes and trails near a location from OSM data.
///
/// The LLM calls this instead of fabricating terrain. When the user asks
/// "propose a 10km run in Prévost", the agent passes the place
/// ("Prévost, Quebec, Canada") and gets back a list of named real trails
/// within the search radius around wherever the platform's one place
/// geocoder resolves it — the place `get_weather_forecast` resolves too.
#[derive(Debug)]
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
                    "Place to search around; latitude/longitude are ignored when it is set. \
                     Give its own name, optionally followed by region and country to choose \
                     among same-named places: 'Prévost, Quebec, Canada'. Write the region \
                     out: an abbreviation such as 'QC' chooses nothing. Omit it and the \
                     coordinates for the athlete's most frequent training place (Dravr's own \
                     app and chats only)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "latitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Search center latitude, decimal degrees.".to_owned()),
                ..Default::default()
            },
        );

        properties.insert(
            "longitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Search center longitude, decimal degrees.".to_owned()),
                ..Default::default()
            },
        );

        properties.insert(
            "sport_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Route kind, one of: 'run', 'trail_running', 'ride', 'mountain_bike', \
                     'gravel_ride', 'ebike_ride', 'hike', 'walk', 'cross_country_skiing', \
                     'alpine_skiing', 'backcountry_skiing', 'snowshoe'. Defaults to 'run'; \
                     other names are refused."
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
                    "Search radius in meters, clamped to 500-50000; default 10000.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "target_distance_meters".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Session distance in meters (15000 for a 15 km run), 500 to 500000. \
                     Routes covering it within a tenth in one pass or one out-and-back lead \
                     the list, each with its `target_fit`. Omit for the nearest routes \
                     regardless of length."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "effort".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "The session's effort: 'easy', 'moderate' or 'hard'. Orders the athlete's \
                     own loops by where their distance and climb fall among their rides there."
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
            "Find routes around a `place` (preferred) or `latitude`/`longitude`: first the \
             athlete's own loops (`source` `your_rides`, past rides that started and ended at a \
             place they ride from), then named OpenStreetMap routes, nearest first. Call it \
             whenever asked to propose or find a route, loop, trail or outdoor session. Pass the \
             session's `effort`, and `target_distance_meters` when it has a distance; say \
             plainly when `target_fit` is `out_and_back` or `repeats`. Never invent street or \
             trail names or terrain not read here; quote `distance_from_center_meters`, never a \
             distance from coordinates. `distance_source`: `mapped_geometry` is a lower bound, \
             `osm_tag` the whole route's declared length. OpenStreetMap routes carry no \
             elevation or surface: never call one flat or hilly. `osm_unavailable` means \
             OpenStreetMap did not answer; an empty list means nothing in that radius: widen \
             `radius_meters` or say so. To show a route, name its `route_ref` in a `dravr-viz` \
             route block with `source_tool` \"discover_routes\" — never coordinates or an \
             activity id. For an itinerary over several stretches, pass their refs in running \
             order to `compose_route`, each meeting the next (as `connects_to` lists). For a \
             planned session, also call `get_weather_forecast` for the place and hour, and give \
             its conditions and sunset.",
            schema,
            annotations("Discover routes", Effect::ReadOnly, Reach::OpenWorld),
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
        // Read here, inside the call's gate: whether this call may read the
        // athlete's places, and so offer their own loops.
        let loops_tenant =
            own_loops::readable_tenant(&ToolExecutionContext::from_tronc(state, ctx));
        let result: AppResult<ToolResult> = async move {
            let resolved = match search_center(state, &args, loops_tenant, user_id).await? {
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
            let effort = parse_effort(&args)?;

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
                own_loops = loops_tenant.is_some(),
                display_name = %resolved.display_name.as_deref().unwrap_or("(raw coordinates)"),
                "discover_routes: querying Overpass and the athlete's own loops"
            );

            // One task, two reads at once: the Overpass walk, bounded by its
            // own few seconds, and the store's loops. Never spawned, so both
            // run under this call's gate and provenance. The instances are
            // the ones the deployment configuration names.
            let service = RouteDiscoveryService::new(
                state
                    .config()
                    .external_services
                    .overpass_api_base
                    .as_deref(),
            );
            let walk = service.discover_routes_for_sport(
                &sport,
                resolved.latitude,
                resolved.longitude,
                Some(radius),
                target_distance_meters,
            );
            let loops = async {
                let Some(tenant) = loops_tenant else {
                    return Vec::new();
                };
                let search = LoopSearch {
                    sport: sport.clone(),
                    latitude: resolved.latitude,
                    longitude: resolved.longitude,
                    radius_meters: f64::from(radius),
                    effort,
                    target_distance_meters,
                };
                own_loops::search(state, &tenant, user_id, &search).await
            };
            let (walk, loops) = future::join(walk, loops).await;
            let (routes, osm_unavailable) = own_loops::settle_walk(walk, !loops.is_empty())?;

            let mut listed = own_loops::entries(state, tenant_id, user_id, &sport, loops).await;
            let own_count = listed.len();
            let drawable = store_drawable_routes(state.cache(), tenant_id, user_id, &routes).await;
            listed.extend(
                routes
                    .iter()
                    .zip(drawable)
                    .map(|(route, drawable)| discovered_route_to_json(route, drawable)),
            );
            let count = listed.len();
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
                routes: listed,
                osm_unavailable,
                next_step: FORECAST_NEXT_STEP.to_owned(),
            };

            info!(
                count,
                own_loops = own_count,
                "discover_routes: returning the athlete's loops and OSM-grounded routes"
            );
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

/// Where the search is centred: the place or coordinates the caller named,
/// or, when they named neither and this call may read the athlete's places,
/// the athlete's most frequent training place, rounded to 0.01° as the
/// forecast reads it.
///
/// # Errors
///
/// Returns the repository error when the athlete's places cannot be read,
/// and the configuration error when no geocoder is configured; a location
/// that resolves to nothing is the LLM-facing `Err` payload.
async fn search_center(
    state: &Arc<dyn ToolRuntime>,
    args: &Value,
    loops_tenant: Option<TenantId>,
    user_id: Uuid,
) -> AppResult<Result<ResolvedCenter, Value>> {
    let located = ["place", "latitude", "longitude"]
        .iter()
        .any(|name| args.get(*name).is_some_and(|value| !value.is_null()));
    if let Some(tenant) = loops_tenant.filter(|_| !located) {
        if let Some(place) = default_place(state.repos(), tenant, user_id).await? {
            return Ok(Ok(ResolvedCenter {
                latitude: place.latitude,
                longitude: place.longitude,
                display_name: place.place.name,
            }));
        }
    }
    Ok(resolve_center(state.weather().geocoder()?, args).await)
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
        activity_id: None,
        date: None,
        place_name: None,
        elevation_gain_meters: None,
        relative_effort: None,
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

/// Read the optional `effort` argument: absent or `null` is none; a word
/// that is no effort is refused with the words that are.
fn parse_effort(args: &Value) -> AppResult<Option<Effort>> {
    let Some(raw) = args.get("effort").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    raw.as_str()
        .and_then(Effort::parse)
        .map(Some)
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "effort must be one of easy, moderate, hard, got {raw}"
            ))
        })
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
// A route lookup: OpenStreetMap data, cached for the caller's own next
// call.
crate::declare_security!(
    DiscoverRoutesTool => empty,
    reads: ToolData::of(DataClass::NoSubject, &[]),
);

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_tronc::dates::{Timestamp, Zone};
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
            "timestamp": Timestamp::now().to_rfc3339(),
        });
        let weather = forecast(
            &DummyWeatherProvider::temperate(),
            &geocoder,
            &weather_args,
            &Zone::utc(),
        )
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
        assert!(
            drawable[1].connects_to.is_empty(),
            "expected empty, got {:?}",
            drawable[1].connects_to
        );
        assert!(
            drawable[2].connects_to.is_empty(),
            "expected empty, got {:?}",
            drawable[2].connects_to
        );
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
        assert!(
            drawable[0].connects_to.is_empty(),
            "expected empty, got {:?}",
            drawable[0].connects_to
        );
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
