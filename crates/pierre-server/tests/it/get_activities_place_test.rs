// ABOUTME: get_activities serves each row's place as {id, name}: the athlete's own name everywhere, an automatic one first-party only
// ABOUTME: One read of the served rows gives every row its own place, past the detail window and past a full page of copies

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Slice 6 (carnet#1484): a row's place on `get_activities`.
//!
//! Every served list reads its cached rows once
//! (`activity_detail_store::stored_served_rows`), and each row whose start
//! belongs to one of the athlete's places carries it as `place: {id, name}`.
//! A name the athlete gave is served on every transport; a city or segment
//! name only on Dravr's own surfaces (OQ8). A place is never located: no
//! centroid, radius or point reaches any reader.
//!
//! The athlete is the coach-reads fixture's, whose uploaded runs reach
//! `get_activities` with no provider resolved; the tool is driven through the
//! dispatch chokepoint, so the transport gate runs as it does in production.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::coach_athlete_reads_test::{fixture, ok, run, Fixture, Person, ATHLETE_RUNS, UPLOAD};

use std::slice;
use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_core::models::{Activity, CachedRowKey};
use pierre_database::repositories::{AthletePlace, PlaceNameSource, PlaceRun};
use pierre_fitness_compute::places::PLACES_VERSION;
use pierre_tool_runtime::activity_detail_store::store_detail;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

/// The athlete's name for the place their first run starts from.
pub(crate) const ATHLETE_NAME: &str = "Gai-Luron";
/// The automatic (city) name of the place their second run starts from.
pub(crate) const CITY_NAME: &str = "Montréal";

/// A place of `person`'s, `name` from `source`, at no particular point (the
/// test never reads one back).
fn place(name: &str, source: PlaceNameSource) -> AthletePlace {
    let now = Utc::now();
    AthletePlace {
        id: Uuid::new_v4().to_string(),
        centroid_lat: 45.5,
        centroid_lon: -73.6,
        radius_m: 300.0,
        activity_count: 1,
        name: Some(name.to_owned()),
        name_source: Some(source),
        version: PLACES_VERSION,
        computed_at: now,
        created_at: now,
    }
}

fn key(provider: &str, activity_id: &str) -> CachedRowKey {
    CachedRowKey {
        provider: provider.to_owned(),
        activity_id: activity_id.to_owned(),
    }
}

/// Give `person`'s rows automatic places, as a clustering would: each
/// `(provider, activity id, city)` its own place named for the city. Returns
/// the place ids, in order.
pub(crate) async fn city_places(
    f: &Fixture,
    person: Person,
    rows: &[(&str, &str, &str)],
) -> Vec<String> {
    let places: Vec<AthletePlace> = rows
        .iter()
        .map(|(_, _, city)| place(city, PlaceNameSource::City))
        .collect();
    let assignments: Vec<(CachedRowKey, Option<String>)> = rows
        .iter()
        .zip(&places)
        .map(|((provider, id, _), place)| (key(provider, id), Some(place.id.clone())))
        .collect();
    let run_row = PlaceRun {
        rows_seen: 0,
        newest_synced_at: None,
        version: PLACES_VERSION,
        computed_at: Utc::now(),
    };
    f.repos()
        .athlete_places
        .replace_clustering(&person.tenant, person.id, &places, &assignments, &run_row)
        .await
        .unwrap();
    places.into_iter().map(|place| place.id).collect()
}

/// Give `person`'s row under `provider`/`activity_id` a place of its own the
/// athlete named `name`. Returns the place id.
pub(crate) async fn athlete_place(
    f: &Fixture,
    person: Person,
    provider: &str,
    activity_id: &str,
    name: &str,
) -> String {
    let named = place(name, PlaceNameSource::Athlete);
    f.repos()
        .athlete_places
        .create_athlete_place(
            &person.tenant,
            person.id,
            &named,
            &key(provider, activity_id),
        )
        .await
        .unwrap();
    named.id
}

/// The listed row of `id`.
pub(crate) fn row<'a>(payload: &'a Value, id: &str) -> &'a Value {
    payload["activities"]
        .as_array()
        .expect("activities is a list")
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("bug: {id} is not listed: {payload}"))
}

