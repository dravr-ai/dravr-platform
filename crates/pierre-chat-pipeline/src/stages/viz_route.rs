// ABOUTME: Reads the recorded track an activity's route block names and carries it on the block
// ABOUTME: A track is thousands of points, so the agent names an id and the platform reads the geometry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Route geometry for inline visual blocks.
//!
//! A `chart` or `table` block carries its own numbers, because a coach can
//! write a dozen points into a reply. A `route` cannot: a recorded track runs
//! to thousands of coordinates, so the block names an activity and the
//! platform reads that activity's geometry here. Coordinates a model produced
//! itself would be invented ones, which is why the schema gives it nowhere to
//! put them.
//!
//! Every series carried alongside the coordinates is either absent or exactly
//! as long as them. A padded elevation array would put a climb marker on the
//! wrong kilometre, which is worse than drawing no marker at all.

use std::collections::BTreeMap;

use pierre_core::models::TimeSeriesData;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_fitness_compute::routes::{
    build_route_summary_from_streams, haversine_meters_between, ClimbCategory,
};
use pierre_fitness_compute::{trim_route_endpoints, DEFAULT_PRIVACY_RADIUS_METERS};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use serde::Serialize;
use serde_json::{json, Value};
use tracing::warn;

use std::sync::Arc;

use super::structured_output::SchemaTexts;
use super::viz_blocks::{next_fence, validated_block, FENCE_INFO};
use crate::{ChatPipelineContext, TurnInput};

/// The block kind whose geometry the platform reads on the coach's behalf.
const ROUTE_KIND: &str = "route";

/// The `highlight` value that asks for the climbs to be marked.
const HIGHLIGHT_CLIMBS: &str = "climbs";

/// The tool that returns one activity's recorded per-second streams.
const ROUTE_STREAM_TOOL: &str = "extract_activity_streams";

/// How many activities one reply may have tracks read for.
///
/// Each read is a provider round trip against the athlete's own rate-limited
/// account, and the number of them is decided by model output. A reply that
/// draws five maps is not a reply anyone reads, so the budget is small and the
/// blocks past it are refused with a reason the repair re-ask can act on. The
/// budget covers the whole turn: the repair pass shares this map, so a second
/// extraction cannot buy a second round of reads.
const MAX_ROUTE_TRACKS_PER_REPLY: usize = 4;

/// What reading one named activity's track produced.
///
/// The failure half is a sentence rather than a typed error because its only
/// two readers are the WARN line and the repair prompt, and both want the same
/// thing: which activity, and why there is no map.
pub type RouteTrackResult = Result<RouteTrack, String>;

/// Per-activity read outcomes for the route blocks in one reply, keyed by the
/// activity id the block cited.
pub type RouteTracks = BTreeMap<String, RouteTrackResult>;

/// The corners of a drawn track, so a client frames the map without walking
/// every point first.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RouteBounds {
    /// Southernmost latitude on the track, in degrees.
    pub min_latitude: f64,
    /// Northernmost latitude on the track, in degrees.
    pub max_latitude: f64,
    /// Westernmost longitude on the track, in degrees.
    pub min_longitude: f64,
    /// Easternmost longitude on the track, in degrees.
    pub max_longitude: f64,
}

/// One sustained ascent along a track.
///
/// The indices address [`RouteTrack::coordinates`], so a client marks the
/// climb by slicing the line it already drew.
#[derive(Debug, Clone, Serialize)]
pub struct RouteClimb {
    /// First coordinate of the climb.
    pub start_index: usize,
    /// Last coordinate of the climb, inclusive.
    pub end_index: usize,
    /// Average gradient in percent — 5.4 means 5.4 %.
    ///
    /// The compute crate reports a fraction; the conversion happens here
    /// because every consumer of this struct renders a percentage, and a
    /// fraction reaching a `%` template prints a 5.4 % climb as 0.1 %.
    pub avg_gradient: f64,
    /// Strava-style grade as the athlete reads it: `HC`, or `1` through `4`.
    ///
    /// `None` for an ascent below the category threshold, so a client omits
    /// the label rather than captioning it "Cat none". The compute crate's
    /// enum is the authority on which grade a climb earns; this is only how
    /// that grade is spelled for display.
    pub category: Option<String>,
}

