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
//! 4. The read plan (carnet#1494): a session that is not revoked serves
//!    first, a live grant stands behind it, and a write goes to the backend
//!    of the plan that carries its capability.

use crate::common;

use std::sync::Arc;

use chrono::Utc;
use common::{create_test_server_resources, create_test_user};
use pierre_auth::connect_order::{set_connect_order, ConnectOrder};
use pierre_cache::{CacheKey, CacheResource};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{
    ActivityBuilder, Athlete, ConnectionType, SportType, TenantId, UserOAuthToken,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::backend_resolver::{self, BackendKind, CoalescedStatus, ReadPlan};
use pierre_providers::spi::ProviderCapabilities;
use pierre_providers::ProviderRegistry;
use pierre_tool_runtime::implementations::athlete_stats::GetAthleteTool;
use pierre_tool_runtime::implementations::connection::{
    ConnectProviderTool, DisconnectProviderTool, GetConnectionStatusTool,
};
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::protocol::reauth_notice::new_strava_sign_in_falls_back;
use pierre_tool_runtime::protocol::types::META_AUTH_REQUIRED_PROVIDER;

/// The reconciliation statement of migration 20260714000001, run here verbatim.
const RECONCILE: &str = "DELETE FROM provider_connections \
     WHERE connection_type IN ('oauth', 'manual') \
       AND NOT EXISTS ( \
         SELECT 1 FROM user_oauth_tokens t \
         WHERE CAST(t.user_id AS TEXT) = CAST(provider_connections.user_id AS TEXT) \
           AND CAST(t.tenant_id AS TEXT) = CAST(provider_connections.tenant_id AS TEXT) \
           AND t.provider = provider_connections.provider )";
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
        Some(Utc::now() + chrono::Duration::hours(1)),
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
    assert_eq!(backend_resolver::user_facing_name("whoop"), "whoop");
}

#[test]
fn mirror_backend_for_only_maps_the_scraped_providers() {
    assert_eq!(
        backend_resolver::mirror_backend_for("strava"),
        Some(oauth_providers::SCIOTTE)
    );
    assert_eq!(
        backend_resolver::mirror_backend_for("garmin"),
        Some(oauth_providers::SCIOTTE_GARMIN)
    );
    assert_eq!(
        backend_resolver::mirror_backend_for("trainingpeaks"),
        Some(oauth_providers::SCIOTTE_TRAININGPEAKS)
    );
    assert_eq!(backend_resolver::mirror_backend_for("terra"), None);
    assert_eq!(backend_resolver::mirror_backend_for("whoop"), None);
}

#[test]
fn serving_backends_lists_both_garmin_backends() {
    // carnet#352's invariant still holds: only a backend routing reads may
    // count. Since carnet#1155 the Garmin API backend serves too, behind the
    // usr/pwd session or alone; which one serves first is the read plan's to
    // say (carnet#1494), never this list's order.
    assert_eq!(
        backend_resolver::serving_backends("garmin"),
        vec![oauth_providers::GARMIN, oauth_providers::SCIOTTE_GARMIN]
    );
    // TrainingPeaks and COROS still have no API Pierre can call.
    assert_eq!(
        backend_resolver::serving_backends("coros"),
        vec![oauth_providers::SCIOTTE_COROS]
    );
}

#[test]
fn the_health_sync_keeps_pulling_garmin_through_its_mirror() {
    // Garmin's API pushes; the pull sync reads the usr/pwd session only, and
    // skips a user who holds the pushed backend.
    assert_eq!(
        backend_resolver::sync_backend("garmin"),
        oauth_providers::SCIOTTE_GARMIN
    );
    assert_eq!(
        backend_resolver::pushed_backend_for("garmin"),
        Some(oauth_providers::GARMIN)
    );
    assert_eq!(
        backend_resolver::pushed_backend_for("sciotte_garmin"),
        Some(oauth_providers::GARMIN)
    );
    assert_eq!(
        backend_resolver::sync_backend("coros"),
        oauth_providers::SCIOTTE_COROS
    );
    assert_eq!(
        backend_resolver::sync_backend("strava"),
        oauth_providers::STRAVA
    );
    assert_eq!(backend_resolver::sync_backend("whoop"), "whoop");
    assert_eq!(backend_resolver::pushed_backend_for("coros"), None);
    assert_eq!(backend_resolver::pushed_backend_for("strava"), None);
}

/// Sciotte first (carnet#1494): an athlete with nothing connected is sent to
/// every mirror-backed provider's hosted sign-in, Strava's included.
#[test]
fn every_mirror_pair_sends_an_unconnected_user_to_its_mirror() {
    assert!(backend_resolver::mirror_is_default("strava"));
    assert!(backend_resolver::mirror_is_default("sciotte"));
    assert!(backend_resolver::mirror_is_default("garmin"));
    assert!(backend_resolver::mirror_is_default("sciotte_garmin"));
    assert!(backend_resolver::mirror_is_default("trainingpeaks"));
    assert!(backend_resolver::mirror_is_default("coros"));
    assert!(!backend_resolver::mirror_is_default("whoop"));
}

#[test]
fn serving_backends_accepts_either_strava_backend() {
    // Strava's OAuth backend is real: it stands behind the session, or serves
    // alone, so either row is genuinely usable.
    let strava = backend_resolver::serving_backends("strava");
    assert!(strava.iter().any(|b| b == oauth_providers::STRAVA));
    assert!(strava.iter().any(|b| b == oauth_providers::SCIOTTE));
}

#[test]
fn serving_backends_falls_back_to_the_provider_itself() {
    // A provider with no mirror serves itself — returning an empty list here
    // would silently read every such provider as disconnected.
    assert_eq!(backend_resolver::serving_backends("terra"), vec!["terra"]);
    assert_eq!(backend_resolver::serving_backends("whoop"), vec!["whoop"]);
}

#[test]
fn serving_backends_answers_the_same_for_either_half_of_a_card() {
    // A caller naming the mirror must get the card's answer, as with
    // `backend_pair_for` — the two halves are one card.
    assert_eq!(
        backend_resolver::serving_backends("sciotte_garmin"),
        backend_resolver::serving_backends("garmin")
    );
    assert_eq!(
        backend_resolver::serving_backends("sciotte"),
        backend_resolver::serving_backends("strava")
    );
}

#[test]
fn is_mirror_backend_identifies_internal_names() {
    assert!(backend_resolver::is_mirror_backend("sciotte"));
    assert!(backend_resolver::is_mirror_backend("sciotte_garmin"));
    assert!(backend_resolver::is_mirror_backend("sciotte_trainingpeaks"));
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Only the mirror row exists → resolver picks sciotte
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos,
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos,
        user_id,
        Some(tenant_id),
        oauth_providers::STRAVA,
    )
    .await;
    assert_eq!(resolved, oauth_providers::STRAVA);
}

