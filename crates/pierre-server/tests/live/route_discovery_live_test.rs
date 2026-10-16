// ABOUTME: Live route discovery against the public Overpass and Open-Meteo geocoding APIs
// ABOUTME: Built only with the live-e2e feature; fails, never skips, when either API is unreachable
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Live route-discovery tests: real requests to the public Overpass API
//! (routes) and Open-Meteo's gazetteer (forward geocoding). Nothing here skips
//! (carnet#805): an unreachable service or a failed request fails the test.
//! Run with:
//!
//! ```bash
//! cargo test --features live-e2e --test route_discovery_live_test -- --nocapture
//! ```

use std::time::{Duration, Instant};

use dravr_meteo::Geocoder;
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::SportType;
use pierre_fitness_compute::geocoding::{free_text_query, place_geocoder};
use pierre_fitness_compute::{DiscoveredRoute, RouteDiscoveryService, RouteSource, RouteType};
use tokio::sync::Mutex;
use tokio::time::sleep;

/// Prévost, Québec — reference point for route-discovery integration tests.
/// Nominatim resolves this to roughly 45.87, -74.08. Well-trafficked OSM area
/// with named trails.
const PREVOST_QC_LAT: f64 = 45.87;
const PREVOST_QC_LON: f64 = -74.08;

/// Minimum number of routes we expect a real Overpass query around a
/// well-mapped area to return. Below this, either OSM coverage collapsed
/// or we're parsing the response wrong.
const MIN_EXPECTED_REAL_ROUTES: usize = 1;

/// Attempts per Overpass query. The service already walks every public mirror
/// within its own deadline; a second and third pass only matters when all of
/// them are busy at once, which is what the scheduled runs hit (HTTP
/// 504/429/500 from every mirror on 10-07, 10-08, 10-09 and 10-11, carnet#867).
const OVERPASS_ATTEMPTS: u32 = 3;
const OVERPASS_BACKOFF: Duration = Duration::from_secs(45);

/// Serialises the Overpass-backed tests. The live-apis lane already passes
/// `--test-threads=1`, but the run command in the module docs does not, and
/// `overpass-api.de` forbids parallel requests from one client — the policy
/// the service honours by walking its instances sequentially.
static OVERPASS_GATE: Mutex<()> = Mutex::const_new(());

/// Queries Overpass one test at a time and retries only
/// `ExternalServiceUnavailable`. Any other error, and the last unavailable
/// one, is returned so the test still fails: nothing here skips (carnet#805).
async fn discover_with_retry(
    sport: &SportType,
    radius_meters: u32,
) -> AppResult<Vec<DiscoveredRoute>> {
    let _gate = OVERPASS_GATE.lock().await;
    let service = RouteDiscoveryService::with_defaults();
    let mut attempt = 1;
    loop {
        let result = service
            .discover_routes_for_sport(
                sport,
                PREVOST_QC_LAT,
                PREVOST_QC_LON,
                Some(radius_meters),
                None,
            )
            .await;
        match result {
            Err(e)
                if e.code == ErrorCode::ExternalServiceUnavailable
                    && attempt < OVERPASS_ATTEMPTS =>
            {
                eprintln!(
                    "overpass attempt {attempt} unavailable, retrying: {}",
                    e.message
                );
                attempt += 1;
                sleep(OVERPASS_BACKOFF).await;
            }
            other => return other,
        }
    }
}

#[tokio::test]
async fn test_discover_running_routes_around_prevost() {
    let routes = discover_with_retry(&SportType::Run, 10_000)
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
    // SportType::Run should dispatch to the running route query
    let run_routes = discover_with_retry(&SportType::Run, 5_000)
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
    let ski_routes = discover_with_retry(&SportType::CrossCountrySkiing, 20_000)
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
    let query = free_text_query("Prévost, Quebec", Some("CA".to_owned())).unwrap();
    let resolution = place_geocoder()
        .unwrap()
        .resolve(&query)
        .await
        .expect("the gazetteer should resolve 'Prévost, Quebec'");
    let place = resolution.place;

    // Prévost is in the Laurentides region of Québec; expect a roughly
    // +45.8 lat, -74.1 lon area, with a generous tolerance for the gazetteer's
    // choice of centroid.
    assert!(
        (45.5..=46.2).contains(&place.latitude),
        "latitude {} outside expected Laurentides range",
        place.latitude
    );
    assert!(
        (-74.5..=-73.5).contains(&place.longitude),
        "longitude {} outside expected Laurentides range",
        place.longitude
    );
    assert_eq!(place.country_code.as_deref(), Some("CA"));
}

#[tokio::test]
async fn test_forward_geocode_cache_is_process_wide() {
    let first = place_geocoder()
        .unwrap()
        .resolve(&free_text_query("Saint-Alexis-des-Monts", None).unwrap())
        .await
        .expect("first geocode call should succeed");

    // The second call reads the one process-wide geocoder again, as every
    // tool call does, with the name in a different case: a cache keyed per
    // caller or by the raw spelling would miss and open a second request.
    //
    // We can't directly assert "didn't hit network", so assert the result is
    // identical and the call completes in <50ms (round-trips take longer).
    let before = Instant::now();
    let second = place_geocoder()
        .unwrap()
        .resolve(&free_text_query("saint-alexis-des-monts", None).unwrap())
        .await
        .expect("second geocode call should succeed");
    let elapsed = before.elapsed();

    assert!(
        (first.place.latitude - second.place.latitude).abs() < f64::EPSILON,
        "cached call returned different latitude"
    );
    assert!(
        (first.place.longitude - second.place.longitude).abs() < f64::EPSILON,
        "cached call returned different longitude"
    );
    assert!(
        elapsed < Duration::from_millis(50),
        "cached call took {elapsed:?} — the cache is not shared across calls"
    );
}
