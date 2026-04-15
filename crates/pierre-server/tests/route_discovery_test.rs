// ABOUTME: Integration tests for RouteDiscoveryService + discover_routes MCP tool
// ABOUTME: Live Overpass hits are gated behind DRAVR_LIVE_OVERPASS_TESTS to keep CI deterministic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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

use pierre_mcp_server::intelligence::{RouteDiscoveryService, RouteSource, RouteType};
use pierre_mcp_server::models::SportType;

/// Prévost, Québec — the exact reference point from ChefFamille's Telegram
/// prompt that motivated the route discovery work. Nominatim resolves this
/// to roughly 45.87, -74.08. Well-trafficked OSM area with named trails.
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

    let mut service = RouteDiscoveryService::with_defaults();
    let routes = service
        .discover_running_routes(PREVOST_QC_LAT, PREVOST_QC_LON, Some(10_000))
        .await
        .expect("Overpass query should succeed");

    assert!(
        routes.len() >= MIN_EXPECTED_REAL_ROUTES,
        "expected at least {MIN_EXPECTED_REAL_ROUTES} running route(s) near Prévost, got {}",
        routes.len()
    );

    for route in &routes {
        assert_eq!(route.route_type, RouteType::Running);
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
    }
}

#[tokio::test]
async fn test_discover_routes_for_sport_dispatches_by_type() {
    if !live_tests_enabled() {
        eprintln!("skipping live Overpass test (set DRAVR_LIVE_OVERPASS_TESTS=1 to enable)");
        return;
    }

    let mut service = RouteDiscoveryService::with_defaults();

    // SportType::Run should dispatch to the running route query
    let run_routes = service
        .discover_routes_for_sport(&SportType::Run, PREVOST_QC_LAT, PREVOST_QC_LON, Some(5_000))
        .await
        .expect("run dispatch should succeed");
    for route in &run_routes {
        assert_eq!(route.route_type, RouteType::Running);
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
    let mut service = RouteDiscoveryService::with_defaults();
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
