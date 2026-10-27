// ABOUTME: MCP tool that joins discovered trail stretches into one drawable itinerary, by their route_refs
// ABOUTME: Loads each stretch's stored line, composes them where they meet, and stores the result under its own route_ref
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `compose_route`
//!
//! `discover_routes` answers with stretches, one named trail each, and stores
//! the mapped line of each under a `route_ref`. A session the coach proposes
//! often runs along several of them, and this tool joins those stretches,
//! in the order given, into one line through
//! [`route_compose::compose`] — the platform measures the itinerary and
//! stores its line, so the model quotes a distance it did not add up and
//! draws a map from a reference it did not have to copy coordinates into.

use crate::annotations::{annotations, Effect, Reach};
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::future::join_all;
use serde::Serialize;
use serde_json::Value;
use tracing::info;

use pierre_fitness_compute::route_compose::{
    self, ComposeError, Stretch, JOIN_TOLERANCE_METERS, MAX_STRETCHES,
};

use super::routes::route_owner;
use crate::conversions::{answers_with, ok_typed, tool_definition, tool_result_to_response};
use crate::route_store::{self, StoredRoute};
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

/// What separates the stretch names in a composed route's own name.
const STRETCH_NAME_SEPARATOR: &str = " → ";

/// What `compose_route` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ComposeRouteResult {
    /// The reference the composed line is kept under for 24 hours. To show
    /// the itinerary, name it as `route_ref` in a `dravr-viz` route block
    /// whose `source_tool` is `compose_route`; it can also be passed back to
    /// `compose_route` as one stretch of a longer itinerary.
    pub route_ref: String,
    /// Length of the composed line in metres, rounded, measured along the
    /// line itself and doubled for an out-and-back: the distance to quote,
    /// never a sum of the stretches' own distances.
    pub distance_meters: f64,
    /// Whether the route returns along itself to where it started.
    pub out_and_back: bool,
    /// The stretches joined, in running order, as they were passed.
    pub stretches: Vec<ComposedStretch>,
    /// Metres of unmapped ground crossed at each join, rounded: one per pair
    /// of consecutive stretches, each at most 30.
    pub joins_meters: Vec<f64>,
}

/// One stretch of a composed route.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ComposedStretch {
    /// The stretch's own reference, as it was passed.
    pub route_ref: String,
    /// The stretch's name: the trail's name in OSM, or a composed route's
    /// stretch names joined in running order.
    pub name: String,
}

