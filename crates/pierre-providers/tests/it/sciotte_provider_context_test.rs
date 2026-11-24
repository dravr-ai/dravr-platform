// ABOUTME: Pins the scraped Strava's provider-context reads: zones, gear, records, best efforts, weight, paged search
// ABOUTME: Drives the provider against a loopback scraper stub whose unstubbed paths answer 500, so a stray request never reads as absence

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider context of the scraped Strava session (carnet#1622, sciotte 2.3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use dravr_sciotte::models::AuthSession;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::ErrorCode;
use pierre_core::models::provider_context::{ProviderActivitySearch, ProviderGearKind};
use pierre_providers::core::{
    CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory,
};
use pierre_providers::provider_context::is_unsupported;
use pierre_providers::sciotte_provider::{
    startup_reads, SciotteGarminProviderFactory, SciotteProviderFactory,
};
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// One stubbed scraper route: the exact path, the status, the JSON body.
type Route = (&'static str, u16, Value);

/// The request lines the stub served, in order.
type Seen = Arc<Mutex<Vec<String>>>;

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

/// Serve the session import and `routes`, matched on the exact request path.
/// Any other path answers 500: a request the test did not expect is never
/// mistaken for an absent value.
fn spawn_scraper_stub(listener: TcpListener, routes: Vec<Route>, seen: Seen) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0_u8; 8192];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let line = request.lines().next().unwrap_or_default().to_owned();
            let target = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let path = target.split('?').next().unwrap_or_default().to_owned();
            seen.lock().unwrap().push(line);
            let (status, body) = if path == "/auth/import-session" {
                (200, json!({ "session_id": "stub-session" }))
            } else {
                routes
                    .iter()
                    .find(|(route, _, _)| *route == path)
                    .map_or_else(
                        || (500, json!({ "error": format!("unstubbed {path}") })),
                        |(_, status, body)| (*status, body.clone()),
                    )
            };
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                reason(status),
                body.len()
            );
            drop(stream.write_all(response.as_bytes()).await);
        }
    });
}

async fn connected(base_url: &str, factory_name: &str, garmin: bool) -> Box<dyn FitnessProvider> {
    let config = ProviderConfig {
        name: factory_name.to_owned(),
        auth_url: String::new(),
        token_url: String::new(),
        api_base_url: String::new(),
        revoke_url: None,
        default_scopes: vec![],
    };
    let client = startup_reads(&[String::from(base_url)], None).expect("loopback client");
    let provider = if garmin {
        SciotteGarminProviderFactory::new(Some(client)).create(config)
    } else {
        SciotteProviderFactory::new(Some(client)).create(config)
    }
    .expect("sciotte provider construction is infallible");
    let session = AuthSession {
        session_id: "stub-session".to_owned(),
        cookies: vec![],
        created_at: Timestamp::parse_from_rfc3339("2026-11-07T12:00:00Z").unwrap(),
        expires_at: None,
    };
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: SecretString::default(),
            access_token: Some(SecretString::from(
                serde_json::to_string(&session).expect("AuthSession serializes"),
            )),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            kind: CredentialKind::OAuthBearer,
            request_budget: None,
        })
        .await
        .expect("a serialized session is accepted");
    provider
}

async fn strava(routes: Vec<Route>) -> (Box<dyn FitnessProvider>, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let seen = Seen::default();
    spawn_scraper_stub(listener, routes, Arc::clone(&seen));
    (
        connected(&format!("http://{addr}"), "sciotte", false).await,
        seen,
    )
}

/// The scraper's `500` for a provider answer that may pass, as it writes it.
fn provider_answered(status: u16) -> Value {
    json!({ "error": format!("network error: https://www.strava.com/x answered HTTP {status}") })
}

