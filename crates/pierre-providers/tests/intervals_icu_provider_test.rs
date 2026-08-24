// ABOUTME: Intervals.icu provider unit tests (auth validation, defaults, registry registration, push)
// ABOUTME: Unit tests plus loopback-stub tests that pin the on-the-wire HTTP Basic credential pair
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Intervals.icu provider unit tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{NaiveDate, Utc};
use pierre_providers::core::{FitnessProvider, OAuth2Credentials};
use pierre_providers::intervals_icu_provider::{default_config, IntervalsIcuProvider};
use pierre_providers::models::{
    IntensityDistribution, SportType, WorkoutTargetZones, WorkoutTemplate,
};
use pierre_providers::ProviderRegistry;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

fn sample_template() -> WorkoutTemplate {
    WorkoutTemplate {
        id: uuid::Uuid::new_v4(),
        tenant_id: None,
        user_id: None,
        slug: "long_run_z2".to_owned(),
        name: "Long Run Z2".to_owned(),
        sport: SportType::Run,
        duration_minutes: 90,
        intensity_distribution: IntensityDistribution::Polarized,
        structure: Vec::new(),
        target_zones: WorkoutTargetZones {
            hr_pct_of_lt2: None,
            power_pct_of_ftp: None,
        },
        is_compiled_in: true,
        updated_at: Utc::now(),
    }
}

fn empty_credentials() -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: String::new(),
        client_secret: String::new(),
        access_token: None,
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
    }
}

fn good_credentials() -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: "i123456".to_owned(),
        client_secret: String::new(),
        access_token: Some("test-api-key".to_owned()),
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
    }
}

#[tokio::test]
async fn provider_name_is_intervals_icu() {
    let provider = IntervalsIcuProvider::new();
    assert_eq!(provider.name(), "intervals_icu");
}

#[tokio::test]
async fn default_config_has_endurance_endpoints() {
    let cfg = default_config();
    assert_eq!(cfg.name, "intervals_icu");
    assert!(cfg.api_base_url.contains("intervals.icu"));
    assert!(cfg.auth_url.contains("intervals.icu"));
    assert!(cfg.revoke_url.is_none());
    assert!(cfg.default_scopes.is_empty());
}

#[tokio::test]
async fn set_credentials_rejects_missing_athlete_id() {
    let provider = IntervalsIcuProvider::new();
    let mut creds = good_credentials();
    creds.client_id.clear();
    let err = provider
        .set_credentials(creds)
        .await
        .expect_err("missing athlete id");
    assert!(format!("{err}").contains("athlete id"));
}

#[tokio::test]
async fn set_credentials_rejects_missing_api_key() {
    let provider = IntervalsIcuProvider::new();
    let mut creds = good_credentials();
    creds.access_token = None;
    let err = provider
        .set_credentials(creds)
        .await
        .expect_err("missing api key");
    assert!(format!("{err}").contains("API key"));
}

#[tokio::test]
async fn set_credentials_rejects_empty_api_key() {
    let provider = IntervalsIcuProvider::new();
    let mut creds = good_credentials();
    creds.access_token = Some(String::new());
    let err = provider
        .set_credentials(creds)
        .await
        .expect_err("empty api key");
    assert!(format!("{err}").contains("API key"));
}

#[tokio::test]
async fn is_authenticated_false_until_credentials_set() {
    let provider = IntervalsIcuProvider::new();
    assert!(!provider.is_authenticated().await);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("ok credentials");
    assert!(provider.is_authenticated().await);
}

#[tokio::test]
async fn refresh_token_is_noop_for_api_key_auth() {
    let provider = IntervalsIcuProvider::new();
    // API keys never expire; refresh must be a no-op (Ok(())) regardless of state.
    provider
        .refresh_token_if_needed()
        .await
        .expect("api-key refresh is always Ok");
}

#[tokio::test]
async fn unauthenticated_calls_return_auth_error() {
    let provider = IntervalsIcuProvider::new();
    let err = provider.get_athlete().await.expect_err("missing auth");
    let msg = format!("{err}");
    assert!(
        msg.contains("intervals.icu")
            || msg.contains("link your account")
            || msg.contains("athlete id")
            || msg.contains("API key"),
        "expected auth error, got: {msg}"
    );
}

#[tokio::test]
async fn empty_credentials_struct_rejects_at_set() {
    let provider = IntervalsIcuProvider::new();
    let err = provider
        .set_credentials(empty_credentials())
        .await
        .expect_err("empty creds rejected");
    let msg = format!("{err}");
    assert!(msg.contains("athlete id") || msg.contains("API key"));
}

