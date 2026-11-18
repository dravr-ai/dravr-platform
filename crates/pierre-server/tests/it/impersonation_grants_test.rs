// ABOUTME: An admin's support session gives no consent, starts no connection, stores no key, buys nothing, deletes nothing
// ABOUTME: Each route refuses the impersonation with a 403 it audits, and serves the same request on the athlete's own session

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An admin may act as an athlete to debug their account and write on their
//! behalf, attributed to the admin. What outlives the support session is
//! refused: a consent only the athlete gives, a provider connection, a stored
//! LLM key or bot credential, a billing session, a coach-access request the
//! admin would then grant, the account's deletion (carnet#1634). A withdrawal
//! is still served.
//!
//! Every test sends one request twice on the same athlete: under an
//! impersonation token it is refused with a `403`, written to the admin audit
//! trail, and nothing changes; on the athlete's own session it is served.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use common::{create_test_server_resources, create_test_tenant};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::impersonation::{
    assert_refusal_audited, assert_support_refused, impersonation_of, support_grant,
};
use pierre_auth::auth::IMPERSONATION_WRITE_REFUSED;
use pierre_auth::connect_order::{set_connect_order, ConnectOrder};
use pierre_core::constants::oauth::providers::WHOOP;
use pierre_core::models::User;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::coach_access::CoachAccessRoutes;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use pierre_routes_admin::llm_settings::LlmSettingsRoutes;
use pierre_routes_auth::AuthRoutes;
use pierre_services::provider_notice::ai_consent_state;
use pierre_tool_runtime::context::SUPPORT_SESSION;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

/// The password `create_test_tenant`'s account signs in with.
const PASSWORD: &str = "password123";

/// An athlete with a tenant, their own session, and an admin's support
/// session on them.
struct Athlete {
    resources: Arc<ServerContext>,
    user: User,
    tenant_id: Uuid,
    session: String,
    support: String,
}

async fn athlete(label: &str) -> Athlete {
    let resources = create_test_server_resources().await.unwrap();
    let (user, token) = create_test_tenant(&resources, &format!("{label}@example.com"))
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()[0]
        .id
        .as_uuid();
    let session = format!("Bearer {token}");
    let support = impersonation_of(&resources, &session).await;
    Athlete {
        resources,
        user,
        tenant_id,
        session,
        support,
    }
}

impl Athlete {
    fn auth_routes(&self) -> Router {
        AuthRoutes::routes(self.resources.auth_routes_context())
    }

    async fn audited(&self, attempted: &str) {
        assert_refusal_audited(&self.resources, self.user.id, attempted).await;
    }
}

fn refused(response: &AxumTestResponse, what: &str) {
    assert_support_refused(response.status(), &response.body_text(), what);
}

fn assert_served(response: &AxumTestResponse, status: StatusCode) {
    assert_eq!(response.status_code(), status, "{}", response.body_text());
}

#[tokio::test]
async fn a_support_session_gives_no_ai_consent_but_withdraws_one() {
    let a = athlete("imp-ai-consent").await;
    let path = format!("/api/providers/{WHOOP}/ai-consent");
    let state = || ai_consent_state(&a.resources.common.repos, a.tenant_id, a.user.id, WHOOP);

    let response = AxumTestRequest::delete(&path)
        .header("authorization", &a.support)
        .send(a.auth_routes())
        .await;
    assert_served(&response, StatusCode::OK);

    let response = AxumTestRequest::put(&path)
        .header("authorization", &a.support)
        .send(a.auth_routes())
        .await;
    refused(&response, "give consent to AI use");
    assert_eq!(state().await, Some(false), "no consent was given");
    a.audited("give consent to AI use of a provider's data")
        .await;

    let response = AxumTestRequest::put(&path)
        .header("authorization", &a.session)
        .send(a.auth_routes())
        .await;
    assert_served(&response, StatusCode::OK);
    assert_eq!(
        state().await,
        Some(true),
        "the athlete's own consent is given"
    );
}