#[tokio::test]
async fn zones_cross_as_the_providers_with_each_family_apart() {
    let (provider, _) = strava(vec![(
        "/api/activities/16001/zones",
        200,
        json!({
            "heart_rate": [
                { "zone": 1, "label": "Recovery", "min_bpm": 0, "max_bpm": 120, "seconds": 600.5 },
                { "zone": 5, "label": "Anaerobic", "min_bpm": 175, "seconds": 30.0 }
            ]
        }),
    )])
    .await;
    let zones = provider
        .read_activity_zones("16001")
        .await
        .expect("read")
        .expect("zones");
    let heart_rate = zones.heart_rate.expect("heart rate family");
    assert_eq!(
        heart_rate.len(),
        2,
        "bug: a zone the provider reported was dropped"
    );
    assert_eq!(
        (
            heart_rate[0].label.as_str(),
            heart_rate[0].max_bpm,
            heart_rate[0].seconds
        ),
        ("Recovery", Some(120), 600.5),
        "bug: the provider's zone was altered"
    );
    assert_eq!(
        heart_rate[1].max_bpm, None,
        "bug: the open top zone got a bound"
    );
    assert!(
        zones.pace.is_none(),
        "bug: a family the provider did not report was invented"
    );
}

#[tokio::test]
async fn zones_of_neither_family_are_absent() {
    let (provider, _) = strava(vec![("/api/activities/16001/zones", 200, json!({}))]).await;
    assert!(
        provider
            .read_activity_zones("16001")
            .await
            .expect("read")
            .is_none(),
        "bug: zones of neither family read as a figure"
    );
}

#[tokio::test]
async fn a_404_is_absence_on_every_context_read() {
    let missing = json!({ "error": "not found" });
    let (provider, _) = strava(vec![
        ("/api/activities/16001/zones", 404, missing.clone()),
        ("/api/activities/16001/best-efforts", 404, missing.clone()),
        ("/api/gear", 404, missing.clone()),
        ("/api/personal-records", 404, missing.clone()),
        ("/api/athlete/weight", 404, missing),
    ])
    .await;
    assert!(
        provider
            .read_activity_zones("16001")
            .await
            .expect("zones")
            .is_none(),
        "bug: a 404 on zones is not absence"
    );
    assert!(
        provider
            .read_activity_best_efforts("16001")
            .await
            .expect("efforts")
            .is_none(),
        "bug: a 404 on best efforts is not absence"
    );
    assert!(
        provider.read_gear().await.expect("gear").is_none(),
        "bug: a 404 on gear is not absence"
    );
    assert!(
        provider
            .read_personal_records()
            .await
            .expect("records")
            .is_none(),
        "bug: a 404 on records is not absence"
    );
    assert!(
        provider.read_weight().await.expect("weight").is_none(),
        "bug: a 404 on weight is not absence"
    );
}

#[tokio::test]
async fn a_failure_is_never_absence() {
    let (provider, _) = strava(vec![
        ("/api/gear", 500, provider_answered(429)),
        ("/api/personal-records", 500, provider_answered(503)),
        ("/api/athlete/weight", 429, json!({})),
        (
            "/api/activities/16001/zones",
            500,
            json!({ "error": "scraping error: x" }),
        ),
    ])
    .await;
    let rate_limited = provider.read_gear().await.expect_err("a 429 is no absence");
    assert_eq!(
        rate_limited.code,
        ErrorCode::ExternalRateLimited,
        "bug: a relayed 429 is not a rate limit"
    );
    let unavailable = provider
        .read_personal_records()
        .await
        .expect_err("a 5xx is no absence");
    assert_eq!(
        unavailable.code,
        ErrorCode::ExternalServiceUnavailable,
        "bug: a relayed 5xx is not a retryable outage"
    );
    let bare = provider
        .read_weight()
        .await
        .expect_err("a bare 429 is no absence");
    assert_eq!(
        bare.code,
        ErrorCode::ExternalRateLimited,
        "bug: a bare 429 is not a rate limit"
    );
    let internal = provider
        .read_activity_zones("16001")
        .await
        .expect_err("a plain 500");
    assert_eq!(
        internal.code,
        ErrorCode::InternalError,
        "bug: a plain scraper 500 is not an internal error"
    );
    let unstubbed = provider.read_activity_best_efforts("16001").await;
    assert!(unstubbed.is_err(), "bug: an unstubbed path read as absence");
}

