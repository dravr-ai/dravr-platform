// ABOUTME: Integration tests for the hosted connect picker routes (page, oauth-init, success)
// ABOUTME: Pins connect-token gating (missing/invalid/narrow-scope), picker render, and the Strava-OAuth connected merge
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_link_token, MintProviderLinkTokenArgs,
};
use pierre_routes_auth::AuthRoutes;
use uuid::Uuid;

async fn test_setup() -> (Arc<ServerContext>, Uuid, TenantId) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _) = common::create_test_user(&resources.coach.database)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    (resources, user_id, tenant_id)
}

fn connect_token(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) -> String {
    mint_connect_link_token(
        user_id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint connect token")
}

/// `compute_providers_status` reads connected state from `provider_connections`
/// (the single source of truth), so the merge test registers a real connection
/// row rather than seeding a bare oauth token.
async fn register_connection(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, provider, &ConnectionType::OAuth, None)
        .await
        .expect("register test connection");
}

// ============================================================================
// GET /providers/connect — token gating + picker render
// ============================================================================

#[tokio::test]
async fn connect_page_without_token_renders_error_page() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/providers/connect").send(app).await;

    assert_eq!(resp.status(), 200, "error page is a friendly 200 HTML page");
    let body = resp.text();
    assert!(
        body.contains("Missing connect token"),
        "missing-token copy expected: {body}"
    );
    assert!(
        !body.contains("\"provider\""),
        "no provider cards may render without a token"
    );
}

#[tokio::test]
async fn connect_page_rejects_garbage_token() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/providers/connect?token=not-a-jwt")
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("invalid or has expired"),
        "invalid-token copy expected: {body}"
    );
    assert!(!body.contains("\"provider\""));
}

/// A narrow per-provider token (the Canot-minted `provider:sciotte:login`) must
/// never open the connect picker — only the connect superset scope may.
#[tokio::test]
async fn connect_page_rejects_narrow_sciotte_token() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let narrow = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();

    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&narrow)
    ))
    .send(app)
    .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("invalid or has expired"),
        "narrow scope must be rejected: {body}"
    );
    assert!(!body.contains("\"provider\""));
}

#[tokio::test]
async fn connect_page_renders_picker_for_valid_connect_token() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let token = connect_token(&resources, user_id, tenant_id);
    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(app)
    .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    // The cards are serialized as JSON into the page; the Strava-path and
    // Garmin-path cards must both be offered, and with no connection seeded
    // nothing may render as already connected.
    assert!(
        body.contains("\"target\":\"strava\""),
        "Strava-path card expected: {body}"
    );
    assert!(
        body.contains("\"target\":\"garmin\""),
        "Garmin-path card expected: {body}"
    );
    assert!(
        !body.contains("\"connected\":true"),
        "no card may show connected without a seeded connection"
    );
}

/// Regression: the raw `strava` OAuth row is hidden from the picker, but its
/// connected state must merge into the Strava-path card — an already-connected
/// user must not be offered a needless re-consent (web
/// `ProviderConnectionCards` parity).
#[tokio::test]
async fn connect_page_merges_strava_oauth_connection_into_card() {
    let (resources, user_id, tenant_id) = test_setup().await;
    register_connection(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let token = connect_token(&resources, user_id, tenant_id);
    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(app)
    .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("\"connected\":true"),
        "the Strava-path card must show connected when a strava OAuth row exists: {body}"
    );
}

// ============================================================================
// GET /api/providers/connect/oauth-init/{provider} — token gating
// ============================================================================

#[tokio::test]
async fn oauth_init_rejects_missing_token() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/api/providers/connect/oauth-init/strava")
        .send(app)
        .await;

    assert_ne!(resp.status(), 302, "no redirect without a token");
    assert!(
        resp.status() >= 400,
        "missing token must be an auth error, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn oauth_init_rejects_narrow_sciotte_token() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let narrow = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();

    let resp = AxumTestRequest::get(&format!(
        "/api/providers/connect/oauth-init/strava?token={}",
        urlencoding::encode(&narrow)
    ))
    .send(app)
    .await;

    assert_ne!(resp.status(), 302, "narrow scope must not initiate OAuth");
    assert!(
        resp.status() >= 400,
        "narrow token must be an auth error, got {}",
        resp.status()
    );
}

/// An unknown provider path param must never 302 — the provider registry is
/// the allowlist, and the failure renders the friendly error page.
#[tokio::test]
async fn oauth_init_rejects_unknown_provider() {
    let (resources, user_id, tenant_id) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let token = connect_token(&resources, user_id, tenant_id);
    let resp = AxumTestRequest::get(&format!(
        "/api/providers/connect/oauth-init/not_a_provider?token={}",
        urlencoding::encode(&token)
    ))
    .send(app)
    .await;

    assert_ne!(resp.status(), 302, "unknown provider must not redirect");
    let body = resp.text();
    assert!(
        body.contains("couldn't start the connection") || body.contains("error"),
        "unknown provider renders the error path: {body}"
    );
}

// ============================================================================
// GET /providers/connect/success
// ============================================================================

#[tokio::test]
async fn success_page_renders_with_channel_and_target() {
    let (resources, _, _) = test_setup().await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/providers/connect/success?channel=telegram&target=strava")
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.to_lowercase().contains("telegram"),
        "success page should reference the originating channel: {body}"
    );
}