#[tokio::test]
async fn a_support_session_turns_analytics_consent_off_never_on() {
    let a = athlete("imp-analytics").await;
    let consent = |authorization: &str, enabled: bool| {
        AxumTestRequest::put("/api/user/analytics-consent")
            .header("authorization", authorization)
            .json(&json!({ "enabled": enabled }))
            .send(a.auth_routes())
    };
    let stored = || async {
        a.resources
            .common
            .repos
            .users
            .get_global(a.user.id)
            .await
            .unwrap()
            .unwrap()
            .analytics_consent
    };

    assert_served(&consent(&a.support, false).await, StatusCode::OK);
    refused(
        &consent(&a.support, true).await,
        "give consent to analytics",
    );
    assert!(!stored().await, "no consent was given");
    a.audited("give consent to analytics").await;

    assert_served(&consent(&a.session, true).await, StatusCode::OK);
    assert!(stored().await, "the athlete's own consent is given");
}

#[tokio::test]
async fn a_support_session_starts_no_oauth_connection() {
    let a = athlete("imp-oauth").await;
    let action = "start a provider's OAuth connection";

    for path in [
        "/api/oauth/authorize/strava",
        "/api/oauth/mobile/init/strava",
    ] {
        let response = AxumTestRequest::get(path)
            .header("authorization", &a.support)
            .send(a.auth_routes())
            .await;
        refused(&response, action);
    }
    a.audited(action).await;

    let response = AxumTestRequest::get("/api/oauth/authorize/strava")
        .header("authorization", &a.session)
        .send(a.auth_routes())
        .await;
    assert_served(&response, StatusCode::FOUND);
    let response = AxumTestRequest::get("/api/oauth/mobile/init/strava")
        .header("authorization", &a.session)
        .send(a.auth_routes())
        .await;
    assert_served(&response, StatusCode::OK);
}

#[tokio::test]
async fn a_support_session_stores_no_messaging_bot_credentials() {
    let a = athlete("imp-bot").await;
    let routes = || MessagingRoutes::routes(Arc::clone(&a.resources));
    let config = json!({
        "enabled": true,
        "credentials": { "bot_token": "12345:ABC-DEF", "webhook_secret": "tg_secret" }
    });
    let stored = || async {
        let body: Value = AxumTestRequest::get("/api/messaging/channels/telegram")
            .header("authorization", &a.session)
            .send(routes())
            .await
            .json();
        !body["config"].is_null()
    };

    let response = AxumTestRequest::put("/api/messaging/channels/telegram")
        .header("authorization", &a.support)
        .json(&config)
        .send(routes())
        .await;
    refused(&response, "store a messaging bot's credentials");
    assert!(!stored().await, "no credentials were stored");
    a.audited("store a messaging bot's credentials").await;

    let response = AxumTestRequest::put("/api/messaging/channels/telegram")
        .header("authorization", &a.session)
        .json(&config)
        .send(routes())
        .await;
    assert_served(&response, StatusCode::OK);
    assert!(stored().await, "the athlete's own credentials are stored");
}

#[tokio::test]
async fn a_support_session_stores_no_llm_key() {
    let a = athlete("imp-llm-key").await;
    let routes = || LlmSettingsRoutes::routes(Arc::clone(&a.resources));

    for scope in ["user", "tenant"] {
        let response = AxumTestRequest::put("/api/user/llm-settings")
            .header("authorization", &a.support)
            .json(&json!({ "provider": "groq", "api_key": "gsk-support", "scope": scope }))
            .send(routes())
            .await;
        refused(&response, &format!("store a {scope} LLM key"));
    }
    a.audited("store an LLM provider key").await;

    let response = AxumTestRequest::put("/api/user/llm-settings")
        .header("authorization", &a.session)
        .json(&json!({ "provider": "groq", "api_key": "gsk-own", "scope": "user" }))
        .send(routes())
        .await;
    assert_served(&response, StatusCode::OK);
    let saved: Value = response.json();
    assert_eq!(saved["success"], true, "{saved}");
}