#[tokio::test]
async fn a_dead_session_is_a_reconnect_not_an_absence() {
    let (provider, _) = strava(vec![(
        "/api/gear",
        401,
        json!({ "error": "session_expired", "message": "the provider refused the session" }),
    )])
    .await;
    let error = provider.read_gear().await.expect_err("a dead session");
    assert_eq!(
        error.provider_auth_required_provider().as_deref(),
        Some("sciotte"),
        "bug: a dead session is not a reconnect of the session's backend"
    );
}

#[tokio::test]
async fn gear_keeps_bikes_and_the_providers_mileage() {
    let (provider, _) = strava(vec![(
        "/api/gear",
        200,
        json!({
            "count": 3,
            "gear": [
                { "id": "g1", "kind": "shoe", "name": "Daily", "brand": "Acme", "model": "Fast 3",
                  "total_distance_m": 412_300.0, "primary": true, "retired": false },
                { "id": "g2", "kind": "shoe", "name": "Old pair", "primary": false, "retired": true },
                { "id": "b1", "kind": "bike", "name": "Gravel", "total_distance_m": 8_000_000.0,
                  "primary": true, "retired": false }
            ]
        }),
    )])
    .await;
    let gear = provider.read_gear().await.expect("read").expect("gear");
    assert_eq!(
        gear.len(),
        3,
        "bug: a piece of the provider's gear was dropped"
    );
    assert_eq!(
        gear[0].total_distance_m,
        Some(412_300.0),
        "bug: the provider's mileage was altered"
    );
    assert_eq!(
        gear[0].brand.as_deref(),
        Some("Acme"),
        "bug: the provider's brand was altered"
    );
    assert_eq!(
        gear[1].total_distance_m, None,
        "bug: an unreported mileage became a figure"
    );
    assert!(gear[1].retired, "bug: the retired flag was lost");
    assert_eq!(
        gear[2].kind,
        ProviderGearKind::Bike,
        "bug: a bike read as a shoe"
    );
}

#[tokio::test]
async fn an_empty_gear_list_is_a_read_that_found_none() {
    let (provider, _) = strava(vec![("/api/gear", 200, json!({ "count": 0, "gear": [] }))]).await;
    let gear = provider.read_gear().await.expect("read");
    assert_eq!(
        gear.map(|g| g.len()),
        Some(0),
        "bug: an empty gear list is not a read that found none"
    );
}

#[tokio::test]
async fn records_best_efforts_and_weight_cross_as_the_providers() {
    let (provider, _) = strava(vec![
        (
            "/api/personal-records",
            200,
            json!({ "count": 2, "personal_records": [
                { "sport": "Run", "label": "5K", "distance_m": 5000.0, "elapsed_seconds": 1212, "activity_id": "9201001" },
                { "sport": "Ride", "label": "Longest Ride", "value_m": 161_000.0, "activity_id": "9201002" }
            ]}),
        ),
        (
            "/api/activities/9201001/best-efforts",
            200,
            json!({ "count": 1, "best_efforts": [
                { "label": "1K", "distance_m": 1000.0, "elapsed_seconds": 238, "start_index": 10, "end_index": 250 }
            ]}),
        ),
        ("/api/athlete/weight", 200, json!({ "weight_kg": 71.4 })),
    ])
    .await;
    let records = provider
        .read_personal_records()
        .await
        .expect("read")
        .expect("records");
    assert_eq!(
        records[0].elapsed_seconds,
        Some(1212),
        "bug: a timed record lost its time"
    );
    assert_eq!(
        records[0].value_m, None,
        "bug: a timed record gained a value"
    );
    assert_eq!(
        records[1].value_m,
        Some(161_000.0),
        "bug: a distance record lost its value"
    );
    assert_eq!(
        records[1].elapsed_seconds, None,
        "bug: a distance record gained a time"
    );
    let efforts = provider
        .read_activity_best_efforts("9201001")
        .await
        .expect("read")
        .expect("efforts");
    assert_eq!(
        (efforts[0].label.as_str(), efforts[0].elapsed_seconds),
        ("1K", 238),
        "bug: the provider's best effort was altered"
    );
    let weight = provider.read_weight().await.expect("read").expect("weight");
    assert!(
        (weight.weight_kg - 71.4).abs() < f64::EPSILON,
        "bug: the provider's weight was altered"
    );
}

