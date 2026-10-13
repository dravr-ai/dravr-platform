// ABOUTME: Reads the line a route block names — an activity's recorded track or a proposed route's stored line
// ABOUTME: A line is thousands of points, so the agent names a reference and the platform reads the geometry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Route geometry for inline visual blocks.
//!
//! A `chart` or `table` block carries its own numbers, because an agent can
//! write a dozen points into a reply. A `route` cannot: a line runs to
//! thousands of coordinates, so the block names one and the platform reads
//! its geometry here. Coordinates a model produced itself would be invented
//! ones, which is why the schema gives it nowhere to put them.
//!
//! A block names one of two things ([`RouteSubject`]):
//!
//! - an `activity_id`, whose recorded track is read through the stream tool
//!   and trimmed of its endpoints ([`RouteTrack::from_streams`]);
//! - a `route_ref`, the line of a route the coach proposes, which
//!   `discover_routes` or `compose_route` stored for this athlete
//!   ([`route_store`]) and which is drawn end to end
//!   ([`RouteTrack::from_public_path`]): a trailhead is where the coach sends
//!   the athlete, not an address to hide.
//!
//! Every series carried alongside the coordinates is either absent or exactly
//! as long as them. A padded elevation array would put a climb marker on the
//! wrong kilometre, which is worse than drawing no marker at all. The track
//! itself — the validity gate, the privacy trim, the climbs — is
//! [`RouteTrack`], the one derivation every surface that draws a route shares.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use pierre_cache::Cache;
use pierre_core::models::{TenantId, TimeSeriesData};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_fitness_compute::route_track::RouteTrack;
use pierre_providers::ai_scope;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use pierre_tool_runtime::route_store;
use serde_json::{json, Map, Value};
use tracing::warn;
use uuid::Uuid;

use super::viz_blocks::{next_fence, validated_block, FENCE_INFO};
use super::viz_schema::SchemaTexts;
use crate::{ChatPipelineContext, TurnInput};

/// The block kind whose geometry the platform reads on the agent's behalf.
const ROUTE_KIND: &str = "route";

/// The `highlight` value that asks for the climbs to be marked.
const HIGHLIGHT_CLIMBS: &str = "climbs";

/// The tool that returns one activity's recorded per-second streams.
const ROUTE_STREAM_TOOL: &str = "extract_activity_streams";

/// The tools that store a proposed route's line, and so the only ones a block
/// naming a `route_ref` may cite: a ref is copied from one of their results.
const PROPOSED_ROUTE_TOOLS: [&str; 2] = ["discover_routes", "compose_route"];

/// How many maps one reply may have lines read for, of either kind.
///
/// An activity's track is a provider round trip against the athlete's own
/// rate-limited account; a proposed route's line is a cache read. The budget is
/// about maps, not about what a read costs: a reply that draws five maps is not
/// a reply anyone reads, and how many there are is decided by model output. So
/// one budget covers both kinds, and the blocks past it are refused with a
/// reason the repair re-ask can act on. It covers the whole turn: the repair
/// pass shares this map, so a second extraction cannot buy a second round.
const MAX_ROUTE_TRACKS_PER_REPLY: usize = 4;

/// What reading one named line produced.
///
/// The failure half is a sentence rather than a typed error because its only
/// two readers are the WARN line and the repair prompt, and both want the same
/// thing: which route, and why there is no map.
pub type RouteTrackResult = Result<RouteTrack, String>;

/// What a route block names: the line the platform reads for it.
///
/// The key every read is held under for a reply. A variant per kind, so an
/// activity id and a route reference can never collide, however either is
/// spelled — a provider is free to issue an activity id that reads `rt_…`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteSubject {
    /// An activity the athlete recorded, by the id the tool that reported it
    /// returned; its track is read through the stream tool.
    Activity(String),
    /// A route the coach proposes, by the `route_ref` `discover_routes` or
    /// `compose_route` returned; its line is read from the route store.
    Proposed(String),
}

