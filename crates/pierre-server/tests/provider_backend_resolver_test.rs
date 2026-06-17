// ABOUTME: Integration tests for the provider backend_resolver and connection-status coalescing
// ABOUTME: Verifies sciotte* backends are hidden from LLM output and gate OAuth reconnect per provider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Tests that exercise the real `backend_resolver` + `GetConnectionStatusTool`
//! + `ConnectProviderTool` execution paths against a seeded test database.
//!
//! What we assert:
//!
//! 1. Unit-level resolver behaviour (sync helpers).
//! 2. `get_connection_status` multi-provider output hides sciotte / `sciotte_garmin`
//!    completely and coalesces their presence into the user-facing provider
//!    with `backend: "mirror"`.
//! 3. `get_connection_status` rejects an explicit query for "sciotte" so the
//!    LLM cannot probe the internal backend name.
//! 4. `connect_provider(garmin)` returns an error when the user has already
//!    opted into the sciotte mirror (Garmin stays on the scraper), while
//!    `connect_provider(strava)` is allowed through so a sciotte-Strava user
//!    can migrate to OAuth.

mod common;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::backend_resolver::{self, BackendKind, CoalescedStatus};
use pierre_tool_runtime::context::{AuthMethod, ToolExecutionContext};
use pierre_tool_runtime::implementations::connection::{
    ConnectProviderTool, GetConnectionStatusTool,
};
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::protocol::types::META_AUTH_REQUIRED_PROVIDER;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::traits::McpTool;
use serde_json::json;
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

async fn seed_token(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    let token = UserOAuthToken::new(
        user_id,
        tenant_id.to_string(),
        provider.to_owned(),
        "test_access_token".to_owned(),
        Some("test_refresh_token".to_owned()),
        // Sciotte rows have no expiry in production; tokens we insert here
        // are wall-clock valid so the resolver sees them as present.
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
}

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

fn tool_context(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> ToolExecutionContext {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    ToolExecutionContext::new(user_id, Some(tenant_id), runtime, AuthMethod::JwtBearer)
}

// ============================================================================
// Unit-level resolver behaviour
// ============================================================================

#[test]
fn user_facing_name_strips_mirror_backends() {
    assert_eq!(backend_resolver::user_facing_name("sciotte"), "strava");
    assert_eq!(
        backend_resolver::user_facing_name("sciotte_garmin"),
        "garmin"
    );
    assert_eq!(backend_resolver::user_facing_name("strava"), "strava");
    assert_eq!(backend_resolver::user_facing_name("fitbit"), "fitbit");
}

#[test]
fn mirror_backend_for_only_maps_strava_and_garmin() {
    assert_eq!(
        backend_resolver::mirror_backend_for("strava"),
        Some(oauth_providers::SCIOTTE)
    );
    assert_eq!(
        backend_resolver::mirror_backend_for("garmin"),
        Some(oauth_providers::SCIOTTE_GARMIN)
    );
    assert_eq!(backend_resolver::mirror_backend_for("fitbit"), None);
    assert_eq!(backend_resolver::mirror_backend_for("whoop"), None);
}

#[test]
fn is_mirror_backend_identifies_internal_names() {
    assert!(backend_resolver::is_mirror_backend("sciotte"));
    assert!(backend_resolver::is_mirror_backend("sciotte_garmin"));
    assert!(!backend_resolver::is_mirror_backend("strava"));
    assert!(!backend_resolver::is_mirror_backend("garmin"));
}

#[test]
fn backend_kind_strings_round_trip() {
    assert_eq!(BackendKind::None.as_str(), "none");
    assert_eq!(BackendKind::Oauth.as_str(), "oauth");
    assert_eq!(BackendKind::Mirror.as_str(), "mirror");
}

// ============================================================================
// Resolver integration (DB-backed)
// ============================================================================

#[tokio::test]
async fn resolve_backend_prefers_sciotte_when_row_present() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Only the mirror row exists → resolver picks sciotte
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos.auth_repos(),
        user_id,
        Some(tenant_id),
        oauth_providers::STRAVA,
    )
    .await;
    assert_eq!(resolved, oauth_providers::SCIOTTE);
}