#[tokio::test]
async fn personal_records_returns_empty_no_endpoint() {
    let provider = IntervalsIcuProvider::new();
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");
    // No Intervals.icu endpoint exposes personal records in a coach-relevant
    // shape; the provider returns an empty list rather than an error.
    let records = provider
        .get_personal_records()
        .await
        .expect("personal_records ok");
    assert!(records.is_empty());
}

#[tokio::test]
async fn registry_registers_intervals_icu_as_non_oauth() {
    // The provider is registered (factory + descriptor) and reports as an
    // API-key (non-OAuth) provider so the connect UI offers the key modal
    // rather than an OAuth redirect.
    let registry = ProviderRegistry::new();
    assert!(
        registry.is_supported("intervals_icu"),
        "intervals_icu must be registered"
    );
    assert!(
        !registry.requires_oauth("intervals_icu"),
        "intervals_icu is API-key, not OAuth"
    );
    let provider = registry
        .create_provider("intervals_icu")
        .expect("factory creates provider");
    assert_eq!(provider.name(), "intervals_icu");
}

#[tokio::test]
async fn push_planned_workout_requires_credentials() {
    let provider = IntervalsIcuProvider::new();
    let date = NaiveDate::from_ymd_opt(2026, 6, 10).expect("valid date");
    let err = provider
        .push_planned_workout(&sample_template(), date)
        .await
        .expect_err("push without credentials must fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("intervals.icu")
            || msg.contains("link your account")
            || msg.contains("athlete id")
            || msg.contains("API key"),
        "expected auth error, got: {msg}"
    );
}

/// One-shot loopback HTTP stub. Accepts a single connection, captures the
/// request head verbatim, answers with `body`, and returns the captured head so
/// a test can assert on the exact bytes the provider put on the wire.
async fn stub_once(body: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut head = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            let n = socket.read(&mut buf).await.expect("read request");
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
            if head.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        socket.flush().await.expect("flush response");
        String::from_utf8_lossy(&head).into_owned()
    });
    (format!("http://{addr}"), handle)
}

/// Pull the decoded `Authorization: Basic ...` credential pair out of a captured
/// request head.
fn basic_credentials(head: &str) -> String {
    let encoded = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("authorization") {
                value.trim().strip_prefix("Basic ")
            } else {
                None
            }
        })
        .expect("request carries an Authorization: Basic header");
    let decoded = STANDARD.decode(encoded).expect("base64 credential pair");
    String::from_utf8(decoded).expect("utf-8 credential pair")
}

#[tokio::test]
async fn basic_auth_username_is_the_literal_api_key_not_the_athlete_id() {
    // Intervals.icu's API-key scheme is `curl -u API_KEY:<key>` — the username
    // is the fixed string `API_KEY`. Sending the athlete id there returns 401 on
    // every endpoint, which is exactly how the link flow shipped broken: the
    // athlete id addresses the URL path, never the credential pair.
    let (base_url, stub) = stub_once(r#"{"id":"i123456","name":"Test Athlete"}"#).await;

    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");

    let athlete = provider.get_athlete().await.expect("get_athlete succeeds");
    assert_eq!(athlete.id, "i123456");

    let head = stub.await.expect("stub task joins");
    assert_eq!(
        basic_credentials(&head),
        "API_KEY:test-api-key",
        "Basic credentials must be the literal API_KEY username plus the key"
    );
    assert!(
        !head.contains("i123456:test-api-key"),
        "the athlete id must never appear in the credential pair"
    );
    assert!(
        head.starts_with("GET /api/v1/athlete/i123456 "),
        "the athlete id addresses the URL path; got head: {head}"
    );
}

#[tokio::test]
async fn every_athlete_scoped_call_uses_the_literal_api_key_username() {
    // The username is a per-call-site decision (seven of them). Cover a second,
    // structurally different endpoint so a partial fix cannot pass.
    let (base_url, stub) = stub_once("[]").await;

    let mut config = default_config();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).expect("valid from date");
    let to = NaiveDate::from_ymd_opt(2026, 6, 10).expect("valid to date");
    let wellness = provider
        .get_wellness(from, to)
        .await
        .expect("get_wellness succeeds");
    assert!(wellness.is_empty(), "stub returns an empty wellness list");

    let head = stub.await.expect("stub task joins");
    assert_eq!(basic_credentials(&head), "API_KEY:test-api-key");
}
