// ABOUTME: Pins the sciotte_coros backend: registry wiring, target names, and a scrape routed to the COROS scraper
// ABOUTME: Drives the provider against a loopback scraper stub and asserts the session import names "coros"
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `sciotte_coros` backend contract.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! COROS is read through the dravr-sciotte service (its `coros` provider,
//! released in dravr-sciotte v0.15.0) until the partner API of carnet#509 is
//! approved. The platform's `sciotte_coros` backend is one more
//! [`SciotteTarget`]: its rows are named `sciotte_coros`, and every call it
//! makes to the multi-provider scraper service names the service's own
//! provider, `coros`. A backend that sent the wrong name would scrape Strava
//! with a COROS session, so the import request is asserted on the wire.
//!
//! One test drives the network because the scraper URL is the process-wide
//! `DRAVR_SCIOTTE_REMOTE_URL`; separate network `#[test]`s would race on it.

use std::env;
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_core::constants::oauth_providers::SCIOTTE_COROS;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory};
use pierre_providers::coros_self_report::feel_from_coros;
use pierre_providers::models::Feel;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::sciotte_provider::{SciotteCorosProviderFactory, SciotteTarget};
use pierre_providers::sciotte_remote::{ENV_AUDIENCE, ENV_REMOTE_URL};
use pierre_providers::spi::ProviderCapabilities;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[test]
fn the_coros_target_names_its_backend_and_the_scrapers_provider() {
    let target = SciotteTarget::from_target_param("coros");
    assert_eq!(target.provider_name(), "sciotte_coros");
    assert_eq!(target.scraper_provider_name(), "coros");
    // A row named sciotte_coros resolves back to the same target.
    assert_eq!(
        SciotteTarget::from_backend_name("sciotte_coros").scraper_provider_name(),
        "coros"
    );
    // The two targets that existed before are unchanged.
    assert_eq!(
        SciotteTarget::from_target_param("garmin").provider_name(),
        "sciotte_garmin"
    );
    assert_eq!(
        SciotteTarget::from_backend_name("sciotte").scraper_provider_name(),
        "strava"
    );
}

#[test]
fn the_five_coros_faces_map_onto_the_named_scale_in_order() {
    // The Training Hub's own order: 1 very easy .. 5 weak.
    assert_eq!(feel_from_coros(1), Some(Feel::Strong));
    assert_eq!(feel_from_coros(2), Some(Feel::Good));
    assert_eq!(feel_from_coros(3), Some(Feel::Normal));
    assert_eq!(feel_from_coros(4), Some(Feel::Poor));
    assert_eq!(feel_from_coros(5), Some(Feel::Weak));
    // 0 is no pick; anything past 5 is not a face.
    assert_eq!(feel_from_coros(0), None);
    assert_eq!(feel_from_coros(6), None);
}

#[test]
fn the_registry_serves_sciotte_coros_as_coros() {
    let registry = ProviderRegistry::new();
    assert!(registry.is_supported(SCIOTTE_COROS));
    let descriptor = registry
        .get_descriptor(SCIOTTE_COROS)
        .expect("sciotte_coros registers a descriptor");
    assert_eq!(descriptor.name(), "sciotte_coros");
    assert_eq!(descriptor.display_name(), "COROS");
    // Activities are scraped on demand; resting HR, sleep HRV and VO2max are
    // synced from the Training Hub's daily analysis, which holds no sleep.
    assert_eq!(
        descriptor.capabilities(),
        ProviderCapabilities::ACTIVITIES
            .union(ProviderCapabilities::RECOVERY_METRICS)
            .union(ProviderCapabilities::HEALTH_METRICS)
    );
    assert!(!descriptor.capabilities().supports_sleep());
    assert!(
        descriptor.oauth_endpoints().is_none(),
        "a scraped backend has no OAuth flow"
    );
    let provider = registry
        .create_provider(SCIOTTE_COROS)
        .expect("sciotte_coros builds a provider");
    assert_eq!(provider.name(), "sciotte_coros");
}

/// Every request line and body the stub received, in order.
type Seen = Arc<Mutex<Vec<String>>>;