#[tokio::test]
async fn a_profile_with_no_weight_is_none() {
    let (provider, _) = strava(vec![("/api/athlete/weight", 200, json!({}))]).await;
    assert!(
        provider.read_weight().await.expect("read").is_none(),
        "bug: a profile with no weight read as a figure"
    );
}

#[tokio::test]
async fn an_id_that_is_not_digits_never_reaches_the_scraper() {
    let (provider, seen) = strava(vec![]).await;
    for bad in ["12a", "", "../x"] {
        let zones = provider
            .read_activity_zones(bad)
            .await
            .expect_err("refused");
        assert_eq!(zones.code, ErrorCode::InvalidInput, "{bad:?}");
        let efforts = provider
            .read_activity_best_efforts(bad)
            .await
            .expect_err("refused");
        assert_eq!(efforts.code, ErrorCode::InvalidInput, "{bad:?}");
    }
    assert!(
        seen.lock().unwrap().is_empty(),
        "bug: a bad id cost a scraper call: {:?}",
        seen.lock().unwrap()
    );
}

#[tokio::test]
async fn search_sends_its_page_and_says_whether_the_walk_was_complete() {
    let (provider, seen) = strava(vec![(
        "/api/activity-search",
        200,
        json!({
            "count": 1,
            "head_complete": false,
            "activities": [{
                "id": "16001", "name": "Commute", "sport_type": "ride",
                "start_date": "2026-11-07T13:00:00Z", "duration_seconds": 1_800,
                "distance_meters": 9_000.0, "provider": "strava"
            }]
        }),
    )])
    .await;
    let page = provider
        .search_activities(&ProviderActivitySearch {
            page: Some(3),
            limit: Some(50),
        })
        .await
        .expect("search");
    assert_eq!(
        page.activities.len(),
        1,
        "bug: the page's activity was dropped"
    );
    assert!(!page.complete, "bug: a short walk read as the whole answer");
    let lines = seen.lock().unwrap().join("\n");
    assert!(
        lines.contains("page=3"),
        "bug: the page never reached the scraper: {lines}"
    );
    assert!(
        provider.head_complete(),
        "bug: the search moved the list read's head completeness"
    );
}

#[tokio::test]
async fn the_other_scraped_backends_refuse_every_context_read() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let seen = Seen::default();
    spawn_scraper_stub(listener, vec![], Arc::clone(&seen));
    let garmin = connected(&format!("http://{addr}"), "sciotte_garmin", true).await;
    let refusals = [
        garmin.read_activity_zones("16001").await.map(|_| ()),
        garmin.read_activity_best_efforts("16001").await.map(|_| ()),
        garmin.read_gear().await.map(|_| ()),
        garmin.read_personal_records().await.map(|_| ()),
        garmin.read_weight().await.map(|_| ()),
        garmin
            .search_activities(&ProviderActivitySearch::default())
            .await
            .map(|_| ()),
    ];
    for refusal in refusals {
        let error = refusal.expect_err("a Garmin mirror serves no Strava context");
        assert!(is_unsupported(&error), "bug: {error}");
        assert!(
            !error.message.contains("sciotte"),
            "bug: backend slug leaked: {error}"
        );
    }
    assert!(
        seen.lock().unwrap().is_empty(),
        "bug: a refused read still asked the scraper"
    );
}
