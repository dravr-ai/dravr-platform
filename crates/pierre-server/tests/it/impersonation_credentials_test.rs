// ABOUTME: An admin's support session leaves no credential behind: MCP tokens, API keys, A2A clients, OAuth apps, passwords, links, devices
// ABOUTME: Each route refuses the impersonation token and serves the same request on the athlete's own session

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An admin may act as an athlete to debug their account, for an hour at
//! most, through a session that ends. A credential minted under it would not
//! end with it: a personal MCP token, an API key, an A2A client, an OAuth app,
//! a new password, a linked chat or a push device keeps authenticating as the
//! athlete, or reaching their data, with no admin recorded (carnet#1634).
//!
//! Every test sends one request twice on the same athlete: under an
//! impersonation token it is refused with a `403` that keeps the admin signed
//! in, and nothing is created; on the athlete's own session
//! it is served. The two headers differ only in the impersonation.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_tenant};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::impersonation::{assert_refusal_audited, assert_support_refused, impersonation_of};
use pierre_core::models::User;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::api_keys::ApiKeyRoutes;
use pierre_mcp_server::routes::user_mcp_tokens::UserMcpTokenRoutes;
use pierre_routes_a2a::{A2ARoutes, A2ARoutesState};
use pierre_routes_auth::AuthRoutes;
use pierre_routes_identity::UserOAuthAppRoutes;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;

/// An athlete with a tenant, their own session, and an admin's support
/// session on them.
struct Athlete {
    resources: Arc<ServerContext>,
    user: User,
    session: String,
    support: String,
}

async fn athlete(label: &str) -> Athlete {
    let resources = create_test_server_resources().await.unwrap();
    let (user, token) = create_test_tenant(&resources, &format!("{label}@example.com"))
        .await
        .unwrap();
    let session = format!("Bearer {token}");
    let support = impersonation_of(&resources, &session).await;
    Athlete {
        resources,
        user,
        session,
        support,
    }
}

/// The refusal an impersonation token gets wherever only the athlete's own
/// session may act.
fn assert_refused(response: &AxumTestResponse, what: &str) {
    assert_support_refused(response.status(), &response.body_text(), what);
}

#[tokio::test]
async fn a_support_session_creates_no_personal_mcp_token() {
    let a = athlete("imp-mcp-token").await;
    let routes = || UserMcpTokenRoutes::routes(Arc::clone(&a.resources));
    let request = json!({ "name": "Claude Desktop" });

    let response = AxumTestRequest::post("/api/user/mcp-tokens")
        .header("authorization", &a.support)
        .json(&request)
        .send(routes())
        .await;
    assert_refused(&response, "create an MCP token");
    let tokens = &a.resources.common.repos.user_mcp_tokens;
    assert!(
        tokens.list_tokens(a.user.id).await.unwrap().is_empty(),
        "the refused request minted no MCP token"
    );
    assert_refusal_audited(&a.resources, a.user.id, "create a personal MCP token").await;

    let response = AxumTestRequest::post("/api/user/mcp-tokens")
        .header("authorization", &a.session)
        .json(&request)
        .send(routes())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::CREATED,
        "{}",
        response.body_text()
    );
    assert_eq!(
        tokens.list_tokens(a.user.id).await.unwrap().len(),
        1,
        "the athlete's own session mints one MCP token"
    );
}

#[tokio::test]
async fn a_support_session_creates_no_api_key() {
    let a = athlete("imp-api-key").await;
    let routes = || ApiKeyRoutes::routes(Arc::clone(&a.resources));
    let request = json!({ "name": "support key", "description": "left behind" });

    let response = AxumTestRequest::post("/api/keys")
        .header("authorization", &a.support)
        .json(&request)
        .send(routes())
        .await;
    assert_refused(&response, "create an API key");
    let keys = &a.resources.common.repos.api_keys;
    assert!(
        keys.get_for_user(a.user.id).await.unwrap().is_empty(),
        "the refused request minted no API key"
    );
    assert_refusal_audited(&a.resources, a.user.id, "create an API key").await;

    let response = AxumTestRequest::post("/api/keys")
        .header("authorization", &a.session)
        .json(&request)
        .send(routes())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::CREATED,
        "{}",
        response.body_text()
    );
    assert_eq!(
        keys.get_for_user(a.user.id).await.unwrap().len(),
        1,
        "the athlete's own session mints one API key"
    );
}

/// The A2A router as the server mounts it.
fn a2a_routes(resources: &Arc<ServerContext>) -> axum::Router {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    A2ARoutes::routes(A2ARoutesState {
        ctx: Arc::clone(resources),
        client_manager: resources.a2a.a2a_client_manager.clone(),
        auth_middleware: resources.auth.auth_middleware.clone(),
        tool_runtime,
        coaching: None,
    })
}

#[tokio::test]
async fn a_support_session_registers_no_a2a_client() {
    let a = athlete("imp-a2a-client").await;
    let request = json!({
        "name": "support-client",
        "description": "left behind",
        "capabilities": ["fitness-data-analysis"],
        "contact_email": "support@example.com",
    });

    let response = AxumTestRequest::post("/a2a/clients")
        .header("authorization", &a.support)
        .json(&request)
        .send(a2a_routes(&a.resources))
        .await;
    assert_refused(&response, "register an A2A client");
    let clients = &a.resources.common.repos.a2a;
    assert!(
        clients.list_clients(&a.user.id).await.unwrap().is_empty(),
        "the refused request registered no A2A client"
    );

    let response = AxumTestRequest::post("/a2a/clients")
        .header("authorization", &a.session)
        .json(&request)
        .send(a2a_routes(&a.resources))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::CREATED,
        "{}",
        response.body_text()
    );
    assert_eq!(
        clients.list_clients(&a.user.id).await.unwrap().len(),
        1,
        "the athlete's own session registers one A2A client"
    );
}