/// Serve the scraper endpoints a single-activity fetch calls, recording each
/// request: `POST /auth/import-session`, then `GET /api/activities/{id}`
/// answered with a COROS detail as dravr-sciotte v0.15.0 returns it.
fn spawn_scraper_stub(listener: TcpListener, seen: Seen) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0_u8; 16384];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            seen.lock().unwrap().push(request.clone());

            let body = if request.contains("/auth/import-session") {
                r#"{"session_id":"stub-session"}"#.to_owned()
            } else {
                coros_activity_json()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
}

/// A COROS run as the scraper serves it: moving and elapsed time apart (a
/// 60 s pause) and a lap timed both ways.
fn coros_activity_json() -> String {
    serde_json::json!({
        "id": "910000000000000001",
        "name": "Tempo Tuesday",
        "sport_type": "run",
        "sport_type_detail": "Run",
        "start_date": "2026-09-10T00:26:40Z",
        "duration_seconds": 3_540,
        "elapsed_time_seconds": 3_600,
        "distance_meters": 10_204.56,
        "elevation_gain": 98.0,
        "average_heart_rate": 148,
        "max_heart_rate": 176,
        "calories": 712,
        "device_name": "COROS PACE 4",
        "temperature": 21.2,
        "feel": 4,
        "provider": "scraper",
        "laps": [{
            "id": "1",
            "index": 1,
            "distance_meters": 2_500.0,
            "elapsed_time_seconds": 960,
            "moving_time_seconds": 900,
            "average_heart_rate": 136
        }]
    })
    .to_string()
}

async fn connected_provider() -> Box<dyn FitnessProvider> {
    let config = ProviderConfig {
        name: SCIOTTE_COROS.to_owned(),
        auth_url: String::new(),
        token_url: String::new(),
        api_base_url: String::new(),
        revoke_url: None,
        default_scopes: vec![],
    };
    let provider = SciotteCorosProviderFactory
        .create(config)
        .expect("sciotte provider construction is infallible"); // Safe: factory returns Ok unconditionally
    let session = AuthSession {
        session_id: "stub-session".to_owned(),
        cookies: vec![],
        created_at: Utc.with_ymd_and_hms(2026, 9, 10, 12, 0, 0).unwrap(), // Safe: literal calendar date is valid
        expires_at: None,
    };
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Some(serde_json::to_string(&session).expect("AuthSession serializes")), // Safe: plain data struct
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
        })
        .await
        .expect("a serialized session is accepted"); // Safe: the JSON above is a valid AuthSession
    provider
}

#[tokio::test]
async fn a_coros_scrape_imports_its_session_as_coros_and_keeps_the_run_whole() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback"); // Safe: ephemeral port on loopback
    let addr = listener
        .local_addr()
        .expect("bound listener has an address"); // Safe: listener is bound
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    spawn_scraper_stub(listener, Arc::clone(&seen));

    env::set_var(ENV_REMOTE_URL, format!("http://{addr}"));
    env::remove_var(ENV_AUDIENCE);

    let provider = connected_provider().await;
    let activity = provider
        .get_activity("910000000000000001")
        .await
        .expect("the stub answers a parseable activity"); // Safe: the stub body follows the scraper's model

    let requests = seen.lock().unwrap().clone();
    let import = requests
        .iter()
        .find(|r| r.contains("/auth/import-session"))
        .expect("the session is imported before the scrape");
    assert!(
        import.contains(r#""provider":"coros""#),
        "the multi-provider service must be told this is a COROS session: {import}"
    );
    assert!(
        requests
            .iter()
            .any(|r| r.contains("/api/activities/910000000000000001")),
        "the detail call names the COROS labelId"
    );

    assert_eq!(activity.name(), "Tempo Tuesday");
    assert_eq!(activity.duration_seconds(), 3_540);
    assert_eq!(activity.distance_meters(), Some(10_204.56));
    assert_eq!(activity.max_heart_rate(), Some(176));
    assert_eq!(activity.calories(), Some(712));
    // The athlete picked "tired", COROS' fourth face; COROS records no RPE.
    assert_eq!(activity.feel(), Some(Feel::Poor));
    assert_eq!(activity.perceived_exertion(), None);
    // A COROS lap is timed both ways: 900 s moving inside 960 s elapsed.
    let laps = activity.laps().expect("the lap survives conversion");
    assert_eq!(laps.len(), 1);
    assert_eq!(laps[0].moving_time_seconds, Some(900));
    assert_eq!(laps[0].elapsed_time_seconds, 960);
}
