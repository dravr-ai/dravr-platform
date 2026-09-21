// ABOUTME: Covers the OAuth 2.0 server repository against whichever backend DATABASE_URL names
// ABOUTME: Clients, single-use auth codes, rotated refresh tokens, single-use CSRF states, grants and device codes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The OAuth 2.0 server's persistence is where the RFC 6749 single-use
//! guarantees live: an authorization code is exchanged once and only for the
//! client and redirect it was minted for (§4.1.2, §10.5), a refresh token
//! rotates once for its client, and a CSRF state is redeemed once (§10.12).
//! Every one of those is a single `UPDATE … RETURNING` statement, and both
//! backends must agree on every column that comes back.
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, Utc};
use pierre_core::models::{
    DeviceAuthorization, OAuth2AuthCode, OAuth2Client, OAuth2RefreshToken, OAuth2State,
    OAuthClientGrant,
};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Register a distinct client; the code, token and state tables all reference it.
async fn fresh_client(repos: &RepositoryRegistry) -> OAuth2Client {
    let client = OAuth2Client {
        id: Uuid::new_v4().to_string(),
        client_id: format!("client-{}", Uuid::new_v4()),
        client_secret_hash: "argon2-hash-placeholder".to_owned(),
        redirect_uris: vec![
            "https://app.example/callback".to_owned(),
            "http://localhost:3000/callback".to_owned(),
        ],
        grant_types: vec!["authorization_code".to_owned(), "refresh_token".to_owned()],
        response_types: vec!["code".to_owned()],
        client_name: Some("Repository Test Client".to_owned()),
        client_uri: None,
        scope: Some("read write".to_owned()),
        created_at: Utc::now(),
        expires_at: None,
    };
    repos.oauth2_server.store_client(&client).await.unwrap();
    client
}

#[tokio::test]
async fn a_registered_client_reads_back_with_its_lists() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(&repos).await;

    let stored = repos
        .oauth2_server
        .get_client(&client.client_id)
        .await
        .unwrap()
        .expect("the client just registered must read back");
    assert_eq!(stored.id, client.id);
    assert_eq!(stored.client_secret_hash, "argon2-hash-placeholder");
    assert_eq!(stored.redirect_uris, client.redirect_uris);
    assert_eq!(stored.grant_types, client.grant_types);
    assert_eq!(stored.response_types, vec!["code".to_owned()]);
    assert_eq!(
        stored.client_name.as_deref(),
        Some("Repository Test Client")
    );
    assert_eq!(stored.client_uri, None);
    assert_eq!(stored.scope.as_deref(), Some("read write"));
    assert_eq!(stored.expires_at, None);

    assert!(
        repos
            .oauth2_server
            .get_client("no-such-client")
            .await
            .unwrap()
            .is_none(),
        "an unknown client_id is None, not an error"
    );
}

#[tokio::test]
async fn an_auth_code_is_exchanged_once_and_only_by_its_client_and_redirect() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(&repos).await;
    let user_id = Uuid::new_v4();
    let code = OAuth2AuthCode {
        code: format!("code-{}", Uuid::new_v4()),
        client_id: client.client_id.clone(),
        user_id,
        tenant_id: "tenant-a".to_owned(),
        redirect_uri: "https://app.example/callback".to_owned(),
        scope: Some("read".to_owned()),
        expires_at: Utc::now() + Duration::minutes(10),
        used: false,
        state: Some("xyz".to_owned()),
        code_challenge: Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_owned()),
        code_challenge_method: Some("S256".to_owned()),
    };
    repos.oauth2_server.store_auth_code(&code).await.unwrap();

    assert!(
        repos
            .oauth2_server
            .consume_auth_code(
                &code.code,
                &client.client_id,
                "https://elsewhere.example/callback",
                Utc::now()
            )
            .await
            .unwrap()
            .is_none(),
        "a redirect_uri other than the one the code was issued for matches nothing"
    );
    assert!(
        repos
            .oauth2_server
            .consume_auth_code(&code.code, "another-client", &code.redirect_uri, Utc::now())
            .await
            .unwrap()
            .is_none(),
        "another client cannot exchange the code"
    );

    let consumed = repos
        .oauth2_server
        .consume_auth_code(
            &code.code,
            &client.client_id,
            &code.redirect_uri,
            Utc::now(),
        )
        .await
        .unwrap()
        .expect("the right client with the right redirect exchanges the code");
    assert_eq!(consumed.user_id, user_id);
    assert_eq!(consumed.tenant_id, "tenant-a");
    assert_eq!(consumed.scope.as_deref(), Some("read"));
    assert_eq!(consumed.state.as_deref(), Some("xyz"));
    assert_eq!(
        consumed.code_challenge.as_deref(),
        Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
    );
    assert_eq!(consumed.code_challenge_method.as_deref(), Some("S256"));
    assert!(consumed.used, "the returned row is the row after the flip");
    assert_eq!(
        consumed.expires_at.timestamp(),
        code.expires_at.timestamp(),
        "expires_at round-trips to the second"
    );

    assert!(
        repos
            .oauth2_server
            .consume_auth_code(
                &code.code,
                &client.client_id,
                &code.redirect_uri,
                Utc::now()
            )
            .await
            .unwrap()
            .is_none(),
        "a second exchange of the same code matches nothing"
    );

    let expired = OAuth2AuthCode {
        code: format!("code-{}", Uuid::new_v4()),
        expires_at: Utc::now() - Duration::seconds(1),
        ..code
    };
    repos.oauth2_server.store_auth_code(&expired).await.unwrap();
    assert!(
        repos
            .oauth2_server
            .consume_auth_code(
                &expired.code,
                &client.client_id,
                &expired.redirect_uri,
                Utc::now()
            )
            .await
            .unwrap()
            .is_none(),
        "an expired code is never exchanged"
    );
}

