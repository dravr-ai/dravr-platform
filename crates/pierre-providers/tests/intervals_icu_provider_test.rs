// ABOUTME: Endurance Phase 4 — Intervals.icu provider unit tests (auth validation, defaults, FitnessProvider trait)
// ABOUTME: Pure unit tests; the e2e suite that hits the live API lives in intervals_icu_e2e_test.rs (env-gated)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials};
use pierre_providers::intervals_icu_provider::{default_config, IntervalsIcuProvider};

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
async fn disconnect_clears_credentials() {
    let provider = IntervalsIcuProvider::new();
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");
    assert!(provider.is_authenticated().await);
    provider.disconnect().await.expect("disconnect");
    assert!(!provider.is_authenticated().await);
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
async fn personal_records_returns_empty_for_pull_only_phase() {
    let provider = IntervalsIcuProvider::new();
    provider
        .set_credentials(good_credentials())
        .await
        .expect("set creds");
    // Personal records aren't part of Phase 4 (no Intervals.icu endpoint
    // exposes them in a coach-relevant shape); the provider must return
    // an empty list rather than an error.
    let records = provider
        .get_personal_records()
        .await
        .expect("personal_records ok");
    assert!(records.is_empty());
    let _ = Utc::now(); // touch the import so tests compile cleanly without warnings.
}
