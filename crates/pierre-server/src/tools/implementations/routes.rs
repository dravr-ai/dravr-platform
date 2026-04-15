// ABOUTME: MCP tool for discovering real running, cycling, hiking, and ski routes from OSM data
// ABOUTME: Wraps RouteDiscoveryService to expose Overpass-backed route search to the LLM
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use crate::errors::AppResult;
use crate::intelligence::{DiscoveredRoute, RouteDiscoveryService};
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::models::SportType;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

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
         grounded in OpenStreetMap data via the Overpass API. Use this whenever the user \
         asks you to propose, suggest, or find a route, trail, or outdoor session in a \
         specific area. Never invent street names, trail names, or terrain you have not \
         verified via this tool. Returns up to 20 named routes with coordinates so you \
         can share exact locations with the user. For ski queries this falls back to \
         OSM piste:type data (same source as OpenSkiMap)."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();

        properties.insert(
            "latitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Latitude of the search center, in decimal degrees. Example: 45.87 for \
                     Prévost, Québec. Required."
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
                    "Longitude of the search center, in decimal degrees. Example: -74.08 \
                     for Prévost, Québec. Required."
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
                    "Search radius around the lat/lon center, in meters. Clamped between \
                     500 and 50000. Defaults to 10000 (10 km)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["latitude".to_owned(), "longitude".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(discover_annotations())
    }

    async fn execute(&self, args: Value, _context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let latitude = match args.get("latitude").and_then(Value::as_f64) {
            Some(lat) if (-90.0..=90.0).contains(&lat) => lat,
            _ => {
                return Ok(ToolResult::error(json!({
                    "error": "latitude is required and must be between -90 and 90 decimal degrees"
                })));
            }
        };

        let longitude = match args.get("longitude").and_then(Value::as_f64) {
            Some(lon) if (-180.0..=180.0).contains(&lon) => lon,
            _ => {
                return Ok(ToolResult::error(json!({
                    "error": "longitude is required and must be between -180 and 180 decimal degrees"
                })));
            }
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
            latitude,
            longitude,
            sport = %sport_label(&sport),
            radius,
            "discover_routes: querying Overpass for real OSM routes"
        );

        let mut service = RouteDiscoveryService::with_defaults();
        let routes = service
            .discover_routes_for_sport(&sport, latitude, longitude, Some(radius))
            .await?;

        let count = routes.len();
        let response = json!({
            "sport_type": sport_label(&sport),
            "center": {
                "latitude": latitude,
                "longitude": longitude,
            },
            "radius_meters": radius,
            "count": count,
            "routes": routes.iter().map(discovered_route_to_json).collect::<Vec<_>>(),
        });

        info!(count, "discover_routes: returning OSM-grounded routes");
        Ok(ToolResult::ok(response))
    }
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
fn parse_sport_type(raw: &str) -> Option<SportType> {
    match raw.trim().to_lowercase().as_str() {
        "run" | "running" => Some(SportType::Run),
        "trail_running" | "trail_run" | "trailrun" => Some(SportType::TrailRunning),
        "ride" | "cycling" | "bike" | "bicycle" => Some(SportType::Ride),
        "mountain_bike" | "mtb" | "mountainbike" => Some(SportType::MountainBike),
        "gravel_ride" | "gravel" => Some(SportType::GravelRide),
        "ebike_ride" | "ebike" => Some(SportType::EbikeRide),
        "hike" | "hiking" => Some(SportType::Hike),
        "walk" | "walking" => Some(SportType::Walk),
        "cross_country_skiing" | "xc_ski" | "nordic_skiing" | "nordic" => {
            Some(SportType::CrossCountrySkiing)
        }
        "alpine_skiing" | "downhill_skiing" | "downhill" => Some(SportType::AlpineSkiing),
        "backcountry_skiing" | "backcountry" | "skitour" | "ski_tour" => {
            Some(SportType::BackcountrySkiing)
        }
        "snowshoe" | "snowshoeing" => Some(SportType::Snowshoe),
        _ => None,
    }
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
