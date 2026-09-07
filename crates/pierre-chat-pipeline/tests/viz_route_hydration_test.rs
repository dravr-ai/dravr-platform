// ABOUTME: A route block carries the platform's reading of a recorded track, never the coach's
// ABOUTME: Asserts the hydrated block's real coordinates, its trimmed endpoints and its climb marks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use pierre_chat_pipeline::stages::viz_route::{hydrate_route, RouteTrack, RouteTracks};
use pierre_core::models::TimeSeriesData;
use serde_json::{json, Value};

/// Points in the fixture track.
const POINTS: usize = 21;

/// Latitude step between two samples: 0.00045° is ~50.03 m due north.
const LATITUDE_STEP: f64 = 0.000_45;

/// Metres between two samples, at [`LATITUDE_STEP`].
const SAMPLE_METERS: f64 = 50.03;

/// The fixture's longitude — it runs due north, so this never moves.
const LONGITUDE: f64 = -73.6;

/// Metres of altitude gained per sample: 2 m over ~50 m is a 4 % grade.
const ALTITUDE_STEP: f32 = 2.0;

/// First sample surviving the 200 m privacy trim — 3 × 50.03 m is still
/// inside it, 4 × 50.03 m is the first point outside.
const FIRST_DRAWN: usize = 4;

/// Last sample surviving the trim, by the same arithmetic from the far end.
const LAST_DRAWN: usize = 16;

/// How many samples the drawn line keeps.
const DRAWN_POINTS: usize = LAST_DRAWN - FIRST_DRAWN + 1;

/// Latitude of the fixture's `index`-th sample.
fn latitude_at(index: usize) -> f64 {
    LATITUDE_STEP.mul_add(index as f64, 45.5)
}

/// A track heading due north out of Montréal up a steady 4 % grade.
///
/// One kilometre long, climbing 40 m. Long enough that the 200 m privacy trim
/// leaves a middle worth drawing, and steep and long enough past both climb
/// floors (250 m, 1.5 %) that the climb assertions below are about a real
/// detected climb rather than an empty vector.
fn hill_streams() -> TimeSeriesData {
    let mut altitude = Vec::with_capacity(POINTS);
    let mut gps_coordinates = Vec::with_capacity(POINTS);
    for step in 0..POINTS {
        altitude.push(ALTITUDE_STEP.mul_add(step as f32, 100.0));
        gps_coordinates.push((latitude_at(step), LONGITUDE));
    }
    TimeSeriesData {
        timestamps: (0..POINTS as u32).collect(),
        heart_rate: None,
        power: None,
        cadence: None,
        speed: None,
        altitude: Some(altitude),
        temperature: None,
        gps_coordinates: Some(gps_coordinates),
    }
}

/// A route block as a coach writes one: a reference, with no geometry in it.
fn route_block(highlight: Option<&str>) -> Value {
    let mut block = json!({
        "type": "route",
        "source_tool": "get_activities",
        "title": "Ta sortie de dimanche",
        "activity_id": "14872003941",
    });
    if let Some(highlight) = highlight {
        block["highlight"] = json!(highlight);
    }
    block
}

/// The read of one activity, as the extraction stage receives it.
fn tracks_for(activity_id: &str, outcome: Result<RouteTrack, String>) -> RouteTracks {
    let mut tracks = RouteTracks::new();
    tracks.insert(activity_id.to_owned(), outcome);
    tracks
}

#[test]
fn the_drawn_track_is_the_ride_minus_its_endpoints() {
    let track = RouteTrack::from_streams(&hill_streams()).expect("the hill is a drawable track");

    assert_eq!(
        track.coordinates.len(),
        DRAWN_POINTS,
        "200 m off each end of a kilometre leaves the middle 600 m"
    );
    assert_eq!(track.coordinates[0], (latitude_at(FIRST_DRAWN), LONGITUDE));
    assert_eq!(
        track.coordinates[DRAWN_POINTS - 1],
        (latitude_at(LAST_DRAWN), LONGITUDE)
    );
    assert!(
        track
            .coordinates
            .iter()
            .all(|&(latitude, _)| (latitude - 45.5).abs() > 1e-9),
        "the coordinate the ride started at is a doorstep and must not be on the map"
    );

    assert!((track.bounds.min_latitude - latitude_at(FIRST_DRAWN)).abs() < 1e-12);
    assert!((track.bounds.max_latitude - latitude_at(LAST_DRAWN)).abs() < 1e-12);
    assert!((track.bounds.min_longitude - LONGITUDE).abs() < 1e-12);
    assert!((track.bounds.max_longitude - LONGITUDE).abs() < 1e-12);
}

