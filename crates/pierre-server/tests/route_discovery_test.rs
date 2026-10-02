// ABOUTME: Integration tests for RouteDiscoveryService + discover_routes MCP tool
// ABOUTME: Live Overpass hits are gated behind DRAVR_LIVE_OVERPASS_TESTS to keep CI deterministic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Route discovery tests.
//!
//! The live tests make real requests to the public Overpass API. They are
//! skipped unless `DRAVR_LIVE_OVERPASS_TESTS=1` is set in the environment,
//! so CI stays deterministic and doesn't hammer a shared free service.
//! Run them locally with:
//!
//! ```bash
//! DRAVR_LIVE_OVERPASS_TESTS=1 cargo test --test route_discovery_test -- --nocapture
//! ```

use std::env;
use std::time::{Duration, Instant};

use pierre_core::models::SportType;
use pierre_fitness_compute::location::LocationService;
use pierre_fitness_compute::{
    build_overpass_query, routes_from_overpass_json, DiscoveredRoute, DistanceSource,
    RouteDiscoveryService, RouteSource, RouteType, TargetFit, TargetUse,
};

/// Prévost, Québec — reference point for route-discovery integration tests.
/// Nominatim resolves this to roughly 45.87, -74.08. Well-trafficked OSM area
/// with named trails.
const PREVOST_QC_LAT: f64 = 45.87;
const PREVOST_QC_LON: f64 = -74.08;

/// Minimum number of routes we expect a real Overpass query around a
/// well-mapped area to return. Below this, either OSM coverage collapsed
/// or we're parsing the response wrong.
const MIN_EXPECTED_REAL_ROUTES: usize = 1;

fn live_tests_enabled() -> bool {
    env::var("DRAVR_LIVE_OVERPASS_TESTS").ok().as_deref() == Some("1")
}

#[tokio::test]
async fn test_discover_running_routes_around_prevost() {
    if !live_tests_enabled() {
        eprintln!("skipping live Overpass test (set DRAVR_LIVE_OVERPASS_TESTS=1 to enable)");
        return;
    }

    let service = RouteDiscoveryService::with_defaults();
    let routes = service
        .discover_routes_for_sport(
            &SportType::Run,
            PREVOST_QC_LAT,
            PREVOST_QC_LON,
            Some(10_000),
            None,
        )
        .await
        .expect("Overpass query should succeed");

    assert!(
        routes.len() >= MIN_EXPECTED_REAL_ROUTES,
        "expected at least {MIN_EXPECTED_REAL_ROUTES} running route(s) near Prevost, got {}",
        routes.len()
    );

    for route in &routes {
        assert_eq!(route.source, RouteSource::Overpass);
        assert!(
            (-90.0..=90.0).contains(&route.latitude),
            "invalid latitude: {}",
            route.latitude
        );
        assert!(
            (-180.0..=180.0).contains(&route.longitude),
            "invalid longitude: {}",
            route.longitude
        );
        // A route the agent cannot name is a route it cannot recommend.
        assert!(
            !route.name.trim().is_empty() && !route.name.starts_with("Unnamed"),
            "unnamed placeholder leaked into results: {}",
            route.name
        );
        assert!(
            route.distance_from_center_meters <= 10_000.0 * 1.5,
            "route {} reported {} m from a 10 km search center",
            route.name,
            route.distance_from_center_meters
        );
    }
}

#[tokio::test]
async fn test_discover_routes_for_sport_dispatches_by_type() {
    if !live_tests_enabled() {
        eprintln!("skipping live Overpass test (set DRAVR_LIVE_OVERPASS_TESTS=1 to enable)");
        return;
    }

    let service = RouteDiscoveryService::with_defaults();

    // SportType::Run should dispatch to the running route query
    let run_routes = service
        .discover_routes_for_sport(
            &SportType::Run,
            PREVOST_QC_LAT,
            PREVOST_QC_LON,
            Some(5_000),
            None,
        )
        .await
        .expect("run dispatch should succeed");
    for route in &run_routes {
        assert!(
            matches!(route.route_type, RouteType::Running | RouteType::MultiUse),
            "run dispatch returned {:?}",
            route.route_type
        );
    }

    // SportType::CrossCountrySkiing should dispatch to the ski query and
    // yield piste-tagged results labelled as either XC or downhill ski
    let ski_routes = service
        .discover_routes_for_sport(
            &SportType::CrossCountrySkiing,
            PREVOST_QC_LAT,
            PREVOST_QC_LON,
            Some(20_000),
            None,
        )
        .await
        .expect("xc ski dispatch should succeed");
    for route in &ski_routes {
        assert!(
            matches!(
                route.route_type,
                RouteType::CrossCountrySki | RouteType::DownhillSki
            ),
            "ski query returned non-ski route_type: {:?}",
            route.route_type
        );
        assert_eq!(route.source, RouteSource::OpenSkiMap);
    }
}