#[tokio::test]
async fn a_support_session_registers_no_oauth_app() {
    let a = athlete("imp-oauth-app").await;
    let routes =
        || UserOAuthAppRoutes::routes::<ServerContext>().with_state(Arc::clone(&a.resources));
    let request = json!({
        "provider": "strava",
        "client_id": "support-app",
        "client_secret": "support-secret",
    });
    let apps = &a.resources.common.repos.oauth_tokens;

    let response = AxumTestRequest::post("/api/users/oauth-apps")
        .header("authorization", &a.support)
        .json(&request)
        .send(routes())
        .await;
    assert_refused(&response, "register an OAuth app");
    assert!(
        apps.list_user_oauth_apps(a.user.id)
            .await
            .unwrap()
            .is_empty(),
        "the refused request registered no OAuth app"
    );

    let response = AxumTestRequest::post("/api/users/oauth-apps")
        .header("authorization", &a.session)
        .json(&request)
        .send(routes())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::CREATED,
        "{}",
        response.body_text()
    );
    assert_eq!(
        apps.list_user_oauth_apps(a.user.id).await.unwrap().len(),
        1,
        "the athlete's own session registers one OAuth app"
    );
}

#[tokio::test]
async fn a_support_session_changes_no_password() {
    let a = athlete("imp-password").await;
    let routes = || AuthRoutes::routes(a.resources.auth_routes_context());
    // The fixture's password: the attempt knows it, as an admin may have
    // been told it, so nothing but the impersonation stands in its way.
    let request = json!({
        "current_password": "password123",
        "new_password": "SupportChose456",
    });
    let password_hash = || async {
        // SECURITY: Global lookup — the fixture user's own row
        a.resources
            .common
            .repos
            .users
            .get_global(a.user.id)
            .await
            .unwrap()
            .unwrap()
            .password_hash
    };
    let before = password_hash().await;

    let response = AxumTestRequest::put("/api/user/change-password")
        .header("authorization", &a.support)
        .json(&request)
        .send(routes())
        .await;
    assert_refused(&response, "change the password");
    assert_eq!(password_hash().await, before, "the password is unchanged");

    let response = AxumTestRequest::put("/api/user/change-password")
        .header("authorization", &a.session)
        .json(&request)
        .send(routes())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.body_text()
    );
    assert_ne!(
        password_hash().await,
        before,
        "the athlete's own change lands"
    );
}

#[cfg(feature = "client-messaging")]
#[tokio::test]
async fn a_support_session_starts_no_channel_link() {
    use pierre_database::backends::{MessagingRepository, UpsertChannelConfigParams};
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use uuid::Uuid;

    let a = athlete("imp-channel-link").await;
    let tenant_id = a
        .resources
        .common
        .repos
        .tenants
        .list_for_user(a.user.id)
        .await
        .unwrap()
        .first()
        .expect("the athlete has a tenant")
        .id;
    // Messenger: a deep-link channel whose destination is the configured
    // page, so starting a link reaches no channel API.
    let db: &dyn MessagingRepository = &*a.resources.common.repos.messaging;
    db.upsert_channel_config(&UpsertChannelConfigParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        channel_type: "messenger",
        api_key: Some("page-token"),
        api_secret: None,
        webhook_secret: None,
        verify_token: None,
        account_id: Some("page-1"),
        phone_number: None,
        bot_token: None,
        is_active: true,
    })
    .await
    .unwrap();
    let routes = || MessagingRoutes::routes(Arc::clone(&a.resources));

    let response = AxumTestRequest::post("/api/messaging/link/init/messenger")
        .header("authorization", &a.support)
        .send(routes())
        .await;
    assert_refused(&response, "link a messaging channel");

    let response = AxumTestRequest::post("/api/messaging/link/init/messenger")
        .header("authorization", &a.session)
        .send(routes())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.body_text()
    );
    let body: serde_json::Value = response.json();
    assert!(
        body["code"].is_string(),
        "the athlete's own link has a code: {body}"
    );
}

#[cfg(feature = "client-notifications")]
#[tokio::test]
async fn a_support_session_registers_no_push_device() {
    use pierre_routes_groups::NotificationRoutes;

    let a = athlete("imp-push-device").await;
    let routes = || NotificationRoutes::routes(Arc::clone(&a.resources));
    let request = json!({
        "expo_push_token": "ExponentPushToken[support-phone]",
        "platform": "ios",
        "device_name": "Support phone",
    });
    let devices = |bearer: String| {
        let notifications = routes();
        async move {
            let response = AxumTestRequest::get("/api/notifications/device")
                .header("authorization", &bearer)
                .send(notifications)
                .await;
            assert_eq!(
                response.status_code(),
                StatusCode::OK,
                "{}",
                response.body_text()
            );
            response.json::<Vec<serde_json::Value>>().len()
        }
    };

    let response = AxumTestRequest::post("/api/notifications/device")
        .header("authorization", &a.support)
        .json(&request)
        .send(routes())
        .await;
    assert_refused(&response, "register a push device");
    assert_eq!(
        devices(a.session.clone()).await,
        0,
        "no device was registered"
    );

    let response = AxumTestRequest::post("/api/notifications/device")
        .header("authorization", &a.session)
        .json(&request)
        .send(routes())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::CREATED,
        "{}",
        response.body_text()
    );
    assert_eq!(
        devices(a.session.clone()).await,
        1,
        "the athlete's own session registers one push device"
    );
}