#[tokio::test]
async fn resolve_backend_keeps_oauth_when_only_oauth_row_exists() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos.auth_repos(),
        user_id,
        Some(tenant_id),
        oauth_providers::STRAVA,
    )
    .await;
    assert_eq!(resolved, oauth_providers::STRAVA);
}

#[tokio::test]
async fn resolve_backend_strava_prefers_oauth_when_both_rows_exist() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Both rows: for Strava the OAuth backend wins (Strava → OAuth-API
    // migration). The sciotte row goes dormant; the user is served from the API.
    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos.auth_repos(),
        user_id,
        Some(tenant_id),
        oauth_providers::STRAVA,
    )
    .await;
    assert_eq!(resolved, oauth_providers::STRAVA);
}

#[tokio::test]
async fn resolve_backend_garmin_prefers_mirror_when_both_rows_exist() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Both rows: for Garmin the mirror still wins — Garmin's official API is
    // partner-gated, so it stays on the scraper (no OAuth-API migration).
    seed_token(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;
    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos.auth_repos(),
        user_id,
        Some(tenant_id),
        oauth_providers::GARMIN,
    )
    .await;
    assert_eq!(resolved, oauth_providers::SCIOTTE_GARMIN);
}

#[tokio::test]
async fn coalesced_status_reports_mirror_backend_when_sciotte_present() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let status = backend_resolver::coalesced_status(
        &resources.common.repos.auth_repos(),
        user_id,
        tenant_id,
        oauth_providers::STRAVA,
    )
    .await;
    assert_eq!(
        status,
        CoalescedStatus {
            user_facing: oauth_providers::STRAVA,
            connected: true,
            backend_kind: BackendKind::Mirror,
        }
    );
}

// ============================================================================
// GetConnectionStatusTool — multi-provider mode
// ============================================================================

#[tokio::test]
async fn multi_provider_status_hides_sciotte_entries() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // User has a sciotte row but no OAuth row.
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let tool = GetConnectionStatusTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool.execute(json!({}), &ctx).await.unwrap();

    let data = result.content;
    let providers = data
        .get("providers")
        .and_then(|v| v.as_object())
        .expect("providers map in response");

    // Mirror backends are never visible to the LLM.
    assert!(
        !providers.contains_key(oauth_providers::SCIOTTE),
        "sciotte must not appear in multi-provider output"
    );
    assert!(
        !providers.contains_key(oauth_providers::SCIOTTE_GARMIN),
        "sciotte_garmin must not appear in multi-provider output"
    );

    // Strava is reported as connected via the mirror backend.
    let strava = providers
        .get(oauth_providers::STRAVA)
        .expect("strava entry present");
    assert_eq!(
        strava.get("connected").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        strava.get("backend").and_then(|v| v.as_str()),
        Some("mirror")
    );

    // Garmin (no row either way) is reported as disconnected / none.
    let garmin = providers
        .get(oauth_providers::GARMIN)
        .expect("garmin entry present");
    assert_eq!(
        garmin.get("connected").and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(garmin.get("backend").and_then(|v| v.as_str()), Some("none"));
}

#[tokio::test]
async fn multi_provider_status_reports_oauth_backend_when_no_mirror() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;

    let tool = GetConnectionStatusTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool.execute(json!({}), &ctx).await.unwrap();

    let providers = result
        .content
        .get("providers")
        .and_then(|v| v.as_object())
        .unwrap()
        .clone();
    let strava = providers.get(oauth_providers::STRAVA).unwrap();
    assert_eq!(
        strava.get("connected").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        strava.get("backend").and_then(|v| v.as_str()),
        Some("oauth")
    );
}

#[tokio::test]
async fn multi_provider_status_strava_prefers_oauth_when_both_rows_exist() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let tool = GetConnectionStatusTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool.execute(json!({}), &ctx).await.unwrap();

    let providers = result
        .content
        .get("providers")
        .and_then(|v| v.as_object())
        .unwrap()
        .clone();
    let strava = providers.get(oauth_providers::STRAVA).unwrap();
    assert_eq!(
        strava.get("connected").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    // Status must match routing: Strava with an OAuth token reports OAuth, not
    // the dormant sciotte mirror (Strava → OAuth-API migration).
    assert_eq!(
        strava.get("backend").and_then(|v| v.as_str()),
        Some("oauth"),
        "oauth must win for Strava when both rows exist"
    );
}

