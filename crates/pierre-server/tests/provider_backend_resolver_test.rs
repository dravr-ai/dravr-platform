// ABOUTME: Integration tests for the provider backend_resolver and connection-status coalescing
// ABOUTME: Verifies sciotte* backends are hidden from LLM-visible output and block OAuth reconnect
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
//! 4. `connect_provider(strava)` returns an error when the user has already
//!    opted into the sciotte mirror — protecting the "stay with sciotte even
//!    if expired" invariant.

mod common;

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user};
use pierre_cache::{CacheKey, CacheResource};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{Athlete, ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::backend_resolver::{self, BackendKind, CoalescedStatus};
use pierre_tool_runtime::implementations::connection::{
    ConnectProviderTool, DisconnectProviderTool, GetConnectionStatusTool,
};
use pierre_tool_runtime::implementations::data::GetAthleteTool;
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::protocol::types::META_AUTH_REQUIRED_PROVIDER;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};

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

/// Shared runtime façade handed to a tool's `execute`.
fn tool_state(resources: &Arc<ServerContext>) -> Arc<dyn ToolRuntime> {
    resources.clone()
}

/// Per-call tronc context the MCP dispatch layer would resolve.
fn tool_context(user_id: Uuid, tenant_id: TenantId) -> ToolContext {
    ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer")
}

/// Extract the structured JSON payload a connection tool emits.
///
/// The connection tools build their result with `ToolResult::ok`/`error`,
/// which `tool_result_to_response` carries through as `structuredContent`.
fn structured(response: &ToolResponse) -> &Value {
    response
        .structured_content
        .as_ref()
        .expect("connection tool result carries structured content")
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

    // Both rows: for Strava the OAuth backend wins (Strava → OAuth-API migration).
    // The sciotte row goes dormant; the user is served from the API.
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
async fn resolve_backend_garmin_stays_on_mirror_when_no_token() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // No sciotte_garmin token row at all (e.g. after an orphaned-row disconnect).
    // Garmin must STILL resolve to its mirror — never fall through to the raw
    // `garmin` OAuth backend, which is uncredentialed and fails the turn with
    // "No Garmin OAuth credentials". A missing session then surfaces as a
    // reconnect-Garmin auth error downstream, not an OAuth-credentials error.
    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos.auth_repos(),
        user_id,
        Some(tenant_id),
        oauth_providers::GARMIN,
    )
    .await;
    assert_eq!(
        resolved,
        oauth_providers::SCIOTTE_GARMIN,
        "garmin with no mirror token must stay on the mirror, not fall through to OAuth garmin"
    );
}

