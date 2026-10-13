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
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
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

fn compose_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        // Reads lines discover_routes already stored; queries nothing outside.
        open_world_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

/// Tool that joins discovered stretches into one itinerary, measured and
/// stored so it can be quoted and drawn.
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
                    "The route_ref of each stretch, in running order, from 1 to {MAX_STRETCHES}: \
                     each one a route_ref that discover_routes (or an earlier compose_route) \
                     returned in this conversation. Each stretch must physically meet the next — \
                     a route's connects_to lists the ones it meets."
                )),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some(
                        "A route_ref, such as \"rt_3f9a0c41d27b8e65\", copied as it was returned."
                            .to_owned(),
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
                    "True to come back the same way to the start (an aller-retour): the line is \
                     drawn out and back, and the distance returned covers both ways. Defaults to \
                     false."
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
                "Join stretches found by discover_routes into one itinerary, drawn as a single line \
             and measured along it. Call it when the session you propose runs along more than \
             one discovered route — the riverside path out, the rail trail on — or comes back \
             the way it went. Pass `route_refs` in running order and `out_and_back: true` for an \
             aller-retour. Stretches are joined only where they meet on the ground, within \
             {JOIN_TOLERANCE_METERS:.0} m: each must meet the next, which is what a route's \
             `connects_to` lists. A wider gap is \
             refused rather than bridged, because a straight line across it may cross water, a \
             fence or a road; the refusal names the two stretches that do not meet, so drop one, \
             reorder them, or put a stretch that connects them in between — never propose a \
             refused itinerary anyway. Returns one `route_ref` for the whole line and \
             `distance_meters`: that is the distance to quote, never a sum you work out from \
             the stretches. To show the itinerary, name that `route_ref` in a `dravr-viz` route \
             block with `source_tool` \"compose_route\" — never write coordinates. A route_ref \
             is kept for 24 hours: an unknown or expired one means calling discover_routes \
             again. Elevation and surface are not known here either: never describe the \
             itinerary as flat or hilly on the strength of this tool."
            ),
            schema,
            Some(compose_annotations()),
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
                    length_meters: composed.length_meters,
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
