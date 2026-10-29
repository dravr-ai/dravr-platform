// ABOUTME: An Intervals.icu card says whether OAuth is offered and how it was linked, and a chat reconnect goes back the same way
// ABOUTME: Pins /api/providers own_app_required and connection_type, and auth_recovery's key form for a dead API-key link

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Intervals.icu links two ways (carnet#47): an athlete authorizes Dravr, and
//! a coach pastes an API key, the only credential that reads the athletes they
//! coach. Every client decides from `/api/providers` whether to offer the
//! authorization (`own_app_required` is `false` once a client resolves) and,
//! for a connected card, which way a reconnect goes (`connection_type`). The
//! chat's reconnect offer follows the same rule on the server: a dead API-key
//! link gets the hosted Intervals.icu page, whose key form relinks it, and
//! never an authorization URL that would silently cost a coach their roster.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate, and the surviving crate doc keeps `missing_docs` quiet.
#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use common::{
    create_test_server_resources, create_test_server_resources_with_config, create_test_user,
    test_server_config_with_app,
};
use helpers::axum_test::AxumTestRequest;
use pierre_chat_pipeline::stages::auth_recovery::{
    apply_auth_recovery, AuthRecovery, AuthRecoveryDeps,
};
use pierre_chat_pipeline::{
    InputSource, QuotaState, SurfaceId, SurfaceProfile, SurfaceRequest, TurnInput, TurnOrigin,
};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{ConnectionType, ConversationTurnId, TenantId, User};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{verify_link_token, CONNECT_PROVIDER};
use pierre_routes_auth::AuthRoutes;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use serde_json::Value;
use uuid::Uuid;

/// The client id the server's Intervals.icu app is configured with, when a
/// test configures one.
const SERVER_CLIENT_ID: &str = "icu_reconnect_client";

/// The base URL the minted reconnect link is built on.
const TEST_BASE_URL: &str = "https://api.test.dravr.ai";

/// The secret the reconnect link-token is signed with here.
const LINK_SECRET: &str = "test-admin-jwt-secret-for-intervals-reconnect";

/// A server whose own Intervals.icu app is configured.
async fn server_with_app() -> Arc<ServerContext> {
    create_test_server_resources_with_config(test_server_config_with_app(
        INTERVALS_ICU,
        SERVER_CLIENT_ID,
        "icu_reconnect_secret",
    ))
    .await
    .unwrap()
}

/// A user, their personal tenant and a session bearer for it.
async fn athlete(resources: &Arc<ServerContext>) -> (User, TenantId, String) {
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .expect("a new user has a personal tenant")
        .id;
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .unwrap();
    (user, tenant_id, format!("Bearer {token}"))
}

/// Link the athlete's Intervals.icu the way `kind` names: a pasted key
/// registers a `Manual` connection, an authorization an `OAuth` one.
async fn link(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    kind: &ConnectionType,
) {
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, INTERVALS_ICU, kind, None)
        .await
        .unwrap();
}

/// The Intervals.icu card `/api/providers` serves this session.
async fn intervals_card(resources: &Arc<ServerContext>, auth: &str) -> Value {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", auth)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(resp.status(), 200, "{}", resp.text());
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == INTERVALS_ICU)
        .unwrap_or_else(|| panic!("no Intervals.icu card: {body}"))
        .clone()
}

// ============================================================================
// /api/providers
// ============================================================================

/// The authorization is offered exactly while a client resolves for it: a
/// server with no app (the placeholder seed reads as none) offers the key
/// alone, a configured app is a client.
#[tokio::test]
async fn the_card_offers_the_authorization_only_while_a_client_resolves() {
    let without_app = create_test_server_resources().await.unwrap();
    let (_, _, auth) = athlete(&without_app).await;
    let card = intervals_card(&without_app, &auth).await;
    assert_eq!(card["requires_oauth"], true, "{card}");
    assert_eq!(
        card["own_app_required"], true,
        "no server app is no client: the card offers the key alone: {card}"
    );

    let resources = server_with_app().await;
    let (_, _, auth) = athlete(&resources).await;
    let card = intervals_card(&resources, &auth).await;
    assert_eq!(
        card["own_app_required"], false,
        "the server's app resolves: the card offers the authorization: {card}"
    );
    assert!(
        card.get("connection_type").is_none(),
        "a card not connected names no link: {card}"
    );
}

