// ABOUTME: MCP tool for discovering real running, cycling, hiking, and ski routes from OSM data
// ABOUTME: Wraps RouteDiscoveryService to expose Overpass-backed route search to the LLM
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::errors::AppResult;
use crate::intelligence::location::{ForwardGeocodeResult, LocationService};
use crate::intelligence::{DiscoveredRoute, RouteDiscoveryService};
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::models::SportType;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};
use pierre_core::errors::ErrorCode;

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
/// "propose a 10km run in Prévost, QC", the coach looks up the approximate
/// coordinates of Prévost and invokes this tool to get a list of named
/// real trails within the search radius.
pub struct DiscoverRoutesTool;

#[async_trait]
impl McpTool for DiscoverRoutesTool {
    fn name(&self) -> &'static str {
        "discover_routes"
    }

    fn description(&self) -> &'static str {
        "Discover real named running, cycling, hiking, or ski routes near a location, \
         grounded in OpenStreetMap data via the Overpass API. Pass either a place name \
         via `place` (preferred — e.g. \"Prévost, QC\" or \"Saint-Alexis-des-Monts\") or \
         explicit `latitude`/`longitude` coordinates. Place names are forward-geocoded \
         via Nominatim before the Overpass query so the LLM never has to guess \
         coordinates. Use this whenever the user asks you to propose, suggest, or find \
         a route, trail, or outdoor session in a specific area. Never invent street \
         names, trail names, or terrain you have not verified via this tool. Returns up \
         to 20 named routes with coordinates so you can share exact locations with the \
         user. For ski queries this falls back to OSM piste:type data (same source as \
         OpenSkiMap)."
    }

    fn input_schema(&self) -> JsonSchema {
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
                     'snowshoe'. Defaults to 'run'."
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

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            // `place` XOR (`latitude`+`longitude`) is enforced at runtime in
            // execute() — JsonSchema has no "one-of" construct that maps cleanly
            // through our existing PropertySchema abstraction, so the LLM sees
            // the params as all-optional and the error message when neither is
            // set explains the requirement.
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(discover_annotations())
    }

    async fn execute(&self, args: Value, _context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let resolved = match resolve_center(&args).await {
            Ok(center) => center,
            Err(err) => return Ok(ToolResult::error(err)),
        };

        let sport = args
            .get("sport_type")
            .and_then(Value::as_str)
            .and_then(parse_sport_type)
            .unwrap_or(SportType::Run);

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

        let mut service = RouteDiscoveryService::with_defaults();
        let routes = service
            .discover_routes_for_sport(&sport, resolved.latitude, resolved.longitude, Some(radius))
            .await?;

        let count = routes.len();
        let mut center_json = json!({
            "latitude": resolved.latitude,
            "longitude": resolved.longitude,
        });
        if let Some(name) = resolved.display_name {
            if let Some(obj) = center_json.as_object_mut() {
                obj.insert("display_name".to_owned(), Value::String(name));
            }
        }
        let response = json!({
            "sport_type": sport_label(&sport),
            "center": center_json,
            "radius_meters": radius,
            "count": count,
            "routes": routes.iter().map(discovered_route_to_json).collect::<Vec<_>>(),
        });

        info!(count, "discover_routes: returning OSM-grounded routes");
        Ok(ToolResult::ok(response))
    }
}

/// Resolved query center for a `discover_routes` call.
///
/// Carries the Nominatim display name when the caller passed a `place`
/// string so we can echo the canonical resolution back to the LLM — the
/// coach then has something it can safely quote to the user (e.g.
/// "I found running trails near Prévost, QC").
struct ResolvedCenter {
    latitude: f64,
    longitude: f64,
    display_name: Option<String>,
}

/// Resolve the search center from the tool arguments.
///
/// Resolution priority:
/// 1. If `place` is a non-empty string, forward-geocode it via Nominatim
///    and return the first match. Forward geocoding is the preferred path
///    — the LLM should not have to guess coordinates.
/// 2. Otherwise, require both `latitude` and `longitude` numeric params
///    within valid WGS84 ranges.
///
/// Returns an LLM-facing JSON error payload when neither path works, so
/// the coach gets a structured message it can act on instead of a panic.
async fn resolve_center(args: &Value) -> Result<ResolvedCenter, Value> {
    if let Some(place) = args
        .get("place")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let mut location_service = LocationService::new();
        return match location_service.forward_geocode(place).await {
            Ok(ForwardGeocodeResult {
                latitude,
                longitude,
                display_name,
            }) => Ok(ResolvedCenter {
                latitude,
                longitude,
                display_name: Some(display_name),
            }),
            Err(e) if e.code == ErrorCode::ResourceNotFound => {
                warn!(
                    place,
                    "discover_routes: Nominatim returned no matches for place"
                );
                Err(json!({
                    "error": format!("no place matched '{place}'. Try a more specific name (city, region, country) or pass latitude/longitude directly."),
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

/// Serialize a [`DiscoveredRoute`] into the LLM-facing JSON shape.
///
/// Kept tight — the LLM doesn't need the enum's `snake_case` wrapper and
/// benefits from a flat structure when it wants to read a specific field.
fn discovered_route_to_json(route: &DiscoveredRoute) -> Value {
    json!({
        "name": route.name,
        "route_type": format!("{:?}", route.route_type).to_lowercase(),
        "distance_meters": route.distance_meters,
        "difficulty": route.difficulty,
        "source": format!("{:?}", route.source).to_lowercase(),
        "latitude": route.latitude,
        "longitude": route.longitude,
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
    pierre_core::models::resolve_sport_type(raw)
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

/// Create route-discovery tools for registration.
#[must_use]
pub fn create_route_tools() -> Vec<Box<dyn McpTool>> {
    vec![Box::new(DiscoverRoutesTool)]
}