/// One activity's recorded track, ready to be carried on a route block.
///
/// Every series here is either absent or exactly as long as `coordinates`:
/// they are built from the same filtered pass and sliced by the same privacy
/// trim, so a client can index one by the other without checking. A padded
/// series would put a climb marker on the wrong kilometre, which is worse than
/// no marker.
#[derive(Debug, Clone, Serialize)]
pub struct RouteTrack {
    /// `(latitude, longitude)` in degrees, in recorded order.
    pub coordinates: Vec<(f64, f64)>,
    /// Corners of `coordinates`.
    pub bounds: RouteBounds,
    /// Altitude per coordinate. Absent when the provider recorded none, which
    /// is also when `climbs` is empty — a climb needs the vertical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_meters: Option<Vec<f64>>,
    /// Distance into the *recorded* ride at each coordinate, in metres. The
    /// first entry is where the drawn line picks the ride up, which is the
    /// privacy trim's radius rather than zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distances_meters: Option<Vec<f64>>,
    /// Sustained ascents along the track, in order.
    pub climbs: Vec<RouteClimb>,
}

impl RouteTrack {
    /// Build the drawable track from an activity's recorded streams.
    ///
    /// The line this returns is the ride minus its endpoint neighbourhoods:
    /// where a trace starts and stops is an address, and it is removed before
    /// the geometry exists as a block rather than at the surface that draws it.
    ///
    /// # Errors
    ///
    /// Returns the sentence a refused block carries when the activity has no
    /// GPS channel, when fewer than two of its points survive the validity
    /// gate — a single point is a pin, not a route — or when the whole ride
    /// sits inside the privacy radius, where the only honest map is none.
    pub fn from_streams(streams: &TimeSeriesData) -> RouteTrackResult {
        let Some(recorded) = streams.gps_coordinates.as_deref() else {
            return Err(
                "the activity has no recorded GPS track, so no map can be drawn".to_owned(),
            );
        };
        let (recorded_points, altitudes) = paired_points(recorded, streams.altitude.as_deref());
        // The vertical rides on the pairing above: with an altitude channel
        // present a point is kept only when both halves are usable, so equal
        // lengths hold by construction and are what the index alignment means.
        let has_vertical = altitudes.len() == recorded_points.len();
        let recorded_elevations: Option<Vec<f64>> =
            has_vertical.then(|| altitudes.iter().copied().map(f64::from).collect());
        // Measured on the whole ride, then sliced with it: a surviving sample
        // keeps the distance it sat at in the real ride, so an elevation
        // profile still reads as "kilometres in" rather than restarting at the
        // trim.
        let recorded_distances = cumulative_distances(&recorded_points);
        // The endpoints are the athlete's door. Trimmed here rather than in a
        // client because the geometry travels: the messaging path hands a
        // render URL to third-party servers, which fetch the whole payload.
        let Some(trimmed) = trim_route_endpoints(
            &recorded_points,
            recorded_elevations.as_deref(),
            Some(&recorded_distances),
            DEFAULT_PRIVACY_RADIUS_METERS,
        ) else {
            return Err(
                "the activity's track is too short, or too close to where it started, to draw \
                 without publishing that address"
                    .to_owned(),
            );
        };
        // Two survivors are guaranteed by the trim, so the corners come off the
        // same slice the map is drawn from.
        let bounds = match trimmed.coordinates.split_first() {
            Some((&first, rest)) => bounds_of(first, rest),
            None => return Err("the trimmed track has no points left to draw".to_owned()),
        };
        // Climbs are found on the trimmed line, not the recorded one, because
        // their indices address the coordinates this block carries.
        let climbs = trimmed
            .elevations
            .as_deref()
            .map_or_else(Vec::new, |elevations| {
                detect_climbs(&trimmed.coordinates, elevations)
            });
        Ok(Self {
            coordinates: trimmed.coordinates,
            bounds,
            elevation_meters: trimmed.elevations,
            distances_meters: trimmed.distances,
            climbs,
        })
    }
}