#[tokio::test]
async fn resolve_backend_strava_prefers_the_session_when_both_rows_exist() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Both rows: the session serves first and the grant stands behind it
    // (carnet#1494).
    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos,
        user_id,
        Some(tenant_id),
        oauth_providers::STRAVA,
    )
    .await;
    assert_eq!(resolved, oauth_providers::SCIOTTE);
}

#[tokio::test]
async fn resolve_backend_garmin_prefers_the_session_when_both_rows_exist() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // Both token rows: the usr/pwd session serves first, and the Garmin API
    // grant stands behind it whatever it pushed (carnet#1494).
    seed_token(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;
    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    push_garmin_activity(&resources, user_id, tenant_id).await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos,
        user_id,
        Some(tenant_id),
        oauth_providers::GARMIN,
    )
    .await;
    assert_eq!(resolved, oauth_providers::SCIOTTE_GARMIN);
}

/// carnet#352's coupling, restated for carnet#1155: the backend routing picks
/// for a user whose only Garmin row is the API one is that row's backend, and
/// status counts it. A row only counts when routing reads it.
#[tokio::test]
async fn a_garmin_api_row_alone_routes_to_the_api_and_reads_as_connected() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;

    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos,
        user_id,
        Some(tenant_id),
        oauth_providers::GARMIN,
    )
    .await;
    assert_eq!(resolved, oauth_providers::GARMIN);

    let serving = backend_resolver::serving_backends(oauth_providers::GARMIN);
    assert!(
        serving.iter().any(|b| b == &resolved),
        "status must count the backend routing actually picks ({resolved})"
    );
}

/// The status half: `coalesced_status` reports the usr/pwd session as the
/// active backend when a Garmin API grant stands behind it, as routing does.
#[tokio::test]
async fn coalesced_status_reports_the_garmin_session_over_the_grant_behind_it() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;
    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    let status = backend_resolver::coalesced_status(
        &resources.common.repos,
        user_id,
        tenant_id,
        oauth_providers::GARMIN,
    )
    .await;
    assert_eq!(
        status,
        CoalescedStatus {
            user_facing: oauth_providers::GARMIN,
            connected: true,
            backend_kind: BackendKind::Mirror,
            delegation: None,
        }
    );
}

/// A user on the usr/pwd connection alone is reported on the mirror, as
/// before carnet#1155.
#[tokio::test]
async fn coalesced_status_keeps_a_usr_pwd_garmin_user_on_the_mirror() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    let status = backend_resolver::coalesced_status(
        &resources.common.repos,
        user_id,
        tenant_id,
        oauth_providers::GARMIN,
    )
    .await;
    assert_eq!(status.backend_kind, BackendKind::Mirror);
    assert!(status.connected);
}

#[tokio::test]
async fn resolve_backend_garmin_stays_on_mirror_when_no_token() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // No Garmin row of either backend (e.g. after an orphaned-row
    // disconnect). Garmin must STILL resolve to its mirror — never fall
    // through to the push-only Garmin API, which holds nothing for a user who
    // never connected it. A missing session then surfaces as a
    // reconnect-Garmin auth error downstream.
    let resolved = backend_resolver::resolve_backend(
        &resources.common.repos,
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
        &resources.common.repos,
        user_id,
        Some(tenant_id),
        oauth_providers::GARMIN,
    )
    .await;
    let from_connection = backend_resolver::resolve_backend(
        &resources.common.repos,
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
        sex: None,
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let status = backend_resolver::coalesced_status(
        &resources.common.repos,
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
            delegation: None,
        }
    );
}

// ============================================================================
// GetConnectionStatusTool — multi-provider mode
// ============================================================================

#[tokio::test]
async fn multi_provider_status_hides_sciotte_entries() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
    assert_eq!(strava.get("connected").and_then(Value::as_bool), Some(true));
    assert_eq!(
        strava.get("backend").and_then(|v| v.as_str()),
        Some("mirror")
    );

    // Garmin (no row either way) is reported as disconnected / none.
    let garmin = providers
        .get(oauth_providers::GARMIN)
        .expect("garmin entry present");
    assert_eq!(
        garmin.get("connected").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(garmin.get("backend").and_then(|v| v.as_str()), Some("none"));
}

#[tokio::test]
async fn multi_provider_status_reports_oauth_backend_when_no_mirror() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
    assert_eq!(strava.get("connected").and_then(Value::as_bool), Some(true));
    assert_eq!(
        strava.get("backend").and_then(|v| v.as_str()),
        Some("oauth")
    );
}

#[tokio::test]
async fn multi_provider_status_strava_prefers_the_session_when_both_rows_exist() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
    assert_eq!(strava.get("connected").and_then(Value::as_bool), Some(true));
    // Status must match routing: the session serves first, the grant behind
    // it (carnet#1494).
    assert_eq!(
        strava.get("backend").and_then(|v| v.as_str()),
        Some("mirror"),
        "the session must win for Strava when both rows exist"
    );
}

#[tokio::test]
async fn multi_provider_status_garmin_prefers_the_session_when_both_rows_exist() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
    assert_eq!(garmin.get("connected").and_then(Value::as_bool), Some(true));
    // Status must match routing: the usr/pwd session serves first and the
    // Garmin API grant stands behind it, as Strava's does (carnet#1494).
    assert_eq!(
        garmin.get("backend").and_then(|v| v.as_str()),
        Some("mirror"),
        "the Garmin session must win when both rows exist"
    );
}

// ============================================================================
// GetConnectionStatusTool — single-provider mode
// ============================================================================

#[tokio::test]
async fn single_provider_status_rejects_explicit_sciotte_query() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
        data.get("connected").and_then(Value::as_bool),
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;

    let tool = GetConnectionStatusTool;
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let result = tool
        .execute(&state, &ctx, json!({ "provider": "strava" }))
        .await;

    let data = structured(&result);
    assert_eq!(data.get("connected").and_then(Value::as_bool), Some(true));
    assert_eq!(data.get("backend").and_then(|v| v.as_str()), Some("mirror"));
}

// ============================================================================
// GetConnectionStatusTool — needs_reauth surfacing
// ============================================================================

#[tokio::test]
async fn single_provider_status_reports_needs_reauth_after_refresh_failure() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
        Utc::now(),
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
        data.get("needs_reauth").and_then(Value::as_bool),
        Some(true)
    );
}

#[tokio::test]
async fn multi_provider_status_reports_needs_reauth() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
        Utc::now(),
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
        whoop.get("needs_reauth").and_then(Value::as_bool),
        Some(true)
    );
}

// ============================================================================
// ConnectProviderTool — mirror re-auth block
// ============================================================================

/// Assert a `connect_provider` answer is the hosted-login hand-off for
/// `mirror`: the auth-required sentinel the executor re-raises, so the chat
/// pipeline mints a login link and an MCP client gets the structured
/// reconnect code. An OAuth authorization URL here is the failure: neither
/// Garmin nor TrainingPeaks has an OAuth backend Pierre can complete.
fn assert_hosted_login_handoff(result: &ToolResponse, provider: &str, mirror: &str) {
    assert!(
        result.is_error,
        "connect_provider({provider}) must not answer an OAuth URL: {:?}",
        result.structured_content
    );
    let data = structured(result);
    assert_eq!(
        data.get("error_code").and_then(Value::as_str),
        Some("provider_auth_required"),
        "connect_provider({provider}) must raise the auth-required signal: {data:?}"
    );
    assert_eq!(
        data.get("provider").and_then(Value::as_str),
        Some(mirror),
        "the signal carries the backend slug the hosted-login mint keys on: {data:?}"
    );
}

