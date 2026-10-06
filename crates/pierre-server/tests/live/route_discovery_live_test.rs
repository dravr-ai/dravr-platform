// ABOUTME: Live route discovery against the public Overpass and Nominatim APIs
// ABOUTME: Built only with the live-e2e feature; fails, never skips, when either API is unreachable
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Live route-discovery tests: real requests to the public Overpass API
//! (routes) and Nominatim (forward geocoding). Nothing here skips
//! (carnet#805): an unreachable service or a failed request fails the test.
//! Run with:
//!
//! ```bash
//! cargo test --features live-e2e --test route_discovery_live_test -- --nocapture
//! ```

use std::time::{Duration, Instant};

use pierre_core::models::SportType;
use pierre_fitness_compute::location::LocationService;
use pierre_fitness_compute::{RouteDiscoveryService, RouteSource, RouteType};

/// Prévost, Québec — reference point for route-discovery integration tests.
/// Nominatim resolves this to roughly 45.87, -74.08. Well-trafficked OSM area
/// with named trails.
const PREVOST_QC_LAT: f64 = 45.87;
const PREVOST_QC_LON: f64 = -74.08;

/// Minimum number of routes we expect a real Overpass query around a
/// well-mapped area to return. Below this, either OSM coverage collapsed
/// or we're parsing the response wrong.
const MIN_EXPECTED_REAL_ROUTES: usize = 1;

#[tokio::test]
async fn test_discover_running_routes_around_prevost() {
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
async fn test_forward_geocode_prevost_resolves_into_quebec() {
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
