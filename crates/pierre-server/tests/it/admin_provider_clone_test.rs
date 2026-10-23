// ABOUTME: carnet#1144 — a super-admin copies one account's scrape-session provider connection onto another
// ABOUTME: The copy decrypts to the same session, the source is untouched; OAuth grants, dead sessions and lesser tokens are refused

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `POST /admin/users/{id}/providers/{provider}/clone` gives an e2e test
//! account a working Garmin connection by copying an operator's scrape
//! session. Every test drives the real handler, with the session installer the
//! composition root wires (the auth route context), against a real database,
//! and asserts on rows: a refusal that answered the right status while writing
//! a token would still fail here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::{Extension, Json};
use chrono::{Duration, Utc};
use common::{create_test_server_resources, create_test_user_with_email};
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    EvidenceRegistry, MessagingStringsRegistry, PromptRegistry, ToolDescriptionRegistry,
    TrainingCatalogueRegistry,
};
use pierre_core::admin::models::{
    AdminAction, AdminPermission, AdminPermissions, CreateAdminTokenRequest, ValidatedAdminToken,
};
use pierre_core::constants::oauth_providers::{SCIOTTE_GARMIN, TOKEN_TYPE_SESSION};
use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId, UserOAuthToken};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::handlers::provider_clone::handle_clone_user_provider;
use pierre_routes_admin::handlers::types::CloneProviderRequest;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use uuid::Uuid;

const JWT_SECRET: &str = "test_admin_jwt_secret_for_provider_clone";

/// The admin context the composition root builds, with the auth route
/// context as the session installer.
fn admin_context(resources: &Arc<ServerContext>) -> Arc<AdminApiContext> {
    let mut context = AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: JWT_SECRET.to_owned(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(PromptRegistry::new()),
        tool_description_registry: Arc::new(ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(TrainingCatalogueRegistry::new()),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    });
    context.session_installer = Some(Arc::new(resources.auth_routes_context()));
    Arc::new(context)
}

/// A stored admin token, so the clone's audit row has a token to reference.
async fn admin_token(resources: &Arc<ServerContext>, is_super_admin: bool) -> ValidatedAdminToken {
    let generated = resources
        .common
        .repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "provider-clone-test".to_owned(),
                service_description: None,
                permissions: None,
                expires_in_days: None,
                is_super_admin,
                tenant_id: None,
                operator_user_id: None,
            },
            JWT_SECRET,
            resources.auth.jwks_manager.as_ref(),
        )
        .await
        .unwrap();
    ValidatedAdminToken {
        token_id: generated.token_id,
        service_name: "provider-clone-test".to_owned(),
        permissions: if is_super_admin {
            AdminPermissions::super_admin()
        } else {
            AdminPermissions::new(vec![AdminPermission::ManageUsers])
        },
        is_super_admin,
        tenant_id: None,
        user_info: None,
        operator_user_id: None,
    }
}

/// An active user in a tenant of their own.
async fn user(resources: &Arc<ServerContext>, label: &str) -> (Uuid, TenantId) {
    let email = format!("clone-{label}-{}@example.test", Uuid::new_v4());
    let (user_id, _) = create_test_user_with_email(&resources.agent.database, &email)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .expect("the test user has a tenant")
        .id;
    (user_id, tenant_id)
}

/// A scrape session as dravr-sciotte serializes it.
fn session_json(session_id: &str) -> String {
    json!({
        "session_id": session_id,
        "cookies": [{
            "name": "SESSIONID",
            "value": "cookie-material-do-not-log",
            "domain": ".garmin.com",
            "path": "/",
            "secure": true,
            "http_only": true,
        }],
        "created_at": serde_json::to_value(Utc::now()).unwrap(),
    })
    .to_string()
}

/// `backend` stored for the user as `token_type` holding `material`, with
/// its connection registered as `connection_type`.
async fn store_token(
    repos: &RepositoryRegistry,
    (user_id, tenant_id): (Uuid, TenantId),
    backend: &str,
    token_type: &str,
    material: &str,
) {
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: backend.to_owned(),
            access_token: SecretString::from(material.to_owned()),
            refresh_token: None,
            token_type: token_type.to_owned(),
            expires_at: Some(Utc::now() + Duration::days(30)),
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let connection_type = if token_type == TOKEN_TYPE_SESSION {
        ConnectionType::Manual
    } else {
        ConnectionType::OAuth
    };
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, backend, &connection_type, None)
        .await
        .unwrap();
}

async fn clone(
    context: &Arc<AdminApiContext>,
    token: ValidatedAdminToken,
    to: Uuid,
    provider: &str,
    from: Uuid,
) -> Response {
    handle_clone_user_provider(
        State(Arc::clone(context)),
        Extension(token),
        Path((to.to_string(), provider.to_owned())),
        Json(CloneProviderRequest {
            from_user_id: from.to_string(),
            reason: Some("onboarding e2e".to_owned()),
        }),
    )
    .await
    .expect("the clone answers a response")
}

