// ABOUTME: A sciotte session rotated on a health-sync or TrainingPeaks profile read is stored over its token row
// ABOUTME: Only while that row still stands: a reconnect that replaced it during the read keeps its own session
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin rotates its `session` cookie on a read and logs the old value out.
//! The platform stores each scrape session on a token row and imports it
//! before every read (ADR-021), so the copy the service now holds must
//! replace the stored one, or the next import re-seeds a dead session. The
//! health sync and the `TrainingPeaks` profile read reach the service outside
//! any provider, so each writes the rotated copy over the row the session
//! came from, while that row still stands.
//!
//! The scraper is a loopback stand-in (a test double, per the repo's mock
//! rule): every read is answered rotated, and an export answers the session
//! with its `session` cookie changed. It can replace the token row on the
//! first read, as a reconnect landing during the read would.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate, so without a surviving crate doc the command-line
// `-D warnings` trips `missing_docs`.
#![cfg(feature = "health-sync")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};

use chrono::Utc;
use dravr_enforme::traits::credential_store::CredentialStore;
use dravr_sciotte::client::SciotteClient;
use dravr_sciotte::models::AuthSession;
use dravr_sciotte::wire::{SESSION_ROTATED, SESSION_ROTATED_HEADER};
use pierre_core::constants::oauth_providers::{
    SCIOTTE_GARMIN, SCIOTTE_TRAININGPEAKS, TOKEN_TYPE_SESSION,
};
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_providers::core::{CredentialKind, OAuth2Credentials};
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::token_writeback::StoredTokenRow;
use pierre_services::health_sync::PierreSyncStorage;
use pierre_services::trainingpeaks_accounts::trainingpeaks_profile;
use pierre_test_support::db::create_test_db_with_key;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::db_fixtures::seed_user;

const SESSION_ID: &str = "sess-rotating";
const STORED_COOKIE: &str = "stored-cookie";
const ROTATED_COOKIE: &str = "rotated-cookie";
const RECONNECTED_COOKIE: &str = "reconnected-cookie";

/// A row a reconnect stores while a read is in flight, and where.
type Replacement = Arc<Mutex<Option<(Arc<RepositoryRegistry>, UserOAuthToken)>>>;

fn session_with(cookie: &str) -> AuthSession {
    serde_json::from_value(json!({
        "session_id": SESSION_ID,
        "cookies": [{
            "name": "session", "value": cookie, "domain": ".connect.garmin.com",
            "path": "/", "secure": true, "http_only": true
        }],
        "created_at": "2026-10-29T00:00:00Z"
    }))
    .unwrap()
}

/// A token row exactly as a sciotte login stores it.
fn session_row(user_id: Uuid, tenant: TenantId, backend: &str, cookie: &str) -> UserOAuthToken {
    let now = Utc::now();
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant.to_string(),
        provider: backend.to_owned(),
        access_token: SecretString::from(serde_json::to_string(&session_with(cookie)).unwrap()),
        refresh_token: None,
        token_type: TOKEN_TYPE_SESSION.to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// The `session` cookie of the session stored on `user_id`'s `backend` row.
async fn stored_cookie(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    backend: &str,
) -> String {
    let row = repos
        .oauth_tokens
        .get_token(user_id, tenant, backend)
        .await
        .unwrap()
        .expect("the row is stored");
    let session: AuthSession = serde_json::from_str(row.access_token.expose_secret()).unwrap();
    session.cookies[0].value.clone()
}

/// The date a daily-summary read asks for.
fn requested_date(request: &str) -> String {
    request
        .split("date=")
        .nth(1)
        .map(|rest| rest.chars().take(10).collect())
        .unwrap_or_default()
}

/// Serve the stand-in; returns its base URL and how many exports it answered.
async fn spawn_rotating_scraper(replacement: Replacement) -> (String, Arc<Mutex<usize>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let exports = Arc::new(Mutex::new(0_usize));
    let counted = Arc::clone(&exports);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0_u8; 16384];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let line = request.lines().next().unwrap_or_default().to_owned();
            let (body, rotated): (Value, bool) = if line.contains("/auth/import-session") {
                (json!({ "session_id": SESSION_ID }), false)
            } else if line.contains("/export") {
                *counted.lock().unwrap() += 1;
                (
                    json!({ "provider": "garmin", "session": session_with(ROTATED_COOKIE) }),
                    false,
                )
            } else {
                let pending = replacement.lock().unwrap().take();
                if let Some((repos, row)) = pending {
                    repos.oauth_tokens.upsert_token(&row).await.unwrap();
                }
                if line.contains("/api/daily-summary") {
                    (
                        json!({ "date": requested_date(&line), "provider": "garmin", "resting_heart_rate": 47 }),
                        true,
                    )
                } else {
                    (json!({ "id": "900001", "role": "athlete" }), true)
                }
            };
            let body = body.to_string();
            let rotated_header = if rotated {
                format!("{SESSION_ROTATED_HEADER}: {SESSION_ROTATED}\r\n")
            } else {
                String::new()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n{rotated_header}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            drop(stream.write_all(response.as_bytes()).await);
        }
    });
    (base, exports)
}

