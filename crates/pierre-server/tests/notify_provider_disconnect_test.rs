// ABOUTME: Asserts provider.disconnected fires from the domain chokepoint on every disconnect surface
// ABOUTME: Plus /mcp routing to the registry tool, which removes the provider_connections row too

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression tests for dravr-carnet#29.
//!
//! `provider.disconnected` was emitted only from the REST route, so a
//! disconnect through the chat tool loop or the `/mcp` surface was
//! invisible to `PostHog` — connects and disconnects were counted on different
//! surfaces. Worse, `/mcp` hand-rolled a raw-name token delete: it
//! never resolved the sciotte mirror backend and never removed the
//! `provider_connections` row, so a chat user who disconnected kept a row
//! claiming the provider was still connected.
//!
//! Every disconnect surface now funnels through
//! `OAuthService::disconnect_provider`. These tests drive each surface and
//! assert on the emitted event's fields AND on both storage rows, so a
//! regression that re-splits the paths (or re-orphans the connection row)
//! fails loudly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, only, NotifyEvent};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::mcp::tool_handlers::ToolHandlers;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_tool_runtime::implementations::connection::DisconnectProviderTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

use dravr_tronc::mcp::tool::{McpTool, ToolContext};

// ============================================================================
// Fixtures
// ============================================================================

async fn user_primary_tenant(resources: &Arc<ServerContext>, user_id: Uuid) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id
}

/// Seed a token row + a connection row for `backend`, exactly as the connect
/// paths write them.
async fn seed_connected_provider(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    connection_type: &ConnectionType,
) {
    let token = UserOAuthToken::new(
        user_id,
        tenant_id.to_string(),
        backend.to_owned(),
        "test_access_token".to_owned(),
        Some("test_refresh_token".to_owned()),
        Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        Some("read".to_owned()),
    );
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .expect("upsert test token");
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, backend, connection_type, None)
        .await
        .expect("register test connection");
}

/// Assert both sources of truth are gone for `backend`.
async fn assert_fully_disconnected(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    let token = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, backend)
        .await
        .unwrap();
    assert!(token.is_none(), "the {backend} token row must be deleted");

    let conns = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        !conns.iter().any(|c| c.provider == backend),
        "the {backend} connection row must be removed, not left orphaned"
    );
}

fn assert_event_attributed(
    event: &NotifyEvent,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    assert_eq!(event.field("provider"), provider);
    assert_eq!(event.field("user_id"), user_id.to_string());
    assert_eq!(event.field("tenant_id"), tenant_id.to_string());
}

/// A `provider.disconnected` the athlete caused: attributed, and saying so,
/// so it never counts with an operator's or the seat reclaimer's.
fn assert_athlete_disconnect(
    event: &NotifyEvent,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    assert_event_attributed(event, user_id, tenant_id, provider);
    assert_eq!(event.field("reason"), "athlete");
}

// ============================================================================
// Service chokepoint (REST path)
// ============================================================================

/// The service deletes the token + connection row in lockstep and emits a
/// fully-attributed `provider.disconnected`. A stale copy that skipped the
/// connection row, or an emit that relied on span fields, fails here.
#[tokio::test]
async fn service_disconnect_cleans_both_rows_and_emits() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_connected_provider(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::STRAVA,
        &ConnectionType::OAuth,
    )
    .await;

    let (events, _guard) = capture_notify();
    let service = OAuthService::new(resources.data(), resources.common.config.clone());
    service
        .disconnect_provider(
            user_id,
            oauth_providers::STRAVA,
            Some(tenant_id.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("disconnect must succeed");

    assert_fully_disconnected(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    let event = only(&events, "provider.disconnected");
    assert_athlete_disconnect(&event, user_id, tenant_id, oauth_providers::STRAVA);
}

// ============================================================================
// Chat tool loop (registry DisconnectProviderTool)
// ============================================================================

/// The chat tool resolves the sciotte mirror ("garmin" → `sciotte_garmin`),
/// cleans both rows for the mirror backend, and emits the event under the
/// user-facing provider name — the same name `provider.connected` uses, so
/// the connect/disconnect pair is measured on one axis.
#[tokio::test]
async fn chat_tool_disconnect_resolves_mirror_and_emits() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_connected_provider(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
        &ConnectionType::Manual,
    )
    .await;

    let (events, _guard) = capture_notify();
    let tool = DisconnectProviderTool;
    let state: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer");
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "garmin" }))
        .await;
    assert!(
        !result.is_error,
        "disconnect must succeed: {:?}",
        result.structured_content
    );

    assert_fully_disconnected(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    let event = only(&events, "provider.disconnected");
    assert_athlete_disconnect(&event, user_id, tenant_id, oauth_providers::GARMIN);
}