/// Garmin is routed to its mirror by default, so asking to connect it hands
/// off to the hosted pages whether or not a session row exists. With no row,
/// this used to mint an OAuth URL for the then-uncredentialed Garmin API,
/// which failed as an `oauth_configuration_error` the agent relayed as
/// "Garmin is broken". Since carnet#1155 the Garmin API takes a seat of a
/// capped pool, which only the hosted picker weighs, so the chat tool still
/// mints no Garmin OAuth URL itself.
#[tokio::test]
async fn connect_provider_hands_garmin_to_the_hosted_login() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    let tool = ConnectProviderTool;
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);

    let no_row = tool
        .execute(&state, &ctx, json!({ "provider": "garmin" }))
        .await;
    assert_hosted_login_handoff(&no_row, "garmin", oauth_providers::SCIOTTE_GARMIN);

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    let with_row = tool
        .execute(&state, &ctx, json!({ "provider": "garmin" }))
        .await;
    assert_hosted_login_handoff(&with_row, "garmin", oauth_providers::SCIOTTE_GARMIN);
}

/// With the operator's Strava order on the sign-in first (carnet#1477), the
/// chat connects Strava the way its card does: the hosted sign-in, never an
/// OAuth URL. An athlete who already holds a Strava grant re-signs it through
/// the OAuth, and with the order back on OAuth first the hand-off stops.
#[tokio::test]
async fn connect_provider_follows_a_sign_in_first_strava_order() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let settings = resources.common.repos.system_settings.as_ref();
    set_connect_order(settings, oauth_providers::STRAVA, ConnectOrder::SignInFirst)
        .await
        .unwrap();

    let tool = ConnectProviderTool;
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    let strava = json!({ "provider": "strava" });

    let new_athlete = tool.execute(&state, &ctx, strava.clone()).await;
    assert_hosted_login_handoff(&new_athlete, "strava", oauth_providers::SCIOTTE);

    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    let grant_holder = structured(&tool.execute(&state, &ctx, strava.clone()).await).clone();
    assert_ne!(
        grant_holder.get("error_code").and_then(Value::as_str),
        Some("provider_auth_required"),
        "bug: a Strava grant holder was sent to the sign-in instead of re-signing their grant: {grant_holder:?}"
    );

    let (other, _) =
        common::create_test_user_with_email(&resources.agent.database, "oauth-first@example.com")
            .await
            .unwrap();
    let other_tenant = user_primary_tenant(&resources, other).await;
    set_connect_order(settings, oauth_providers::STRAVA, ConnectOrder::OAuthFirst)
        .await
        .unwrap();
    let oauth_first = structured(
        &tool
            .execute(&state, &tool_context(other, other_tenant), strava)
            .await,
    )
    .clone();
    assert_ne!(
        oauth_first.get("error_code").and_then(Value::as_str),
        Some("provider_auth_required"),
        "bug: with OAuth first the chat still hands Strava to the sign-in: {oauth_first:?}"
    );
}

/// The chat's link for the Strava sign-in hand-off is the connect picker,
/// whose card carries the OAuth fallback, only for a NEW Strava connection
/// with the sign-in first and a seat free (carnet#1477): an athlete with a
/// Strava session or grant keeps the sign-in link, and with OAuth first there
/// is nothing to fall back from.
#[tokio::test]
async fn a_new_strava_sign_in_falls_back_only_with_the_sign_in_first_and_no_connection() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let state = tool_state(&resources);
    let falls_back =
        || new_strava_sign_in_falls_back(&state, user_id, tenant_id, oauth_providers::SCIOTTE);

    assert!(
        !falls_back().await,
        "bug: OAuth first offers a sign-in fallback"
    );

    let settings = resources.common.repos.system_settings.as_ref();
    set_connect_order(settings, oauth_providers::STRAVA, ConnectOrder::SignInFirst)
        .await
        .unwrap();
    assert!(
        falls_back().await,
        "bug: a new sign-in-first Strava connection gets no fallback link"
    );
    assert!(
        !new_strava_sign_in_falls_back(&state, user_id, tenant_id, oauth_providers::SCIOTTE_COROS)
            .await,
        "bug: COROS has no OAuth to fall back to"
    );

    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;
    assert!(
        !falls_back().await,
        "bug: an athlete re-signing their Strava session was sent to the picker"
    );
}

#[tokio::test]
async fn connect_provider_allows_oauth_for_strava_when_mirror_active() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    // A sciotte-Strava user authorizing OAuth: with the operator's order on
    // OAuth first (the default kill switch), the connector mints an OAuth URL
    // instead of steering them back to the mirror flow, and the grant it
    // mints stands behind the session as its fallback (carnet#1494).
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    let tool = ConnectProviderTool;
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    for (mirror, user_facing) in [("sciotte", "strava"), ("sciotte_garmin", "garmin")] {
        let result = tool
            .execute(&state, &ctx, json!({ "provider": mirror }))
            .await;

        assert!(result.is_error);
        let err_text = structured(&result)
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            err_text.contains(&format!("Use '{user_facing}' instead")),
            "error should steer caller to the user-facing provider: {err_text}"
        );
        assert!(
            !err_text.to_lowercase().contains("sciotte"),
            "the refusal must not echo the backend back to the model: {err_text}"
        );
    }
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
        Utc::now(),
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
/// showing "Connected" and routed the next fetch to the then-uncredentialed
/// OAuth backend. The chat tool disconnects by the user-facing name "garmin", so it
/// must resolve to the `sciotte_garmin` backend before deleting.
#[tokio::test]
async fn disconnect_provider_tool_removes_both_token_and_connection_for_garmin() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
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
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(RECONCILE).execute(db.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(RECONCILE).execute(db.pool()).await.unwrap();
        }
    }

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

// ============================================================================
// TrainingPeaks — a mirror-only provider with no OAuth backend at all
// ============================================================================