/// Pair the GPS and altitude channels and keep the points both halves record.
///
/// Mirrors the gate the terrain analysis applies to the same two streams — the
/// shorter channel ends the track, and a point with a non-finite or
/// out-of-Earth value is dropped rather than carried into a haversine. The
/// returned altitudes are empty when the provider recorded none.
fn paired_points(
    recorded: &[(f64, f64)],
    altitudes: Option<&[f32]>,
) -> (Vec<(f64, f64)>, Vec<f32>) {
    let mut coordinates = Vec::with_capacity(recorded.len());
    let mut kept_altitudes = Vec::new();
    for (index, &(latitude, longitude)) in recorded.iter().enumerate() {
        let elevation = match altitudes.map(|series| series.get(index)) {
            // The altitude channel ran out first, so the track ends here.
            Some(None) => break,
            Some(Some(&sample)) => Some(sample),
            None => None,
        };
        if !latitude.is_finite() || !longitude.is_finite() {
            continue;
        }
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            continue;
        }
        if let Some(sample) = elevation {
            if !sample.is_finite() {
                continue;
            }
            kept_altitudes.push(sample);
        }
        coordinates.push((latitude, longitude));
    }
    (coordinates, kept_altitudes)
}

/// The corners of a track, taken from its first point and everything after.
fn bounds_of(first: (f64, f64), rest: &[(f64, f64)]) -> RouteBounds {
    let (first_latitude, first_longitude) = first;
    let mut bounds = RouteBounds {
        min_latitude: first_latitude,
        max_latitude: first_latitude,
        min_longitude: first_longitude,
        max_longitude: first_longitude,
    };
    for &(latitude, longitude) in rest {
        bounds.min_latitude = bounds.min_latitude.min(latitude);
        bounds.max_latitude = bounds.max_latitude.max(latitude);
        bounds.min_longitude = bounds.min_longitude.min(longitude);
        bounds.max_longitude = bounds.max_longitude.max(longitude);
    }
    bounds
}

/// Distance from the start at each coordinate, measured along the track.
///
/// Through the compute crate's haversine rather than a local one, so "18 km
/// in" means the same number here as in the terrain analysis the same track
/// feeds.
fn cumulative_distances(coordinates: &[(f64, f64)]) -> Vec<f64> {
    let mut distances = Vec::with_capacity(coordinates.len());
    let mut travelled = 0.0;
    distances.push(travelled);
    for (&(from_latitude, from_longitude), &(to_latitude, to_longitude)) in
        coordinates.iter().zip(coordinates.iter().skip(1))
    {
        travelled +=
            haversine_meters_between(from_latitude, from_longitude, to_latitude, to_longitude);
        distances.push(travelled);
    }
    distances
}

/// The sustained ascents the terrain analysis finds in this track.
///
/// The analysis re-runs its own validity filter, so its indices only address
/// our coordinates when it kept every point we did. It does — both sides apply
/// the same gate — and the count check is what proves it on the day someone
/// changes one of them: a mismatch drops the marks and keeps the map, rather
/// than drawing a climb across the wrong kilometres.
fn detect_climbs(coordinates: &[(f64, f64)], elevations: &[f64]) -> Vec<RouteClimb> {
    // Narrowed back to the width the provider recorded them at: every value
    // here came from an `f32` altitude sample and was widened on the way in,
    // so this is the exact inverse and loses nothing.
    #[allow(clippy::cast_possible_truncation)]
    let altitudes: Vec<f32> = elevations.iter().map(|&metres| metres as f32).collect();
    let Some(summary) = build_route_summary_from_streams(coordinates, &altitudes) else {
        return Vec::new();
    };
    if summary.point_count != coordinates.len() {
        warn!(
            analysed = summary.point_count,
            carried = coordinates.len(),
            "viz-blocks: terrain analysis kept a different point count; drawing the route \
             without climb marks"
        );
        return Vec::new();
    }
    summary
        .climbs
        .into_iter()
        .map(|climb| RouteClimb {
            start_index: climb.start_index,
            end_index: climb.end_index,
            avg_gradient: climb.avg_gradient * 100.0,
            category: match climb.category {
                ClimbCategory::Hc => Some("HC".to_owned()),
                ClimbCategory::Cat1 => Some("1".to_owned()),
                ClimbCategory::Cat2 => Some("2".to_owned()),
                ClimbCategory::Cat3 => Some("3".to_owned()),
                ClimbCategory::Cat4 => Some("4".to_owned()),
                ClimbCategory::None => None,
            },
        })
        .collect()
}