/// Tool that joins discovered stretches into one itinerary, measured and
/// stored so it can be quoted and drawn.
#[derive(Debug)]
pub struct ComposeRouteTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ComposeRouteTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();

        properties.insert(
            "route_refs".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some(format!(
                    "Each stretch's route_ref, in running order, 1 to {MAX_STRETCHES}, as \
                     discover_routes (or an earlier compose_route) returned it in this \
                     conversation."
                )),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some(
                        "A route_ref, e.g. \"rt_3f9a0c41d27b8e65\", copied as returned.".to_owned(),
                    ),
                    ..Default::default()
                })),
                min_items: Some(1),
                max_items: u64::try_from(MAX_STRETCHES).ok(),
                ..Default::default()
            },
        );

        properties.insert(
            "out_and_back".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some(
                    "True to come back the same way to the start (an aller-retour); the \
                     returned distance then covers both ways."
                        .to_owned(),
                ),
                default_value: Some(Value::Bool(false)),
                ..Default::default()
            },
        );

        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties.into_iter().collect()),
            required: Some(vec!["route_refs".to_owned()]),
            ..Default::default()
        };
        answers_with::<ComposeRouteResult>(tool_definition(
            "compose_route",
            &format!(
                "Join stretches found by discover_routes into one itinerary, measured along one \
             line. Call it when the session you propose runs along more than one discovered \
             route, or comes back the way it went. Stretches join only where they meet on the \
             ground, within {JOIN_TOLERANCE_METERS:.0} m: each must meet the next, as a \
             route's `connects_to` lists. A wider gap is refused, never bridged; the refusal \
             names the two that do not meet: drop one, reorder, or insert a stretch joining \
             them — never propose a refused itinerary anyway. Quote the returned \
             `distance_meters`, never a sum of the stretches. To show the itinerary, name the \
             returned `route_ref` in a `dravr-viz` route block with `source_tool` \
             \"compose_route\" — never write coordinates. An unknown or expired route_ref \
             (kept 24 hours) means calling discover_routes again. Elevation and surface are \
             unknown: never describe the itinerary as flat or hilly on the strength of this \
             tool."
            ),
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
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
            let route_refs = parse_route_refs(&args)?;
            let out_and_back = parse_out_and_back(&args)?;
            let cache = state.cache();

            let loaded =
                join_all(route_refs.iter().map(|route_ref| {
                    route_store::load_route(cache, tenant_id, user_id, route_ref)
                }))
                .await;
            let mut stretches: Vec<StoredRoute> = Vec::with_capacity(route_refs.len());
            for (route_ref, stored) in route_refs.iter().zip(loaded) {
                stretches.push(stored?.ok_or_else(|| unknown_route_ref(route_ref))?);
            }

            let composed = route_compose::compose(
                &stretches
                    .iter()
                    .map(|stretch| Stretch {
                        name: &stretch.name,
                        coordinates: &stretch.coordinates,
                    })
                    .collect::<Vec<_>>(),
                out_and_back,
            )
            .map_err(|error| AppError::invalid_input(error.to_string()))?;

            let distance_meters = composed.length_meters.round();
            let joins_meters = composed.joins_meters.iter().map(|m| m.round()).collect();
            let route_ref = route_store::store_route(
                cache,
                tenant_id,
                user_id,
                &StoredRoute {
                    name: stretches
                        .iter()
                        .map(|stretch| stretch.name.as_str())
                        .collect::<Vec<_>>()
                        .join(STRETCH_NAME_SEPARATOR),
                    coordinates: composed.coordinates,
                    out_and_back,
                },
            )
            .await?;

            info!(
                stretches = stretches.len(),
                distance_meters, out_and_back, "compose_route: stored the composed itinerary"
            );
            ok_typed(
                "compose_route",
                ComposeRouteResult {
                    route_ref,
                    distance_meters,
                    out_and_back,
                    stretches: route_refs
                        .into_iter()
                        .zip(stretches)
                        .map(|(route_ref, stretch)| ComposedStretch {
                            route_ref,
                            name: stretch.name,
                        })
                        .collect(),
                    joins_meters,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Read the `route_refs` argument: an array of 1 to [`MAX_STRETCHES`]
/// strings, in running order.
///
/// A count outside that range is refused with the sentence
/// [`route_compose::compose`] itself would give, so the model reads one
/// wording whichever check catches it — and the store is not read for a
/// list that could never compose.
fn parse_route_refs(args: &Value) -> AppResult<Vec<String>> {
    let Some(raw) = args.get("route_refs").filter(|value| !value.is_null()) else {
        return Err(AppError::invalid_input(
            "route_refs is required: the route_ref of each stretch, in running order, as \
             discover_routes returned them",
        ));
    };
    let Some(items) = raw.as_array() else {
        return Err(AppError::invalid_input(format!(
            "route_refs must be an array of route_ref strings in running order, even for one \
             stretch, got {raw}"
        )));
    };
    if items.is_empty() {
        return Err(AppError::invalid_input(
            ComposeError::NoStretches.to_string(),
        ));
    }
    if items.len() > MAX_STRETCHES {
        return Err(AppError::invalid_input(
            ComposeError::TooManyStretches { max: MAX_STRETCHES }.to_string(),
        ));
    }
    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                AppError::invalid_input(format!(
                    "each of route_refs is a route_ref string such as \"rt_3f9a0c41d27b8e65\", \
                     got {item}"
                ))
            })
        })
        .collect()
}

/// Read the optional `out_and_back` flag. Absent or `null` is a one-way
/// route; anything but a boolean is refused rather than read as one.
fn parse_out_and_back(args: &Value) -> AppResult<bool> {
    match args.get("out_and_back") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(other) => Err(AppError::invalid_input(format!(
            "out_and_back must be true or false, got {other}"
        ))),
    }
}

/// The refusal for a reference the athlete's store does not hold, worded so
/// the model searches again instead of retrying the same reference.
fn unknown_route_ref(route_ref: &str) -> AppError {
    AppError::invalid_input(format!(
        "unknown or expired route_ref \"{route_ref}\": a route_ref comes from discover_routes or \
         compose_route in this conversation and is kept for 24 hours; call discover_routes \
         again for fresh ones"
    ))
}

// Guardian security classification (see `crate::security`): the answer carries
// the stretches' names, which volunteers typed into OpenStreetMap.
crate::declare_security!(ComposeRouteTool => UNTRUSTED_OUTPUT);