/// No key of `payload`, at any depth, locates a place.
pub(crate) fn assert_no_location(payload: &Value) {
    fn walk(value: &Value, path: &str) {
        match value {
            Value::Object(map) => {
                for (key, inner) in map {
                    assert!(
                        !["centroid_lat", "centroid_lon", "centroid", "radius_m"]
                            .contains(&key.as_str()),
                        "bug: a place's location reached a reader at {path}.{key}"
                    );
                    walk(inner, &format!("{path}.{key}"));
                }
            }
            Value::Array(items) => {
                for (i, inner) in items.iter().enumerate() {
                    walk(inner, &format!("{path}[{i}]"));
                }
            }
            _ => {}
        }
    }
    walk(payload, "$");
}

/// The athlete's first run at a place they named, their second at a city's,
/// their third at none.
async fn placed_fixture() -> (Fixture, String, String) {
    let f = fixture().await;
    let city = city_places(&f, f.athlete, &[(UPLOAD, "athlete-run-2", CITY_NAME)]).await;
    let named = athlete_place(&f, f.athlete, UPLOAD, "athlete-run-1", ATHLETE_NAME).await;
    (f, named, city[0].clone())
}

async fn own_list(f: &Fixture) -> Value {
    ok(
        f.call(
            &f.own_executor(),
            f.athlete,
            "get_activities",
            json!({ "limit": 50 }),
        )
        .await,
        "get_activities",
    )
}

#[tokio::test]
async fn a_first_party_read_serves_each_rows_place_with_its_name() {
    let (f, named, city) = placed_fixture().await;
    let listed = own_list(&f).await;

    assert_eq!(
        row(&listed, "athlete-run-1")["place"],
        json!({ "id": named, "name": ATHLETE_NAME }),
        "bug: the place the athlete named is not on its row: {listed}"
    );
    assert_eq!(
        row(&listed, "athlete-run-2")["place"],
        json!({ "id": city, "name": CITY_NAME }),
        "bug: a first-party read lost the automatic name of the row's place: {listed}"
    );
    assert!(
        row(&listed, "athlete-run-3").get("place").is_none(),
        "bug: a row with no place was given one: {listed}"
    );
    assert_no_location(&listed);
}

#[tokio::test]
async fn an_external_read_serves_only_the_athletes_own_names() {
    let (f, named, city) = placed_fixture().await;
    let listed = ok(
        f.call(
            &f.mcp_executor(),
            f.athlete,
            "get_activities",
            json!({ "limit": 50 }),
        )
        .await,
        "get_activities over MCP",
    );

    assert_eq!(
        row(&listed, "athlete-run-1")["place"],
        json!({ "id": named, "name": ATHLETE_NAME }),
        "bug: the athlete's own place name was withheld from an MCP client: {listed}"
    );
    assert_eq!(
        row(&listed, "athlete-run-2")["place"],
        json!({ "id": city }),
        "bug: an automatic place name reached an MCP client (OQ8): {listed}"
    );
    assert!(
        !listed.to_string().contains(CITY_NAME),
        "bug: the city's name reached an MCP client elsewhere in the answer: {listed}"
    );
    assert_no_location(&listed);
}

/// The second identical read is the response-cache hit, the other builder
/// call site: a run stored after the first read is not in it. A hit serves
/// only a list whose recent rows all have their detail stored, so every run
/// has one, and the hit renders them in detail mode.
#[tokio::test]
async fn a_cached_list_serves_the_same_places() {
    let (f, named, city) = placed_fixture().await;
    let runtime: Arc<dyn ToolRuntime> = f.res.clone();
    for day in 1..=ATHLETE_RUNS {
        assert!(
            store_detail(
                runtime.repos(),
                runtime.live_activities().as_ref(),
                f.athlete.tenant,
                f.athlete.id,
                UPLOAD,
                &run(&format!("athlete-run-{day}"), day),
            )
            .await,
            "the fixture's detail was not stored"
        );
    }
    let first = own_list(&f).await;
    upload_run(&f, "athlete-run-after-the-first-read", 0).await;
    let hit = own_list(&f).await;
    assert!(
        !hit.to_string().contains("athlete-run-after-the-first-read"),
        "the second identical read must be the response-cache hit, or this proves nothing"
    );

    assert_eq!(
        hit["mode"], "detailed",
        "the hit must render its rows from their stored details: {hit}"
    );
    for listed in [&first, &hit] {
        assert_eq!(
            row(listed, "athlete-run-1")["place"],
            json!({ "id": named, "name": ATHLETE_NAME }),
            "bug: the cache hit lost the row's place: {listed}"
        );
        assert_eq!(
            row(listed, "athlete-run-2")["place"],
            json!({ "id": city, "name": CITY_NAME }),
            "bug: the cache hit lost the row's place: {listed}"
        );
    }
}

