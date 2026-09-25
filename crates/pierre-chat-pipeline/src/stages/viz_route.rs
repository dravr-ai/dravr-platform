// ABOUTME: Reads the recorded track an activity's route block names and carries it on the block
// ABOUTME: A track is thousands of points, so the agent names an id and the platform reads the geometry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Route geometry for inline visual blocks.
//!
//! A `chart` or `table` block carries its own numbers, because an agent can
//! write a dozen points into a reply. A `route` cannot: a recorded track runs
//! to thousands of coordinates, so the block names an activity and the
//! platform reads that activity's geometry here. Coordinates a model produced
//! itself would be invented ones, which is why the schema gives it nowhere to
//! put them.
//!
//! Every series carried alongside the coordinates is either absent or exactly
//! as long as them. A padded elevation array would put a climb marker on the
//! wrong kilometre, which is worse than drawing no marker at all. The track
//! itself — the validity gate, the privacy trim, the climbs — is
//! [`RouteTrack`], the one derivation every surface that draws a ride shares.

use std::collections::BTreeMap;

use pierre_core::models::TimeSeriesData;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_fitness_compute::route_track::RouteTrack;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use serde_json::{json, Value};
use tracing::warn;

use std::sync::Arc;

use super::viz_blocks::{next_fence, validated_block, FENCE_INFO};
use super::viz_schema::SchemaTexts;
use crate::{ChatPipelineContext, TurnInput};

/// The block kind whose geometry the platform reads on the agent's behalf.
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
    let wanted: Vec<String> = route_activity_ids(&ctx.viz_schemas, granted, tools_called, reply)
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
/// Runs every gate extraction runs — the schema, the agent's grant, the
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