#[tokio::test]
async fn a_refresh_token_rotates_once_for_its_client_and_is_stored_hashed() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(&repos).await;
    let user_id = Uuid::new_v4();
    let raw = format!("rt-{}", Uuid::new_v4());
    let token = OAuth2RefreshToken {
        token: raw.clone(),
        client_id: client.client_id.clone(),
        user_id,
        tenant_id: "tenant-b".to_owned(),
        scope: Some("read write".to_owned()),
        expires_at: Utc::now() + Duration::days(30),
        created_at: Utc::now(),
        revoked: false,
    };
    repos
        .oauth2_server
        .store_refresh_token(&token)
        .await
        .unwrap();

    let stored = repos
        .oauth2_server
        .get_refresh_token_by_value(&raw)
        .await
        .unwrap()
        .expect("the token reads back by its raw value");
    assert_ne!(stored.token, raw, "only the HMAC of the token is stored");
    assert_eq!(stored.user_id, user_id);
    assert_eq!(stored.tenant_id, "tenant-b");
    assert_eq!(stored.scope.as_deref(), Some("read write"));
    assert!(!stored.revoked);

    assert!(
        repos
            .oauth2_server
            .consume_refresh_token(&raw, "another-client", Utc::now())
            .await
            .unwrap()
            .is_none(),
        "another client cannot rotate the token"
    );

    let rotated = repos
        .oauth2_server
        .consume_refresh_token(&raw, &client.client_id, Utc::now())
        .await
        .unwrap()
        .expect("the owning client rotates the token");
    assert!(
        rotated.revoked,
        "the returned row is the row after the flip"
    );
    assert_eq!(rotated.user_id, user_id);
    assert_eq!(rotated.client_id, client.client_id);

    assert!(
        repos
            .oauth2_server
            .consume_refresh_token(&raw, &client.client_id, Utc::now())
            .await
            .unwrap()
            .is_none(),
        "a replayed refresh token matches nothing"
    );
    assert!(
        repos
            .oauth2_server
            .get_refresh_token_by_value(&raw)
            .await
            .unwrap()
            .expect("a revoked token still reads back")
            .revoked
    );
}

#[tokio::test]
async fn a_csrf_state_is_redeemed_once_for_its_client_with_its_pkce_pair() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(&repos).await;
    let user_id = Uuid::new_v4();
    let state = OAuth2State {
        state: format!("state-{}", Uuid::new_v4()),
        client_id: client.client_id.clone(),
        user_id: Some(user_id),
        tenant_id: Some("tenant-c".to_owned()),
        redirect_uri: "https://app.example/callback".to_owned(),
        scope: Some("read".to_owned()),
        code_challenge: Some("challenge".to_owned()),
        code_challenge_method: Some("S256".to_owned()),
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::minutes(10),
        used: false,
    };
    repos.oauth2_server.store_state(&state).await.unwrap();

    assert!(
        repos
            .oauth2_server
            .consume_state(&state.state, "another-client", Utc::now())
            .await
            .unwrap()
            .is_none(),
        "a state is bound to the client it was minted for"
    );

    let consumed = repos
        .oauth2_server
        .consume_state(&state.state, &client.client_id, Utc::now())
        .await
        .unwrap()
        .expect("the owning client redeems the state");
    assert_eq!(consumed.user_id, Some(user_id));
    assert_eq!(consumed.tenant_id.as_deref(), Some("tenant-c"));
    assert_eq!(consumed.redirect_uri, "https://app.example/callback");
    assert_eq!(consumed.code_challenge.as_deref(), Some("challenge"));
    assert_eq!(consumed.code_challenge_method.as_deref(), Some("S256"));
    assert!(consumed.used);

    assert!(
        repos
            .oauth2_server
            .consume_state(&state.state, &client.client_id, Utc::now())
            .await
            .unwrap()
            .is_none(),
        "a replayed state matches nothing"
    );

    let anonymous = OAuth2State {
        state: format!("state-{}", Uuid::new_v4()),
        user_id: None,
        tenant_id: None,
        ..state
    };
    repos.oauth2_server.store_state(&anonymous).await.unwrap();
    let consumed = repos
        .oauth2_server
        .consume_state(&anonymous.state, &client.client_id, Utc::now())
        .await
        .unwrap()
        .expect("a state minted before the user is known still redeems");
    assert_eq!(consumed.user_id, None);
    assert_eq!(consumed.tenant_id, None);
}