/// Read the recorded track of every activity this reply's route blocks name.
///
/// Fills `tracks` in place and skips ids it already holds, so the repair
/// re-ask can be handed the same map: a repaired reply naming the same
/// activity costs nothing, and one naming a different activity is read once.
///
/// Nothing here fails the turn. An activity that cannot be read leaves its
/// reason in the map, the block that named it is refused during extraction,
/// and the athlete gets the prose — the same outcome as any other refused
/// block.
pub async fn read_route_tracks(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    granted: &[String],
    tools_called: &[String],
    reply: &str,
    tracks: &mut RouteTracks,
) {
    if !granted.iter().any(|kind| kind == ROUTE_KIND) {
        return;
    }
    let wanted: Vec<String> =
        route_activity_ids(&ctx.structured_output_schemas, granted, tools_called, reply)
            .into_iter()
            .filter(|activity_id| !tracks.contains_key(activity_id))
            .collect();
    if wanted.is_empty() {
        return;
    }
    // Same construction as tool dispatch (stage 9) and the capability-recovery
    // fetch: the athlete's own turn, on the turn's Guardian budget.
    let executor = UniversalExecutor::new(Arc::clone(&ctx.tool_runtime))
        .with_scopes(OAuthScope::self_grant())
        .with_conversation_id(input.conversation_id.clone())
        .with_conversation_tenant(input.conversation_tenant_id.as_uuid())
        .with_turn_token(input.turn_id.0.to_string());

    let budget = MAX_ROUTE_TRACKS_PER_REPLY.saturating_sub(tracks.len());
    for (position, activity_id) in wanted.into_iter().enumerate() {
        let outcome = if position < budget {
            read_one_track(&executor, input, &activity_id).await
        } else {
            Err(format!(
                "this reply already draws {MAX_ROUTE_TRACKS_PER_REPLY} maps, which is the most \
                 one reply carries; keep the ones that earn their place"
            ))
        };
        tracks.insert(activity_id, outcome);
    }
}

/// Activity ids named by the route blocks this reply would actually render.
///
/// Runs every gate extraction runs — the schema, the coach's grant, the
/// attribution check — so a block that is going to be refused never costs a
/// provider read. In reply order, deduplicated: two blocks drawing the same
/// activity are one read.
fn route_activity_ids(
    schemas: &SchemaTexts,
    granted: &[String],
    tools_called: &[String],
    reply: &str,
) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    if !reply.contains(FENCE_INFO) {
        return ids;
    }
    let mut rest = reply;
    while let Some(fence) = next_fence(rest) {
        rest = &rest[fence.end..];
        // The kind is read before the gates run, so a chart's refusal is
        // logged once — by the extraction that owns it — rather than here too.
        let Ok(claimed) = serde_json::from_str::<Value>(fence.body.trim()) else {
            continue;
        };
        if claimed.get("type").and_then(Value::as_str) != Some(ROUTE_KIND) {
            continue;
        }
        let Ok(block) = validated_block(schemas, granted, tools_called, fence.body) else {
            continue;
        };
        if let Some(activity_id) = block.get("activity_id").and_then(Value::as_str) {
            if !ids.iter().any(|seen| seen == activity_id) {
                ids.push(activity_id.to_owned());
            }
        }
    }
    ids
}