impl RouteSubject {
    /// What a route block's fields name, or why they name nothing the platform
    /// may draw.
    ///
    /// The schema already holds a block to exactly one of the two ids. What it
    /// cannot check is the citation behind each: only a tool that stores a
    /// proposed route's line can have handed the model a `route_ref`, so a
    /// block citing any other tool did not copy its ref from a result — and
    /// those tools return no activity, so an `activity_id` citing one did not
    /// come from them either. Holding both ways is what lets a client tell a
    /// proposed map from a recorded one by its `source_tool` alone.
    fn of_block(fields: &Map<String, Value>) -> Result<Self, String> {
        let source_tool = fields
            .get("source_tool")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let proposed_tool = PROPOSED_ROUTE_TOOLS.contains(&source_tool);
        if let Some(route_ref) = fields.get("route_ref").and_then(Value::as_str) {
            if !proposed_tool {
                return Err(format!(
                    "route_ref \"{route_ref}\" names a proposed route, whose line only \
                     discover_routes or compose_route stores, so source_tool must name the one \
                     of those that returned it, not \"{source_tool}\""
                ));
            }
            return Ok(Self::Proposed(route_ref.to_owned()));
        }
        let Some(activity_id) = fields.get("activity_id").and_then(Value::as_str) else {
            return Err(
                "a route block names the route it draws, by activity_id or by route_ref".to_owned(),
            );
        };
        if proposed_tool {
            return Err(format!(
                "activity_id \"{activity_id}\" names a recorded activity, which \"{source_tool}\" \
                 never returns; cite the tool that reported the activity, or name the route_ref \
                 \"{source_tool}\" returned"
            ));
        }
        Ok(Self::Activity(activity_id.to_owned()))
    }
}

impl fmt::Display for RouteSubject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Activity(activity_id) => write!(f, "activity \"{activity_id}\""),
            Self::Proposed(route_ref) => write!(f, "route_ref \"{route_ref}\""),
        }
    }
}

/// Per-route read outcomes for the route blocks in one reply, keyed by what
/// each block named.
pub type RouteTracks = BTreeMap<RouteSubject, RouteTrackResult>;

/// Read the line of every route this reply's route blocks name.
///
/// Fills `tracks` in place and skips the routes it already holds, so the
/// repair re-ask can be handed the same map: a repaired reply naming the same
/// route costs nothing, and one naming a different route is read once.
///
/// Nothing here fails the turn. A route that cannot be read leaves its reason
/// in the map, the block that named it is refused during extraction, and the
/// athlete gets the prose — the same outcome as any other refused block.
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
    let wanted: Vec<RouteSubject> = route_subjects(&ctx.viz_schemas, granted, tools_called, reply)
        .into_iter()
        .filter(|subject| !tracks.contains_key(subject))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let (read_now, past_budget) = within_budget(wanted, tracks.len());
    // Same construction as tool dispatch (stage 9) and the capability-recovery
    // fetch: the athlete's own turn, on the turn's Guardian budget.
    let executor = UniversalExecutor::new(Arc::clone(&ctx.tool_runtime))
        .with_scopes(OAuthScope::self_grant())
        .with_conversation_id(input.conversation_id.clone())
        .with_conversation_tenant(input.conversation_tenant_id.as_uuid())
        .with_turn_token(input.turn_id.0.to_string());

    for subject in read_now {
        let outcome = match &subject {
            RouteSubject::Activity(activity_id) => {
                read_one_track(&executor, input, activity_id).await
            }
            // Keyed as the tool call that stored the line keyed it: tool
            // dispatch runs every call under the turn's tool tenant and the
            // athlete (`UniversalRequest { tenant_id: tool_tenant_id,
            // user_id }`), and the tool stores under that context's pair.
            RouteSubject::Proposed(route_ref) => {
                read_proposed_route(
                    ctx.tool_runtime.cache(),
                    input.tool_tenant_id,
                    &input.user_id,
                    route_ref,
                )
                .await
            }
        };
        tracks.insert(subject, outcome);
    }
    for subject in past_budget {
        tracks.insert(
            subject,
            Err(format!(
                "this reply already draws {MAX_ROUTE_TRACKS_PER_REPLY} maps, which is the most \
                 one reply carries; keep the ones that earn their place"
            )),
        );
    }
}