#[tokio::test]
async fn resolve_backend_collapses_garmin_aliases_to_one_cache_key() {
    // Regression for the historical-backfill loop: get_activities keyed the
    // durable cache, the backfill-coverage gate, and the completion push on the
    // resolved provider name — which is the LLM's explicit `provider` arg
    // ("garmin") on one turn and the stored connection ("sciotte_garmin") on the
    // next. Keyed raw, the two named the same provider but split the cache into
    // parallel keys that never saw each other, so every deep re-ask re-backfilled
    // and never served or pushed. get_activities now canonicalizes via
    // resolve_backend before any cache op; this pins the property that makes that
    // correct — both aliases of one connected provider resolve to ONE key, while
    // the user still sees the friendly name.
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // The user's only Garmin connection is the scraper mirror (no OAuth row) —
    // the real-world shape that triggered the split.
    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    let from_llm_arg = backend_resolver::resolve_backend(
        &resources.common.repos.auth_repos(),
        user_id,
        Some(tenant_id),
        oauth_providers::GARMIN,
    )
    .await;
    let from_connection = backend_resolver::resolve_backend(
        &resources.common.repos.auth_repos(),
        user_id,
        Some(tenant_id),
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    assert_eq!(
        from_llm_arg, from_connection,
        "LLM arg 'garmin' and connection 'sciotte_garmin' must collapse to one cache key"
    );
    assert_eq!(from_llm_arg, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(
        backend_resolver::user_facing_name(&from_llm_arg),
        oauth_providers::GARMIN,
        "user-visible copy still shows the friendly name"
    );
}

#[tokio::test]
async fn get_athlete_serves_canonical_cache_key_for_garmin_alias() {
    // End-to-end pin that GetAthleteTool canonicalizes BEFORE its cache read
    // (the resolver property above is necessary but not sufficient — a
    // regression that drops or reorders the resolve_backend call site would
    // pass it). Seed the athlete profile under the canonical backend key
    // ("sciotte_garmin"), then ask with the LLM-alias "garmin": a cache hit
    // proves the canonical key was used; keyed raw, the read would miss and
    // the tool would fail on live provider auth instead.
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

    let athlete = Athlete {
        id: "424242".to_owned(),
        username: "cached_runner".to_owned(),
        firstname: None,
        lastname: None,
        profile_picture: None,
        provider: oauth_providers::SCIOTTE_GARMIN.to_owned(),
    };
    let canonical_key = CacheKey::new(
        tenant_id,
        user_id,
        oauth_providers::SCIOTTE_GARMIN.to_owned(),
        CacheResource::AthleteProfile,
    );
    let state = tool_state(&resources);
    state
        .cache()
        .set(
            &canonical_key,
            &athlete,
            CacheResource::AthleteProfile.recommended_ttl(),
        )
        .await
        .expect("seed athlete cache");

    let tool = GetAthleteTool;
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "garmin" }))
        .await;

    assert!(
        !result.is_error,
        "alias 'garmin' must hit the canonical sciotte_garmin cache entry: {:?}",
        result.structured_content
    );
    let data = structured(&result);
    assert_eq!(
        data.get("athlete")
            .and_then(|a| a.get("username"))
            .and_then(|v| v.as_str()),
        Some("cached_runner"),
        "response must carry the cached profile, not a live fetch: {data:?}"
    );
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool.execute(&state, &ctx, json!({})).await;

    let data = structured(&result);
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool.execute(&state, &ctx, json!({})).await;

    let providers = structured(&result)
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool.execute(&state, &ctx, json!({})).await;

    let providers = structured(&result)
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool.execute(&state, &ctx, json!({})).await;

    let providers = structured(&result)
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "sciotte" }))
        .await;

    let data = structured(&result);
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "strava" }))
        .await;

    let data = structured(&result);
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "whoop" }))
        .await;

    let data = structured(&result);
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool.execute(&state, &ctx, json!({})).await;

    let providers = structured(&result)
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "garmin" }))
        .await;

    // Garmin keeps the mirror block: its official API is partner-gated, so the
    // connector must refuse to mint an OAuth URL while the mirror is active.
    assert!(
        result.is_error,
        "connect_provider(garmin) must fail rather than return an OAuth URL when the mirror is active"
    );
    let err_text = structured(&result)
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
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "strava" }))
        .await;

    // The mirror block must be lifted for Strava: the connector reaches the
    // OAuth-minting stage instead of steering the user back to the mirror flow.
    // (Minting itself may still fail downstream on tenant OAuth credentials —
    // an orthogonal concern — so we assert the block is gone, not end-to-end
    // success.)
    let content = structured(&result);
    assert!(
        content.get("requires_mirror_reauth").is_none(),
        "Strava connect must not return the mirror-reauth block: {content:?}"
    );
    let err_text = content.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        !err_text.to_lowercase().contains("direct login"),
        "Strava connect must not steer to the mirror direct-login flow: {err_text}"
    );
}

#[tokio::test]
async fn connect_provider_rejects_explicit_sciotte_name() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    let tool = ConnectProviderTool;
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "sciotte" }))
        .await;

    assert!(result.is_error);
    let err_text = structured(&result)
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

/// The drift case: a connection whose row still reads Active but whose token
/// is gone must ALSO signal auth-required. Sciotte sessions never refresh, so
/// no refresh failure ever flips their status — gating the tag on
/// `needs_reauth` left the athlete a generic error instead of the reconnect
/// link (live incident 2026-08-11). `Ok(None)` from the token lookup is
/// definitive: (re)connecting is the only action that can fix it.
#[tokio::test]
async fn create_authenticated_provider_signals_reauth_for_active_but_tokenless_connection() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Registered and Active — but no token row is ever seeded.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            oauth_providers::WHOOP,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let auth_service = AuthService::new(runtime);
    let tenant_str = tenant_id.to_string();
    let response = auth_service
        .create_authenticated_provider(oauth_providers::WHOOP, user_id, Some(&tenant_str))
        .await
        .err()
        .expect("a tokenless connection must fail provider creation");

    let signalled = response
        .metadata
        .and_then(|m| m.get(META_AUTH_REQUIRED_PROVIDER).cloned());
    assert_eq!(
        signalled,
        Some(json!(oauth_providers::WHOOP)),
        "an Active-but-tokenless connection must still tag the auth-required provider"
    );
}