#[test]
fn trainingpeaks_mirror_pair_resolves_both_ways() {
    assert_eq!(
        backend_resolver::user_facing_name(oauth_providers::SCIOTTE_TRAININGPEAKS),
        oauth_providers::TRAININGPEAKS
    );
    assert_eq!(
        backend_resolver::mirror_backend_for(oauth_providers::TRAININGPEAKS),
        Some(oauth_providers::SCIOTTE_TRAININGPEAKS)
    );
    assert!(backend_resolver::is_mirror_backend(
        oauth_providers::SCIOTTE_TRAININGPEAKS
    ));
    assert!(!backend_resolver::is_mirror_backend(
        oauth_providers::TRAININGPEAKS
    ));

    // Mirror only, from either half of the card: there is no TrainingPeaks
    // OAuth backend, so a bare `trainingpeaks` row can serve nothing.
    let serving = backend_resolver::serving_backends(oauth_providers::TRAININGPEAKS);
    assert_eq!(serving, vec![oauth_providers::SCIOTTE_TRAININGPEAKS]);
    assert_eq!(
        backend_resolver::serving_backends(oauth_providers::SCIOTTE_TRAININGPEAKS),
        serving
    );
    assert_eq!(
        backend_resolver::backend_pair_for(oauth_providers::SCIOTTE_TRAININGPEAKS),
        vec![
            oauth_providers::TRAININGPEAKS,
            oauth_providers::SCIOTTE_TRAININGPEAKS
        ]
    );
}

#[test]
fn hosted_login_targets_are_the_mirrors_user_facing_names() {
    assert_eq!(
        backend_resolver::hosted_login_target(oauth_providers::SCIOTTE),
        Some("strava")
    );
    assert_eq!(
        backend_resolver::hosted_login_target(oauth_providers::SCIOTTE_GARMIN),
        Some("garmin")
    );
    assert_eq!(
        backend_resolver::hosted_login_target(oauth_providers::SCIOTTE_TRAININGPEAKS),
        Some("trainingpeaks")
    );
    assert_eq!(
        backend_resolver::hosted_login_target(oauth_providers::SCIOTTE_COROS),
        Some("coros")
    );
    // An OAuth provider reconnects through its authorization URL, never the
    // hosted login — including the user-facing half of a mirror pair.
    assert_eq!(backend_resolver::hosted_login_target("strava"), None);
    assert_eq!(backend_resolver::hosted_login_target("whoop"), None);
    assert_eq!(
        backend_resolver::hosted_login_targets(),
        vec!["strava", "garmin", "trainingpeaks", "coros"]
    );
}

#[test]
fn brand_name_reads_either_half_of_a_mirror_pair_from_the_registry() {
    let registry = ProviderRegistry::new();
    assert_eq!(
        backend_resolver::brand_name(&registry, "trainingpeaks"),
        Some("TrainingPeaks")
    );
    assert_eq!(
        backend_resolver::brand_name(&registry, "sciotte_trainingpeaks"),
        Some("TrainingPeaks")
    );
    // The athlete connected Garmin through the scrape: the OAuth descriptor's
    // "Garmin Connect" names an API they never touched.
    assert_eq!(
        backend_resolver::brand_name(&registry, "garmin"),
        Some("Garmin")
    );
    assert_eq!(
        backend_resolver::brand_name(&registry, "sciotte"),
        Some("Strava")
    );
    assert_eq!(
        backend_resolver::brand_name(&registry, "no_such_provider"),
        None
    );
}

#[tokio::test]
async fn resolve_backend_keeps_trainingpeaks_on_its_mirror_with_or_without_a_row() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let repos = &resources.common.repos;

    // No row at all: the general mirror-only rule, not a Garmin special case,
    // is what keeps the request routable to a reconnect prompt.
    let no_row = backend_resolver::resolve_backend(
        repos,
        user_id,
        Some(tenant_id),
        oauth_providers::TRAININGPEAKS,
    )
    .await;
    assert_eq!(no_row, oauth_providers::SCIOTTE_TRAININGPEAKS);

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;
    let from_llm_arg = backend_resolver::resolve_backend(
        repos,
        user_id,
        Some(tenant_id),
        oauth_providers::TRAININGPEAKS,
    )
    .await;
    let from_connection = backend_resolver::resolve_backend(
        repos,
        user_id,
        Some(tenant_id),
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;
    assert_eq!(from_llm_arg, oauth_providers::SCIOTTE_TRAININGPEAKS);
    assert_eq!(
        from_llm_arg, from_connection,
        "LLM arg 'trainingpeaks' and connection 'sciotte_trainingpeaks' must collapse to one cache key"
    );
}

// ============================================================================
// COROS — Garmin's shape: an OAuth API Pierre cannot call (carnet#509)
// ============================================================================

#[test]
fn coros_mirror_pair_resolves_both_ways_and_serves_only_the_mirror() {
    assert_eq!(
        backend_resolver::user_facing_name(oauth_providers::SCIOTTE_COROS),
        oauth_providers::COROS
    );
    assert_eq!(
        backend_resolver::mirror_backend_for(oauth_providers::COROS),
        Some(oauth_providers::SCIOTTE_COROS)
    );
    assert!(backend_resolver::is_mirror_backend(
        oauth_providers::SCIOTTE_COROS
    ));
    assert!(!backend_resolver::is_mirror_backend(oauth_providers::COROS));
    // The partner API is not approved, so the raw `coros` backend serves
    // nothing: only the mirror, from either half of the card.
    let serving = backend_resolver::serving_backends(oauth_providers::COROS);
    assert_eq!(serving, vec![oauth_providers::SCIOTTE_COROS]);
    assert_eq!(
        backend_resolver::serving_backends(oauth_providers::SCIOTTE_COROS),
        serving
    );
    assert_eq!(
        backend_resolver::backend_pair_for(oauth_providers::SCIOTTE_COROS),
        vec![oauth_providers::COROS, oauth_providers::SCIOTTE_COROS]
    );
    let registry = ProviderRegistry::new();
    assert_eq!(
        backend_resolver::brand_name(&registry, "coros"),
        Some("COROS")
    );
    assert_eq!(
        backend_resolver::brand_name(&registry, "sciotte_coros"),
        Some("COROS")
    );
}

#[tokio::test]
async fn resolve_backend_keeps_coros_on_its_mirror_with_or_without_a_row() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let repos = &resources.common.repos;

    // No row: the mirror-only rule routes to a reconnect prompt, never to the
    // raw `coros` OAuth backend, which has no credentials.
    let no_row =
        backend_resolver::resolve_backend(repos, user_id, Some(tenant_id), oauth_providers::COROS)
            .await;
    assert_eq!(no_row, oauth_providers::SCIOTTE_COROS);

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_COROS,
    )
    .await;
    let from_llm_arg =
        backend_resolver::resolve_backend(repos, user_id, Some(tenant_id), oauth_providers::COROS)
            .await;
    let from_connection = backend_resolver::resolve_backend(
        repos,
        user_id,
        Some(tenant_id),
        oauth_providers::SCIOTTE_COROS,
    )
    .await;
    assert_eq!(from_llm_arg, oauth_providers::SCIOTTE_COROS);
    assert_eq!(
        from_llm_arg, from_connection,
        "LLM arg 'coros' and connection 'sciotte_coros' must collapse to one cache key"
    );
}

