// ABOUTME: Unit tests for SciotteProvider::is_authenticated honest expiry handling (ADR-021 Phase 4)
// ABOUTME: A stored session whose expires_at is in the past must report NOT authenticated, never live
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte session-expiry honesty unit tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Since the Phase 4 cutover the platform holds no in-pod Chrome to probe a
//! stored sciotte session, so `is_authenticated` derives liveness from the
//! session's own `expires_at`. These tests pin that an already-expired session
//! reports NOT authenticated (so a caller never treats a dead session as live),
//! while a future or unknown expiry reports authenticated.

use chrono::{DateTime, Duration, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory};
use pierre_providers::sciotte_provider::SciotteProviderFactory;

/// Build a bare sciotte provider (no scraper, no env — the remote client is only
/// touched on an actual fetch, never on construction or `is_authenticated`).
fn sciotte_provider() -> Box<dyn FitnessProvider> {
    let config = ProviderConfig {
        name: "sciotte".to_owned(),
        auth_url: String::new(),
        token_url: String::new(),
        api_base_url: String::new(),
        revoke_url: None,
        default_scopes: vec![],
    };
    SciotteProviderFactory
        .create(config)
        .expect("sciotte provider construction is infallible")
}

/// Serialize an `AuthSession` with the given expiry into the JSON blob
/// `set_credentials` expects in `access_token`.
fn session_json(expires_at: Option<DateTime<Utc>>) -> String {
    let session = AuthSession {
        session_id: "test-session".to_owned(),
        cookies: vec![],
        created_at: Utc::now(),
        expires_at,
    };
    serde_json::to_string(&session).expect("AuthSession serializes")
}

async fn set_session(provider: &dyn FitnessProvider, expires_at: Option<DateTime<Utc>>) {
    let creds = OAuth2Credentials {
        client_id: String::new(),
        client_secret: String::new(),
        access_token: Some(session_json(expires_at)),
        refresh_token: None,
        expires_at: None,
        scopes: vec![],
    };
    provider
        .set_credentials(creds)
        .await
        .expect("restoring a well-formed session succeeds");
}

#[tokio::test]
async fn expired_session_reports_not_authenticated() {
    let provider = sciotte_provider();
    set_session(provider.as_ref(), Some(Utc::now() - Duration::hours(1))).await;
    assert!(
        !provider.is_authenticated().await,
        "a session whose expires_at is in the past must report NOT authenticated"
    );
}

#[tokio::test]
async fn future_session_reports_authenticated() {
    let provider = sciotte_provider();
    set_session(provider.as_ref(), Some(Utc::now() + Duration::hours(1))).await;
    assert!(
        provider.is_authenticated().await,
        "a session whose expires_at is in the future must report authenticated"
    );
}

#[tokio::test]
async fn unknown_expiry_session_reports_authenticated() {
    let provider = sciotte_provider();
    set_session(provider.as_ref(), None).await;
    assert!(
        provider.is_authenticated().await,
        "a session with no expires_at is unknown, assumed usable — the service re-auths on import"
    );
}

#[tokio::test]
async fn no_session_reports_not_authenticated() {
    let provider = sciotte_provider();
    assert!(
        !provider.is_authenticated().await,
        "a provider with no restored session must report NOT authenticated"
    );
}