/// Read one activity's streams and shape them into a drawable track.
///
/// The provider's own words stay in the WARN line: what travels out is a
/// sentence about the athlete's activity, because the caller feeds it to the
/// repair prompt.
async fn read_one_track(
    executor: &UniversalExecutor,
    input: &TurnInput,
    activity_id: &str,
) -> RouteTrackResult {
    let request = UniversalRequest {
        tool_name: ROUTE_STREAM_TOOL.to_owned(),
        parameters: json!({ "activity_id": activity_id }),
        user_id: input.user_id.clone(),
        protocol: "chat".to_owned(),
        tenant_id: Some(input.tool_tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };
    let response = match executor.execute_tool(request).await {
        Ok(response) => response,
        Err(e) => {
            warn!(error = %e, activity_id, "viz-blocks: reading an activity's track failed");
            return Err(unreadable(activity_id));
        }
    };
    let series = match decode_streams(response) {
        Ok(series) => series,
        Err(reason) => {
            warn!(
                error = %reason,
                activity_id, "viz-blocks: the activity's track was not returned"
            );
            return Err(unreadable(activity_id));
        }
    };
    RouteTrack::from_streams(&series)
        .map_err(|reason| format!("activity \"{activity_id}\": {reason}"))
}

/// The sentence a block carries when its activity's track could not be read.
///
/// One wording for every way the read can fail: the athlete is told nothing
/// here, and the model is told the only thing it can act on — this activity
/// has no map, so name another or drop the block.
fn unreadable(activity_id: &str) -> String {
    format!("activity \"{activity_id}\" has no track the platform can read, so no map can be drawn")
}

/// The time series inside a stream-read response, or the reason there is none.
///
/// Says which way it went wrong and logs none of it: one WARN at the caller,
/// naming the activity, reads better in a log than four scattered through the
/// shapes a response can take.
fn decode_streams(response: UniversalResponse) -> Result<TimeSeriesData, String> {
    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| "the stream read reported failure".to_owned()));
    }
    let Some(Value::Object(mut result)) = response.result else {
        return Err("the stream read returned no result".to_owned());
    };
    let Some(streams) = result.remove("streams") else {
        return Err("the stream read carried no streams".to_owned());
    };
    serde_json::from_value(streams).map_err(|e| format!("the streams did not decode: {e}"))
}

/// Carry the read geometry onto a route block, in place.
///
/// A block of any other kind is left exactly as it was. A route block whose
/// activity yielded no track is refused with the reason the read recorded —
/// there is no half-drawn map, and an empty one would read as "you went
/// nowhere".
///
/// `highlight` is consumed here rather than carried: it asks a question the
/// platform answers, and a renderer that had to answer it again could answer
/// it differently. `climbs` empty is the whole of "no marks".
///
/// # Errors
///
/// Returns the refusal reason for a route block that cannot be hydrated.
pub fn hydrate_route(block: &mut Value, tracks: &RouteTracks) -> Result<(), String> {
    let Some(fields) = block.as_object_mut() else {
        return Ok(());
    };
    if fields.get("type").and_then(Value::as_str) != Some(ROUTE_KIND) {
        return Ok(());
    }
    let activity_id = fields
        .get("activity_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let track = match tracks.get(&activity_id) {
        Some(Ok(track)) => track,
        Some(Err(reason)) => return Err(reason.clone()),
        None => {
            return Err(format!(
                "activity \"{activity_id}\" was not read this turn, so no map can be drawn"
            ))
        }
    };
    let Ok(Value::Object(geometry)) = serde_json::to_value(track) else {
        return Err(format!(
            "activity \"{activity_id}\"'s track could not be encoded for the renderer"
        ));
    };
    let marks_climbs = fields.get("highlight").and_then(Value::as_str) == Some(HIGHLIGHT_CLIMBS);
    fields.remove("highlight");
    for (name, value) in geometry {
        fields.insert(name, value);
    }
    if !marks_climbs {
        fields.insert("climbs".to_owned(), Value::Array(Vec::new()));
    }
    Ok(())
}