#[tokio::test]
async fn a_client_grant_is_idempotent_and_revocable_by_its_owner_only() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let client = fresh_client(&repos).await;
    let user_id = Uuid::new_v4().to_string();
    let grant = OAuthClientGrant {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.clone(),
        tenant_id: "tenant-d".to_owned(),
        client_id: client.client_id.clone(),
        scope: "read".to_owned(),
        granted_at: Utc::now(),
        revoked_at: None,
    };
    repos
        .oauth2_server
        .store_client_grant(&grant)
        .await
        .unwrap();
    let again = OAuthClientGrant {
        id: Uuid::new_v4().to_string(),
        ..grant.clone()
    };
    repos
        .oauth2_server
        .store_client_grant(&again)
        .await
        .unwrap();

    let listed = repos
        .oauth2_server
        .list_client_grants(&user_id, "tenant-d")
        .await
        .unwrap();
    assert_eq!(listed.len(), 1, "re-consent is a no-op, not a second grant");
    assert_eq!(listed[0].id, grant.id, "the first grant is the one kept");
    assert_eq!(listed[0].client_id, client.client_id);
    assert_eq!(listed[0].scope, "read");
    assert_eq!(listed[0].revoked_at, None);
    assert!(
        (listed[0].granted_at - Utc::now()).num_seconds().abs() < 60,
        "granted_at is stamped by the database, got {}",
        listed[0].granted_at
    );

    let found = repos
        .oauth2_server
        .find_active_client_grant(&user_id, "tenant-d", &client.client_id, "read")
        .await
        .unwrap()
        .expect("the active grant is found by its four-part key");
    assert_eq!(found.id, grant.id);
    assert!(
        repos
            .oauth2_server
            .find_active_client_grant(&user_id, "tenant-d", &client.client_id, "write")
            .await
            .unwrap()
            .is_none(),
        "a different scope is a different grant"
    );

    assert!(
        !repos
            .oauth2_server
            .revoke_client_grant(&grant.id, "someone-else", "tenant-d")
            .await
            .unwrap(),
        "another user cannot revoke the grant"
    );
    assert!(
        repos
            .oauth2_server
            .revoke_client_grant(&grant.id, &user_id, "tenant-d")
            .await
            .unwrap(),
        "the owner revokes it"
    );
    assert!(
        !repos
            .oauth2_server
            .revoke_client_grant(&grant.id, &user_id, "tenant-d")
            .await
            .unwrap(),
        "revoking twice changes nothing"
    );
    assert!(repos
        .oauth2_server
        .find_active_client_grant(&user_id, "tenant-d", &client.client_id, "read")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_device_authorization_is_approved_once_and_consumed_once() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let now = Utc::now().timestamp();
    let da = DeviceAuthorization {
        device_code_hash: format!("hash-{}", Uuid::new_v4()),
        user_code: format!("UC-{}", &Uuid::new_v4().to_string()[..8]),
        status: "pending".to_owned(),
        approved_by: None,
        created_at: now,
        expires_at: now + 600,
    };
    repos
        .oauth2_server
        .create_device_authorization(&da)
        .await
        .unwrap();

    let pending = repos
        .oauth2_server
        .get_device_authorization_by_code_hash(&da.device_code_hash)
        .await
        .unwrap()
        .expect("the authorization reads back by its device code hash");
    assert_eq!(pending.user_code, da.user_code);
    assert_eq!(pending.status, "pending");
    assert_eq!(pending.approved_by, None);
    assert_eq!(pending.created_at, now);
    assert_eq!(pending.expires_at, now + 600);

    assert!(repos
        .oauth2_server
        .approve_device_authorization(&da.user_code, "admin@example.com")
        .await
        .unwrap());
    assert!(
        !repos
            .oauth2_server
            .approve_device_authorization(&da.user_code, "someone@example.com")
            .await
            .unwrap(),
        "an approved authorization is no longer pending"
    );
    assert!(
        !repos
            .oauth2_server
            .deny_device_authorization(&da.user_code)
            .await
            .unwrap(),
        "nor can it be denied afterwards"
    );

    let approved = repos
        .oauth2_server
        .get_device_authorization_by_user_code(&da.user_code)
        .await
        .unwrap()
        .expect("the authorization reads back by its user code");
    assert_eq!(approved.status, "approved");
    assert_eq!(approved.approved_by.as_deref(), Some("admin@example.com"));
    assert_eq!(approved.device_code_hash, da.device_code_hash);

    assert!(repos
        .oauth2_server
        .delete_device_authorization(&da.device_code_hash)
        .await
        .unwrap());
    assert!(
        !repos
            .oauth2_server
            .delete_device_authorization(&da.device_code_hash)
            .await
            .unwrap(),
        "a second poll cannot consume it again"
    );
    assert!(repos
        .oauth2_server
        .get_device_authorization_by_user_code(&da.user_code)
        .await
        .unwrap()
        .is_none());
}
