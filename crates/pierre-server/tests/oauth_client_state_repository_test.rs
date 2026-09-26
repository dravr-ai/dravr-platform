// ABOUTME: Repository-level tests for OAuthClientStateRepository — store, consume exactly once, and what the row decodes to
// ABOUTME: Runs on whichever driver the test factory selects, so the same assertions prove SQLite and Postgres
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `OAuthClientStateRepository` below the OAuth flow.
//!
//! `oauth_launch_reaper_test` proves the sweep; this file proves the
//! consume path: the one statement that verifies a CSRF `state`, marks it
//! used and hands back the PKCE verifier the token exchange needs. Every
//! column is read back — the optional user and tenant, the timestamps, the
//! pinned app — so a decode that differs between the two backends fails
//! here rather than at a provider callback.

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::OAuthClientState;
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

fn minted(name: &str, user_id: Option<Uuid>, ttl: Duration) -> OAuthClientState {
    let now = Utc::now();
    OAuthClientState {
        state: name.to_owned(),
        provider: "strava".to_owned(),
        user_id,
        tenant_id: Some("tenant-a".to_owned()),
        redirect_uri: "https://example.test/api/oauth/callback".to_owned(),
        scope: Some("activity:read_all".to_owned()),
        pkce_code_verifier: Some("verifier-123".to_owned()),
        oauth_app_client_id: Some("app-7".to_owned()),
        bridge_callback_token: Some("b".repeat(64)),
        created_at: now - Duration::minutes(1),
        expires_at: now + ttl,
        used: false,
    }
}

fn same_instant(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    a.timestamp_micros() == b.timestamp_micros()
}

#[tokio::test]
async fn consume_returns_every_column_and_marks_the_state_used() {
    let db = create_test_db().await.unwrap();
    let repo = &db.repositories().oauth_client_state;
    let user = Uuid::new_v4();
    let wanted = minted("state-1", Some(user), Duration::minutes(10));
    repo.store_oauth_client_state(&wanted).await.unwrap();

    let consumed = repo
        .consume_oauth_client_state("state-1", "strava", Utc::now())
        .await
        .unwrap()
        .expect("a live state consumes");
    assert_eq!(consumed.state, "state-1");
    assert_eq!(consumed.provider, "strava");
    assert_eq!(consumed.user_id, Some(user));
    assert_eq!(consumed.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(consumed.redirect_uri, wanted.redirect_uri);
    assert_eq!(consumed.scope.as_deref(), Some("activity:read_all"));
    assert_eq!(consumed.pkce_code_verifier.as_deref(), Some("verifier-123"));
    assert_eq!(consumed.oauth_app_client_id.as_deref(), Some("app-7"));
    assert_eq!(
        consumed.bridge_callback_token.as_deref(),
        Some("b".repeat(64).as_str())
    );
    assert!(same_instant(consumed.created_at, wanted.created_at));
    assert!(same_instant(consumed.expires_at, wanted.expires_at));
    assert!(consumed.used, "the row comes back already marked used");

    let again = repo
        .consume_oauth_client_state("state-1", "strava", Utc::now())
        .await
        .unwrap();
    assert!(again.is_none(), "a state consumes exactly once");
}

#[tokio::test]
async fn a_state_without_a_user_or_tenant_reads_back_as_none() {
    let db = create_test_db().await.unwrap();
    let repo = &db.repositories().oauth_client_state;
    let mut anonymous = minted("state-anon", None, Duration::minutes(10));
    anonymous.tenant_id = None;
    anonymous.scope = None;
    anonymous.pkce_code_verifier = None;
    anonymous.oauth_app_client_id = None;
    anonymous.bridge_callback_token = None;
    repo.store_oauth_client_state(&anonymous).await.unwrap();

    let consumed = repo
        .consume_oauth_client_state("state-anon", "strava", Utc::now())
        .await
        .unwrap()
        .expect("a live state consumes");
    assert_eq!(consumed.user_id, None);
    assert_eq!(consumed.tenant_id, None);
    assert_eq!(consumed.scope, None);
    assert_eq!(consumed.pkce_code_verifier, None);
    assert_eq!(consumed.oauth_app_client_id, None);
    assert_eq!(consumed.bridge_callback_token, None);
}

#[tokio::test]
async fn the_wrong_provider_an_expired_state_and_an_unknown_state_do_not_consume() {
    let db = create_test_db().await.unwrap();
    let repo = &db.repositories().oauth_client_state;
    repo.store_oauth_client_state(&minted("live", Some(Uuid::new_v4()), Duration::minutes(10)))
        .await
        .unwrap();
    repo.store_oauth_client_state(&minted(
        "stale",
        Some(Uuid::new_v4()),
        Duration::minutes(-1),
    ))
    .await
    .unwrap();

    let now = Utc::now();
    assert!(repo
        .consume_oauth_client_state("live", "whoop", now)
        .await
        .unwrap()
        .is_none());
    assert!(repo
        .consume_oauth_client_state("stale", "strava", now)
        .await
        .unwrap()
        .is_none());
    assert!(repo
        .consume_oauth_client_state("never-minted", "strava", now)
        .await
        .unwrap()
        .is_none());

    // The provider mismatch consumed nothing, so the live state still does.
    assert!(repo
        .consume_oauth_client_state("live", "strava", now)
        .await
        .unwrap()
        .is_some());

    // The expired, unconsumed row is what the reaper exists for.
    let reaped = repo.reap_expired_oauth_client_states(now).await.unwrap();
    assert_eq!(reaped, vec![("strava".to_owned(), 1_u64)]);
}