#[tokio::test]
async fn a_coachs_thread_serves_the_athletes_places() {
    let (f, named, city) = placed_fixture().await;
    let listed = ok(
        f.in_thread("get_activities", json!({ "limit": 50 })).await,
        "get_activities in the coach's thread",
    );

    assert_eq!(
        row(&listed, "athlete-run-1")["place"],
        json!({ "id": named, "name": ATHLETE_NAME }),
        "bug: the coach's read of the athlete lost the row's place: {listed}"
    );
    assert_eq!(
        row(&listed, "athlete-run-2")["place"],
        json!({ "id": city, "name": CITY_NAME }),
        "bug: the coach's in-app read lost the row's automatic name: {listed}"
    );
    assert_no_location(&listed);
}

/// A run of the athlete's `days_ago`, uploaded.
async fn upload_run(f: &Fixture, id: &str, days_ago: i64) -> Activity {
    let activity = run(id, days_ago);
    f.repos()
        .activity_cache
        .upsert_activities(
            f.athlete.id,
            &f.athlete.tenant,
            UPLOAD,
            slice::from_ref(&activity),
        )
        .await
        .unwrap();
    activity
}

#[tokio::test]
async fn a_row_past_the_detail_window_carries_its_place() {
    let f = fixture().await;
    let old = upload_run(&f, "athlete-run-old", 40).await;
    let named = athlete_place(&f, f.athlete, UPLOAD, old.id(), ATHLETE_NAME).await;
    let after = (Utc::now() - Duration::days(60)).timestamp();
    let before = (Utc::now() - Duration::days(30)).timestamp();

    let listed = ok(
        f.call(
            &f.own_executor(),
            f.athlete,
            "get_activities",
            json!({ "after": after, "before": before, "limit": 10 }),
        )
        .await,
        "get_activities, an older window",
    );

    assert_eq!(
        row(&listed, "athlete-run-old")["place"],
        json!({ "id": named, "name": ATHLETE_NAME }),
        "bug: a row older than the fourteen-day detail window lost its place: {listed}"
    );
}

/// The newest run, served alone, held in four more copies filed under keys
/// that sort before `upload` in the read's order, each with a place of its
/// own, stored before the upload copy. A list of one reads four rows a page,
/// so the upload copy's row is the fifth the read takes, past a full first
/// page, and only it is the listed copy.
#[tokio::test]
async fn the_listed_copy_gives_the_place_even_past_a_full_page_of_copies() {
    let f = fixture().await;
    let copy = run("athlete-run-twin", 0);
    let others = ["zz-copy-1", "zz-copy-2", "zz-copy-3", "zz-copy-4"];
    for other in others {
        f.repos()
            .activity_cache
            .upsert_activities(
                f.athlete.id,
                &f.athlete.tenant,
                other,
                slice::from_ref(&copy),
            )
            .await
            .unwrap();
    }
    f.repos()
        .activity_cache
        .upsert_activities(
            f.athlete.id,
            &f.athlete.tenant,
            UPLOAD,
            slice::from_ref(&copy),
        )
        .await
        .unwrap();
    let rows: Vec<(&str, &str, &str)> = others
        .iter()
        .map(|other| (*other, "athlete-run-twin", "Elsewhere"))
        .collect();
    city_places(&f, f.athlete, &rows).await;
    let listed_place = athlete_place(&f, f.athlete, UPLOAD, "athlete-run-twin", ATHLETE_NAME).await;

    let listed = ok(
        f.call(
            &f.own_executor(),
            f.athlete,
            "get_activities",
            json!({ "limit": 1 }),
        )
        .await,
        "get_activities, the newest run",
    );

    assert_eq!(
        row(&listed, "athlete-run-twin")["place"],
        json!({ "id": listed_place, "name": ATHLETE_NAME }),
        "bug: the served row took another copy's place, or the read stopped at a full page: \
         {listed}"
    );
}