// ============================================================================
// The /mcp surface (dravr-carnet#29's stale-row path)
// ============================================================================

/// TrainingPeaks through the chat tool: `trainingpeaks` has no backend of its
/// own, so the disconnect must pass provider validation on its mirror, clean
/// both `sciotte_trainingpeaks` rows, and name the event for the provider the
/// athlete connected — the same name `provider.connected` carries.
#[tokio::test]
async fn chat_tool_disconnect_resolves_the_trainingpeaks_mirror_and_emits() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_connected_provider(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
        &ConnectionType::Manual,
    )
    .await;

    let (events, _guard) = capture_notify();
    let state: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer");
    let result = DisconnectProviderTool
        .execute(&state, &ctx, json!({ "provider": "trainingpeaks" }))
        .await;
    assert!(
        !result.is_error,
        "disconnect must succeed: {:?}",
        result.structured_content
    );

    assert_fully_disconnected(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;
    let event = only(&events, "provider.disconnected");
    assert_athlete_disconnect(&event, user_id, tenant_id, oauth_providers::TRAININGPEAKS);
}

/// `/mcp` used to answer `disconnect_provider` from a hand-rolled carve-out:
/// first a raw-named token delete with no connection-row removal and no event
/// (carnet#29), later a `{message, provider, tenant_id, success}` body with no
/// `structuredContent` for the advertised outputSchema (carnet#552). It now
/// routes to the registry tool, so the disconnect leaves no orphaned row,
/// emits the attributed event, and answers with the schema's structured part.
#[tokio::test]
async fn mcp_disconnect_routes_to_the_registry_tool_and_emits() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_connected_provider(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::STRAVA,
        &ConnectionType::OAuth,
    )
    .await;

    let (events, _guard) = capture_notify();
    let mut tool_context = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer");
    // The grant `/mcp` authenticated the caller with. The carve-out never
    // checked one; the registry tool requires `profile:write`.
    tool_context.scopes = vec!["profile:write".to_owned()];
    let state: Arc<dyn ToolRuntime> = resources.clone();
    let response = ToolHandlers::dispatch_tool_call(
        &resources,
        &state,
        &tool_context,
        user_id,
        tenant_id,
        "disconnect_provider",
        json!({ "provider": oauth_providers::STRAVA }),
    )
    .await;
    assert!(
        !response.is_error,
        "disconnect must succeed: {:?}",
        response.content
    );
    let structured = response
        .structured_content
        .as_ref()
        .expect("the outputSchema's structured part is present");
    assert_eq!(
        structured["provider"],
        oauth_providers::STRAVA,
        "the structured part names the provider: {structured}"
    );
    assert!(
        structured.get("tenant_id").is_none(),
        "the result no longer leaks the tenant id: {structured}"
    );

    assert_fully_disconnected(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    let event = only(&events, "provider.disconnected");
    assert_athlete_disconnect(&event, user_id, tenant_id, oauth_providers::STRAVA);
}

// ============================================================================
// The sciotte session routes (dravr-carnet#30's uncounted surface)
// ============================================================================

/// The scrape surface used to emit nothing on either side, so the sciotte
/// cohort was absent from analytics entirely. Both routes now emit the
/// catalogued pair under the user-facing provider name — the same axis the
/// OAuth surface counts on — with `backend` carrying the scrape cohort.
#[cfg(feature = "provider-sciotte")]
#[tokio::test]
async fn sciotte_session_routes_emit_the_provider_event_pair() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");
    let router = pierre_routes_auth::AuthRoutes::routes(resources.auth_routes_context());

    let (events, _guard) = capture_notify();

    let response = AxumTestRequest::post("/api/providers/sciotte/connect")
        .header("authorization", &auth_token)
        .json(&serde_json::json!({ "session_id": "test-scrape-session" }))
        .send(router.clone())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "connect must succeed"
    );

    let connected = only(&events, "provider.connected");
    assert_event_attributed(&connected, user_id, tenant_id, oauth_providers::STRAVA);
    assert_eq!(connected.field("backend"), oauth_providers::SCIOTTE);

    let token_row = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, oauth_providers::SCIOTTE)
        .await
        .unwrap();
    assert!(token_row.is_some(), "connect must persist the session row");

    let response = AxumTestRequest::delete("/api/providers/sciotte/disconnect")
        .header("authorization", &auth_token)
        .send(router)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::NO_CONTENT,
        "disconnect must succeed"
    );

    let disconnected = only(&events, "provider.disconnected");
    assert_athlete_disconnect(&disconnected, user_id, tenant_id, oauth_providers::STRAVA);
    assert_eq!(disconnected.field("backend"), oauth_providers::SCIOTTE);
    assert_fully_disconnected(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;
}