#[tokio::test]
async fn a_bare_coros_row_never_reads_as_connected_and_the_mirror_does() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let repos = &resources.common.repos;

    seed_token(&resources, user_id, tenant_id, oauth_providers::COROS).await;
    let status =
        backend_resolver::coalesced_status(repos, user_id, tenant_id, oauth_providers::COROS).await;
    assert_eq!(
        status,
        CoalescedStatus {
            user_facing: oauth_providers::COROS,
            connected: false,
            backend_kind: BackendKind::None,
            delegation: None,
        },
        "no fetch is ever routed to a raw `coros` backend, so its row must not read as connected"
    );

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_COROS,
    )
    .await;
    let status =
        backend_resolver::coalesced_status(repos, user_id, tenant_id, oauth_providers::COROS).await;
    assert!(
        status.connected,
        "the mirror session is what connects COROS"
    );
}

#[tokio::test]
async fn a_bare_trainingpeaks_row_never_reads_as_connected() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::TRAININGPEAKS,
    )
    .await;

    let status = backend_resolver::coalesced_status(
        &resources.common.repos,
        user_id,
        tenant_id,
        oauth_providers::TRAININGPEAKS,
    )
    .await;
    assert_eq!(
        status,
        CoalescedStatus {
            user_facing: oauth_providers::TRAININGPEAKS,
            connected: false,
            backend_kind: BackendKind::None,
            delegation: None,
        },
        "no fetch is ever routed to a `trainingpeaks` backend, so its row must not read as connected"
    );
}

#[tokio::test]
async fn get_athlete_serves_canonical_cache_key_for_trainingpeaks_alias() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;
    let athlete = Athlete {
        id: "6642427".to_owned(),
        username: "cached_triathlete".to_owned(),
        firstname: None,
        lastname: None,
        profile_picture: None,
        provider: oauth_providers::SCIOTTE_TRAININGPEAKS.to_owned(),
        sex: None,
    };
    let canonical_key = CacheKey::new(
        tenant_id,
        user_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS.to_owned(),
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

    let result = GetAthleteTool
        .execute(
            &state,
            &tool_context(user_id, tenant_id),
            json!({ "provider": "trainingpeaks" }),
        )
        .await;

    assert!(
        !result.is_error,
        "alias 'trainingpeaks' must hit the canonical cache entry: {:?}",
        result.structured_content
    );
    assert_eq!(
        structured(&result)
            .get("athlete")
            .and_then(|a| a.get("username"))
            .and_then(Value::as_str),
        Some("cached_triathlete")
    );
}

#[tokio::test]
async fn multi_provider_status_lists_trainingpeaks_and_hides_its_mirror() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);

    let providers = |result: &ToolResponse| {
        structured(result)
            .get("providers")
            .and_then(Value::as_object)
            .expect("providers map in response")
            .clone()
    };

    // TrainingPeaks has no backend of its own, so it is only ever listed
    // through its mirror — and listed even before anything is connected, or
    // the agent could never offer it.
    let before = providers(
        &GetConnectionStatusTool
            .execute(&state, &ctx, json!({}))
            .await,
    );
    assert!(
        !before.contains_key(oauth_providers::SCIOTTE_TRAININGPEAKS),
        "the mirror backend must never reach the LLM: {before:?}"
    );
    let tp = before
        .get(oauth_providers::TRAININGPEAKS)
        .unwrap_or_else(|| panic!("trainingpeaks must be listed: {before:?}"));
    assert_eq!(tp.get("connected").and_then(Value::as_bool), Some(false));
    assert_eq!(tp.get("backend").and_then(Value::as_str), Some("none"));

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;
    let after = providers(
        &GetConnectionStatusTool
            .execute(&state, &ctx, json!({}))
            .await,
    );
    let tp = after
        .get(oauth_providers::TRAININGPEAKS)
        .expect("trainingpeaks entry present");
    assert_eq!(tp.get("connected").and_then(Value::as_bool), Some(true));
    assert_eq!(tp.get("backend").and_then(Value::as_str), Some("mirror"));
    assert!(!after.contains_key(oauth_providers::SCIOTTE_TRAININGPEAKS));
}

/// A mirror's dead session is stored under the BACKEND name while the status
/// tool reports the user-facing one, so a lookup keyed by the user-facing
/// name never saw it: a Garmin or TrainingPeaks athlete whose scrape session
/// died read "connected" to the agent.
#[tokio::test]
async fn status_reports_needs_reauth_for_a_dead_mirror_session() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let pc = &resources.common.repos.provider_connections;

    for mirror in [
        oauth_providers::SCIOTTE_GARMIN,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    ] {
        seed_token(&resources, user_id, tenant_id, mirror).await;
        pc.register_connection(user_id, tenant_id, mirror, &ConnectionType::Manual, None)
            .await
            .unwrap();
        pc.mark_needs_reauth(
            user_id,
            tenant_id,
            mirror,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();
    }

    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);
    for provider in [oauth_providers::GARMIN, oauth_providers::TRAININGPEAKS] {
        let single = GetConnectionStatusTool
            .execute(&state, &ctx, json!({ "provider": provider }))
            .await;
        assert_eq!(
            structured(&single).get("status").and_then(Value::as_str),
            Some("needs_reauth"),
            "{provider}: a dead mirror session must read needs_reauth, not connected"
        );
    }

    let all = GetConnectionStatusTool
        .execute(&state, &ctx, json!({}))
        .await;
    let providers = structured(&all)
        .get("providers")
        .and_then(Value::as_object)
        .unwrap()
        .clone();
    for provider in [oauth_providers::GARMIN, oauth_providers::TRAININGPEAKS] {
        assert_eq!(
            providers[provider]
                .get("needs_reauth")
                .and_then(Value::as_bool),
            Some(true),
            "{provider}: {providers:?}"
        );
    }
}

/// The other direction: a Strava athlete whose session died beside a working
/// grant is asked to reconnect the session, which serves first; the grant
/// only stands behind it (carnet#1494).
#[tokio::test]
async fn a_dead_session_beside_a_working_strava_grant_is_the_reconnect() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let pc = &resources.common.repos.provider_connections;

    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    seed_token(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;
    pc.register_connection(
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE,
        &ConnectionType::Manual,
        None,
    )
    .await
    .unwrap();
    pc.mark_needs_reauth(
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE,
        Some("session_expired"),
        Utc::now(),
    )
    .await
    .unwrap();

    let result = GetConnectionStatusTool
        .execute(
            &tool_state(&resources),
            &tool_context(user_id, tenant_id),
            json!({ "provider": "strava" }),
        )
        .await;
    let data = structured(&result);
    assert_eq!(
        data.get("status").and_then(Value::as_str),
        Some("needs_reauth"),
        "bug: the dead session that serves first must read as the reconnect: {data:?}"
    );
    assert_eq!(data.get("backend").and_then(Value::as_str), Some("mirror"));
}

#[tokio::test]
async fn connect_provider_hands_trainingpeaks_to_the_hosted_login() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let state = tool_state(&resources);
    let ctx = tool_context(user_id, tenant_id);

    let no_row = ConnectProviderTool
        .execute(&state, &ctx, json!({ "provider": "trainingpeaks" }))
        .await;
    assert_hosted_login_handoff(
        &no_row,
        "trainingpeaks",
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    );

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;
    let with_row = ConnectProviderTool
        .execute(&state, &ctx, json!({ "provider": "trainingpeaks" }))
        .await;
    assert_hosted_login_handoff(
        &with_row,
        "trainingpeaks",
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    );
}