#[test]
fn every_series_is_exactly_as_long_as_the_line() {
    let track = RouteTrack::from_streams(&hill_streams()).expect("the hill is a drawable track");

    let elevations = track
        .elevation_meters
        .as_ref()
        .expect("the provider recorded altitude");
    assert_eq!(elevations.len(), DRAWN_POINTS);
    assert!((elevations[0] - 108.0).abs() < 1e-9, "100 m + 4 × 2 m");
    assert!(
        (elevations[DRAWN_POINTS - 1] - 132.0).abs() < 1e-9,
        "100 m + 16 × 2 m"
    );

    let distances = track
        .distances_meters
        .as_ref()
        .expect("distance is measured from the coordinates");
    assert_eq!(distances.len(), DRAWN_POINTS);
    // Sliced, never rebased: the first drawn sample is 200 m into the real
    // ride, and saying so is what keeps an elevation profile honest.
    let picked_up_at = SAMPLE_METERS * FIRST_DRAWN as f64;
    let left_at = SAMPLE_METERS * LAST_DRAWN as f64;
    assert!(
        (distances[0] - picked_up_at).abs() < 2.0,
        "the line picks the ride up 200 m in, measured: {}",
        distances[0]
    );
    assert!(
        (distances[DRAWN_POINTS - 1] - left_at).abs() < 2.0,
        "and leaves it 800 m in, measured: {}",
        distances[DRAWN_POINTS - 1]
    );
}

#[test]
fn the_climb_is_found_on_the_drawn_line_and_categorised() {
    let track = RouteTrack::from_streams(&hill_streams()).expect("the hill is a drawable track");

    assert_eq!(track.climbs.len(), 1, "the whole track is one steady climb");
    let climb = &track.climbs[0];
    assert_eq!(
        climb.start_index, 0,
        "climb indices address the drawn coordinates, not the recorded ones"
    );
    assert_eq!(climb.end_index, DRAWN_POINTS - 1);
    assert!(
        (climb.avg_gradient - 4.0).abs() < 0.2,
        "24 m of gain over 600 m is 4 %, carried in percent not as a fraction, measured: {}",
        climb.avg_gradient
    );
    assert_eq!(
        climb.category.as_deref(),
        Some("HC"),
        "24 m at 4 % scores 96 on the Fiets index, which is hors catégorie — \
         spelled as the athlete reads it, since the client captions it directly"
    );
}

#[test]
fn a_hydrated_block_carries_the_read_geometry() {
    let track = RouteTrack::from_streams(&hill_streams()).expect("the hill is a drawable track");
    let tracks = tracks_for("14872003941", Ok(track));
    let mut block = route_block(Some("climbs"));

    hydrate_route(&mut block, &tracks).expect("a read track hydrates its block");

    let coordinates = block["coordinates"]
        .as_array()
        .expect("the block carries coordinates");
    assert_eq!(coordinates.len(), DRAWN_POINTS);
    assert_eq!(
        coordinates[0],
        json!([latitude_at(FIRST_DRAWN), LONGITUDE]),
        "the first drawn point reaches the renderer as a (lat, lon) pair"
    );

    assert!(
        (block["bounds"]["min_latitude"].as_f64().unwrap() - latitude_at(FIRST_DRAWN)).abs()
            < 1e-12
    );
    assert!(
        (block["bounds"]["max_latitude"].as_f64().unwrap() - latitude_at(LAST_DRAWN)).abs() < 1e-12
    );

    assert_eq!(
        block["elevation_meters"].as_array().unwrap().len(),
        DRAWN_POINTS,
        "a series is exactly as long as the coordinates, never padded"
    );
    assert_eq!(
        block["distances_meters"].as_array().unwrap().len(),
        DRAWN_POINTS
    );

    let climbs = block["climbs"].as_array().expect("climbs were asked for");
    assert_eq!(climbs.len(), 1);
    assert_eq!(climbs[0]["start_index"], json!(0));
    assert_eq!(climbs[0]["end_index"], json!(DRAWN_POINTS - 1));
    assert_eq!(
        climbs[0]["category"],
        json!("HC"),
        "the grade is carried as the athlete reads it, since the client captions it directly"
    );

    // The reference the coach wrote survives; the question it asked does not,
    // because hydration answered it.
    assert_eq!(block["activity_id"], json!("14872003941"));
    assert_eq!(block["title"], json!("Ta sortie de dimanche"));
    assert_eq!(block["source_tool"], json!("get_activities"));
    assert!(
        block.get("highlight").is_none(),
        "highlight is consumed by hydration, not carried into the renderer"
    );
}

#[test]
fn a_route_without_highlight_draws_the_line_and_no_marks() {
    let track = RouteTrack::from_streams(&hill_streams()).expect("the hill is a drawable track");
    let tracks = tracks_for("14872003941", Ok(track));
    let mut block = route_block(None);

    hydrate_route(&mut block, &tracks).expect("a read track hydrates its block");

    assert_eq!(block["coordinates"].as_array().unwrap().len(), DRAWN_POINTS);
    assert_eq!(
        block["climbs"],
        json!([]),
        "the climb is there in the terrain, but the block did not ask for it"
    );
    assert_eq!(
        block["elevation_meters"].as_array().unwrap().len(),
        DRAWN_POINTS,
        "the profile is carried either way — only the marks are conditional"
    );
}