async fn body_json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn holds_no_token(repos: &RepositoryRegistry, user_id: Uuid) {
    let tokens = repos.oauth_tokens.get_tokens(user_id, None).await.unwrap();
    assert!(
        tokens.is_empty(),
        "nothing may be written for the target: {:?}",
        tokens.iter().map(|t| &t.provider).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_super_admin_clones_a_garmin_session_onto_another_account() {
    let resources = &create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let context = admin_context(resources);
    let source = user(resources, "source").await;
    let target = user(resources, "target").await;
    let material = session_json("sess-garmin-operator");
    store_token(repos, source, SCIOTTE_GARMIN, TOKEN_TYPE_SESSION, &material).await;
    let token = admin_token(resources, true).await;
    let token_id = token.token_id.clone();

    let response = clone(&context, token, target.0, "garmin", source.0).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["from_user_id"], source.0.to_string(), "{body}");
    assert_eq!(body["data"]["to_user_id"], target.0.to_string(), "{body}");
    assert_eq!(body["data"]["provider"], "garmin", "{body}");
    assert_eq!(body["data"]["backend"], SCIOTTE_GARMIN, "{body}");
    assert!(
        !body.to_string().contains("cookie-material-do-not-log"),
        "the session never reaches the answer: {body}"
    );

    let copied = repos
        .oauth_tokens
        .get_token(target.0, target.1, SCIOTTE_GARMIN)
        .await
        .unwrap()
        .expect("the target holds the session in its own tenant");
    assert_eq!(copied.token_type, TOKEN_TYPE_SESSION);
    let copied: Value = serde_json::from_str(copied.access_token.expose_secret()).unwrap();
    let original: Value = serde_json::from_str(&material).unwrap();
    assert_eq!(copied, original, "the target decrypts to the same session");

    let kept = repos
        .oauth_tokens
        .get_token(source.0, source.1, SCIOTTE_GARMIN)
        .await
        .unwrap()
        .expect("the source keeps its session");
    assert_eq!(kept.access_token.expose_secret(), material.as_str());

    let connections = repos
        .provider_connections
        .get_for_user(target.0, Some(target.1))
        .await
        .unwrap();
    let connection = connections
        .iter()
        .find(|c| c.provider == SCIOTTE_GARMIN)
        .unwrap_or_else(|| panic!("the target is connected: {connections:?}"));
    assert_eq!(connection.connection_type, ConnectionType::Manual);
    assert_eq!(connection.status, ConnectionStatus::Active);
    assert!(
        connection.metadata.is_none(),
        "a real connection, not a synthetic seed"
    );

    let audit = repos
        .admin
        .get_token_usage_history(
            &token_id,
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    assert!(
        audit
            .iter()
            .any(|row| row.action == AdminAction::CloneProviderConnection
                && row.success
                && row
                    .target_resource
                    .as_deref()
                    .is_some_and(|t| t.contains(&target.0.to_string()))),
        "the clone is audited: {audit:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_manage_users_token_cannot_clone_and_nothing_is_written() {
    let resources = &create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let context = admin_context(resources);
    let source = user(resources, "source").await;
    let target = user(resources, "target").await;
    store_token(
        repos,
        source,
        SCIOTTE_GARMIN,
        TOKEN_TYPE_SESSION,
        &session_json("sess-garmin-guarded"),
    )
    .await;

    let response = clone(
        &context,
        admin_token(resources, false).await,
        target.0,
        "garmin",
        source.0,
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    holds_no_token(repos, target.0).await;
}

/// An OAuth grant rotates on refresh and is revoked upstream on disconnect,
/// so sharing it would break both accounts: refused, nothing written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_strava_oauth_grant_is_refused() {
    let resources = &create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let context = admin_context(resources);
    let source = user(resources, "source").await;
    let target = user(resources, "target").await;
    store_token(
        repos,
        source,
        "strava",
        "Bearer",
        "access-material-do-not-log",
    )
    .await;

    let response = clone(
        &context,
        admin_token(resources, true).await,
        target.0,
        "strava",
        source.0,
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("OAuth grant")),
        "{body}"
    );
    holds_no_token(repos, target.0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_target_is_not_found() {
    let resources = &create_test_server_resources().await.unwrap();
    let context = admin_context(resources);
    let source = user(resources, "source").await;
    store_token(
        &resources.common.repos,
        source,
        SCIOTTE_GARMIN,
        TOKEN_TYPE_SESSION,
        &session_json("sess-garmin-nowhere"),
    )
    .await;

    let error = handle_clone_user_provider(
        State(Arc::clone(&context)),
        Extension(admin_token(resources, true).await),
        Path((Uuid::new_v4().to_string(), "garmin".to_owned())),
        Json(CloneProviderRequest {
            from_user_id: source.0.to_string(),
            reason: None,
        }),
    )
    .await
    .expect_err("an unknown target is an error");

    assert_eq!(error.http_status(), 404, "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloning_an_account_onto_itself_is_refused() {
    let resources = &create_test_server_resources().await.unwrap();
    let context = admin_context(resources);
    let source = user(resources, "self").await;
    store_token(
        &resources.common.repos,
        source,
        SCIOTTE_GARMIN,
        TOKEN_TYPE_SESSION,
        &session_json("sess-garmin-self"),
    )
    .await;

    let response = clone(
        &context,
        admin_token(resources, true).await,
        source.0,
        "garmin",
        source.0,
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// A session flagged for re-authentication is dead: copying it would hand the
/// target a connection that fails on its first read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_that_needs_reauth_is_refused() {
    let resources = &create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let context = admin_context(resources);
    let source = user(resources, "source").await;
    let target = user(resources, "target").await;
    store_token(
        repos,
        source,
        SCIOTTE_GARMIN,
        TOKEN_TYPE_SESSION,
        &session_json("sess-garmin-dead"),
    )
    .await;
    repos
        .provider_connections
        .mark_needs_reauth(
            source.0,
            source.1,
            SCIOTTE_GARMIN,
            Some("session_expired"),
            Utc::now() + Duration::seconds(1),
        )
        .await
        .unwrap();

    let response = clone(
        &context,
        admin_token(resources, true).await,
        target.0,
        "garmin",
        source.0,
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    holds_no_token(repos, target.0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_source_without_the_session_is_not_found() {
    let resources = &create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let context = admin_context(resources);
    let source = user(resources, "source").await;
    let target = user(resources, "target").await;

    let response = clone(
        &context,
        admin_token(resources, true).await,
        target.0,
        "garmin",
        source.0,
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    holds_no_token(repos, target.0).await;
}