#[tokio::test]
async fn connect_provider_steers_the_trainingpeaks_mirror_name_to_its_brand() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    let result = ConnectProviderTool
        .execute(
            &tool_state(&resources),
            &tool_context(user_id, tenant_id),
            json!({ "provider": "sciotte_trainingpeaks" }),
        )
        .await;

    assert!(result.is_error);
    let err_text = structured(&result)
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        err_text.contains("Use 'trainingpeaks' instead"),
        "the refusal must name the provider the athlete knows: {err_text}"
    );
}

#[tokio::test]
async fn disconnect_provider_tool_removes_both_token_and_connection_for_trainingpeaks() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;

    seed_token(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            oauth_providers::SCIOTTE_TRAININGPEAKS,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();

    let result = DisconnectProviderTool
        .execute(
            &tool_state(&resources),
            &tool_context(user_id, tenant_id),
            json!({ "provider": "trainingpeaks" }),
        )
        .await;
    assert!(
        !result.is_error,
        "disconnecting by the user-facing name must pass provider validation: {:?}",
        result.structured_content
    );

    let token = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, oauth_providers::SCIOTTE_TRAININGPEAKS)
        .await
        .unwrap();
    assert!(token.is_none(), "the mirror token row must be deleted");
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
            .any(|c| c.provider == oauth_providers::SCIOTTE_TRAININGPEAKS),
        "the mirror connection row must be removed, not left orphaned"
    );
}

#[tokio::test]
async fn reconciliation_deletes_an_orphaned_trainingpeaks_mirror_connection() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    let pc = &resources.common.repos.provider_connections;

    pc.register_connection(
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
        &ConnectionType::Manual,
        None,
    )
    .await
    .unwrap();

    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(RECONCILE).execute(db.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(RECONCILE).execute(db.pool()).await.unwrap();
        }
    }

    let conns = pc.get_for_user(user_id, Some(tenant_id)).await.unwrap();
    assert!(
        !conns
            .iter()
            .any(|c| c.provider == oauth_providers::SCIOTTE_TRAININGPEAKS),
        "an orphaned TrainingPeaks mirror connection (no token) must be deleted"
    );
}

/// Register `provider`'s connection row for the user, as a connect does.
async fn register(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, provider, &ConnectionType::Manual, None)
        .await
        .expect("register connection");
}

/// Store one activity Garmin pushed for the user.
async fn push_garmin_activity(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) {
    let pushed = ActivityBuilder::new(
        "20111111111",
        "Pushed Run",
        SportType::Run,
        Utc::now() - chrono::Duration::days(1),
        1_800,
        oauth_providers::GARMIN,
    )
    .source(oauth_providers::GARMIN)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities_synced_at(
            user_id,
            &tenant_id,
            oauth_providers::GARMIN,
            &[pushed],
            Utc::now(),
        )
        .await
        .expect("store the pushed activity");
}

/// Flag `provider`'s connection `needs_reauth`, as a refused refresh or a dead
/// scrape does.
async fn flag(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId, provider: &str) {
    resources
        .common
        .repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant_id,
            provider,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .expect("flag the connection");
}

/// Mark `provider`'s connection revoked. No repository writes the status, so
/// the row is set directly, as a provider-side revocation would leave it.
async fn revoke(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    const REVOKE: &str = "UPDATE provider_connections SET status = 'revoked' \
         WHERE CAST(user_id AS TEXT) = $1 AND CAST(tenant_id AS TEXT) = $2 AND provider = $3";
    let (user, tenant) = (user_id.to_string(), tenant_id.to_string());
    let changed = match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query(REVOKE)
            .bind(&user)
            .bind(&tenant)
            .bind(provider)
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query(REVOKE)
            .bind(&user)
            .bind(&tenant)
            .bind(provider)
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
    };
    assert_eq!(changed, 1, "revoke one {provider} connection");
}

/// Hold `backend` for the user: its token row and its connection row.
async fn hold(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId, backend: &str) {
    seed_token(resources, user_id, tenant_id, backend).await;
    register(resources, user_id, tenant_id, backend).await;
}