#[test]
fn a_track_without_altitude_still_draws_a_line() {
    let mut streams = hill_streams();
    streams.altitude = None;
    let track = RouteTrack::from_streams(&streams).expect("GPS alone is a drawable track");

    assert_eq!(track.coordinates.len(), DRAWN_POINTS);
    assert_eq!(
        track.distances_meters.as_ref().map(Vec::len),
        Some(DRAWN_POINTS)
    );
    assert!(
        track.elevation_meters.is_none(),
        "an absent series is absent, never zeroes"
    );
    assert!(
        track.climbs.is_empty(),
        "a climb needs the vertical the provider did not record"
    );
}

#[test]
fn unusable_points_are_dropped_and_the_series_stay_aligned() {
    let mut streams = hill_streams();
    let coordinates = streams
        .gps_coordinates
        .as_mut()
        .expect("the fixture records GPS");
    coordinates[5] = (f64::NAN, LONGITUDE);
    coordinates[9] = (91.0, LONGITUDE);

    let track = RouteTrack::from_streams(&streams).expect("the rest of the track still draws");

    assert!(
        track
            .coordinates
            .iter()
            .all(|&(latitude, _)| latitude.is_finite() && latitude <= 90.0),
        "a non-finite point and an off-Earth one are both dropped"
    );
    let drawn = track.coordinates.len();
    assert!(
        drawn >= 2,
        "what is left is still a line, got {drawn} points"
    );
    assert_eq!(
        track.elevation_meters.as_ref().map(Vec::len),
        Some(drawn),
        "the altitude series follows the points it belongs to"
    );
    assert_eq!(
        track.distances_meters.as_ref().map(Vec::len),
        Some(drawn),
        "so does the distance series"
    );
    for climb in &track.climbs {
        assert!(
            climb.end_index < drawn,
            "a climb index must address a coordinate the block carries"
        );
    }
}

#[test]
fn a_ride_that_never_leaves_its_own_doorstep_is_refused() {
    // Five samples spanning 44 m: every point sits inside the privacy radius
    // of both ends, so there is no line that does not publish the address.
    let mut streams = hill_streams();
    streams.gps_coordinates = Some(
        (0..5)
            .map(|step| (0.000_1_f64.mul_add(f64::from(step), 45.5), LONGITUDE))
            .collect(),
    );
    streams.altitude = Some(vec![100.0; 5]);

    let reason = RouteTrack::from_streams(&streams)
        .expect_err("a ride that never leaves the block cannot be drawn");
    assert!(
        reason.contains("without publishing that address"),
        "the refusal must say why there is no map: {reason}"
    );
}

#[test]
fn a_single_point_is_not_a_route() {
    let mut streams = hill_streams();
    streams.gps_coordinates = Some(vec![(45.5, LONGITUDE)]);
    streams.altitude = Some(vec![100.0]);

    let reason = RouteTrack::from_streams(&streams).expect_err("one point is a pin, not a route");
    assert!(
        reason.contains("without publishing that address"),
        "one point cannot be trimmed into a line: {reason}"
    );
}

#[test]
fn an_activity_with_no_gps_refuses_the_block() {
    let mut streams = hill_streams();
    streams.gps_coordinates = None;

    let reason =
        RouteTrack::from_streams(&streams).expect_err("a treadmill session has no track to draw");
    assert!(
        reason.contains("no recorded GPS track"),
        "the refusal must name the missing channel: {reason}"
    );
}

#[test]
fn an_unread_activity_refuses_the_block_rather_than_drawing_an_empty_map() {
    let mut block = route_block(Some("climbs"));
    let reason = hydrate_route(&mut block, &RouteTracks::new())
        .expect_err("a block whose activity was never read cannot be drawn");

    assert!(
        reason.contains("14872003941"),
        "the refusal names the activity so the repair re-ask can act on it: {reason}"
    );
    assert!(
        block.get("coordinates").is_none(),
        "a refused block is left as the coach wrote it, never half-hydrated"
    );
}

#[test]
fn a_failed_read_refuses_with_the_reason_it_recorded() {
    let mut block = route_block(None);
    let tracks = tracks_for(
        "14872003941",
        Err("activity \"14872003941\" has no track the platform can read".to_owned()),
    );

    let reason = hydrate_route(&mut block, &tracks).expect_err("an unreadable activity is refused");
    assert_eq!(
        reason, "activity \"14872003941\" has no track the platform can read",
        "the read's own reason travels to the repair prompt unchanged"
    );
}

#[test]
fn a_chart_block_passes_through_untouched() {
    let mut block = json!({
        "type": "chart",
        "kind": "line",
        "source_tool": "analyze_training_load",
        "x": { "label": "Date", "type": "time" },
        "series": [{ "label": "CTL", "points": [["2026-07-01", 42.0], ["2026-07-02", 43.1]] }]
    });
    let before = block.clone();

    hydrate_route(&mut block, &RouteTracks::new()).expect("hydration ignores other block kinds");

    assert_eq!(block, before, "only a route block is hydrated");
}