// ============================================================================
// Provider-connection consistency: symmetric disconnect + orphan reconciliation
// ============================================================================

/// A disconnect must clear BOTH sources of truth — the `user_oauth_tokens` row
/// (drives `resolve_backend` + the scrape session) and the `provider_connections`
/// row (drives the "connected" badge + coaching fetch enumeration). Leaving an
/// orphaned connection row is exactly what made a disconnected Garmin keep
/// showing "Connected" and routed the next fetch to the uncredentialed OAuth
/// backend. The chat tool disconnects by the user-facing name "garmin", so it
/// must resolve to the `sciotte_garmin` backend before deleting.
#[tokio::test]
async fn disconnect_provider_tool_removes_both_token_and_connection_for_garmin() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Connect Garmin via the mirror: a token row + a connection row, exactly as
    // handle_sciotte_login writes them.
    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            oauth_providers::SCIOTTE_GARMIN,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();

    let tool = DisconnectProviderTool;
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "garmin" }))
        .await;
    assert!(
        !result.is_error,
        "disconnect must succeed: {:?}",
        result.structured_content
    );

    let token = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, oauth_providers::SCIOTTE_GARMIN)
        .await
        .unwrap();
    assert!(
        token.is_none(),
        "the mirror token row must be deleted on disconnect"
    );

    let conns = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap();
    assert!(
        !conns
            .iter()
            .any(|c| c.provider == oauth_providers::SCIOTTE_GARMIN),
        "the mirror connection row must be removed, not left orphaned"
    );
}

/// The one-time reconciliation (migration 20260714000001) removes connection
/// rows with no backing token — but only for backends where a token is expected
/// ('oauth'/'manual'). Synthetic connections are tokenless by design and must be
/// spared, and a token-backed connection must never be touched. This exercises
/// the exact DELETE the migration runs (sqlite variant).
#[tokio::test]
async fn reconciliation_deletes_orphans_and_spares_synthetic_and_valid() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let pc = &resources.common.repos.provider_connections;

    // (a) Orphaned mirror connection: 'manual' type, NO token → must be deleted.
    pc.register_connection(
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
        &ConnectionType::Manual,
        None,
    )
    .await
    .unwrap();
    // (b) Synthetic connection: tokenless by design → must be SPARED.
    pc.register_connection(
        user_id,
        tenant_id,
        "synthetic",
        &ConnectionType::Synthetic,
        None,
    )
    .await
    .unwrap();
    // (c) Valid OAuth connection with a backing token → must be SPARED.
    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    pc.register_connection(
        user_id,
        tenant_id,
        oauth_providers::STRAVA,
        &ConnectionType::OAuth,
        None,
    )
    .await
    .unwrap();

    // Run the reconciliation exactly as migration 20260714000001 does.
    let pool = resources
        .coach
        .database
        .sqlite_pool()
        .expect("sqlite test pool");
    sqlx::query(
        "DELETE FROM provider_connections \
         WHERE connection_type IN ('oauth', 'manual') \
           AND NOT EXISTS ( \
             SELECT 1 FROM user_oauth_tokens t \
             WHERE CAST(t.user_id AS TEXT) = CAST(provider_connections.user_id AS TEXT) \
               AND CAST(t.tenant_id AS TEXT) = CAST(provider_connections.tenant_id AS TEXT) \
               AND t.provider = provider_connections.provider )",
    )
    .execute(pool)
    .await
    .unwrap();

    let conns = pc.get_for_user(user_id, Some(tenant_id)).await.unwrap();
    let names: Vec<&str> = conns.iter().map(|c| c.provider.as_str()).collect();
    assert!(
        !names.contains(&oauth_providers::SCIOTTE_GARMIN),
        "orphaned mirror connection (no token) must be deleted: {names:?}"
    );
    assert!(
        names.contains(&"synthetic"),
        "synthetic connection is tokenless-by-design and must be spared: {names:?}"
    );
    assert!(
        names.contains(&oauth_providers::STRAVA),
        "token-backed oauth connection must be spared: {names:?}"
    );
}