#[tokio::test]
async fn a_support_session_opens_no_billing_session() {
    let a = athlete("imp-billing").await;
    let routes = || {
        pierre_routes_billing::billing_routes::<ServerContext>()
            .with_state(Arc::clone(&a.resources))
    };
    let checkout = json!({
        "tier": "professional",
        "success_url": "https://app.example/billing?upgrade=success",
        "cancel_url": "https://app.example/billing?upgrade=cancel",
    });
    let portal = json!({ "return_url": "https://app.example/billing" });

    for (path, body, action) in [
        (
            "/api/billing/checkout",
            &checkout,
            "start a billing checkout",
        ),
        ("/api/billing/portal", &portal, "open the billing portal"),
    ] {
        let response = AxumTestRequest::post(path)
            .header("authorization", &a.support)
            .json(body)
            .send(routes())
            .await;
        refused(&response, action);
        a.audited(action).await;
    }

    let response = AxumTestRequest::post("/api/billing/checkout")
        .header("authorization", &a.session)
        .json(&checkout)
        .send(routes())
        .await;
    assert_served(&response, StatusCode::OK);
    // No subscription yet: the athlete's own portal request is answered past
    // the refusal, by the lookup it needs.
    let response = AxumTestRequest::post("/api/billing/portal")
        .header("authorization", &a.session)
        .json(&portal)
        .send(routes())
        .await;
    assert_served(&response, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_support_session_asks_for_no_coach_access_but_withdraws_it() {
    let a = athlete("imp-coach-access").await;
    let routes = || CoachAccessRoutes::routes(Arc::clone(&a.resources));
    let ask = |authorization: &str| {
        AxumTestRequest::post("/api/me/coach-access-request")
            .header("authorization", authorization)
            .json(&json!({}))
            .send(routes())
    };
    let current = || async {
        let body: Value = AxumTestRequest::get("/api/me/coach-access-request")
            .header("authorization", &a.session)
            .send(routes())
            .await
            .json();
        body["request"].clone()
    };

    refused(&ask(&a.support).await, "ask for coach access");
    assert_eq!(current().await, Value::Null, "no request was made");
    a.audited("ask for coach access").await;

    assert_served(&ask(&a.session).await, StatusCode::CREATED);
    assert_eq!(
        current().await["status"],
        "pending",
        "the athlete's own session files a pending request"
    );
    let response = AxumTestRequest::delete("/api/me/coach-access-request")
        .header("authorization", &a.support)
        .send(routes())
        .await;
    assert_served(&response, StatusCode::OK);
}

#[tokio::test]
async fn a_support_session_deletes_no_account() {
    let a = athlete("imp-delete").await;
    let request = json!({ "confirm_email": a.user.email, "password": PASSWORD });
    let delete = |authorization: &str| {
        AxumTestRequest::post("/api/user/account-deletion")
            .header("authorization", authorization)
            .json(&request)
            .send(a.auth_routes())
    };
    let exists = || async {
        a.resources
            .common
            .repos
            .users
            .get_global(a.user.id)
            .await
            .unwrap()
            .is_some()
    };

    refused(&delete(&a.support).await, "delete the account");
    assert!(exists().await, "the account is still there");
    a.audited("delete the account").await;

    assert_served(&delete(&a.session).await, StatusCode::OK);
    assert!(!exists().await, "the athlete's own delete is served");
}

/// A chat turn an admin typed while acting as the athlete starts no provider
/// connection through `connect_provider`: the link outlives the support
/// session. The tool answers with the refusal, audited; the athlete's own
/// turn gets the authorization.
#[tokio::test]
async fn a_support_sessions_turn_connects_no_provider() {
    let a = athlete("imp-connect-tool").await;
    // The authorization rather than the hosted sign-in, so the athlete's own
    // call answers with a URL minted here (carnet#1494).
    set_connect_order(
        a.resources.common.repos.system_settings.as_ref(),
        "strava",
        ConnectOrder::OAuthFirst,
    )
    .await
    .unwrap();
    let runtime: Arc<dyn ToolRuntime> = a.resources.clone();
    // Built inside the call's future, so an executor made in the support
    // session's turn carries it, as the chat pipeline's does.
    let connect = || async {
        UniversalExecutor::new(Arc::clone(&runtime))
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp)
            .execute_tool(UniversalRequest {
                tool_name: "connect_provider".to_owned(),
                parameters: json!({ "provider": "strava" }),
                user_id: a.user.id.to_string(),
                protocol: "mcp".to_owned(),
                tenant_id: Some(a.tenant_id.to_string()),
            })
            .await
    };

    let support = support_grant(&a.resources, a.user.id).await;
    let refused = SUPPORT_SESSION
        .scope(Some(support), connect())
        .await
        .expect_err("the tool refuses a support session's turn");
    assert!(
        refused.to_string().contains(IMPERSONATION_WRITE_REFUSED),
        "the tool answers with the refusal: {refused}"
    );
    a.audited("start a provider connection").await;

    let served = connect().await.expect("the call completes");
    assert!(served.success, "{served:?}");
    let minted = served.result.expect("a payload");
    assert_eq!(minted["status"], "pending_authorization", "{minted}");
}