#[tokio::test]
async fn multi_provider_status_garmin_prefers_mirror_when_both_rows_exist() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;
    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    let tool = GetConnectionStatusTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool.execute(json!({}), &ctx).await.unwrap();

    let providers = result
        .content
        .get("providers")
        .and_then(|v| v.as_object())
        .unwrap()
        .clone();
    let garmin = providers.get(oauth_providers::GARMIN).unwrap();
    assert_eq!(
        garmin.get("connected").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        garmin.get("backend").and_then(|v| v.as_str()),
        Some("mirror"),
        "mirror must win for Garmin when both rows exist"
    );
}

// ============================================================================
// GetConnectionStatusTool — single-provider mode
// ============================================================================

#[tokio::test]
async fn single_provider_status_rejects_explicit_sciotte_query() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Even if a sciotte row exists, the tool must refuse to confirm it
    // under the internal name — the LLM should use "strava".
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let tool = GetConnectionStatusTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool
        .execute(json!({ "provider": "sciotte" }), &ctx)
        .await
        .unwrap();

    let data = result.content;
    assert_eq!(
        data.get("connected").and_then(serde_json::Value::as_bool),
        Some(false),
        "sciotte must be reported as disconnected when queried by its internal name"
    );
    let note = data.get("note").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        note.contains("strava"),
        "note should redirect the caller to the user-facing provider"
    );
}

#[tokio::test]
async fn single_provider_status_reports_mirror_backend_when_present() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let tool = GetConnectionStatusTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool
        .execute(json!({ "provider": "strava" }), &ctx)
        .await
        .unwrap();

    let data = result.content;
    assert_eq!(
        data.get("connected").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(data.get("backend").and_then(|v| v.as_str()), Some("mirror"));
}

// ============================================================================
// GetConnectionStatusTool — needs_reauth surfacing
// ============================================================================

#[tokio::test]
async fn single_provider_status_reports_needs_reauth_after_refresh_failure() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // WHOOP has a token row (so the resolver reads it as connected) plus a
    // provider_connections row that a dead refresh flipped to needs_reauth.
    seed_token(&resources, user_id, tenant_id, oauth_providers::WHOOP).await;
    let pc = &resources.common.repos.provider_connections;
    pc.register_connection(
        user_id,
        tenant_id,
        oauth_providers::WHOOP,
        &ConnectionType::OAuth,
        None,
    )
    .await
    .unwrap();
    pc.mark_needs_reauth(
        user_id,
        tenant_id,
        oauth_providers::WHOOP,
        Some("invalid_request"),
    )
    .await
    .unwrap();

    let tool = GetConnectionStatusTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool
        .execute(json!({ "provider": "whoop" }), &ctx)
        .await
        .unwrap();

    let data = result.content;
    assert_eq!(
        data.get("status").and_then(|v| v.as_str()),
        Some("needs_reauth"),
        "a connected provider whose refresh died must report needs_reauth, not connected"
    );
    assert_eq!(
        data.get("needs_reauth")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[tokio::test]
async fn multi_provider_status_reports_needs_reauth() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::WHOOP).await;
    let pc = &resources.common.repos.provider_connections;
    pc.register_connection(
        user_id,
        tenant_id,
        oauth_providers::WHOOP,
        &ConnectionType::OAuth,
        None,
    )
    .await
    .unwrap();
    pc.mark_needs_reauth(
        user_id,
        tenant_id,
        oauth_providers::WHOOP,
        Some("invalid_request"),
    )
    .await
    .unwrap();

    let tool = GetConnectionStatusTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool.execute(json!({}), &ctx).await.unwrap();

    let providers = result
        .content
        .get("providers")
        .and_then(|v| v.as_object())
        .unwrap()
        .clone();
    let whoop = providers
        .get(oauth_providers::WHOOP)
        .expect("whoop entry present");
    assert_eq!(
        whoop.get("status").and_then(|v| v.as_str()),
        Some("needs_reauth")
    );
    assert_eq!(
        whoop
            .get("needs_reauth")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

// ============================================================================
// ConnectProviderTool — mirror re-auth block
// ============================================================================

#[tokio::test]
async fn connect_provider_blocks_oauth_for_garmin_when_mirror_active() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    let tool = ConnectProviderTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool
        .execute(json!({ "provider": "garmin" }), &ctx)
        .await
        .unwrap();

    // Garmin keeps the mirror block: its official API is partner-gated, so the
    // connector must refuse to mint an OAuth URL while the mirror is active.
    assert!(
        result.is_error,
        "connect_provider(garmin) must fail rather than return an OAuth URL when the mirror is active"
    );
    let err_text = result
        .content
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        err_text.to_lowercase().contains("direct login")
            || err_text.to_lowercase().contains("mirror"),
        "error message should explain the mirror backend is active: {err_text}"
    );
    assert!(
        !err_text.contains("sciotte"),
        "error message must not leak the internal backend name: {err_text}"
    );
}

