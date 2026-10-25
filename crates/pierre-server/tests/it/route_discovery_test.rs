// ABOUTME: Offline tests for RouteDiscoveryService ranking, Overpass queries, geocode input checks and compose_route
// ABOUTME: Live Overpass and Open-Meteo geocoding tests live in tests/live/route_discovery_live_test.rs (live-e2e)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Route discovery tests that run without the network: Overpass query
//! construction, ranking over captured fixtures, input rejection that
//! happens before any HTTP call, and `compose_route` joining stored stretches
//! through the tool itself. The tests that hit the live Overpass API and
//! Open-Meteo's gazetteer are in `tests/live/route_discovery_live_test.rs`, built only
//! with `--features live-e2e`.

use crate::common;

use std::sync::Arc;

use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_chat_pipeline::stages::viz_blocks::extract_viz_blocks;
use pierre_chat_pipeline::stages::viz_route::{read_route_tracks, RouteTracks};
use pierre_chat_pipeline::{InputSource, QuotaState, TurnInput, TurnOrigin};
use pierre_core::models::{ConversationTurnId, SportType, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_fitness_compute::geocoding::free_text_query;
use pierre_fitness_compute::route_compose;
use pierre_fitness_compute::{
    build_overpass_query, routes_from_overpass_json, DiscoveredRoute, DistanceSource,
    RouteDiscoveryService, TargetFit, TargetUse,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::implementations::compose_route::ComposeRouteTool;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::route_store::{self, StoredRoute};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

/// Prévost, Québec — reference point for the dispatch tests. The place
/// geocoder resolves it to roughly 45.87, -74.08.
const PREVOST_QC_LAT: f64 = 45.87;
const PREVOST_QC_LON: f64 = -74.08;

#[tokio::test]
async fn test_unsupported_sport_returns_empty() {
    // Swim is not a land route — discover_routes_for_sport should return
    // an empty vec without hitting Overpass — a pure logic test.
    let service = RouteDiscoveryService::with_defaults();
    let routes = service
        .discover_routes_for_sport(&SportType::Swim, PREVOST_QC_LAT, PREVOST_QC_LON, None, None)
        .await
        .expect("swim dispatch should succeed without hitting Overpass");
    assert!(
        routes.is_empty(),
        "expected empty result for unsupported sport, got {} routes",
        routes.len()
    );
}

#[tokio::test]
async fn test_forward_geocode_empty_query_rejected() {
    // No live test needed — a blank place is no query, so nothing is sent.
    assert!(free_text_query("   ", None).is_none());
    assert!(free_text_query(" , ", None).is_none());
}

// ============================================================================
// Offline regression tests — these run in CI on every push.
//
// The Shawinigan fixture is a real capture: the twenty unnamed sidewalks the
// shipped query actually returned for an athlete's address on 2026-08-26,
// merged with the named trails a name-filtered query finds around the same
// point. The agent could not name a single trail from the first set, which is
// the failure these tests exist to keep out.
// ============================================================================

/// The athlete's address in the reported failure — 1753 90e Rue, Shawinigan.
const SHAWINIGAN_LAT: f64 = 46.586_422;
const SHAWINIGAN_LON: f64 = -72.706_66;

const SHAWINIGAN_FIXTURE: &str = include_str!("../fixtures/overpass/shawinigan-running.json");

#[test]
fn test_ranking_drops_unnamed_ways_and_surfaces_real_trails() {
    let routes = routes_from_overpass_json(
        SHAWINIGAN_FIXTURE,
        &SportType::Run,
        SHAWINIGAN_LAT,
        SHAWINIGAN_LON,
        None,
    )
    .expect("fixture is a valid Overpass payload");

    assert!(
        !routes.is_empty(),
        "fixture contains named trails but ranking returned nothing"
    );
    for route in &routes {
        assert!(
            !route.name.starts_with("Unnamed"),
            "unnamed placeholder survived ranking: {}",
            route.name
        );
    }

    let names: Vec<&str> = routes.iter().map(|r| r.name.as_str()).collect();
    assert!(
        names.contains(&"Sentier Thibaudeau-Ricard"),
        "the nearest real named trail is missing from {names:?}"
    );
    assert!(
        names.contains(&"Sentier de la Tourbière de Saint-Narcisse"),
        "the trail the athlete was told about is missing from {names:?}"
    );
}

#[test]
fn test_ranking_orders_trails_ahead_of_paved_connectors() {
    let routes = routes_from_overpass_json(
        SHAWINIGAN_FIXTURE,
        &SportType::Run,
        SHAWINIGAN_LAT,
        SHAWINIGAN_LON,
        None,
    )
    .expect("fixture is a valid Overpass payload");

    let position = |name: &str| {
        routes
            .iter()
            .position(|r| r.name == name)
            .unwrap_or_else(|| panic!("{name} missing from {routes:#?}"))
    };

    // "Pont Marc-Trudel" is a named footway bridge 7.3 km out; the singletrack
    // at Vallée du Parc is 9.9 km out but is what a runner asked for.
    assert!(
        position("Petit Castor") < position("Pont Marc-Trudel"),
        "a paved connector outranked a trail: {:?}",
        routes.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    // The fixture carries a signed `route=hiking` relation for the tourbière
    // alongside the seven ways it is split into. A curated itinerary is the
    // best answer there is, so it leads even at 8.4 km.
    assert_eq!(
        routes[0].name,
        "Sentier de la Tourbière de Saint-Narcisse",
        "a signed itinerary relation should lead: {:?}",
        routes.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    // ...and inside the trail class, nearest first. Together these three
    // assertions pin both sort keys; drop either and the list falls back to
    // whatever order Overpass happened to emit.
    assert!(
        position("26e Rue") < position("Sentier Thibaudeau-Ricard")
            && position("Sentier Thibaudeau-Ricard") < position("Petit Castor"),
        "trails are not ordered by distance: {:?}",
        routes
            .iter()
            .map(|r| (&r.name, r.distance_from_center_meters.round()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_ranking_deduplicates_split_trail_segments() {
    let routes = routes_from_overpass_json(
        SHAWINIGAN_FIXTURE,
        &SportType::Run,
        SHAWINIGAN_LAT,
        SHAWINIGAN_LON,
        None,
    )
    .expect("fixture is a valid Overpass payload");

    // OSM carries both "Sentier de la Tourbière de Saint-Narcisse" and a
    // lowercase-t duplicate for the same trail.
    let matches = routes
        .iter()
        .filter(|r| r.name.to_lowercase().contains("tourbière"))
        .count();
    assert_eq!(
        matches,
        1,
        "split segments of one trail were listed separately: {:?}",
        routes.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
}

#[test]
fn test_ranking_measures_distance_from_the_search_center() {
    let routes = routes_from_overpass_json(
        SHAWINIGAN_FIXTURE,
        &SportType::Run,
        SHAWINIGAN_LAT,
        SHAWINIGAN_LON,
        None,
    )
    .expect("fixture is a valid Overpass payload");

    let tourbiere = routes
        .iter()
        .find(|r| r.name.to_lowercase().contains("tourbière"))
        .expect("tourbière trail should be in the results");

    // Measured at 8.1 km from the athlete's address; allow a 500 m band so a
    // fixture refresh that shifts the way's center doesn't red the suite.
    assert!(
        (7_600.0..=8_600.0).contains(&tourbiere.distance_from_center_meters),
        "expected ~8.1 km from the search center, got {} m",
        tourbiere.distance_from_center_meters
    );
}

#[test]
fn test_running_query_requires_names_and_skips_sidewalks() {
    let query = build_overpass_query(&SportType::Run, SHAWINIGAN_LAT, SHAWINIGAN_LON, 10_000)
        .expect("run is a supported sport");

    for clause in query
        .lines()
        .filter(|l| l.trim_start().starts_with(&['w', 'r'][..]))
    {
        assert!(
            clause.contains(r#"["name"]"#),
            "clause admits unnamed ways, which crowd out real trails: {clause}"
        );
    }
    assert!(
        query.contains(r#"["footway"!~"^(sidewalk|crossing)$"]"#),
        "sidewalks are not routes and must be excluded: {query}"
    );
    assert!(
        query.contains("path|track|bridleway"),
        "trails outside foot=designated must be reachable: {query}"
    );
}

#[test]
fn test_queries_fetch_more_elements_than_they_return() {
    // Overpass truncates `out <n>` in element-id order, so a budget the size
    // of the result set hands back whichever ways carry the lowest ids. The
    // fetch budget must exceed the 20 routes the tool returns.
    for sport in [
        SportType::Run,
        SportType::Ride,
        SportType::Hike,
        SportType::CrossCountrySkiing,
    ] {
        let query = build_overpass_query(&sport, SHAWINIGAN_LAT, SHAWINIGAN_LON, 10_000)
            .unwrap_or_else(|| panic!("{sport:?} should be a supported sport"));
        let budget: usize = query
            .rsplit_once("out tags geom ")
            .and_then(|(_, tail)| {
                tail.trim_end_matches(";\n")
                    .trim_end_matches(';')
                    .parse()
                    .ok()
            })
            .unwrap_or_else(|| panic!("{sport:?} query has no parseable out budget: {query}"));
        assert!(
            budget > 20,
            "{sport:?} fetches only {budget} elements for a 20-route result"
        );
    }
}

#[test]
fn test_cycling_query_covers_gravel_and_singletrack() {
    let query = build_overpass_query(
        &SportType::GravelRide,
        SHAWINIGAN_LAT,
        SHAWINIGAN_LON,
        10_000,
    )
    .expect("gravel_ride is a supported sport");

    assert!(
        query.contains("cycleway|track|path"),
        "gravel is highway=track and singletrack is highway=path: {query}"
    );
    assert!(
        query.contains(r#"["bicycle"!~"^(no|dismount)$"]"#),
        "ways closed to bikes must be filtered out of a ride: {query}"
    );
    assert!(
        query.contains(r#"relation["route"~"^(bicycle|mtb)$"]"#),
        "signed cycling itineraries are route relations: {query}"
    );
}

/// Overpass re-evaluates the spatial filter once per clause and that dominates
/// the cost: measured around Prevost, five clauses took 24.9s against a 25s
/// server timeout under a 30s client timeout, where three took 2-9s. A query
/// that grows a fourth clause starts timing out in production, which reaches
/// the athlete as "no trails near you".
#[test]
fn test_queries_stay_within_the_spatial_clause_budget() {
    const MAX_AROUND_CLAUSES: usize = 3;

    for sport in [
        SportType::Run,
        SportType::TrailRunning,
        SportType::Ride,
        SportType::GravelRide,
        SportType::MountainBike,
        SportType::Hike,
        SportType::Walk,
        SportType::CrossCountrySkiing,
        SportType::AlpineSkiing,
        SportType::Snowshoe,
    ] {
        let query = build_overpass_query(&sport, SHAWINIGAN_LAT, SHAWINIGAN_LON, 10_000)
            .unwrap_or_else(|| panic!("{sport:?} should be a supported sport"));
        let clauses = query.matches("(around:").count();
        assert!(
            clauses <= MAX_AROUND_CLAUSES,
            "{sport:?} query has {clauses} spatial clauses (max {MAX_AROUND_CLAUSES}): {query}"
        );
    }
}

#[test]
fn test_hiking_query_does_not_require_alpine_difficulty_tag() {
    let query = build_overpass_query(&SportType::Hike, SHAWINIGAN_LAT, SHAWINIGAN_LON, 10_000)
        .expect("hike is a supported sport");

    // sac_scale is an alpine tag that eastern North American mapping does not
    // set; requiring it returned nothing across whole regions.
    assert!(
        !query.contains("sac_scale"),
        "hiking query still gates on sac_scale: {query}"
    );
}

#[test]
fn test_unsupported_sport_has_no_query() {
    assert!(
        build_overpass_query(&SportType::Swim, SHAWINIGAN_LAT, SHAWINIGAN_LON, 10_000).is_none(),
        "swim has no land route surface and must not build a query"
    );
}

#[test]
fn test_malformed_overpass_body_is_an_error_not_an_empty_list() {
    // A free mirror answering 200 with an HTML error page must fail loudly so
    // the service falls through to the next mirror instead of telling the
    // athlete there are no trails nearby.
    let err = routes_from_overpass_json(
        "<html><body>Internal Server Error</body></html>",
        &SportType::Run,
        SHAWINIGAN_LAT,
        SHAWINIGAN_LON,
        None,
    )
    .expect_err("an HTML body is not a valid Overpass response");
    assert!(
        err.to_string().to_lowercase().contains("json"),
        "error should name the parse failure, got: {err}"
    );
}

#[test]
fn test_ski_query_reads_piste_data() {
    let query = build_overpass_query(
        &SportType::CrossCountrySkiing,
        SHAWINIGAN_LAT,
        SHAWINIGAN_LON,
        20_000,
    )
    .expect("cross_country_skiing is a supported sport");

    assert!(
        query.contains(r#"["piste:type"~"^(downhill|nordic|skitour)$"]"#),
        "ski discovery must read OSM piste data: {query}"
    );
}

// ============================================================================
// Lengths and target distance — offline.
//
// `measured-lengths.json` is hand-built in the shape `out tags geom` answers
// in: a way carries `bounds` and `geometry`, a relation `bounds` and nothing
// else. Every line in it runs due north along one meridian, so its length is
// its span in degrees of latitude times the metres in one degree:
// 6,371,000 m × π / 180 = 111,194.93 m.
//
// `prevost-cycling.json` is a real capture: the ten route relations the
// cycling query returned around Prévost on 2026-10-01, and the fetched ways
// named "Le P'tit Train du Nord" and "Loup-Garou", untouched.
// ============================================================================

const MEASURED_FIXTURE: &str = include_str!("../fixtures/overpass/measured-lengths.json");
const MEASURED_LAT: f64 = 46.0;
const MEASURED_LON: f64 = -72.0;
const METERS_PER_DEGREE_OF_LATITUDE: f64 = 111_194.93;

const PREVOST_CYCLING_FIXTURE: &str = include_str!("../fixtures/overpass/prevost-cycling.json");

fn measured_routes(target: Option<f64>) -> Vec<DiscoveredRoute> {
    routes_from_overpass_json(
        MEASURED_FIXTURE,
        &SportType::Run,
        MEASURED_LAT,
        MEASURED_LON,
        target,
    )
    .expect("fixture is a valid Overpass payload")
}

fn named<'a>(routes: &'a [DiscoveredRoute], name: &str) -> &'a DiscoveredRoute {
    routes
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} missing from {routes:#?}"))
}

fn assert_meters(actual: Option<f64>, expected: f64, what: &str) {
    let actual = actual.unwrap_or_else(|| panic!("{what} has no length"));
    assert!(
        (actual - expected).abs() < 1.0,
        "{what}: expected {expected:.1} m, measured {actual:.1} m"
    );
}

#[test]
fn test_a_way_is_measured_along_its_geometry() {
    let routes = measured_routes(None);

    // One way, 0.005° of latitude.
    let short = named(&routes, "Boucle Courte");
    assert_meters(
        short.distance_meters,
        0.005 * METERS_PER_DEGREE_OF_LATITUDE,
        "Boucle Courte",
    );
    assert_eq!(short.distance_source, Some(DistanceSource::MappedGeometry));

    // Two ways sharing a name and an end point, 0.01° each: one trail, and
    // its length is the chain, not the nearest segment alone.
    assert_meters(
        named(&routes, "Sentier A").distance_meters,
        0.02 * METERS_PER_DEGREE_OF_LATITUDE,
        "Sentier A",
    );

    // No geometry, but a `distance` tag: OSM writes it in kilometres.
    let tagged = named(&routes, "Sans Géométrie");
    assert_eq!(tagged.distance_meters, Some(12_000.0));
    assert_eq!(tagged.distance_source, Some(DistanceSource::OsmTag));

    // Neither: the length is unknown, never zero.
    let unknown = named(&routes, "Sans Longueur");
    assert_eq!(unknown.distance_meters, None);
    assert_eq!(unknown.distance_source, None);
}

#[test]
fn test_same_named_ways_that_do_not_connect_are_not_summed() {
    let routes = measured_routes(None);

    // Two "Chemin du Lac", 0.01° and 0.03°, four kilometres apart: the
    // longest of them, not 0.04° of a trail that does not exist.
    assert_meters(
        named(&routes, "Chemin du Lac").distance_meters,
        0.03 * METERS_PER_DEGREE_OF_LATITUDE,
        "Chemin du Lac",
    );

    // Two carriageways of one avenue, side by side and never meeting: 0.01°
    // once, not twice.
    assert_meters(
        named(&routes, "Avenue Parallèle").distance_meters,
        0.01 * METERS_PER_DEGREE_OF_LATITUDE,
        "Avenue Parallèle",
    );
}

#[test]
fn test_a_relation_is_its_declared_length_or_unknown_never_its_ways() {
    let routes = measured_routes(None);

    // "Voie Verte" is a relation tagged 234 km whose two fetched ways measure
    // 2.2 km: the route is 234 km, and the fragment inside the search is not
    // its length.
    let tagged = named(&routes, "Voie Verte");
    assert_eq!(tagged.distance_meters, Some(234_000.0));
    assert_eq!(tagged.distance_source, Some(DistanceSource::OsmTag));

    // A relation with no `distance` tag has no length to report.
    let untagged = named(&routes, "Grand Tour");
    assert_eq!(untagged.distance_meters, None);
    assert_eq!(untagged.distance_source, None);
}

#[test]
fn test_a_real_capture_measures_relations_and_chains_truthfully() {
    let routes = routes_from_overpass_json(
        PREVOST_CYCLING_FIXTURE,
        &SportType::Ride,
        PREVOST_QC_LAT,
        PREVOST_QC_LON,
        None,
    )
    .expect("capture is a valid Overpass payload");

    // The relation, as Overpass returns it: bounds and tags, with the rail
    // trail's 234 km declared.
    let rail_trail = named(&routes, "Le P’tit Train du Nord");
    assert_eq!(rail_trail.distance_meters, Some(234_000.0));
    assert_eq!(rail_trail.distance_source, Some(DistanceSource::OsmTag));

    // The eight fetched ways of the same trail (mapped with a straight
    // apostrophe, so a separate name) are four separate stretches adding up
    // to 1,272 m; the longest connected one is 1,138 m.
    let ways = named(&routes, "Le P'tit Train du Nord");
    assert_meters(ways.distance_meters, 1_138.2, "Le P'tit Train du Nord ways");
    assert_eq!(ways.distance_source, Some(DistanceSource::MappedGeometry));

    // "Loup-Garou" is a relation without a `distance` tag whose two ways
    // measure 703 m: the relation's length is unknown, not 703 m.
    let loup_garou = named(&routes, "Loup-Garou");
    assert_eq!(loup_garou.distance_meters, None);
    assert_eq!(loup_garou.distance_source, None);
}

#[test]
fn test_an_element_without_a_center_is_placed_at_the_middle_of_its_bounds() {
    let routes = measured_routes(None);
    let long = named(&routes, "Piste Longue");
    // The way runs 46.06 → 46.15: its box is centred on 46.105.
    assert!((long.latitude - 46.105).abs() < 1e-6, "{}", long.latitude);
    assert!((long.longitude - MEASURED_LON).abs() < 1e-6);
    assert_meters(
        Some(long.distance_from_center_meters),
        0.105 * METERS_PER_DEGREE_OF_LATITUDE,
        "Piste Longue from the search center",
    );
}

#[test]
fn test_without_a_target_the_order_is_class_then_distance_and_nothing_is_marked() {
    let routes = measured_routes(None);
    let names: Vec<&str> = routes.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Grand Tour",
            "Voie Verte",
            "Boucle Courte",
            "Sentier A",
            "Sans Géométrie",
            "Piste Longue",
            "Sans Longueur",
            "Chemin du Lac",
            "Avenue Parallèle",
        ]
    );
    assert!(routes.iter().all(|r| r.target_use.is_none()));
}

#[test]
fn test_a_target_distance_ranks_by_closeness_of_the_usable_distance() {
    let routes = measured_routes(Some(10_000.0));
    let ranked: Vec<(&str, Option<TargetUse>)> = routes
        .iter()
        .map(|r| (r.name.as_str(), r.target_use))
        .collect();

    let single = Some(TargetUse {
        fit: TargetFit::SinglePass,
        passes: 1,
    });
    let repeats = |passes| {
        Some(TargetUse {
            fit: TargetFit::Repeats,
            passes,
        })
    };
    assert_eq!(
        ranked,
        [
            // 10,007.5 m — the one near match.
            ("Piste Longue", single),
            // 12,000 m: a fifth over.
            ("Sans Géométrie", single),
            // 234 km: it holds the session, and is the furthest from it.
            ("Voie Verte", single),
            // 3,335.8 m three times is 10,007 m.
            ("Chemin du Lac", repeats(3)),
            // 2,223.9 m five times.
            ("Sentier A", repeats(5)),
            // 1,111.9 m nine times.
            ("Avenue Parallèle", repeats(9)),
            // 556.0 m seventeen times.
            ("Boucle Courte", repeats(17)),
            // Unknown lengths say nothing about the target, so they trail.
            ("Grand Tour", None),
            ("Sans Longueur", None),
        ]
    );
}

#[test]
fn test_an_out_and_back_that_lands_on_the_target_leads_a_route_twice_its_length() {
    let routes = measured_routes(Some(4_500.0));
    // 2,223.9 m out and back is 4,447.8 m — within a tenth of 4.5 km.
    let sentier = named(&routes, "Sentier A");
    assert_eq!(
        sentier.target_use,
        Some(TargetUse {
            fit: TargetFit::OutAndBack,
            passes: 2,
        })
    );
    assert_eq!(routes[0].name, "Sentier A");
    // Piste Longue holds 4.5 km in one pass but is 10 km long: not a match.
    let position = |name: &str| routes.iter().position(|r| r.name == name);
    assert!(position("Sentier A") < position("Piste Longue"));
    assert!(position("Piste Longue") < position("Voie Verte"));
}

/// A way running due north from `south`, `meters` long, as Overpass writes it.
fn northward_way(id: usize, name: &str, south: f64, meters: f64) -> String {
    let north = south + meters / METERS_PER_DEGREE_OF_LATITUDE;
    format!(
        r#"{{"type":"way","id":{id},"bounds":{{"minlat":{south},"minlon":-72.0,"maxlat":{north},"maxlon":-72.0}},"geometry":[{{"lat":{south},"lon":-72.0}},{{"lat":{north},"lon":-72.0}}],"tags":{{"highway":"path","name":"{name}"}}}}"#
    )
}

#[test]
fn test_a_near_match_outranks_a_far_longer_route_and_survives_the_cut() {
    // Twenty-five trails of 22 km and more and a 234 km rail trail, all
    // nearer the centre than one 9.5 km loop: without a target the loop is
    // the twenty-seventh of twenty returned.
    let mut elements: Vec<String> = (0..25_u8)
        .map(|i| {
            let index = usize::from(i);
            northward_way(
                100 + index,
                &format!("Long {index}"),
                f64::from(i).mul_add(0.001, 46.0),
                f64::from(i).mul_add(500.0, 22_000.0),
            )
        })
        .collect();
    elements.push(northward_way(200, "Boucle 9.5", 46.5, 9_500.0));
    elements.push(
        r#"{"type":"relation","id":300,"bounds":{"minlat":45.9,"minlon":-72.0,"maxlat":46.1,"maxlon":-72.0},"tags":{"type":"route","route":"hiking","name":"Rail Trail","distance":"234 km"}}"#
            .to_owned(),
    );
    let body = format!(r#"{{"version":0.6,"elements":[{}]}}"#, elements.join(","));
    let rank = |target| {
        routes_from_overpass_json(&body, &SportType::Run, MEASURED_LAT, MEASURED_LON, target)
            .expect("built payload is valid")
    };

    let unranked = rank(None);
    assert_eq!(unranked.len(), 20);
    assert_eq!(unranked[0].name, "Rail Trail");
    assert!(unranked.iter().all(|r| r.name != "Boucle 9.5"));

    let ranked = rank(Some(10_000.0));
    assert_eq!(ranked.len(), 20);
    // Half a kilometre short of 10 km is a match, in one pass.
    assert_eq!(ranked[0].name, "Boucle 9.5");
    assert_meters(ranked[0].distance_meters, 9_500.0, "Boucle 9.5");
    assert_eq!(
        ranked[0].target_use,
        Some(TargetUse {
            fit: TargetFit::SinglePass,
            passes: 1,
        })
    );
    // Then the trails nearest the target in length; the 234 km one is the
    // furthest from it of all and is the one the cut drops.
    assert_eq!(ranked[1].name, "Long 0");
    assert!(ranked.iter().all(|r| r.name != "Rail Trail"));
}

#[test]
fn test_queries_ask_for_geometry_without_adding_a_clause() {
    let query = build_overpass_query(&SportType::Run, MEASURED_LAT, MEASURED_LON, 10_000)
        .expect("run is a supported sport");
    assert!(query.contains("out tags geom "), "{query}");
    assert_eq!(query.matches("(around:").count(), 3, "{query}");
}

// ============================================================================
// compose_route — stretches joined through the tool, against the store the
// test server builds (in memory), exactly as discover_routes leaves them.
// ============================================================================

/// One athlete's view of the route store and the tool.
struct Athlete {
    runtime: Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    ctx: ToolContext,
}

impl Athlete {
    async fn new() -> Self {
        let runtime: Arc<dyn ToolRuntime> = common::create_test_server_resources()
            .await
            .expect("test server resources");
        Self::on(runtime)
    }

    /// Another athlete on the same server, in a tenant of their own.
    fn on(runtime: Arc<dyn ToolRuntime>) -> Self {
        let (tenant, user_id) = (Uuid::new_v4(), Uuid::new_v4());
        Self {
            runtime,
            tenant_id: TenantId::from_uuid(tenant),
            user_id,
            ctx: ToolContext::new()
                .with_user(user_id.to_string())
                .with_tenant(tenant.to_string())
                .with_auth_method("jwt_bearer"),
        }
    }

    /// Store a stretch the way `discover_routes` does, returning its ref.
    async fn store(&self, name: &str, coordinates: Vec<(f64, f64)>) -> String {
        route_store::store_route(
            self.runtime.cache(),
            self.tenant_id,
            self.user_id,
            &StoredRoute {
                name: name.to_owned(),
                coordinates,
                out_and_back: false,
            },
        )
        .await
        .expect("the stretch is stored")
    }

    async fn load(&self, route_ref: &str) -> Option<StoredRoute> {
        route_store::load_route(
            self.runtime.cache(),
            self.tenant_id,
            self.user_id,
            route_ref,
        )
        .await
        .expect("the store reads")
    }

    /// Call `compose_route`; the payload on success, the error text otherwise.
    async fn compose(&self, args: Value) -> Result<Value, String> {
        let response = ComposeRouteTool
            .execute(&self.runtime, &self.ctx, args)
            .await;
        let payload = response
            .structured_content
            .expect("compose_route answers with structured content");
        if response.is_error {
            Err(payload["error"]
                .as_str()
                .expect("an error carries its text")
                .to_owned())
        } else {
            Ok(payload)
        }
    }
}

/// A line due north along `longitude` from `south`, `meters` long, a node
/// every 100 m.
fn north_line(longitude: f64, south: f64, meters: f64) -> Vec<(f64, f64)> {
    let nodes = (meters / 100.0).round() as usize;
    (0..=nodes)
        .map(|index| {
            (
                south + index as f64 * 100.0 / METERS_PER_DEGREE_OF_LATITUDE,
                longitude,
            )
        })
        .collect()
}

fn assert_route_ref(value: &Value) -> &str {
    let route_ref = value.as_str().expect("a route_ref is a string");
    assert!(
        route_ref.len() == 19
            && route_ref.starts_with("rt_")
            && route_ref[3..]
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "not a route_ref the viz schema accepts: {route_ref}"
    );
    route_ref
}

/// Two stretches that meet end to end become one line, measured along it,
/// stored under the ref the answer carries and named by both stretches.
#[tokio::test]
async fn test_compose_route_joins_two_stretches_that_meet() {
    let athlete = Athlete::new().await;
    let riverside = athlete
        .store("Sentier de la Rivière", north_line(-72.0, 46.0, 1_000.0))
        .await;
    let rail_trail = athlete
        .store(
            "Ancienne voie ferrée",
            north_line(-72.0, 46.0 + 1_000.0 / METERS_PER_DEGREE_OF_LATITUDE, 500.0),
        )
        .await;

    let answer = athlete
        .compose(json!({ "route_refs": [riverside, rail_trail] }))
        .await
        .expect("stretches that meet compose");

    assert_eq!(answer["distance_meters"], 1_500.0, "{answer:#}");
    assert_eq!(answer["out_and_back"], false);
    assert_eq!(answer["joins_meters"], json!([0.0]));
    assert_eq!(
        answer["stretches"],
        json!([
            { "route_ref": riverside, "name": "Sentier de la Rivière" },
            { "route_ref": rail_trail, "name": "Ancienne voie ferrée" },
        ])
    );
    let composed = assert_route_ref(&answer["route_ref"]);
    assert!(composed != riverside && composed != rail_trail);

    let stored = athlete
        .load(composed)
        .await
        .expect("the composed line is stored under the ref it answers with");
    assert_eq!(stored.name, "Sentier de la Rivière → Ancienne voie ferrée");
    assert!(!stored.out_and_back);
    assert_eq!(stored.coordinates.first(), Some(&(46.0, -72.0)));
}

/// Out and back returns along the line: twice the distance, and a route of
/// its own, stored apart from the one-way line.
#[tokio::test]
async fn test_compose_route_out_and_back_doubles_the_distance() {
    let athlete = Athlete::new().await;
    let riverside = athlete
        .store("Sentier de la Rivière", north_line(-72.0, 46.0, 1_000.0))
        .await;
    let rail_trail = athlete
        .store(
            "Ancienne voie ferrée",
            north_line(-72.0, 46.0 + 1_000.0 / METERS_PER_DEGREE_OF_LATITUDE, 500.0),
        )
        .await;
    let refs = json!([riverside, rail_trail]);

    let one_way = athlete
        .compose(json!({ "route_refs": refs }))
        .await
        .expect("one way composes");
    let both_ways = athlete
        .compose(json!({ "route_refs": refs, "out_and_back": true }))
        .await
        .expect("out and back composes");

    assert_eq!(both_ways["distance_meters"], 3_000.0, "{both_ways:#}");
    assert_eq!(both_ways["out_and_back"], true);
    assert_ne!(both_ways["route_ref"], one_way["route_ref"]);
    let stored = athlete
        .load(assert_route_ref(&both_ways["route_ref"]))
        .await
        .expect("the out-and-back is stored");
    assert!(stored.out_and_back);
    assert_eq!(stored.coordinates.first(), stored.coordinates.last());
}

/// A ref the athlete's store does not hold is refused by name, worded so
/// the model searches again — and a ref is no capability: another athlete
/// on the same server cannot compose with it.
#[tokio::test]
async fn test_compose_route_refuses_an_unknown_ref_by_name() {
    let athlete = Athlete::new().await;
    let riverside = athlete
        .store("Sentier de la Rivière", north_line(-72.0, 46.0, 1_000.0))
        .await;

    let error = athlete
        .compose(json!({ "route_refs": [riverside, "rt_0000000000000000"] }))
        .await
        .expect_err("a ref nobody stored is refused");
    assert!(
        error.contains("unknown or expired route_ref \"rt_0000000000000000\"")
            && error.contains("call discover_routes again"),
        "{error}"
    );

    let stranger = Athlete::on(Arc::clone(&athlete.runtime));
    let error = stranger
        .compose(json!({ "route_refs": [riverside] }))
        .await
        .expect_err("another athlete's ref reads nothing");
    assert!(
        error.contains(&format!("unknown or expired route_ref \"{riverside}\"")),
        "{error}"
    );
}

/// Stretches that do not meet are refused with the sentence the composer
/// gives: both names, the width of the gap and the join tolerance.
#[tokio::test]
async fn test_compose_route_refuses_a_gap_naming_both_stretches() {
    let athlete = Athlete::new().await;
    let riverside = athlete
        .store("Sentier de la Rivière", north_line(-72.0, 46.0, 1_000.0))
        .await;
    let canal = athlete
        .store(
            "Piste du canal",
            north_line(-72.0, 46.0 + 1_412.0 / METERS_PER_DEGREE_OF_LATITUDE, 500.0),
        )
        .await;

    let error = athlete
        .compose(json!({ "route_refs": [riverside, canal] }))
        .await
        .expect_err("a 412 m gap is not bridged");
    assert!(
        error.contains("\"Sentier de la Rivière\" and \"Piste du canal\" do not meet")
            && error.contains("412 m")
            && error.contains(&format!(
                "{:.0} m a join may bridge",
                route_compose::JOIN_TOLERANCE_METERS
            )),
        "{error}"
    );
}

/// The argument is checked before the store is read: an absent list, an
/// empty one, one past the bound, and a flag that is no boolean.
#[tokio::test]
async fn test_compose_route_refuses_arguments_it_cannot_compose() {
    let athlete = Athlete::new().await;
    let riverside = athlete
        .store("Sentier de la Rivière", north_line(-72.0, 46.0, 1_000.0))
        .await;
    let too_many = vec![riverside.clone(); route_compose::MAX_STRETCHES + 1];
    let at_most = format!("at most {} stretches", route_compose::MAX_STRETCHES);

    for (args, expected) in [
        (json!({}), "route_refs is required"),
        (
            json!({ "route_refs": riverside }),
            "route_refs must be an array",
        ),
        (json!({ "route_refs": [] }), "no stretches were given"),
        (json!({ "route_refs": too_many }), at_most.as_str()),
        (json!({ "route_refs": [42] }), "is a route_ref string"),
        (
            json!({ "route_refs": [riverside], "out_and_back": "yes" }),
            "out_and_back must be true or false",
        ),
    ] {
        let error = athlete
            .compose(args.clone())
            .await
            .expect_err("the argument is refused");
        assert!(error.contains(expected), "{args}: {error}");
    }
}

/// Over a real capture: a discovered stretch, stored as discovery stores it,
/// composes on its own into exactly its own line — the same reference, the
/// same measured distance — and out and back into twice that distance.
#[tokio::test]
async fn test_compose_route_over_a_real_capture_keeps_the_discovered_line() {
    let routes = routes_from_overpass_json(
        PREVOST_CYCLING_FIXTURE,
        &SportType::Ride,
        PREVOST_QC_LAT,
        PREVOST_QC_LON,
        None,
    )
    .expect("capture is a valid Overpass payload");
    let (route, line) = routes
        .iter()
        .filter_map(|route| Some((route, route.geometry.clone()?)))
        .max_by(|(a, _), (b, _)| {
            let length = |route: &DiscoveredRoute| route.distance_meters.unwrap_or(0.0);
            length(a).total_cmp(&length(b))
        })
        .expect("the capture holds a mapped stretch");
    let measured = route.distance_meters.expect("a mapped stretch is measured");

    let athlete = Athlete::new().await;
    let route_ref = route_store::store_route(
        athlete.runtime.cache(),
        athlete.tenant_id,
        athlete.user_id,
        &StoredRoute {
            name: route.name.clone(),
            coordinates: line.clone(),
            out_and_back: false,
        },
    )
    .await
    .expect("the stretch is stored");
    assert_eq!(route_ref, route_compose::route_ref(&line, false));

    let alone = athlete
        .compose(json!({ "route_refs": [route_ref] }))
        .await
        .unwrap_or_else(|error| panic!("{}: {error}", route.name));
    assert_eq!(alone["route_ref"], route_ref, "one stretch is its own line");
    let distance = alone["distance_meters"].as_f64().expect("a distance");
    assert!(
        (distance - measured).abs() <= 1.0,
        "{}: composed {distance} m, discovered {measured} m",
        route.name
    );

    let both_ways = athlete
        .compose(json!({ "route_refs": [route_ref], "out_and_back": true }))
        .await
        .unwrap_or_else(|error| panic!("{}: {error}", route.name));
    let doubled = both_ways["distance_meters"].as_f64().expect("a distance");
    assert!(
        2.0f64.mul_add(-measured, doubled).abs() <= 1.0,
        "{}: out and back {doubled} m, discovered {measured} m",
        route.name
    );
}

/// A proposed map end to end, keyed by the ids a real chat turn carries.
///
/// The tool runs through the executor exactly as the tool loop dispatches a
/// model's call — `protocol: "chat"`, the turn's tool tenant, the athlete — so
/// it reads the stretches and stores the itinerary under the owner it derives
/// from that call. The reply naming the itinerary is then read and hydrated
/// from the turn's own `tool_tenant_id` and `user_id`, on a shared-room turn
/// whose conversation tenant is another one. A store and a read keyed apart
/// would refuse every proposed map as unknown, and only this path shows it.
#[tokio::test]
async fn test_a_composed_route_is_drawn_from_the_ids_a_chat_turn_carries() {
    let resources = common::create_test_server_resources()
        .await
        .expect("test server resources");
    let (user_id, user) = common::create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants")
        .first()
        .expect("a test user has a tenant")
        .id;
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);

    // The stretches discover_routes would have stored for this athlete.
    let riverside_line = north_line(-72.0, 46.0, 1_000.0);
    let rail_trail_line = north_line(-72.0, 46.0 + 1_000.0 / METERS_PER_DEGREE_OF_LATITUDE, 500.0);
    let mut refs = Vec::new();
    for (name, coordinates) in [
        ("Sentier de la Rivière", riverside_line.clone()),
        ("Ancienne voie ferrée", rail_trail_line.clone()),
    ] {
        refs.push(
            route_store::store_route(
                runtime.cache(),
                tenant,
                user_id,
                &StoredRoute {
                    name: name.to_owned(),
                    coordinates,
                    out_and_back: false,
                },
            )
            .await
            .expect("the stretch is stored"),
        );
    }

    let turn_id = ConversationTurnId::new();
    let executor = UniversalExecutor::new(Arc::clone(&runtime))
        .with_scopes(OAuthScope::self_grant())
        .with_turn_token(turn_id.0.to_string());
    let response = executor
        .execute_tool(UniversalRequest {
            tool_name: "compose_route".to_owned(),
            parameters: json!({ "route_refs": refs }),
            user_id: user_id.to_string(),
            protocol: "chat".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .expect("compose_route dispatches");
    assert!(
        response.success,
        "compose_route failed: {:?}",
        response.error
    );
    let answer = response.result.expect("compose_route answers");
    let composed = assert_route_ref(&answer["route_ref"]).to_owned();

    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        conversation_id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        // A shared room's conversation lives in the bot's tenant; the tools,
        // and so the route store, run in the athlete's.
        conversation_tenant_id: TenantId::generate(),
        tool_tenant_id: tenant,
        is_direct_message: false,
        content: "Propose-moi une sortie de 1,5 km".to_owned(),
        turn_id,
        ambient_context: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
        steps_after: None,
    };
    let block = json!({
        "type": "route",
        "source_tool": "compose_route",
        "route_ref": composed,
        "title": "Rivière puis voie ferrée",
    });
    let reply = format!("Voici l'itinéraire :\n\n```dravr-viz\n{block}\n```");
    let granted = vec!["route".to_owned()];
    let tools_called = vec!["compose_route".to_owned()];
    let ctx = resources.chat_pipeline_context();
    let mut tracks = RouteTracks::new();
    read_route_tracks(&ctx, &input, &granted, &tools_called, &reply, &mut tracks).await;
    let extraction = extract_viz_blocks(&ctx.viz_schemas, &granted, &tools_called, &tracks, &reply)
        .expect("the reply carries a fence");

    assert!(extraction.refusals.is_empty(), "{:?}", extraction.refusals);
    assert_eq!(extraction.blocks.len(), 1);
    let hydrated = &extraction.blocks[0];
    let drawn: Vec<(f64, f64)> =
        serde_json::from_value(hydrated["coordinates"].clone()).expect("coordinates");
    assert_eq!(
        drawn.first(),
        riverside_line.first(),
        "from the riverside's start"
    );
    assert_eq!(
        drawn.last(),
        rail_trail_line.last(),
        "to the rail trail's end"
    );
    let distances = hydrated["distances_meters"]
        .as_array()
        .expect("a public line is measured");
    let drawn_meters = distances
        .last()
        .and_then(Value::as_f64)
        .expect("a last distance");
    assert!(
        (drawn_meters - 1_500.0).abs() < 1.0,
        "the map measures the itinerary compose_route quoted: {drawn_meters} m"
    );
    assert_eq!(hydrated["route_ref"], json!(composed));
}
