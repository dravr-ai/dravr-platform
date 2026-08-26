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
    build_overpass_query, routes_from_overpass_json, RouteDiscoveryService, RouteSource, RouteType,
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
        // A route the coach cannot name is a route it cannot recommend.
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
        .discover_routes_for_sport(&SportType::Run, PREVOST_QC_LAT, PREVOST_QC_LON, Some(5_000))
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
        .discover_routes_for_sport(&SportType::Swim, PREVOST_QC_LAT, PREVOST_QC_LON, None)
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

    let mut service = LocationService::new();
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
async fn test_forward_geocode_cached_second_call_is_instant() {
    if !live_tests_enabled() {
        eprintln!("skipping live Nominatim test (set DRAVR_LIVE_OVERPASS_TESTS=1 to enable)");
        return;
    }

    let mut service = LocationService::new();
    let first = service
        .forward_geocode("Saint-Alexis-des-Monts")
        .await
        .expect("first geocode call should succeed");

    // Second call should hit the in-memory cache and return the exact same
    // coordinates without re-querying Nominatim. We can't directly assert
    // "didn't hit network", but assert the result is byte-identical and
    // the call completes in <10ms (network round-trips take >50ms typically).
    let before = Instant::now();
    let second = service
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
        "cached call took {elapsed:?} — cache key normalization may be broken"
    );
}

#[tokio::test]
async fn test_forward_geocode_empty_query_rejected() {
    // No live test needed — empty input is rejected before the HTTP call.
    let mut service = LocationService::new();
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
// point. The coach could not name a single trail from the first set, which is
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
            .rsplit_once("out tags center ")
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
        query.contains("track"),
        "gravel rides live on highway=track: {query}"
    );
    assert!(
        query.contains(r#"["mtb:scale"]"#),
        "singletrack is tagged mtb:scale: {query}"
    );
    assert!(
        query.contains(r#"relation["route"~"^(bicycle|mtb)$"]"#),
        "signed cycling itineraries are route relations: {query}"
    );
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
