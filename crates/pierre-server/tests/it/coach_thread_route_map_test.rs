// ABOUTME: carnet#1448 — in a coach's thread about one athlete, a route map draws the athlete's ride, never the coach's
// ABOUTME: The map's track is read on the turn's seat, under the coaching gates, and refused once the athlete stops sharing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A coach's training reads in their thread about one athlete read that
//! athlete (carnet#1387), and so does the route map the platform draws for an
//! activity the model discussed: the athlete and the coach each recorded a
//! run under the same id, and the seat, not the id, decides whose track is
//! drawn. These tests read the map's track the way the chat pipeline does, on
//! the turn's seat, after `get_activities` handed the model the athlete's run.

use chrono::Utc;
use pierre_chat_pipeline::stages::viz_route::{read_route_tracks, RouteSubject, RouteTracks};
use pierre_chat_pipeline::{InputSource, QuotaState, TurnInput, TurnOrigin};
use pierre_core::models::{ConnectionType, ConversationTurnId, TimeSeriesData};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::{ActivityStreamsWrite, StreamOrigin};
use pierre_providers::ai_scope;
use pierre_tool_runtime::coach_seat::TurnSeat;
use serde_json::json;
use uuid::Uuid;

use crate::coach_athlete_reads_test::{fixture, ok, run, Fixture, Person, UPLOAD};

/// The run the map draws: the athlete and the coach each recorded one under
/// this id.
const RIDE: &str = "mapped-run-1";

/// Samples in a fixture track.
const POINTS: usize = 21;

/// Latitude step between two samples: about 50 m due north, so the 200 m
/// privacy trim leaves a middle worth drawing.
const LATITUDE_STEP: f64 = 0.000_45;

/// Where the athlete's run starts.
const ATHLETE_START: f64 = 45.5;

/// Where the coach's own run under the same id starts, far from the athlete's.
const COACH_START: f64 = 46.8;

const LONGITUDE: f64 = -73.6;

/// A run heading due north from `start`.
fn track_from(start: f64) -> TimeSeriesData {
    TimeSeriesData {
        timestamps: (0..POINTS as u32).collect(),
        heart_rate: None,
        power: None,
        cadence: None,
        speed: None,
        altitude: None,
        temperature: None,
        gps_coordinates: Some(
            (0..POINTS)
                .map(|step| Some((LATITUDE_STEP.mul_add(step as f64, start), LONGITUDE)))
                .collect(),
        ),
        distance: None,
    }
}

/// `who`'s run [`RIDE`], recorded along the track from `start`,
/// read through a connection as the per-activity reads expect.
async fn record_run(f: &Fixture, who: Person, start: f64) {
    let repos = f.repos();
    repos
        .provider_connections
        .register_connection(who.id, who.tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(who.id, &who.tenant, UPLOAD, &[run(RIDE, 1)])
        .await
        .unwrap();
    repos
        .activity_streams
        .upsert_activity_streams(&ActivityStreamsWrite {
            tenant_id: &who.tenant,
            user_id: who.id,
            provider: UPLOAD,
            activity_id: RIDE,
            origin: StreamOrigin::Upload,
            source_ref: None,
            streams: &track_from(start),
            synced_at: Utc::now(),
        })
        .await
        .unwrap();
}

/// A thread in which the athlete and the coach each recorded a run under the
/// same id, on tracks far apart.
async fn thread_with_two_runs() -> Fixture {
    let f = fixture().await;
    record_run(&f, f.athlete, ATHLETE_START).await;
    record_run(&f, f.coach, COACH_START).await;
    f
}

/// The track the platform reads for a route block naming the run, in the
/// coach's turn in `conversation` on `seat`.
async fn map_of_the_run(f: &Fixture, conversation: &str, seat: TurnSeat) -> RouteTracks {
    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        conversation_id: conversation.to_owned(),
        user_id: f.coach.id.to_string(),
        conversation_tenant_id: f.coach.tenant,
        tool_tenant_id: f.coach.tenant,
        is_direct_message: true,
        content: "Montre-moi sa dernière sortie".to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quote: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
        steps_after: None,
        grant: OAuthScope::self_grant(),
    };
    let block = json!({
        "type": "route",
        "source_tool": "get_activities",
        "activity_id": RIDE,
        "title": "Sa dernière sortie",
    });
    let reply = format!("Voici sa sortie :\n\n```dravr-viz\n{block}\n```");
    let granted = vec!["route".to_owned()];
    let tools_called = vec!["get_activities".to_owned()];
    let ctx = f.res.chat_pipeline_context();
    let mut tracks = RouteTracks::new();
    // Over Dravr's own web app, as `turn_service` declares every turn: the
    // GPS channel reaches only Dravr's own surfaces.
    ai_scope::serve_over(
        Transport::WebApp,
        read_route_tracks(
            &ctx,
            &input,
            seat,
            &granted,
            &tools_called,
            &reply,
            &mut tracks,
        ),
    )
    .await;
    tracks
}

/// The latitude the drawn line starts at, or why there is no line.
fn drawn_start(tracks: &RouteTracks) -> Result<f64, String> {
    let read = tracks
        .get(&RouteSubject::Activity(RIDE.to_owned()))
        .expect("the block's activity was read")
        .clone()?;
    Ok(read.coordinates.first().expect("a drawn line has points").0)
}

#[tokio::test]
async fn the_route_map_in_the_thread_draws_the_athletes_ride_never_the_coachs() {
    let f = thread_with_two_runs().await;
    let listed = ok(
        f.in_thread("get_activities", json!({})).await,
        "get_activities",
    );
    assert!(
        listed.to_string().contains(RIDE),
        "the model was handed the athlete's run: {listed}"
    );

    let start = drawn_start(&map_of_the_run(&f, &f.thread, TurnSeat::AthleteThread).await)
        .expect("the athlete's ride is drawn");
    assert!(
        (start - ATHLETE_START).abs() < 0.01,
        "the athlete's track, past its trimmed start: {start}"
    );

    // The same id read on the coach's own seat is the coach's run: the seat,
    // not the id, decides whose ride the map is.
    let own = Uuid::new_v4().to_string();
    let start = drawn_start(&map_of_the_run(&f, &own, TurnSeat::Subject).await)
        .expect("the coach's own ride is drawn in their own conversation");
    assert!((start - COACH_START).abs() < 0.01, "{start}");
}

#[tokio::test]
async fn the_route_map_in_the_thread_is_refused_once_the_athlete_stops_sharing() {
    let f = thread_with_two_runs().await;
    f.repos()
        .groups
        .update_coach_sharing_consent(&f.group.to_string(), f.athlete.id, false)
        .await
        .unwrap();
    let refused = drawn_start(&map_of_the_run(&f, &f.thread, TurnSeat::AthleteThread).await);
    assert!(
        refused.is_err(),
        "no map once the athlete stops sharing, and never the coach's ride: {refused:?}"
    );
}