/// The plan `read_plan` reads for `provider`.
async fn plan(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> ReadPlan {
    backend_resolver::read_plan(&resources.common.repos, user_id, tenant_id, provider).await
}

/// A plan reading `primary` first, `fallback` behind it.
fn read_plan_of(primary: &str, fallback: Option<&str>) -> ReadPlan {
    ReadPlan {
        primary: primary.to_owned(),
        fallback: fallback.map(str::to_owned),
    }
}

/// A fresh user in their own tenant.
async fn athlete(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId) {
    let (user_id, _) = common::create_test_user_with_email(&resources.agent.database, email)
        .await
        .unwrap();
    (user_id, user_primary_tenant(resources, user_id).await)
}

/// Every row of the read-plan table (carnet#1494), for both pairs whose grant
/// serves beside the mirror, from either half of the pair: a session that is
/// not revoked serves first with a live grant behind it; a live grant alone
/// serves; a dead grant alone, and nothing at all, go to the mirror; a revoked
/// session leaves a live grant serving.
#[tokio::test]
async fn read_plan_puts_the_session_first_and_a_live_grant_behind_it() {
    let resources = create_test_server_resources().await.unwrap();
    for (n, (grant, mirror)) in [
        (oauth_providers::STRAVA, oauth_providers::SCIOTTE),
        (oauth_providers::GARMIN, oauth_providers::SCIOTTE_GARMIN),
    ]
    .into_iter()
    .enumerate()
    {
        let both_live = athlete(&resources, &format!("both-live-{n}@example.com")).await;
        hold(&resources, both_live.0, both_live.1, grant).await;
        hold(&resources, both_live.0, both_live.1, mirror).await;

        let flagged_session =
            athlete(&resources, &format!("flagged-session-{n}@example.com")).await;
        hold(&resources, flagged_session.0, flagged_session.1, grant).await;
        hold(&resources, flagged_session.0, flagged_session.1, mirror).await;
        flag(&resources, flagged_session.0, flagged_session.1, mirror).await;

        let flagged_grant = athlete(&resources, &format!("flagged-grant-{n}@example.com")).await;
        hold(&resources, flagged_grant.0, flagged_grant.1, grant).await;
        hold(&resources, flagged_grant.0, flagged_grant.1, mirror).await;
        flag(&resources, flagged_grant.0, flagged_grant.1, grant).await;

        let grant_only = athlete(&resources, &format!("grant-only-{n}@example.com")).await;
        hold(&resources, grant_only.0, grant_only.1, grant).await;

        let dead_grant = athlete(&resources, &format!("dead-grant-{n}@example.com")).await;
        hold(&resources, dead_grant.0, dead_grant.1, grant).await;
        flag(&resources, dead_grant.0, dead_grant.1, grant).await;

        let revoked_session = athlete(&resources, &format!("revoked-{n}@example.com")).await;
        hold(&resources, revoked_session.0, revoked_session.1, grant).await;
        hold(&resources, revoked_session.0, revoked_session.1, mirror).await;
        revoke(&resources, revoked_session.0, revoked_session.1, mirror).await;

        let nothing = athlete(&resources, &format!("nothing-{n}@example.com")).await;

        for ((user_id, tenant_id), expected, case) in [
            (
                both_live,
                read_plan_of(mirror, Some(grant)),
                "a live session beside a live grant",
            ),
            (
                flagged_session,
                read_plan_of(mirror, Some(grant)),
                "a flagged session beside a live grant",
            ),
            (
                flagged_grant,
                read_plan_of(mirror, None),
                "a live session beside a dead grant",
            ),
            (grant_only, read_plan_of(grant, None), "a live grant alone"),
            (dead_grant, read_plan_of(mirror, None), "a dead grant alone"),
            (
                revoked_session,
                read_plan_of(grant, None),
                "a revoked session beside a live grant",
            ),
            (nothing, read_plan_of(mirror, None), "nothing held"),
        ] {
            for named in [grant, mirror] {
                assert_eq!(
                    plan(&resources, user_id, tenant_id, named).await,
                    expected,
                    "bug: {case}, named {named}"
                );
            }
            assert_eq!(
                backend_resolver::resolve_backend(
                    &resources.common.repos,
                    user_id,
                    Some(tenant_id),
                    grant
                )
                .await,
                expected.primary,
                "bug: resolve_backend is the plan's primary for {case}"
            );
            assert_eq!(
                backend_resolver::resolve_backend(
                    &resources.common.repos,
                    user_id,
                    Some(tenant_id),
                    mirror
                )
                .await,
                mirror,
                "bug: a mirror named directly stays that mirror for {case}"
            );
        }
    }
}

/// TrainingPeaks and COROS are their mirror alone, whatever rows are held; a
/// provider with no mirror is itself alone; and a plan is read in its tenant
/// only: rows held in another tenant are not this one's.
#[tokio::test]
async fn read_plan_keeps_mirror_only_and_unpaired_providers_and_reads_one_tenant() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = athlete(&resources, "plan-alone@example.com").await;
    for backend in [
        oauth_providers::COROS,
        oauth_providers::TRAININGPEAKS,
        oauth_providers::WHOOP,
    ] {
        hold(&resources, user_id, tenant_id, backend).await;
    }
    for (provider, primary) in [
        (oauth_providers::COROS, oauth_providers::SCIOTTE_COROS),
        (
            oauth_providers::SCIOTTE_COROS,
            oauth_providers::SCIOTTE_COROS,
        ),
        (
            oauth_providers::TRAININGPEAKS,
            oauth_providers::SCIOTTE_TRAININGPEAKS,
        ),
        (oauth_providers::WHOOP, oauth_providers::WHOOP),
    ] {
        assert_eq!(
            plan(&resources, user_id, tenant_id, provider).await,
            read_plan_of(primary, None),
            "bug: {provider} has one backend"
        );
    }

    let (elsewhere, other_tenant) = athlete(&resources, "plan-elsewhere@example.com").await;
    hold(&resources, elsewhere, other_tenant, oauth_providers::STRAVA).await;
    hold(&resources, elsewhere, tenant_id, oauth_providers::SCIOTTE).await;
    assert_eq!(
        plan(&resources, elsewhere, other_tenant, oauth_providers::STRAVA).await,
        read_plan_of(oauth_providers::STRAVA, None),
        "bug: a session held in another tenant took over this tenant's grant"
    );
    assert_eq!(
        plan(&resources, elsewhere, tenant_id, oauth_providers::STRAVA).await,
        read_plan_of(oauth_providers::SCIOTTE, None),
        "bug: a grant held in another tenant stood behind this tenant's session"
    );
}