/// Split the routes a reply names, in reply order, into those read now and
/// those past the per-reply budget.
///
/// `held` is how many reads the turn already holds, of either kind: the repair
/// pass reads into the same map, and what the first pass spent is spent.
fn within_budget(
    mut wanted: Vec<RouteSubject>,
    held: usize,
) -> (Vec<RouteSubject>, Vec<RouteSubject>) {
    let budget = MAX_ROUTE_TRACKS_PER_REPLY
        .saturating_sub(held)
        .min(wanted.len());
    let past_budget = wanted.split_off(budget);
    (wanted, past_budget)
}

/// The routes named by the route blocks this reply would actually render.
///
/// Runs every gate extraction runs — the schema, the agent's grant, the
/// attribution check, the citation behind a `route_ref` — so a block that is
/// going to be refused never costs a read. In reply order, deduplicated: two
/// blocks drawing the same route are one read.
fn route_subjects(
    schemas: &SchemaTexts,
    granted: &[String],
    tools_called: &[String],
    reply: &str,
) -> Vec<RouteSubject> {
    let mut subjects: Vec<RouteSubject> = Vec::new();
    if !reply.contains(FENCE_INFO) {
        return subjects;
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
        // A block that names nothing drawable is refused by hydration, which
        // says why; it costs no read.
        let Some(Ok(subject)) = block.as_object().map(RouteSubject::of_block) else {
            continue;
        };
        if !subjects.contains(&subject) {
            subjects.push(subject);
        }
    }
    subjects
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
    };
    // The track becomes a map the athlete sees; the model only ever reads the
    // failure sentence below. Display, so no provider's AI policy applies.
    let response = match ai_scope::for_display(executor.execute_tool(request)).await {
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

/// Read the line a proposed route's `route_ref` names from the athlete's own
/// route store, and shape it into a drawable track.
///
/// `tenant_id` and `user_id` are the pair the storing tool ran under, so a
/// reference handed to one athlete's conversation reads nothing for another's.
/// The sentences that travel out speak of the route, never of an activity:
/// the model reading them proposed this line, it did not record it.
async fn read_proposed_route(
    cache: &Cache,
    tenant_id: TenantId,
    user_id: &str,
    route_ref: &str,
) -> RouteTrackResult {
    let Ok(athlete) = Uuid::parse_str(user_id) else {
        warn!(
            route_ref,
            "viz-blocks: the turn's athlete id is not a UUID, so no route line can be read"
        );
        return Err(unknown_route(route_ref));
    };
    match route_store::load_route(cache, tenant_id, athlete, route_ref).await {
        Ok(Some(stored)) => RouteTrack::from_public_path(&stored.coordinates).map_err(|reason| {
            warn!(
                error = %reason,
                route_ref, "viz-blocks: a stored route line is not drawable"
            );
            format!(
                "route_ref \"{route_ref}\" names a line with no length to draw, so no map can \
                 be drawn; leave the block out"
            )
        }),
        Ok(None) => Err(unknown_route(route_ref)),
        Err(e) => {
            warn!(error = %e, route_ref, "viz-blocks: reading a proposed route's line failed");
            Err(format!(
                "the line route_ref \"{route_ref}\" names could not be read, so no map can be \
                 drawn; leave the block out"
            ))
        }
    }
}

/// The sentence a block carries when its `route_ref` names no stored line:
/// never stored for this athlete, or stored more than a day ago.
fn unknown_route(route_ref: &str) -> String {
    format!(
        "route_ref \"{route_ref}\" is unknown or expired, so no map can be drawn; call \
         discover_routes again for a current one, or leave the block out"
    )
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
/// line could not be read is refused with the reason the read recorded —
/// there is no half-drawn map, and an empty one would read as "you went
/// nowhere". So is a `route_ref` block citing a tool that never stores a line,
/// and an `activity_id` block citing one that does.
///
/// The fields added are the ones photograveur's route spec declares beside
/// what the agent wrote, and no others, because that spec refuses any field it
/// does not declare: `coordinates`, `distances_meters` when the line is
/// measured, `elevation_meters` when it carries a vertical, and `climbs`. The
/// extent is not among them — photograveur frames the line itself, checking
/// every point as it does; a block that carried `bounds` was dropped at
/// resolution and the athlete got the prose alone.
///
/// `climbs` is empty unless the block asked for them, and always empty on a
/// proposed route, whose public line has no vertical to find a climb in. The
/// `highlight` the agent wrote stays on the block, since photograveur reads it
/// as the block's request and resolves a view with no highlight of its own.
/// The reference the block named — `activity_id` or `route_ref` — stays too.
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
    let subject = RouteSubject::of_block(fields)?;
    let track = match tracks.get(&subject) {
        Some(Ok(track)) => track,
        Some(Err(reason)) => return Err(reason.clone()),
        None => {
            return Err(format!(
                "{subject} was not read this turn, so no map can be drawn"
            ))
        }
    };
    let marks_climbs = fields.get("highlight").and_then(Value::as_str) == Some(HIGHLIGHT_CLIMBS);
    let Some(geometry) = route_geometry(track, marks_climbs) else {
        return Err(format!(
            "{subject}'s line could not be encoded for the renderer"
        ));
    };
    fields.extend(geometry);
    Ok(())
}

/// The geometry fields a hydrated route block carries, or `None` when one of
/// them does not encode.
fn route_geometry(track: &RouteTrack, marks_climbs: bool) -> Option<Map<String, Value>> {
    let mut geometry = Map::new();
    geometry.insert(
        "coordinates".to_owned(),
        serde_json::to_value(&track.coordinates).ok()?,
    );
    if let Some(elevations) = &track.elevation_meters {
        geometry.insert(
            "elevation_meters".to_owned(),
            serde_json::to_value(elevations).ok()?,
        );
    }
    if let Some(distances) = &track.distances_meters {
        geometry.insert(
            "distances_meters".to_owned(),
            serde_json::to_value(distances).ok()?,
        );
    }
    let climbs = if marks_climbs {
        serde_json::to_value(&track.climbs).ok()?
    } else {
        Value::Array(Vec::new())
    };
    geometry.insert("climbs".to_owned(), climbs);
    Some(geometry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_contremaitre::schemas::DRAVR_VIZ_SCHEMA;
    use pierre_cache::CacheConfig;
    use pierre_tool_runtime::route_store::{store_route, StoredRoute};

    async fn memory_cache() -> Cache {
        Cache::new(CacheConfig {
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
        .await
        .unwrap()
    }

    /// The Lachine canal path heading west out of the Old Port: a public
    /// line, so its first point is a trailhead and is drawn.
    fn canal() -> StoredRoute {
        StoredRoute {
            name: "Piste du canal de Lachine".to_owned(),
            coordinates: vec![
                (45.498_7, -73.553_1),
                (45.497_2, -73.558_4),
                (45.495_9, -73.563_8),
                (45.494_1, -73.569_9),
            ],
            length_meters: 1_330.0,
            out_and_back: false,
        }
    }

    fn proposed_block(source_tool: &str, route_ref: &str) -> Value {
        json!({
            "type": "route",
            "source_tool": source_tool,
            "route_ref": route_ref,
            "title": "Lachine – Vieux-Port",
        })
    }

    fn schemas() -> SchemaTexts {
        SchemaTexts::from([("dravr-viz".to_owned(), DRAVR_VIZ_SCHEMA.to_owned())])
    }

    fn fenced(body: &Value) -> String {
        format!("```dravr-viz\n{body}\n```")
    }

    /// A route the coach proposes is drawn from the line the route tool
    /// stored for this athlete, end to end, measured from its start, with no
    /// vertical and so no climbs; the block keeps the ref it named.
    #[tokio::test]
    async fn a_route_ref_block_draws_the_stored_line() {
        let cache = memory_cache().await;
        let (tenant_id, user_id) = (TenantId::generate(), Uuid::new_v4());
        let route = canal();
        let route_ref = store_route(&cache, tenant_id, user_id, &route)
            .await
            .unwrap();

        let track = read_proposed_route(&cache, tenant_id, &user_id.to_string(), &route_ref)
            .await
            .expect("a stored line is drawable");
        let mut tracks = RouteTracks::new();
        tracks.insert(RouteSubject::Proposed(route_ref.clone()), Ok(track));
        let mut block = proposed_block("compose_route", &route_ref);
        hydrate_route(&mut block, &tracks).expect("a read line hydrates its block");

        let drawn: Vec<(f64, f64)> = serde_json::from_value(block["coordinates"].clone()).unwrap();
        assert_eq!(
            drawn, route.coordinates,
            "the trailhead is drawn: a public line is not trimmed"
        );
        let distances = block["distances_meters"].as_array().unwrap();
        assert_eq!(distances.len(), drawn.len());
        assert_eq!(distances[0], json!(0.0), "measured from the route's start");
        assert_eq!(block["climbs"], json!([]));
        assert!(block.get("elevation_meters").is_none());
        assert!(
            block.get("bounds").is_none(),
            "photograveur frames the line"
        );
        assert_eq!(block["route_ref"], json!(route_ref));
        assert_eq!(block["source_tool"], json!("compose_route"));
        assert_eq!(block["title"], json!("Lachine – Vieux-Port"));
    }

    /// A ref that was never stored for this athlete — another athlete's, one
    /// past its day, an invented one — is refused with a sentence the repair
    /// re-ask can act on.
    #[tokio::test]
    async fn an_unknown_route_ref_is_refused_with_what_to_do() {
        let cache = memory_cache().await;
        let (tenant_id, user_id) = (TenantId::generate(), Uuid::new_v4());
        let route_ref = store_route(&cache, tenant_id, user_id, &canal())
            .await
            .unwrap();

        let another_athlete =
            read_proposed_route(&cache, tenant_id, &Uuid::new_v4().to_string(), &route_ref)
                .await
                .expect_err("a ref reads only its own athlete's store");
        let another_tenant = read_proposed_route(
            &cache,
            TenantId::generate(),
            &user_id.to_string(),
            &route_ref,
        )
        .await
        .expect_err("nor another tenant's");
        let invented = read_proposed_route(
            &cache,
            tenant_id,
            &user_id.to_string(),
            "rt_0000000000000000",
        )
        .await
        .expect_err("an invented ref names nothing");

        for reason in [&another_athlete, &another_tenant] {
            assert_eq!(*reason, unknown_route(&route_ref));
        }
        assert!(
            invented.contains("\"rt_0000000000000000\" is unknown or expired"),
            "{invented}"
        );
        assert!(
            invented.contains("call discover_routes again"),
            "{invented}"
        );

        let mut tracks = RouteTracks::new();
        tracks.insert(
            RouteSubject::Proposed("rt_0000000000000000".to_owned()),
            Err(invented.clone()),
        );
        let mut block = proposed_block("discover_routes", "rt_0000000000000000");
        let refused = hydrate_route(&mut block, &tracks).expect_err("no line, no map");
        assert_eq!(refused, invented, "the read's reason travels unchanged");
        assert!(
            block.get("coordinates").is_none(),
            "a refused block is never half-hydrated"
        );
    }

    /// Only the tools that store a proposed route's line can have handed the
    /// model a ref; a block citing another is refused, and never read.
    #[test]
    fn a_route_ref_citing_another_tool_is_refused_and_never_read() {
        let route_ref = "rt_3f9a0c41d27b8e65";
        let mut tracks = RouteTracks::new();
        tracks.insert(
            RouteSubject::Proposed(route_ref.to_owned()),
            Err("never consulted".to_owned()),
        );
        let mut block = proposed_block("get_activities", route_ref);
        let reason = hydrate_route(&mut block, &tracks).expect_err("the citation is wrong");
        assert!(
            reason.contains("discover_routes or compose_route"),
            "{reason}"
        );
        assert!(reason.contains("not \"get_activities\""), "{reason}");

        let granted = vec!["route".to_owned()];
        let tools = vec!["get_activities".to_owned()];
        let reply = fenced(&proposed_block("get_activities", route_ref));
        assert!(
            route_subjects(&schemas(), &granted, &tools, &reply).is_empty(),
            "a block that will be refused costs no read"
        );
    }

    /// The route tools return no activity, so an `activity_id` citing one was
    /// not copied from their results: it is refused, and never read. That is
    /// what lets a client call a map proposed by its `source_tool` alone.
    #[test]
    fn an_activity_citing_a_route_tool_is_refused_and_never_read() {
        let activity = json!({
            "type": "route", "source_tool": "discover_routes", "activity_id": "14872003941"
        });
        let mut tracks = RouteTracks::new();
        tracks.insert(
            RouteSubject::Activity("14872003941".to_owned()),
            Err("never consulted".to_owned()),
        );
        let reason =
            hydrate_route(&mut activity.clone(), &tracks).expect_err("a miscited activity");
        assert!(
            reason.contains("names a recorded activity, which \"discover_routes\" never returns"),
            "{reason}"
        );

        let granted = vec!["route".to_owned()];
        let tools = vec!["discover_routes".to_owned(), "get_activities".to_owned()];
        assert!(
            route_subjects(&schemas(), &granted, &tools, &fenced(&activity)).is_empty(),
            "a block that will be refused costs no read"
        );
    }

    /// One reply naming recorded and proposed routes alike reads each once,
    /// in reply order; an activity id spelled like a ref is still an activity.
    #[test]
    fn a_reply_names_both_kinds_in_order_without_collision() {
        let granted = vec!["route".to_owned()];
        let tools = vec!["get_activities".to_owned(), "discover_routes".to_owned()];
        let lookalike = "rt_3f9a0c41d27b8e65";
        let activity = json!({
            "type": "route", "source_tool": "get_activities", "activity_id": lookalike
        });
        let proposed = proposed_block("discover_routes", lookalike);
        let reply = [
            fenced(&proposed),
            fenced(&activity),
            fenced(&proposed),
            fenced(&json!({
                "type": "route", "source_tool": "get_activities", "activity_id": "14872003941"
            })),
        ]
        .join("\n\nEt puis :\n\n");

        assert_eq!(
            route_subjects(&schemas(), &granted, &tools, &reply),
            vec![
                RouteSubject::Proposed(lookalike.to_owned()),
                RouteSubject::Activity(lookalike.to_owned()),
                RouteSubject::Activity("14872003941".to_owned()),
            ]
        );
    }

    /// The per-reply budget counts maps, whichever kind each is: proposed
    /// routes and recorded ones draw on the same four, and a repair pass
    /// starts from what the first pass already spent.
    #[test]
    fn one_budget_covers_both_kinds() {
        let subjects = |names: &[&str]| -> Vec<RouteSubject> {
            names
                .iter()
                .map(|name| {
                    name.strip_prefix("a:").map_or_else(
                        || RouteSubject::Proposed((*name).to_owned()),
                        |id| RouteSubject::Activity(id.to_owned()),
                    )
                })
                .collect()
        };
        let (read_now, past) = within_budget(subjects(&["rt_1", "a:1", "rt_2", "a:2", "rt_3"]), 0);
        assert_eq!(read_now, subjects(&["rt_1", "a:1", "rt_2", "a:2"]));
        assert_eq!(past, subjects(&["rt_3"]));

        let (read_now, past) = within_budget(subjects(&["a:3", "rt_4"]), 3);
        assert_eq!(
            read_now,
            subjects(&["a:3"]),
            "three proposed reads were spent"
        );
        assert_eq!(past, subjects(&["rt_4"]));

        let (read_now, past) = within_budget(subjects(&["rt_5"]), 6);
        assert!(read_now.is_empty());
        assert_eq!(past, subjects(&["rt_5"]));
    }
}