/// A connected card says how it was linked, and a relink the other way is
/// what it says next.
#[tokio::test]
async fn a_connected_card_says_whether_a_key_or_an_authorization_links_it() {
    let resources = create_test_server_resources().await.unwrap();
    let (user, tenant_id, auth) = athlete(&resources).await;

    link(&resources, user.id, tenant_id, &ConnectionType::Manual).await;
    let card = intervals_card(&resources, &auth).await;
    assert_eq!(card["connected"], true, "{card}");
    assert_eq!(card["connection_type"], "manual", "a pasted key: {card}");

    link(&resources, user.id, tenant_id, &ConnectionType::OAuth).await;
    let card = intervals_card(&resources, &auth).await;
    assert_eq!(card["connection_type"], "oauth", "an authorization: {card}");
}

// ============================================================================
// The chat's reconnect offer
// ============================================================================

/// A turn whose Intervals.icu link died, as the tool loop hands it to the
/// recovery stage.
fn dead_intervals_turn() -> ToolLoopResult {
    ToolLoopResult {
        content: String::new(),
        usage: None,
        finish_reason: None,
        activity_list: None,
        tool_calls_count: 1,
        tools_called: vec!["get_activities".to_owned()],
        pending_provider_auth_required: Some(INTERVALS_ICU.to_owned()),
        served_without_provider: None,
        guardian_denied: None,
        guardian_confirm: None,
        capability_claim_unverified: false,
        provider_warnings: Vec::new(),
    }
}

/// Run the recovery stage for the athlete's direct conversation.
async fn recover(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    result: &mut ToolLoopResult,
) -> AuthRecovery {
    let admin_jwt_secret: Arc<str> = Arc::from(LINK_SECRET);
    let registry = Arc::new(MessagingStringsRegistry::new());
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let short_links = resources.common.repos.short_links.clone();
    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        conversation_id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        conversation_tenant_id: tenant,
        tool_tenant_id: tenant,
        is_direct_message: true,
        content: "How was my week?".to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quote: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
        steps_after: None,
    };
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    });
    apply_auth_recovery(
        AuthRecoveryDeps {
            admin_jwt_secret: &admin_jwt_secret,
            base_url: TEST_BASE_URL,
            messaging_strings_registry: &registry,
            tool_runtime: &runtime,
            short_links: &short_links,
        },
        &input,
        &profile,
        result,
    )
    .await
}

/// A coach's key cannot be replaced by an authorization — the roster reads
/// need the key — so even with the server's app configured, a dead API-key
/// link is offered the hosted Intervals.icu page, signed for this athlete.
#[tokio::test]
async fn a_dead_api_key_link_reconnects_through_the_hosted_key_form() {
    let resources = server_with_app().await;
    let (user, tenant_id, _) = athlete(&resources).await;
    link(&resources, user.id, tenant_id, &ConnectionType::Manual).await;

    let mut result = dead_intervals_turn();
    let prompt = recover(&resources, user.id, tenant_id, &mut result)
        .await
        .prompt
        .expect("the athlete gets a reconnect control");
    assert_eq!(prompt.provider, INTERVALS_ICU);
    let code = prompt
        .url
        .strip_prefix(&format!("{TEST_BASE_URL}/r/"))
        .unwrap_or_else(|| panic!("the offer is a short link on this server: {}", prompt.url));
    let target = resources
        .common
        .repos
        .short_links
        .resolve_short_link(code)
        .await
        .unwrap()
        .expect("the short link resolves");
    let token = target
        .strip_prefix(&format!(
            "{TEST_BASE_URL}/providers/connect/{INTERVALS_ICU}?token="
        ))
        .unwrap_or_else(|| panic!("the offer opens the hosted Intervals.icu page: {target}"));
    let token = urlencoding::decode(token).unwrap();
    let claims = verify_link_token(&token, LINK_SECRET, CONNECT_PROVIDER)
        .expect("a connect-scoped token for the hosted page");
    assert_eq!(claims.sub, user.id.to_string());
    assert_eq!(claims.tid, tenant_id.to_string());
}

/// An athlete who authorized Dravr reconnects on Intervals.icu's own
/// authorization page, under the server's app.
#[tokio::test]
async fn a_dead_authorization_reconnects_on_intervals_icu() {
    let resources = server_with_app().await;
    let (user, tenant_id, _) = athlete(&resources).await;
    link(&resources, user.id, tenant_id, &ConnectionType::OAuth).await;

    let mut result = dead_intervals_turn();
    let prompt = recover(&resources, user.id, tenant_id, &mut result)
        .await
        .prompt
        .expect("the athlete gets a reconnect control");
    assert!(
        prompt
            .url
            .starts_with("https://intervals.icu/oauth/authorize?"),
        "an authorization reconnects on Intervals.icu's page: {}",
        prompt.url
    );
    assert!(
        prompt
            .url
            .contains(&format!("client_id={SERVER_CLIENT_ID}")),
        "under the server's app: {}",
        prompt.url
    );
}