async fn setup() -> (Arc<RepositoryRegistry>, Uuid, TenantId, Database) {
    let db = create_test_db_with_key(b"rotated-session-row-key-32-byte!".to_vec())
        .await
        .unwrap();
    let (user_id, tenant) = seed_user(&db).await;
    let repos = Arc::clone(db.repositories());
    (repos, user_id, tenant, db)
}

#[tokio::test]
async fn a_health_sync_read_that_rotated_the_session_stores_it_over_the_row() {
    let (repos, user_id, tenant, _db) = setup().await;
    repos
        .oauth_tokens
        .upsert_token(&session_row(user_id, tenant, SCIOTTE_GARMIN, STORED_COOKIE))
        .await
        .unwrap();
    let (base, exports) = spawn_rotating_scraper(Arc::new(Mutex::new(None))).await;
    let client = SciotteClient::new(&base, None).unwrap();
    let storage = Arc::new(PierreSyncStorage::new(&repos));
    let orchestrator = storage.build_orchestrator(Some(client)).unwrap();

    orchestrator
        .sync_user(&user_id.to_string(), "garmin")
        .await
        .unwrap();

    assert!(
        *exports.lock().unwrap() > 0,
        "the rotated session is exported"
    );
    assert_eq!(
        stored_cookie(&repos, user_id, tenant, SCIOTTE_GARMIN).await,
        ROTATED_COOKIE,
        "the row now holds the copy the service holds"
    );
}

#[tokio::test]
async fn a_health_sync_never_overwrites_a_row_a_reconnect_replaced_during_the_read() {
    let (repos, user_id, tenant, _db) = setup().await;
    repos
        .oauth_tokens
        .upsert_token(&session_row(user_id, tenant, SCIOTTE_GARMIN, STORED_COOKIE))
        .await
        .unwrap();
    let reconnect = session_row(user_id, tenant, SCIOTTE_GARMIN, RECONNECTED_COOKIE);
    let (base, exports) =
        spawn_rotating_scraper(Arc::new(Mutex::new(Some((Arc::clone(&repos), reconnect))))).await;
    let client = SciotteClient::new(&base, None).unwrap();
    let storage = Arc::new(PierreSyncStorage::new(&repos));
    let orchestrator = storage.build_orchestrator(Some(client)).unwrap();

    orchestrator
        .sync_user(&user_id.to_string(), "garmin")
        .await
        .unwrap();

    assert!(
        *exports.lock().unwrap() > 0,
        "the rotated session is exported"
    );
    assert_eq!(
        stored_cookie(&repos, user_id, tenant, SCIOTTE_GARMIN).await,
        RECONNECTED_COOKIE,
        "the reconnect's session stands"
    );
}

#[tokio::test]
async fn a_trainingpeaks_profile_read_stores_a_rotated_session_only_over_a_standing_row() {
    let (repos, user_id, tenant, _db) = setup().await;
    let row = session_row(user_id, tenant, SCIOTTE_TRAININGPEAKS, STORED_COOKIE);
    repos.oauth_tokens.upsert_token(&row).await.unwrap();
    let read_from = StoredTokenRow::of(&row).unwrap();
    let session = session_with(STORED_COOKIE);

    let replacement: Replacement = Arc::new(Mutex::new(None));
    let (base, _) = spawn_rotating_scraper(Arc::clone(&replacement)).await;
    let registry =
        ProviderRegistry::new().with_sciotte_client(Some(SciotteClient::new(&base, None).unwrap()));

    trainingpeaks_profile(&registry, &*repos.oauth_tokens, &read_from, &session)
        .await
        .expect("the profile is read");
    assert_eq!(
        stored_cookie(&repos, user_id, tenant, SCIOTTE_TRAININGPEAKS).await,
        ROTATED_COOKIE,
        "the rotated session is stored over the row it came from"
    );

    // A reconnect stores a fresh row while the next read is in flight.
    *replacement.lock().unwrap() = Some((
        Arc::clone(&repos),
        session_row(user_id, tenant, SCIOTTE_TRAININGPEAKS, RECONNECTED_COOKIE),
    ));
    let stored = repos
        .oauth_tokens
        .get_token(user_id, tenant, SCIOTTE_TRAININGPEAKS)
        .await
        .unwrap()
        .unwrap();
    let read_from = StoredTokenRow::of(&stored).unwrap();
    trainingpeaks_profile(&registry, &*repos.oauth_tokens, &read_from, &session)
        .await
        .expect("the profile is read");
    assert_eq!(
        stored_cookie(&repos, user_id, tenant, SCIOTTE_TRAININGPEAKS).await,
        RECONNECTED_COOKIE,
        "the reconnect's session stands"
    );
}