#[tokio::test]
async fn connect_provider_allows_oauth_for_strava_when_mirror_active() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // A sciotte-Strava user authorizing OAuth: the block is lifted for Strava
    // (Strava → OAuth-API migration), so the connector mints an OAuth URL
    // instead of steering them back to the mirror flow.
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let tool = ConnectProviderTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool
        .execute(json!({ "provider": "strava" }), &ctx)
        .await
        .unwrap();

    // The mirror block must be lifted for Strava: the connector reaches the
    // OAuth-minting stage instead of steering the user back to the mirror flow.
    // (Minting itself may still fail downstream on tenant OAuth credentials —
    // an orthogonal concern — so we assert the block is gone, not end-to-end
    // success.) If the block were still active the response would carry
    // `requires_mirror_reauth` and a "direct login" steer.
    assert!(
        result.content.get("requires_mirror_reauth").is_none(),
        "Strava connect must not return the mirror-reauth block: {:?}",
        result.content
    );
    let err_text = result
        .content
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        !err_text.to_lowercase().contains("direct login"),
        "Strava connect must not be steered back to the mirror flow: {err_text}"
    );
}

#[tokio::test]
async fn connect_provider_rejects_explicit_sciotte_name() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    let tool = ConnectProviderTool;
    let ctx = tool_context(&resources, user_id, tenant_id);
    let result = tool
        .execute(json!({ "provider": "sciotte" }), &ctx)
        .await
        .unwrap();

    assert!(result.is_error);
    let err_text = result
        .content
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        err_text.to_lowercase().contains("unknown") || err_text.contains("strava"),
        "error should steer caller to the user-facing provider: {err_text}"
    );
}

// ============================================================================
// create_authenticated_provider — needs_reauth short-circuit signal
// ============================================================================

/// A provider whose connection died (`needs_reauth`) makes `create_authenticated_provider`
/// tag its failure with `META_AUTH_REQUIRED_PROVIDER`, so the chat tool loop short-circuits
/// the turn and the `auth_recovery` stage renders a localized reconnect message + minted link
/// instead of letting the LLM rephrase a generic "no token" error.
#[tokio::test]
async fn create_authenticated_provider_signals_reauth_for_dead_connection() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    let pc = &resources.common.repos.provider_connections;
    pc.register_connection(
        user_id,
        tenant_id,
        oauth_providers::WHOOP,
        &ConnectionType::OAuth,
        None,
    )
    .await
    .unwrap();
    pc.mark_needs_reauth(
        user_id,
        tenant_id,
        oauth_providers::WHOOP,
        Some("invalid_request"),
    )
    .await
    .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let auth_service = AuthService::new(runtime);
    let tenant_str = tenant_id.to_string();
    let result = auth_service
        .create_authenticated_provider(oauth_providers::WHOOP, user_id, Some(&tenant_str))
        .await;

    let response = result
        .err()
        .expect("a dead connection must fail provider creation");
    let signalled = response
        .metadata
        .and_then(|m| m.get(META_AUTH_REQUIRED_PROVIDER).cloned());
    assert_eq!(
        signalled,
        Some(json!(oauth_providers::WHOOP)),
        "create_authenticated_provider must tag the auth-required provider for the recovery stage"
    );
}