#[tokio::test]
async fn test_unsupported_sport_returns_empty() {
    // Swim is not a land route — discover_routes_for_sport should return
    // an empty vec without hitting Overpass. This is a pure logic test, so
    // it runs unconditionally (no live Overpass hit).
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
async fn test_forward_geocode_prevost_resolves_into_quebec() {
    if !live_tests_enabled() {
        eprintln!("skipping live Nominatim test (set DRAVR_LIVE_OVERPASS_TESTS=1 to enable)");
        return;
    }

    let service = LocationService::new();
    let result = service
        .forward_geocode("Prévost, QC")
        .await
        .expect("Nominatim should resolve 'Prévost, QC'");

    // Prévost is in the Laurentides region of Québec; expect a roughly
    // +45.8 lat, -74.1 lon area. Allow a generous tolerance because
    // Nominatim may return the administrative centroid or a nearby node.
    assert!(
        (45.5..=46.2).contains(&result.latitude),
        "latitude {} outside expected Laurentides range",
        result.latitude
    );
    assert!(
        (-74.5..=-73.5).contains(&result.longitude),
        "longitude {} outside expected Laurentides range",
        result.longitude
    );
    assert!(
        result.display_name.to_lowercase().contains("québec")
            || result.display_name.to_lowercase().contains("quebec"),
        "display name '{}' should include Québec",
        result.display_name
    );
}

#[tokio::test]
async fn test_forward_geocode_cache_survives_a_new_service_instance() {
    if !live_tests_enabled() {
        eprintln!("skipping live Nominatim test (set DRAVR_LIVE_OVERPASS_TESTS=1 to enable)");
        return;
    }

    let first = LocationService::new()
        .forward_geocode("Saint-Alexis-des-Monts")
        .await
        .expect("first geocode call should succeed");

    // The second call goes through a DIFFERENT service instance, because that
    // is what production does: every tool call constructs its own
    // LocationService. A cache owned by the instance would miss here and open
    // a second request against an API that allows one per second.
    //
    // We can't directly assert "didn't hit network", so assert the result is
    // identical and the call completes in <50ms (round-trips take longer).
    let before = Instant::now();
    let second = LocationService::new()
        .forward_geocode("saint-alexis-des-monts") // different case to prove cache key normalization
        .await
        .expect("second geocode call should succeed");
    let elapsed = before.elapsed();

    assert!(
        (first.latitude - second.latitude).abs() < f64::EPSILON,
        "cached call returned different latitude"
    );
    assert!(
        (first.longitude - second.longitude).abs() < f64::EPSILON,
        "cached call returned different longitude"
    );
    assert!(
        elapsed < Duration::from_millis(50),
        "cached call took {elapsed:?} — the cache is not shared across instances"
    );
}

#[tokio::test]
async fn test_forward_geocode_empty_query_rejected() {
    // No live test needed — empty input is rejected before the HTTP call.
    let service = LocationService::new();
    let err = service
        .forward_geocode("   ")
        .await
        .expect_err("empty query should be rejected");
    assert!(
        err.to_string().to_lowercase().contains("empty"),
        "error message should mention empty input, got: {err}"
    );
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

const SHAWINIGAN_FIXTURE: &str = include_str!("fixtures/overpass/shawinigan-running.json");

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

const MEASURED_FIXTURE: &str = include_str!("fixtures/overpass/measured-lengths.json");
const MEASURED_LAT: f64 = 46.0;
const MEASURED_LON: f64 = -72.0;
const METERS_PER_DEGREE_OF_LATITUDE: f64 = 111_194.93;

const PREVOST_CYCLING_FIXTURE: &str = include_str!("fixtures/overpass/prevost-cycling.json");

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
