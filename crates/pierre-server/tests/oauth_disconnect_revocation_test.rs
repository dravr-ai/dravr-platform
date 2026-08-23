// ABOUTME: E2E test for carnet#33 — disconnecting Strava revokes the grant upstream
// ABOUTME: Asserts the /oauth/revoke call (Basic auth + token), and that rows + cached activities die

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use base64::prelude::{Engine as _, BASE64_STANDARD};
use chrono::{Duration, Utc};
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserOAuthToken,
    UserStatus,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::OAuthService;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::spawn_blocking;
use tokio::time::timeout;
use uuid::Uuid;

use crate::common::create_test_server_resources;

/// Serve one canned 200 and hand back the raw request bytes, so the assertion
/// can look at exactly what the revocation put on the wire.
async fn capture_one_request() -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0_u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]).into_owned();
        let response = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = tx.send(request);
    });

    (format!("http://{addr}"), rx)
}

async fn seed_user_with_tenant(resources: &ServerContext) -> (Uuid, TenantId) {
    let password_hash =
        spawn_blocking(|| bcrypt::hash("Revoke123!", bcrypt::DEFAULT_COST).unwrap())
            .await
            .unwrap();
    let mut user = User::new(
        format!("revoke-{}@example.com", Uuid::new_v4()),
        password_hash,
        Some("Revoke User".to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Revocation Tenant".to_owned(),
        slug: format!("revoke-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    (user_id, tenant_id)
}

fn strava_ride(id: &str) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("ride {id}"),
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(30_000.0)
    .build()
}

/// Disconnecting Strava must (1) revoke the grant upstream — the June-2026
/// `/oauth/revoke` shape: HTTP Basic auth with the client credentials and the
/// stored refresh token as `token` — and (2) delete the token row, the
/// connection row, AND the provider-derived cached activities (§7.4's
/// deletion obligation). A disconnect that only deleted local rows was the
/// 'disconnect that revoked nothing' stub class (carnet#33).
#[tokio::test]
async fn disconnect_revokes_upstream_and_deletes_provider_data() {
    // Client credentials resolve through the same env fallback the real
    // authorize/exchange path uses; set them before resources exist. (The
    // revocation URL itself is injected via ServerConfig below — the harness
    // never reads STRAVA_REVOKE_URL.)
    let (revoke_url, captured) = capture_one_request().await;
    env::set_var("STRAVA_CLIENT_ID", "revoke-test-client");
    env::set_var("STRAVA_CLIENT_SECRET", "revoke-test-secret");

    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = seed_user_with_tenant(&resources).await;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: "strava".to_owned(),
            access_token: "access-material-do-not-log".to_owned(),
            refresh_token: Some("refresh-material-do-not-log".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(Utc::now() + Duration::hours(6)),
            scope: Some("read,activity:read_all".to_owned()),
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            user_id,
            &tenant_id,
            "strava",
            &[strava_ride("ride-revoke-1")],
        )
        .await
        .unwrap();

    // The test harness builds `ServerConfig::default()` (it never reads env),
    // so aim the revocation endpoint at the stub through the config the
    // service actually consults — the same knob STRAVA_REVOKE_URL turns in a
    // real deployment.
    let mut config = (*resources.common.config).clone();
    config.external_services.strava_api.revoke_url = revoke_url.clone();
    let oauth_service = OAuthService::new(
        resources.data(),
        Arc::new(config),
        resources.auth.oauth_notification_sender.clone(),
    );
    oauth_service
        .disconnect_provider(user_id, "strava", Some(tenant_id.as_uuid()))
        .await
        .expect("disconnect succeeds");

    // 1. The upstream revocation actually happened, in the new endpoint's
    //    shape. Basic auth of "revoke-test-client:revoke-test-secret".
    // Bounded wait: revocation is best-effort in production, so a skipped
    // call would otherwise leave this await pending forever instead of
    // failing with a reason.
    let request = timeout(StdDuration::from_secs(15), captured)
        .await
        .expect("no revocation request reached the stub within 15s — the best-effort path bailed (see WARN logs)")
        .expect("revocation request captured");
    let expected_basic = format!(
        "authorization: basic {}",
        BASE64_STANDARD
            .encode("revoke-test-client:revoke-test-secret")
            .to_ascii_lowercase()
    );
    let lower = request.to_ascii_lowercase();
    assert!(
        lower.contains(&expected_basic),
        "revocation must carry the client credentials as HTTP Basic auth; got:\n{request}"
    );
    assert!(
        request.contains("token=refresh-material-do-not-log"),
        "revocation must spend the stored refresh token; got:\n{request}"
    );
    assert!(
        request.contains("token_type_hint=refresh_token"),
        "revocation should hint the token type; got:\n{request}"
    );

    // 2. Local rows are gone.
    let token_after = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap();
    assert!(token_after.is_none(), "token row must be deleted");

    let connections = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        connections.iter().all(|c| c.provider != "strava"),
        "connection row must be deleted, got {connections:?}"
    );

    // 3. §7.4: provider-derived cached activities are gone too.
    let cached = resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("strava"),
            Utc::now() - Duration::days(30),
            Utc::now(),
            100,
        )
        .await
        .unwrap();
    assert!(
        cached.is_empty(),
        "cached Strava activities must be deleted on disconnect, got {} rows",
        cached.len()
    );

    env::remove_var("STRAVA_CLIENT_ID");
    env::remove_var("STRAVA_CLIENT_SECRET");
}