/// The session stored on `user_id`'s `backend` row: its id and `session`
/// cookie, and whether the row still holds no refresh token.
async fn stored_session(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    backend: &str,
) -> (String, String, bool) {
    let row = repos
        .oauth_tokens
        .get_token(user_id, tenant, backend)
        .await
        .unwrap()
        .expect("the row is stored");
    let session: AuthSession = serde_json::from_str(row.access_token.expose_secret()).unwrap();
    (
        session.session_id,
        session.cookies[0].value.clone(),
        row.refresh_token.is_none(),
    )
}

/// A super admin's clone copies one Garmin session onto another account, in
/// its own tenant. A provider read on the source account that rotates the
/// session stores the rotated copy over the row it read only, its `NULL`
/// refresh token kept though the auth layer hands the provider an empty one;
/// the clone's row is never written, and keeps the copy Garmin signed out.
#[tokio::test]
async fn a_provider_read_that_rotates_a_cloned_session_stores_it_on_the_row_read_only() {
    let (repos, reader, reader_tenant, db) = setup().await;
    let (clone, clone_tenant) = seed_user(&db).await;
    let read_row = session_row(reader, reader_tenant, SCIOTTE_GARMIN, STORED_COOKIE);
    repos.oauth_tokens.upsert_token(&read_row).await.unwrap();
    repos
        .oauth_tokens
        .upsert_token(&session_row(
            clone,
            clone_tenant,
            SCIOTTE_GARMIN,
            STORED_COOKIE,
        ))
        .await
        .unwrap();

    let (base, exports) = spawn_rotating_scraper(Arc::new(Mutex::new(None))).await;
    let registry =
        ProviderRegistry::new().with_sciotte_client(Some(SciotteClient::new(&base, None).unwrap()));
    let provider = registry.create_provider(SCIOTTE_GARMIN).unwrap();
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: SecretString::default(),
            access_token: Some(read_row.access_token.clone()),
            // What the auth layer reads a NULL refresh-token column as.
            refresh_token: Some(SecretString::from(String::new())),
            expires_at: None,
            scopes: vec![],
            kind: CredentialKind::OAuthBearer,
            request_budget: None,
        })
        .await
        .unwrap();
    provider.set_token_refresh_callback(
        StoredTokenRow::of(&read_row)
            .unwrap()
            .write_back_callback(Arc::clone(&repos.oauth_tokens)),
    );

    provider.get_athlete().await.expect("the profile is read");

    assert_eq!(
        *exports.lock().unwrap(),
        1,
        "the rotated session is exported once"
    );
    assert_eq!(
        stored_session(&repos, reader, reader_tenant, SCIOTTE_GARMIN).await,
        (SESSION_ID.to_owned(), ROTATED_COOKIE.to_owned(), true),
        "the row read takes the rotated copy, its NULL refresh token kept"
    );
    assert_eq!(
        stored_session(&repos, clone, clone_tenant, SCIOTTE_GARMIN).await,
        (SESSION_ID.to_owned(), STORED_COOKIE.to_owned(), true),
        "the super admin's clone in another account is never written"
    );
}

/// A health sync's reader names no athlete, so when a super admin's clone
/// shares the session it read through, it cannot tell the row read from the
/// clone's: nothing is stored, and the clone's row is never written.
#[tokio::test]
async fn a_health_sync_never_writes_a_super_admin_clone_of_the_session() {
    let (repos, reader, reader_tenant, db) = setup().await;
    let (clone, clone_tenant) = seed_user(&db).await;
    for (user, tenant) in [(reader, reader_tenant), (clone, clone_tenant)] {
        repos
            .oauth_tokens
            .upsert_token(&session_row(user, tenant, SCIOTTE_GARMIN, STORED_COOKIE))
            .await
            .unwrap();
    }
    let (base, exports) = spawn_rotating_scraper(Arc::new(Mutex::new(None))).await;
    let storage = Arc::new(PierreSyncStorage::new(&repos));
    let orchestrator = storage
        .build_orchestrator(Some(SciotteClient::new(&base, None).unwrap()))
        .unwrap();

    // The clone's sync has been handed its row, as concurrent syncs of both
    // accounts are, when the reader's sync reads.
    storage
        .get_credentials(&clone.to_string(), "garmin")
        .await
        .unwrap()
        .expect("the clone holds the session");
    orchestrator
        .sync_user(&reader.to_string(), "garmin")
        .await
        .unwrap();

    assert!(
        *exports.lock().unwrap() > 0,
        "the rotated session is exported"
    );
    assert_eq!(
        stored_session(&repos, clone, clone_tenant, SCIOTTE_GARMIN)
            .await
            .1,
        STORED_COOKIE,
        "the clone's row is never written"
    );
    assert_eq!(
        stored_session(&repos, reader, reader_tenant, SCIOTTE_GARMIN)
            .await
            .1,
        STORED_COOKIE,
        "the row read cannot be told apart either, so nothing is stored"
    );
}