/// A Garmin session serves before the grant beside it (carnet#1494): a read
/// naming either backend goes to the session, the grant stands behind it,
/// and the status reports the session.
#[tokio::test]
async fn a_garmin_session_serves_before_the_grant_beside_it() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_token(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;
    hold(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    push_garmin_activity(&resources, user_id, tenant_id).await;
    let repos = &resources.common.repos;

    assert_eq!(
        plan(&resources, user_id, tenant_id, oauth_providers::GARMIN).await,
        read_plan_of(
            oauth_providers::SCIOTTE_GARMIN,
            Some(oauth_providers::GARMIN)
        ),
        "bug: a live grant must stand behind the session, whatever it pushed"
    );
    for requested in [oauth_providers::GARMIN, oauth_providers::SCIOTTE_GARMIN] {
        assert_eq!(
            backend_resolver::resolve_backend(repos, user_id, Some(tenant_id), requested).await,
            oauth_providers::SCIOTTE_GARMIN,
            "bug: a read naming {requested} must go to the session, never the grant behind it"
        );
    }
    let status =
        backend_resolver::coalesced_status(repos, user_id, tenant_id, oauth_providers::GARMIN)
            .await;
    assert_eq!(status.backend_kind, BackendKind::Mirror, "{status:?}");
    assert!(status.connected, "{status:?}");
}

/// `grant_beside_session` holds only for an OAuth grant beside a session of
/// its provider that is not revoked: a usr/pwd-only athlete's sessions, COROS
/// and TrainingPeaks, a dead grant alone and a grant beside a revoked session
/// all read `false`.
#[tokio::test]
async fn only_a_grant_beside_a_session_stands_aside() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = athlete(&resources, "beside@example.com").await;
    for mirror in [
        oauth_providers::SCIOTTE_GARMIN,
        oauth_providers::SCIOTTE_COROS,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
        oauth_providers::SCIOTTE,
    ] {
        hold(&resources, user_id, tenant_id, mirror).await;
    }
    let repos = &resources.common.repos;
    let beside = |backend: &'static str| {
        backend_resolver::grant_beside_session(repos, user_id, tenant_id, backend)
    };

    for backend in [
        oauth_providers::SCIOTTE_GARMIN,
        oauth_providers::SCIOTTE_COROS,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
        oauth_providers::SCIOTTE,
        oauth_providers::COROS,
        oauth_providers::TRAININGPEAKS,
        oauth_providers::WHOOP,
    ] {
        assert!(
            !beside(backend).await,
            "bug: {backend} is no grant beside a session"
        );
    }
    assert!(
        beside(oauth_providers::STRAVA).await && beside(oauth_providers::GARMIN).await,
        "bug: a Strava or Garmin grant beside a live session stands aside"
    );
    hold(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    flag(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;
    assert!(
        beside(oauth_providers::STRAVA).await,
        "bug: a flagged session still serves first, so the grant stands aside"
    );
    assert_eq!(
        backend_resolver::resolve_backend(repos, user_id, Some(tenant_id), oauth_providers::STRAVA)
            .await,
        oauth_providers::SCIOTTE,
        "bug: a read naming the grant beside a session goes to the session"
    );

    let (dead, dead_tenant) = athlete(&resources, "beside-dead@example.com").await;
    hold(&resources, dead, dead_tenant, oauth_providers::GARMIN).await;
    flag(&resources, dead, dead_tenant, oauth_providers::GARMIN).await;
    assert!(
        !backend_resolver::grant_beside_session(repos, dead, dead_tenant, oauth_providers::GARMIN)
            .await,
        "bug: a dead grant alone stands beside nothing; its flag is the reconnect"
    );
    hold(
        &resources,
        dead,
        dead_tenant,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    revoke(
        &resources,
        dead,
        dead_tenant,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    assert!(
        !backend_resolver::grant_beside_session(repos, dead, dead_tenant, oauth_providers::GARMIN)
            .await,
        "bug: a revoked session serves nothing, so the grant beside it does not stand aside"
    );
}

/// The history a read of each backend serves (carnet#1494): a session's
/// window keeps the rows its grant stored, a Garmin grant's keeps what its
/// session imported (carnet#1386), and every other backend has one key.
#[test]
fn history_keys_serve_both_halves_of_a_pair_from_the_session() {
    assert_eq!(
        backend_resolver::history_keys(oauth_providers::SCIOTTE),
        vec![oauth_providers::SCIOTTE, oauth_providers::STRAVA]
    );
    assert_eq!(
        backend_resolver::history_keys(oauth_providers::SCIOTTE_GARMIN),
        vec![oauth_providers::SCIOTTE_GARMIN, oauth_providers::GARMIN]
    );
    assert_eq!(
        backend_resolver::history_keys(oauth_providers::GARMIN),
        vec![oauth_providers::GARMIN, oauth_providers::SCIOTTE_GARMIN]
    );
    assert_eq!(
        backend_resolver::history_keys(oauth_providers::STRAVA),
        vec![oauth_providers::STRAVA]
    );
    assert_eq!(
        backend_resolver::history_keys(oauth_providers::SCIOTTE_COROS),
        vec![oauth_providers::SCIOTTE_COROS]
    );
}

/// A write goes to the backend of the athlete's read plan that carries its
/// capability (carnet#1494): a Garmin workout keeps going to the grant while
/// the session serves the reads; without a capable backend in the plan, the
/// plan's primary, whose refusal names the capability; a mirror named
/// directly and a provider with no mirror are left as named.
#[tokio::test]
async fn a_write_goes_to_the_plans_backend_that_carries_its_capability() {
    let resources = create_test_server_resources().await.unwrap();
    let registry = Arc::clone(&resources.fitness.provider_registry);
    let repos = &resources.common.repos;
    let write = |user_id, tenant_id, requested: &'static str, capability| {
        let registry = Arc::clone(&registry);
        async move {
            backend_resolver::write_backend(
                repos, &registry, user_id, tenant_id, requested, capability,
            )
            .await
        }
    };
    let calendar = ProviderCapabilities::CALENDAR_WRITE;

    let (both, both_tenant) = athlete(&resources, "write-both@example.com").await;
    hold(&resources, both, both_tenant, oauth_providers::GARMIN).await;
    hold(
        &resources,
        both,
        both_tenant,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    assert_eq!(
        write(both, both_tenant, oauth_providers::GARMIN, calendar).await,
        oauth_providers::GARMIN,
        "bug: a Garmin workout went to the session that serves the reads"
    );
    assert_eq!(
        backend_resolver::resolve_backend(repos, both, Some(both_tenant), oauth_providers::GARMIN)
            .await,
        oauth_providers::SCIOTTE_GARMIN,
        "the reads stay on the session"
    );
    assert_eq!(
        write(both, both_tenant, oauth_providers::SCIOTTE_GARMIN, calendar).await,
        oauth_providers::SCIOTTE_GARMIN,
        "a mirror named directly is left as named"
    );

    let (session, session_tenant) = athlete(&resources, "write-session@example.com").await;
    hold(
        &resources,
        session,
        session_tenant,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    assert_eq!(
        write(session, session_tenant, oauth_providers::GARMIN, calendar).await,
        oauth_providers::SCIOTTE_GARMIN,
        "bug: with no capable backend in the plan the write must name the primary"
    );

    let (grant, grant_tenant) = athlete(&resources, "write-grant@example.com").await;
    hold(&resources, grant, grant_tenant, oauth_providers::GARMIN).await;
    assert_eq!(
        write(grant, grant_tenant, oauth_providers::GARMIN, calendar).await,
        oauth_providers::GARMIN
    );
    assert_eq!(
        write(
            grant,
            grant_tenant,
            oauth_providers::TRAININGPEAKS,
            ProviderCapabilities::PLANNED_WORKOUTS
        )
        .await,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
        "bug: TrainingPeaks' planned calendar is its mirror's"
    );
    assert_eq!(
        write(
            grant,
            grant_tenant,
            oauth_providers::INTERVALS_ICU,
            ProviderCapabilities::PLANNED_WORKOUTS
        )
        .await,
        oauth_providers::INTERVALS_ICU,
        "a provider with no mirror is left as named"
    );
}

/// A Garmin grant alone serves; a session that dies beside it later still
/// serves first, so its read raises the hosted sign-in, and the grant stands
/// behind it.
#[tokio::test]
async fn a_dead_garmin_session_still_serves_first_with_the_grant_behind_it() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_token(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;
    let repos = &resources.common.repos;

    assert_eq!(
        plan(&resources, user_id, tenant_id, oauth_providers::GARMIN).await,
        read_plan_of(oauth_providers::GARMIN, None)
    );

    hold(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    flag(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    assert_eq!(
        plan(&resources, user_id, tenant_id, oauth_providers::GARMIN).await,
        read_plan_of(
            oauth_providers::SCIOTTE_GARMIN,
            Some(oauth_providers::GARMIN)
        ),
        "bug: a dead session must still serve first, the grant behind it"
    );
    assert_eq!(
        backend_resolver::resolve_backend(repos, user_id, Some(tenant_id), oauth_providers::GARMIN)
            .await,
        oauth_providers::SCIOTTE_GARMIN
    );
}

/// Strava's session serves before its grant whatever the cache holds: the
/// grant stands behind it.
#[tokio::test]
async fn a_strava_session_serves_before_its_grant_whatever_the_cache_holds() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = user_primary_tenant(&resources, user_id).await;
    seed_token(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;
    hold(&resources, user_id, tenant_id, oauth_providers::SCIOTTE).await;
    let repos = &resources.common.repos;

    assert_eq!(
        plan(&resources, user_id, tenant_id, oauth_providers::STRAVA).await,
        read_plan_of(oauth_providers::SCIOTTE, Some(oauth_providers::STRAVA))
    );
    assert_eq!(
        backend_resolver::resolve_backend(repos, user_id, Some(tenant_id), oauth_providers::STRAVA)
            .await,
        oauth_providers::SCIOTTE
    );
}
