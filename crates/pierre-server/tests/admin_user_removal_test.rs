// ABOUTME: carnet#502 — an operator disconnects a user's provider, or removes the user, through the chokepoint
// ABOUTME: Grants revoked upstream under their own app, no row survives, partial failures named, scoped tokens held in

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `DELETE /admin/users/{id}` used to delete the row and let the cascade take
//! the tokens: our seat count dropped, the grant at Strava did not, and the
//! athlete kept counting against Strava's athlete cap. A user who owned a
//! coaching group could not be deleted at all — the foreign key failed the
//! delete as a 500. And there was no way for an operator to disconnect one
//! provider for someone else.
//!
//! Every test drives the real handlers against a real database and points the
//! Strava revocation endpoint at a local stub, so "revoked at the provider" is
//! asserted on the wire, not inferred from the rows being gone (a cascade
//! deletes the rows too, and revokes nothing).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{Duration, Utc};
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    EvidenceRegistry, MessagingStringsRegistry, PromptRegistry, ToolDescriptionRegistry,
    TrainingCatalogueRegistry,
};
use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{
    ConnectionType, Subscription, SubscriptionStatus, Tenant, TenantId, TenantOAuthCredentials,
    User, UserOAuthToken, UserReferenceKind, UserStatus, UserTier,
};
use pierre_database::backends::factory::Database;
#[cfg(feature = "postgresql")]
use pierre_database::repositories::POSTGRES_USER_PURGE;
use pierre_database::repositories::{
    CreateSessionParams, InsertMessageParams, UserPurge, DELETION_BLOCKERS_SQL,
    POSTGRES_ONLY_USER_OWNED_TABLES, SQLITE_USER_PURGE,
};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::a2a::client::ClientRegistrationRequest;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::handlers::strava_pool::{
    handle_list_strava_seats, handle_upsert_strava_pool_app,
};
use pierre_routes_admin::handlers::types::DeleteUserRequest;
use pierre_routes_admin::handlers::user_removal::{
    handle_delete_user, handle_disconnect_user_provider,
};
use pierre_routes_admin::handlers::users::handle_get_user;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_routes_auth::OAuthService;
use pierre_services::provider_revocation::RevocationOutcome;
use pierre_services::user_removal::{Interruption, ProviderDisconnector};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::common::create_test_server_resources;

/// The refresh token every seeded Strava grant carries; a revocation spends it.
const REFRESH_TOKEN: &str = "refresh-material-do-not-log";

/// The env Strava app every test in this binary configures.
const ENV_CLIENT_ID: &str = "removal-test-client";
const ENV_CLIENT_SECRET: &str = "removal-test-secret";

/// The pool app a seeded grant can name as its issuer, and its secret.
const POOL_APP: &str = "201455";
const POOL_SECRET: &str = "poolsecretvaluewithlength30chr";

/// A stand-in for Strava's revocation endpoint that answers every request
/// with one status and hands the raw bytes back. A request is queued BEFORE
/// its response is written, so once the disconnect has its answer the request
/// is already here to read.
struct RevokeStub {
    url: String,
    requests: mpsc::UnboundedReceiver<String>,
}

impl RevokeStub {
    /// A stub that confirms every revocation.
    async fn start() -> Self {
        Self::answering("200 OK").await
    }

    /// A stub that answers every revocation with `status` (`"401 Unauthorized"`).
    async fn answering(status: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::unbounded_channel();
        let response =
            format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let raw = read_request(&mut stream).await;
                if tx.send(raw).is_err() {
                    return;
                }
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        Self { url, requests: rx }
    }

    /// Every request received so far.
    fn received(&mut self) -> Vec<String> {
        let mut seen = Vec::new();
        while let Ok(raw) = self.requests.try_recv() {
            seen.push(raw);
        }
        seen
    }
}

/// Read one HTTP/1.1 request: the head up to the blank line, then exactly
/// `content-length` bytes of body, however the client split its writes.
async fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return String::from_utf8_lossy(&buf).into_owned();
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
    let content_length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + content_length {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// A server context with the Strava client credentials the revocation's
/// env fallback resolves, set before the resources exist. Every test in this
/// binary sets the same values, so parallel tests cannot disagree.
async fn resources() -> Arc<ServerContext> {
    env::set_var("STRAVA_CLIENT_ID", ENV_CLIENT_ID);
    env::set_var("STRAVA_CLIENT_SECRET", ENV_CLIENT_SECRET);
    create_test_server_resources().await.unwrap()
}

/// The disconnect chokepoint as the composition root builds it, with Strava's
/// revocation endpoint pointed at `revoke_url`.
fn chokepoint(resources: &Arc<ServerContext>, revoke_url: &str) -> Arc<dyn ProviderDisconnector> {
    let mut config = (*resources.common.config).clone();
    revoke_url.clone_into(&mut config.external_services.strava_api.revoke_url);
    Arc::new(OAuthService::new(resources.data(), Arc::new(config)))
}

/// The admin context the composition root builds, with the disconnect
/// chokepoint wired and Strava's revocation endpoint pointed at `revoke_url`.
fn admin_context(resources: &Arc<ServerContext>, revoke_url: &str) -> Arc<AdminApiContext> {
    admin_context_with(resources, chokepoint(resources, revoke_url))
}

/// The admin context with `disconnector` standing in for the chokepoint.
fn admin_context_with(
    resources: &Arc<ServerContext>,
    disconnector: Arc<dyn ProviderDisconnector>,
) -> Arc<AdminApiContext> {
    let mut context = AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: "test_admin_jwt_secret_for_user_removal".to_owned(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
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
    });
    context.provider_disconnector = Some(disconnector);
    Arc::new(context)
}

fn token_with(permissions: AdminPermissions, is_super_admin: bool) -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("test:{}", Uuid::new_v4()),
        service_name: "user-removal-test".to_owned(),
        permissions,
        is_super_admin,
        tenant_id: None,
        user_info: None,
    }
}

/// A `ManageUsers` token scoped to one tenant, as
/// `pierre-cli token generate --permissions manage_users --tenant-id` mints it.
fn scoped_manage_users_token(tenant_id: TenantId) -> ValidatedAdminToken {
    ValidatedAdminToken {
        tenant_id: Some(tenant_id.to_string()),
        ..manage_users_token()
    }
}

fn manage_users_token() -> ValidatedAdminToken {
    token_with(
        AdminPermissions::new(vec![AdminPermission::ManageUsers]),
        false,
    )
}

fn super_admin_token() -> ValidatedAdminToken {
    token_with(AdminPermissions::super_admin(), true)
}

async fn body_json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// An active user with their own tenant, named so the seat listing can be
/// read by email.
async fn seed_user(repos: &RepositoryRegistry, label: &str) -> (Uuid, TenantId, String) {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email.clone(), "hash".to_owned(), Some(label.to_owned()));
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id, email)
}

/// A Strava grant as the OAuth callback writes it: token row plus connection
/// row. `app` is the pool app that issued it (`None` for the env app).
async fn connect_strava(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    app: Option<&str>,
) {
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    store_strava_token(repos, user_id, tenant_id, app).await;
}

/// A Strava token as the OAuth callback stores it, issued by `app`.
fn strava_token(user_id: Uuid, tenant_id: TenantId, app: Option<&str>) -> UserOAuthToken {
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: "strava".to_owned(),
        access_token: "access-material-do-not-log".to_owned(),
        refresh_token: Some(REFRESH_TOKEN.to_owned()),
        token_type: "Bearer".to_owned(),
        expires_at: Some(Utc::now() + Duration::hours(6)),
        scope: Some("read,activity:read_all".to_owned()),
        provider_user_id: None,
        oauth_app_client_id: app.map(str::to_owned),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn store_strava_token(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    app: Option<&str>,
) {
    repos
        .oauth_tokens
        .upsert_token(&strava_token(user_id, tenant_id, app))
        .await
        .unwrap();
}

/// A second tenant `owner` owns, and so belongs to.
async fn extra_tenant(repos: &RepositoryRegistry, owner: Uuid) -> TenantId {
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "second tenant".to_owned(),
            slug: format!("second-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    tenant_id
}

/// A coaching group owned by `owner`, bound to a fresh agent in their tenant.
async fn seed_group(
    repos: &RepositoryRegistry,
    owner: Uuid,
    tenant_id: TenantId,
    name: &str,
) -> Uuid {
    let agent_id = repos
        .agents
        .create_system_agent(
            owner,
            tenant_id,
            &CreateSystemAgentRequest {
                title: format!("{name} coach"),
                description: None,
                system_prompt: "You coach the group.".to_owned(),
                category: AgentCategory::Recovery,
                tags: vec![],
                visibility: AgentVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    let now = Utc::now();
    repos
        .groups
        .create_group(
            tenant_id,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant_id.to_string(),
                name: name.to_owned(),
                description: None,
                agent_id,
                owner_id: owner,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id
}

async fn join_group(
    repos: &RepositoryRegistry,
    group_id: Uuid,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

fn delete_request() -> Json<DeleteUserRequest> {
    Json(DeleteUserRequest {
        reason: Some("carnet#502 test".to_owned()),
    })
}

/// The one Strava revocation the stub saw, in the RFC 7009 shape the
/// chokepoint sends: the stored refresh token, spent under the credentials of
/// the app that issued the grant (`client_id`/`client_secret` as HTTP Basic),
/// since Strava withdraws a grant only for the application that holds it.
fn assert_one_strava_revocation(requests: &[String], client_id: &str, client_secret: &str) {
    assert_eq!(
        requests.len(),
        1,
        "exactly one revocation must reach Strava, got {}: {requests:?}",
        requests.len()
    );
    assert!(
        requests[0].starts_with("POST "),
        "revocation is a POST; got:\n{}",
        requests[0]
    );
    assert!(
        requests[0].contains(&format!("token={REFRESH_TOKEN}")),
        "revocation must spend the stored refresh token; got:\n{}",
        requests[0]
    );
    let basic = BASE64.encode(format!("{client_id}:{client_secret}"));
    assert!(
        requests[0]
            .lines()
            .any(|line| line.eq_ignore_ascii_case(&format!("authorization: Basic {basic}"))),
        "revocation must authenticate as {client_id}; got:\n{}",
        requests[0]
    );
}

#[tokio::test]
async fn admin_disconnect_revokes_upstream_and_removes_the_rows() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, tenant_id, email) = seed_user(repos, "disconnect").await;
    connect_strava(repos, user_id, tenant_id, None).await;

    let response = handle_disconnect_user_provider(
        State(context),
        Extension(manage_users_token()),
        Path((user_id.to_string(), "strava".to_owned())),
    )
    .await
    .expect("disconnect handler");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["email"], email.as_str());
    let disconnected = body["data"]["disconnected"].as_array().unwrap();
    assert_eq!(disconnected.len(), 1, "one tenant held strava: {body}");
    assert_eq!(disconnected[0]["provider"], "strava");
    assert_eq!(disconnected[0]["tenant_id"], tenant_id.to_string().as_str());

    assert_one_strava_revocation(&stub.received(), ENV_CLIENT_ID, ENV_CLIENT_SECRET);
    assert_eq!(disconnected[0]["revocation"]["status"], "revoked");
    assert!(
        repos
            .oauth_tokens
            .get_token(user_id, tenant_id, "strava")
            .await
            .unwrap()
            .is_none(),
        "the token row must be gone"
    );
    assert!(
        repos
            .provider_connections
            .get_for_user(user_id, None)
            .await
            .unwrap()
            .is_empty(),
        "the connection row must be gone"
    );
    assert!(
        repos.users.get_global(user_id).await.unwrap().is_some(),
        "a disconnect removes the grant, never the account"
    );
}

#[tokio::test]
async fn admin_disconnect_without_manage_users_is_forbidden_and_touches_nothing() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, tenant_id, _) = seed_user(repos, "forbidden").await;
    connect_strava(repos, user_id, tenant_id, None).await;

    let response = handle_disconnect_user_provider(
        State(context),
        Extension(token_with(
            AdminPermissions::new(vec![AdminPermission::ManageAdminTokens]),
            false,
        )),
        Path((user_id.to_string(), "strava".to_owned())),
    )
    .await
    .expect("a refusal is a response, not an error");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = body_json(response).await;
    assert_eq!(body["message"], "Permission denied: ManageUsers required");

    assert!(stub.received().is_empty(), "nothing may reach Strava");
    assert!(
        repos
            .oauth_tokens
            .get_token(user_id, tenant_id, "strava")
            .await
            .unwrap()
            .is_some(),
        "the token row must survive a refused disconnect"
    );
    assert_eq!(
        repos
            .provider_connections
            .get_for_user(user_id, None)
            .await
            .unwrap()
            .len(),
        1,
        "the connection row must survive a refused disconnect"
    );
}

#[tokio::test]
async fn admin_disconnect_404s_for_an_unheld_provider_and_an_unknown_user() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, _, _) = seed_user(repos, "unheld").await;

    let response = handle_disconnect_user_provider(
        State(context.clone()),
        Extension(manage_users_token()),
        Path((user_id.to_string(), "strava".to_owned())),
    )
    .await
    .expect("an unheld provider is a response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = body_json(response).await;
    assert_eq!(
        body["message"],
        "User has no strava connection to disconnect"
    );

    let error = handle_disconnect_user_provider(
        State(context),
        Extension(manage_users_token()),
        Path((Uuid::new_v4().to_string(), "strava".to_owned())),
    )
    .await
    .expect_err("an unknown user is not found");
    assert_eq!(error.code, ErrorCode::ResourceNotFound);
    assert!(stub.received().is_empty(), "nothing may reach Strava");
}

#[tokio::test]
async fn get_user_lists_the_providers_a_delete_would_disconnect() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, tenant_id, email) = seed_user(repos, "preview").await;
    connect_strava(repos, user_id, tenant_id, None).await;

    let response = handle_get_user(
        State(context),
        Extension(manage_users_token()),
        Path(user_id.to_string()),
    )
    .await
    .expect("get user handler");
    let body = body_json(response.into_response()).await;
    assert_eq!(body["data"]["email"], email.as_str());
    let providers = body["data"]["connected_providers"].as_array().unwrap();
    assert_eq!(providers.len(), 1, "one strava grant: {body}");
    assert_eq!(providers[0]["provider"], "strava");
    assert_eq!(providers[0]["tenant_id"], tenant_id.to_string().as_str());
}

#[tokio::test]
async fn delete_revokes_every_grant_then_removes_the_user_and_its_tokens() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, tenant_id, email) = seed_user(repos, "deleted").await;
    repos
        .oauth_tokens
        .upsert_strava_pool_app(POOL_APP, POOL_SECRET, 10, Some("pool-2"))
        .await
        .unwrap();
    connect_strava(repos, user_id, tenant_id, Some(POOL_APP)).await;

    let response = handle_delete_user(
        State(context),
        Extension(manage_users_token()),
        Path(user_id.to_string()),
        delete_request(),
    )
    .await
    .expect("delete handler");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["deleted_user"]["email"], email.as_str());
    let disconnected = body["data"]["disconnected"].as_array().unwrap();
    assert_eq!(disconnected.len(), 1, "strava was disconnected: {body}");
    assert_eq!(disconnected[0]["provider"], "strava");
    assert_eq!(disconnected[0]["revocation"]["status"], "revoked");
    assert_eq!(body["data"]["not_revocable"].as_array().unwrap().len(), 0);
    assert_eq!(body["data"]["reason"], "carnet#502 test");

    // Revoked at Strava, not just dropped by the cascade, and under the pool
    // app that issued the grant: the env client could not withdraw it.
    assert_one_strava_revocation(&stub.received(), POOL_APP, POOL_SECRET);
    assert!(
        repos.users.get_global(user_id).await.unwrap().is_none(),
        "the user must be gone"
    );
    assert!(
        repos
            .oauth_tokens
            .list_token_providers(user_id)
            .await
            .unwrap()
            .is_empty(),
        "no token row may survive the delete"
    );
    assert!(
        repos
            .provider_connections
            .get_for_user(user_id, None)
            .await
            .unwrap()
            .is_empty(),
        "no connection row may survive the delete"
    );
}

#[tokio::test]
async fn delete_of_a_group_owner_is_refused_naming_the_group_and_deletes_nothing() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (owner_id, tenant_id, _) = seed_user(repos, "owner").await;
    connect_strava(repos, owner_id, tenant_id, None).await;
    let group_id = seed_group(repos, owner_id, tenant_id, "Tuesday Intervals").await;

    let response = handle_delete_user(
        State(context),
        Extension(manage_users_token()),
        Path(owner_id.to_string()),
        delete_request(),
    )
    .await
    .expect("a refusal is a response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        body["message"],
        "User cannot be deleted until these are reassigned or removed: authored agent 'Tuesday Intervals coach', which other users rely on; owns coaching group 'Tuesday Intervals'"
    );
    let blockers = body["data"]["blockers"].as_array().unwrap();
    assert_eq!(
        blockers.len(),
        2,
        "the ownership and the group's tenant agent block: {body}"
    );
    assert_eq!(
        blockers[0]["kind"],
        UserReferenceKind::AuthoredAgent.as_str()
    );
    assert_eq!(blockers[0]["detail"], "Tuesday Intervals coach");
    assert_eq!(
        blockers[1]["kind"],
        UserReferenceKind::OwnsCoachingGroup.as_str()
    );
    assert_eq!(blockers[1]["detail"], "Tuesday Intervals");

    // Nothing was touched: no revocation, the grant, the group and the user all stand.
    assert!(
        stub.received().is_empty(),
        "a refused delete revokes nothing"
    );
    assert!(repos.users.get_global(owner_id).await.unwrap().is_some());
    assert!(
        repos
            .oauth_tokens
            .get_token(owner_id, tenant_id, "strava")
            .await
            .unwrap()
            .is_some(),
        "the grant must survive a refused delete"
    );
    assert!(repos
        .groups
        .get_group(&group_id.to_string(), tenant_id)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn delete_of_a_plain_member_removes_the_membership_and_the_user() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (owner_id, owner_tenant, _) = seed_user(repos, "keeper").await;
    let (member_id, member_tenant, _) = seed_user(repos, "member").await;
    let group_id = seed_group(repos, owner_id, owner_tenant, "Long Run Crew").await;
    join_group(repos, group_id, owner_id, owner_tenant).await;
    join_group(repos, group_id, member_id, member_tenant).await;
    assert_eq!(
        repos
            .groups
            .count_members(&group_id.to_string())
            .await
            .unwrap(),
        2
    );

    let response = handle_delete_user(
        State(context),
        Extension(manage_users_token()),
        Path(member_id.to_string()),
        delete_request(),
    )
    .await
    .expect("delete handler");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["memberships_removed"], 1);
    assert_eq!(body["data"]["disconnected"].as_array().unwrap().len(), 0);

    assert!(repos.users.get_global(member_id).await.unwrap().is_none());
    assert!(
        repos
            .groups
            .get_member(&group_id.to_string(), member_id)
            .await
            .unwrap()
            .is_none(),
        "the membership row must be gone"
    );
    let members = repos
        .groups
        .list_members(&group_id.to_string())
        .await
        .unwrap();
    assert_eq!(members.len(), 1, "the owner stays in the group");
    assert_eq!(members[0].user_id, owner_id);
}

#[tokio::test]
async fn a_residual_foreign_key_on_delete_is_a_conflict_not_a_database_error() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (owner_id, tenant_id, _) = seed_user(repos, "residual").await;
    seed_group(repos, owner_id, tenant_id, "Hill Repeats").await;

    // Straight at the repository, past the blocker read: the database itself
    // refuses, and the refusal must say "still referenced", not "failed".
    let error = repos
        .users
        .delete(owner_id)
        .await
        .expect_err("the group's owner_id must block the delete");
    assert_eq!(error.code, ErrorCode::ResourceLocked);
    assert!(repos.users.get_global(owner_id).await.unwrap().is_some());
}

#[tokio::test]
async fn seats_listing_names_each_holder_and_whether_it_counts() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);

    // Live on the env app, live on a pool app, dead (needs_reauth over a
    // dead grant), BYO app, a token with no connection row (legacy), live on
    // a disabled pool app, and needs_reauth over our own client credentials:
    // five hold a seat, two do not.
    let (live_env, t1, live_env_email) = seed_user(repos, "live-env").await;
    connect_strava(repos, live_env, t1, None).await;
    repos
        .oauth_tokens
        .upsert_strava_pool_app(POOL_APP, POOL_SECRET, 10, Some("pool-2"))
        .await
        .unwrap();
    let (live_pool, t2, live_pool_email) = seed_user(repos, "live-pool").await;
    connect_strava(repos, live_pool, t2, Some(POOL_APP)).await;
    let (dead, t3, dead_email) = seed_user(repos, "dead").await;
    connect_strava(repos, dead, t3, None).await;
    repos
        .provider_connections
        .mark_needs_reauth(dead, t3, "strava", Some("invalid_grant"), Utc::now())
        .await
        .unwrap();
    let (byo, t4, byo_email) = seed_user(repos, "byo").await;
    connect_strava(repos, byo, t4, None).await;
    repos
        .oauth_tokens
        .store_user_oauth_app(byo, "strava", "byo-client", "byo-secret", "https://x/cb")
        .await
        .unwrap();
    let (legacy, t5, legacy_email) = seed_user(repos, "legacy").await;
    store_strava_token(repos, legacy, t5, None).await;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("301455", POOL_SECRET, 10, Some("drained"))
        .await
        .unwrap();
    repos
        .oauth_tokens
        .set_strava_pool_app_enabled("301455", false)
        .await
        .unwrap();
    let (drained, t6, drained_email) = seed_user(repos, "drained").await;
    connect_strava(repos, drained, t6, Some("301455")).await;
    let (stale_client, t7, stale_client_email) = seed_user(repos, "stale-client").await;
    connect_strava(repos, stale_client, t7, None).await;
    repos
        .provider_connections
        .mark_needs_reauth(
            stale_client,
            t7,
            "strava",
            Some("invalid_client"),
            Utc::now(),
        )
        .await
        .unwrap();
    let (expired, t8, expired_email) = seed_user(repos, "expired-session").await;
    connect_strava(repos, expired, t8, None).await;
    repos
        .provider_connections
        .mark_needs_reauth(expired, t8, "strava", Some("session_expired"), Utc::now())
        .await
        .unwrap();

    let forbidden =
        handle_list_strava_seats(State(context.clone()), Extension(manage_users_token()))
            .await
            .expect("a refusal is a response");
    assert_eq!(
        forbidden.status(),
        StatusCode::FORBIDDEN,
        "the seat listing takes the pool routes' super-admin gate"
    );

    let response = handle_list_strava_seats(State(context), Extension(super_admin_token()))
        .await
        .expect("seats handler");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let holders = body["data"]["holders"].as_array().unwrap();
    let ours: Vec<&Value> = holders
        .iter()
        .filter(|h| {
            [
                &live_env_email,
                &live_pool_email,
                &dead_email,
                &byo_email,
                &legacy_email,
                &drained_email,
                &stale_client_email,
                &expired_email,
            ]
            .iter()
            .any(|e| h["email"] == e.as_str())
        })
        .collect();
    assert_eq!(ours.len(), 8, "one row per stored Strava token: {body}");
    let holder = |email: &str| -> &Value {
        ours.iter()
            .copied()
            .find(|h| h["email"] == email)
            .unwrap_or_else(|| panic!("{email} missing from {body}"))
    };

    let row = holder(&live_env_email);
    assert_eq!(row["counts_as_seat"], true);
    assert_eq!(row["app"], Value::Null, "the env app is a null app");
    assert_eq!(row["status"], "active");
    assert_eq!(row["user_id"], live_env.to_string().as_str());

    let row = holder(&live_pool_email);
    assert_eq!(row["counts_as_seat"], true);
    assert_eq!(row["app"], POOL_APP);

    let row = holder(&dead_email);
    assert_eq!(row["counts_as_seat"], false, "needs_reauth frees the seat");
    assert_eq!(row["status"], "needs_reauth");

    let row = holder(&byo_email);
    assert_eq!(
        row["counts_as_seat"], false,
        "a BYO app holds no shared seat"
    );

    let row = holder(&legacy_email);
    assert_eq!(
        row["counts_as_seat"], true,
        "no connection row still counts"
    );
    assert_eq!(row["status"], Value::Null);
    assert!(row["connected_at"].as_str().is_some_and(|s| !s.is_empty()));

    let row = holder(&drained_email);
    assert_eq!(
        row["counts_as_seat"], true,
        "Strava still counts an athlete on a disabled app"
    );
    assert_eq!(row["app"], "301455");

    let row = holder(&stale_client_email);
    assert_eq!(
        row["counts_as_seat"], true,
        "a refresh refused over our own client credentials leaves the grant live at Strava"
    );
    assert_eq!(row["status"], "needs_reauth");

    let row = holder(&expired_email);
    assert_eq!(
        row["counts_as_seat"], false,
        "a needs_reauth for anything but our own client credentials is a grant we cannot use"
    );
    assert_eq!(row["status"], "needs_reauth");

    // Five athletes hold a seat on some app. The offered capacity counts the
    // env app and the enabled pool app only, so the one on the disabled app
    // is held but not in use of what a newcomer can take.
    assert_eq!(body["data"]["seats_held"], 5);
    assert_eq!(body["data"]["seats"]["used"], 4);
    assert_eq!(
        body["message"],
        format!(
            "8 Strava token holder(s), 5 holding a seat on any app; 4 of {} offered seat(s) in use",
            body["data"]["seats"]["total"]
        )
    );
}

/// A disconnect the provider refuses still removes the rows, and says the
/// grant may still be authorized instead of reporting it revoked.
#[tokio::test]
async fn an_unconfirmed_revocation_is_reported_as_such_not_as_revoked() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::answering("401 Unauthorized").await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, tenant_id, _) = seed_user(repos, "unconfirmed").await;
    connect_strava(repos, user_id, tenant_id, None).await;

    let response = handle_disconnect_user_provider(
        State(context),
        Extension(manage_users_token()),
        Path((user_id.to_string(), "strava".to_owned())),
    )
    .await
    .expect("disconnect handler");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let disconnected = body["data"]["disconnected"].as_array().unwrap();
    assert_eq!(disconnected.len(), 1, "{body}");
    assert_eq!(disconnected[0]["revocation"]["status"], "unconfirmed");
    assert_eq!(
        disconnected[0]["revocation"]["reason"],
        "the provider answered HTTP 401 Unauthorized"
    );

    assert_one_strava_revocation(&stub.received(), ENV_CLIENT_ID, ENV_CLIENT_SECRET);
    assert!(
        repos
            .oauth_tokens
            .list_token_providers(user_id)
            .await
            .unwrap()
            .is_empty(),
        "the local rows go whatever the provider answered"
    );
}

/// The delete takes every row the user owns in a table no foreign key
/// cascades to, a provider this build cannot revoke included, and reports
/// each table it cleared.
#[tokio::test]
async fn delete_clears_every_row_no_foreign_key_cascades_to() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (owner_id, owner_tenant, _) = seed_user(repos, "keeper-of-rows").await;
    let group_id = seed_group(repos, owner_id, owner_tenant, "Track Tuesdays").await;
    let (user_id, tenant_id, _) = seed_user(repos, "orphan-maker").await;
    let user = user_id.to_string();
    let tenant = tenant_id.to_string();

    // `synthetic` is registered in no build, so nothing can revoke it: its
    // token and connection rows are the delete's to clear.
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            "synthetic",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            provider: "synthetic".to_owned(),
            ..strava_token(user_id, tenant_id, None)
        })
        .await
        .unwrap();
    join_group(repos, group_id, user_id, tenant_id).await;
    repos
        .user_onboarding
        .set_onboarding_step(&user, "welcome", "done", None, Some(&tenant))
        .await
        .unwrap();
    let code = format!("code{}", Uuid::new_v4().simple());
    repos
        .short_links
        .create_short_link(
            &code,
            "https://example.com/plan",
            &tenant,
            &user,
            Utc::now() + Duration::days(1),
        )
        .await
        .unwrap();
    repos
        .usage_counters
        .increment_counter(&tenant, &user, "chat_turns", "2026-09", 3)
        .await
        .unwrap();

    let response = handle_delete_user(
        State(context),
        Extension(manage_users_token()),
        Path(user),
        delete_request(),
    )
    .await
    .expect("delete handler");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;

    let not_revocable = body["data"]["not_revocable"].as_array().unwrap();
    assert_eq!(not_revocable.len(), 1, "{body}");
    assert_eq!(not_revocable[0]["provider"], "synthetic");
    assert_eq!(body["data"]["disconnected"].as_array().unwrap().len(), 0);
    assert_eq!(body["data"]["memberships_removed"], 1);
    let rows_removed = body["data"]["rows_removed"].as_object().unwrap();
    let cleared: BTreeSet<&str> = rows_removed.keys().map(String::as_str).collect();
    assert_eq!(
        cleared,
        BTreeSet::from([
            "coaching_group_members",
            "provider_connections",
            "short_links",
            "usage_counters",
            "user_oauth_tokens",
            "user_onboarding",
        ]),
        "each seeded table is cleared by the delete itself: {body}"
    );
    assert!(rows_removed.values().all(|n| n == 1), "{body}");
    assert!(stub.received().is_empty(), "nothing is revocable here");

    assert!(repos.users.get_global(user_id).await.unwrap().is_none());
    assert!(repos
        .oauth_tokens
        .list_token_providers(user_id)
        .await
        .unwrap()
        .is_empty());
    assert!(repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap()
        .is_empty());
    assert!(repos
        .groups
        .get_member(&group_id.to_string(), user_id)
        .await
        .unwrap()
        .is_none());
    assert!(repos
        .user_onboarding
        .get_onboarding_steps(&user_id.to_string())
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        repos.short_links.resolve_short_link(&code).await.unwrap(),
        None
    );
}

/// An agent the `PostgreSQL` schema quarantined into `agents_orphaned` keeps its
/// author's id with no foreign key to take it along, so the delete clears it
/// itself and reports it, and the row is gone afterwards: its title and prompt
/// are the author's words. The row is looked for again by its own id, not by
/// the author predicate the purge and its survivor check share. `SQLite` never
/// ran that quarantine, so its schema has no such table to outlive a user in.
#[tokio::test]
async fn delete_clears_the_users_quarantined_agents() {
    const QUARANTINE_TABLE: &str = "agents_orphaned";

    /// Delete `user_id` through the handler and return the response body.
    async fn delete_through_handler(context: Arc<AdminApiContext>, user_id: Uuid) -> Value {
        let response = handle_delete_user(
            State(context),
            Extension(manage_users_token()),
            Path(user_id.to_string()),
            delete_request(),
        )
        .await
        .expect("delete handler");
        assert_eq!(response.status(), StatusCode::OK);
        body_json(response).await
    }

    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, ..) = seed_user(repos, "quarantined-author").await;

    match resources.agent.database.as_ref() {
        Database::SQLite(db) => {
            let tables: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = $1",
            )
            .bind(QUARANTINE_TABLE)
            .fetch_one(db.pool())
            .await
            .unwrap();
            assert_eq!(tables, 0, "SQLite carries no agent quarantine");

            let body = delete_through_handler(context, user_id).await;
            let rows_removed = body["data"]["rows_removed"]
                .as_object()
                .expect("the delete reports what it cleared");
            assert!(
                !rows_removed.contains_key(QUARANTINE_TABLE),
                "nothing to clear there: {body}"
            );
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => {
            // What the quarantine moved: an agent whose tenant_id named no
            // tenant, which the column's new foreign key could not hold.
            const QUARANTINED_TENANT: &str = "tenant-that-never-was";
            let agent_id = format!("agent-{}", Uuid::new_v4());
            sqlx::query(
                "INSERT INTO agents_orphaned
                     (id, user_id, tenant_id, title, system_prompt, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $6)",
            )
            .bind(&agent_id)
            .bind(user_id)
            .bind(QUARANTINED_TENANT)
            .bind("Hill repeats")
            .bind("Coach the athlete through hill repeats.")
            .bind(Utc::now())
            .execute(db.pool())
            .await
            .unwrap();

            let body = delete_through_handler(context, user_id).await;
            assert_eq!(
                body["data"]["rows_removed"][QUARANTINE_TABLE], 1,
                "the quarantined agent is cleared: {body}"
            );
            let left: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM agents_orphaned WHERE id = $1")
                    .bind(&agent_id)
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
            assert_eq!(left, 0, "the quarantined agent outlived its author");
        }
    }
    assert!(repos.users.get_global(user_id).await.unwrap().is_none());
}

/// Every table carrying a `user_id` whose foreign key does not cascade (or
/// set NULL) the account delete to it is one the delete clears itself, on
/// whichever engine the test runs, so no user-keyed row outlives the account.
/// The engine's own purge is the one read: `PostgreSQL`'s names the tables only
/// its schema carries, which `SQLite` has none of.
#[tokio::test]
async fn every_uncascaded_user_table_is_cleared_by_the_delete() {
    const SQLITE_UNCASCADED: &str = r#"
        SELECT m.name AS tbl
        FROM sqlite_master m, pragma_table_info(m.name) c
        WHERE m.type = 'table' AND c.name = 'user_id'
          AND NOT EXISTS (
              SELECT 1 FROM pragma_foreign_key_list(m.name) f
              WHERE f."from" = 'user_id' AND f."table" = 'users'
                AND f.on_delete IN ('CASCADE', 'SET NULL'))
        ORDER BY m.name"#;
    #[cfg(feature = "postgresql")]
    const POSTGRES_UNCASCADED: &str = r"
        SELECT CAST(c.relname AS TEXT) AS tbl
        FROM pg_catalog.pg_attribute a
        JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
        WHERE a.attname = 'user_id' AND NOT a.attisdropped AND c.relkind = 'r'
          AND pg_catalog.pg_table_is_visible(c.oid)
          AND NOT EXISTS (
              SELECT 1 FROM pg_catalog.pg_constraint k
              WHERE k.conrelid = c.oid AND k.contype = 'f'
                AND k.confrelid = CAST('users' AS regclass)
                AND a.attnum = ANY (k.conkey)
                AND k.confdeltype IN ('c', 'n'))
        ORDER BY 1";

    const SQLITE_TABLES: &str = "SELECT name FROM sqlite_master WHERE type = 'table'";

    let resources = resources().await;
    let (uncascaded, purge): (Vec<String>, UserPurge) = match resources.agent.database.as_ref() {
        Database::SQLite(db) => {
            let tables: BTreeSet<String> = sqlx::query_scalar(SQLITE_TABLES)
                .fetch_all(db.pool())
                .await
                .unwrap()
                .into_iter()
                .collect();
            let present: Vec<&&str> = POSTGRES_ONLY_USER_OWNED_TABLES
                .iter()
                .filter(|table| tables.contains(**table))
                .collect();
            assert!(
                present.is_empty(),
                "a table declared PostgreSQL-only exists on SQLite too: {present:?}"
            );
            let uncascaded = sqlx::query_scalar(SQLITE_UNCASCADED)
                .fetch_all(db.pool())
                .await
                .unwrap();
            (uncascaded, SQLITE_USER_PURGE)
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => {
            let uncascaded = sqlx::query_scalar(POSTGRES_UNCASCADED)
                .fetch_all(db.pool())
                .await
                .unwrap();
            (uncascaded, POSTGRES_USER_PURGE)
        }
    };
    assert!(
        uncascaded.len() > 10,
        "the catalog read must see the schema's user tables: {uncascaded:?}"
    );
    let cleared: BTreeSet<&str> = purge.tables.iter().copied().collect();
    let missed: Vec<&String> = uncascaded
        .iter()
        .filter(|table| !cleared.contains(table.as_str()))
        .collect();
    assert!(
        missed.is_empty(),
        "these tables keep a user's rows after the account is deleted: {missed:?}"
    );
}

/// A reference written between the blocker read and the delete (here, while
/// the grant is being withdrawn) fails the delete as a 409 through the handler,
/// and the whole delete rolls back: the account, its membership and its rows
/// stand, and the response says the grant was already withdrawn.
#[tokio::test]
async fn a_reference_written_mid_removal_is_a_409_naming_what_was_done() {
    struct GroupWritingDisconnector {
        resources: Arc<ServerContext>,
    }

    #[async_trait]
    impl ProviderDisconnector for GroupWritingDisconnector {
        async fn disconnect(
            &self,
            user_id: Uuid,
            _: &str,
            tenant_id: TenantId,
        ) -> AppResult<RevocationOutcome> {
            seed_group(&self.resources.common.repos, user_id, tenant_id, "Race Day").await;
            Ok(RevocationOutcome::Revoked)
        }

        fn supports(&self, provider: &str) -> bool {
            provider == "strava"
        }
    }

    let resources = resources().await;
    let repos = &resources.common.repos;
    let context = admin_context_with(
        &resources,
        Arc::new(GroupWritingDisconnector {
            resources: resources.clone(),
        }),
    );
    let (owner_id, owner_tenant, _) = seed_user(repos, "host").await;
    let group_id = seed_group(repos, owner_id, owner_tenant, "Easy Sundays").await;
    let (user_id, tenant_id, _) = seed_user(repos, "raced").await;
    connect_strava(repos, user_id, tenant_id, None).await;
    join_group(repos, group_id, user_id, tenant_id).await;

    let response = handle_delete_user(
        State(context),
        Extension(manage_users_token()),
        Path(user_id.to_string()),
        delete_request(),
    )
    .await
    .expect("a conflict is a response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        body["message"],
        format!(
            "The account delete failed: User is still referenced by a row that does not cascade; reassign it and retry. Already disconnected: strava (tenant {tenant_id}) revoked at the provider. The account, its memberships and its other rows were left in place."
        )
    );
    let disconnected = body["data"]["disconnected"].as_array().unwrap();
    assert_eq!(disconnected.len(), 1, "{body}");
    assert_eq!(disconnected[0]["revocation"]["status"], "revoked");
    assert_eq!(body["data"]["failed"], Value::Null);
    assert_eq!(body["data"]["account_removed"], false);

    assert!(repos.users.get_global(user_id).await.unwrap().is_some());
    assert!(
        repos
            .groups
            .get_member(&group_id.to_string(), user_id)
            .await
            .unwrap()
            .is_some(),
        "the membership delete rolled back with the account delete"
    );
    assert_eq!(
        repos
            .oauth_tokens
            .list_token_providers(user_id)
            .await
            .unwrap(),
        vec![(tenant_id.to_string(), "strava".to_owned())],
        "the token row this stand-in left behind was not deleted either"
    );
}

/// A delete whose second disconnect fails reports the first as done and the
/// second as failed, and leaves the account in place.
#[tokio::test]
async fn a_disconnect_failing_after_another_succeeded_names_both() {
    struct SecondCallFails {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ProviderDisconnector for SecondCallFails {
        async fn disconnect(&self, _: Uuid, _: &str, _: TenantId) -> AppResult<RevocationOutcome> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(RevocationOutcome::Revoked)
            } else {
                Err(AppError::internal("the second revocation broke"))
            }
        }

        fn supports(&self, _: &str) -> bool {
            true
        }
    }

    let resources = resources().await;
    let repos = &resources.common.repos;
    let context = admin_context_with(
        &resources,
        Arc::new(SecondCallFails {
            calls: AtomicUsize::new(0),
        }),
    );
    let (user_id, first_tenant, _) = seed_user(repos, "two-tenants").await;
    let second_tenant = extra_tenant(repos, user_id).await;
    connect_strava(repos, user_id, first_tenant, None).await;
    connect_strava(repos, user_id, second_tenant, None).await;
    let mut tenants = [first_tenant.to_string(), second_tenant.to_string()];
    tenants.sort();

    let response = handle_delete_user(
        State(context),
        Extension(manage_users_token()),
        Path(user_id.to_string()),
        delete_request(),
    )
    .await
    .expect("an interrupted removal is a response");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(response).await;
    let cause = AppError::internal("the second revocation broke").sanitized_message();
    assert_eq!(
        body["message"],
        format!(
            "Disconnecting strava (tenant {}) failed: {cause}. Its grant may already have been revoked at the provider. Already disconnected: strava (tenant {}) revoked at the provider. The account was left in place.",
            tenants[1], tenants[0]
        )
    );
    assert_eq!(
        body["data"]["disconnected"][0]["tenant_id"],
        tenants[0].as_str()
    );
    assert_eq!(body["data"]["failed"]["tenant_id"], tenants[1].as_str());
    assert!(repos.users.get_global(user_id).await.unwrap().is_some());
}

/// A token scoped to one tenant cannot disconnect or delete a user held in
/// another; scoped to the user's own tenant, it can.
#[tokio::test]
async fn a_tenant_scoped_token_acts_only_inside_its_tenant() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (_, foreign_tenant, _) = seed_user(repos, "other-club").await;
    let (user_id, tenant_id, _) = seed_user(repos, "scoped").await;
    connect_strava(repos, user_id, tenant_id, None).await;
    let refusal = format!(
        "Token is scoped to tenant {foreign_tenant}; the user belongs to or holds a provider in a tenant outside it"
    );

    let error = handle_disconnect_user_provider(
        State(context.clone()),
        Extension(scoped_manage_users_token(foreign_tenant)),
        Path((user_id.to_string(), "strava".to_owned())),
    )
    .await
    .expect_err("another tenant's token is refused");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert_eq!(error.message, refusal);

    let error = handle_delete_user(
        State(context.clone()),
        Extension(scoped_manage_users_token(foreign_tenant)),
        Path(user_id.to_string()),
        delete_request(),
    )
    .await
    .expect_err("another tenant's token cannot delete the account");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert_eq!(error.message, refusal);
    assert!(stub.received().is_empty(), "nothing may reach Strava");
    assert!(repos.users.get_global(user_id).await.unwrap().is_some());
    assert_eq!(
        repos
            .oauth_tokens
            .list_token_providers(user_id)
            .await
            .unwrap()
            .len(),
        1
    );

    let response = handle_disconnect_user_provider(
        State(context),
        Extension(scoped_manage_users_token(tenant_id)),
        Path((user_id.to_string(), "strava".to_owned())),
    )
    .await
    .expect("the user's own tenant's token may disconnect");
    assert_eq!(response.status(), StatusCode::OK);
    assert_one_strava_revocation(&stub.received(), ENV_CLIENT_ID, ENV_CLIENT_SECRET);
}

/// Delete one user through the handler with a chokepoint that confirms every
/// revocation, and return the response.
async fn delete_user(resources: &Arc<ServerContext>, user_id: Uuid) -> Response {
    let stub = RevokeStub::start().await;
    handle_delete_user(
        State(admin_context(resources, &stub.url)),
        Extension(manage_users_token()),
        Path(user_id.to_string()),
        delete_request(),
    )
    .await
    .expect("a delete is a response")
}

/// The blockers a refused delete names, as `(kind, detail)`.
fn blockers_of(body: &Value) -> Vec<(String, String)> {
    body["data"]["blockers"]
        .as_array()
        .unwrap_or_else(|| panic!("a refusal names its blockers: {body}"))
        .iter()
        .map(|b| {
            (
                b["kind"].as_str().unwrap().to_owned(),
                b["detail"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// An agent `author` wrote in `tenant_id`, visible as `visibility` says.
async fn seed_agent(
    repos: &RepositoryRegistry,
    author: Uuid,
    tenant_id: TenantId,
    title: &str,
    visibility: AgentVisibility,
) -> String {
    repos
        .agents
        .create_system_agent(
            author,
            tenant_id,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: None,
                system_prompt: "You coach.".to_owned(),
                category: AgentCategory::Recovery,
                tags: vec![],
                visibility,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

/// A messaging session with an inbound message, its delivery receipt and a
/// queued retry that names no user: rows only the session reaches.
#[tokio::test]
async fn delete_clears_a_users_messaging_history() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (user_id, tenant_id, _) = seed_user(repos, "texter").await;
    let user = user_id.to_string();
    let session_id = Uuid::new_v4().to_string();
    let message_id = Uuid::new_v4().to_string();
    repos
        .messaging
        .create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &user,
            tenant_id,
            channel_type: "telegram",
            channel_user_id: "tg-4242",
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .unwrap();
    assert!(repos
        .messaging
        .insert_message(&InsertMessageParams {
            id: &message_id,
            tenant_id,
            session_id: &session_id,
            direction: "outbound",
            channel_type: "telegram",
            channel_message_id: "tg-msg-1",
            sender_id: "bot",
            content_type: "text",
            content_body: Some("Easy run today."),
            correlation_id: "corr-1",
            raw_payload: None,
            chat_message_id: None,
        })
        .await
        .unwrap());
    repos
        .messaging
        .insert_delivery_receipt(
            &Uuid::new_v4().to_string(),
            tenant_id,
            &message_id,
            None,
            "sent",
        )
        .await
        .unwrap();
    repos
        .messaging
        .enqueue_outbound(
            &Uuid::new_v4().to_string(),
            &message_id,
            tenant_id,
            None,
            "telegram",
            "{}",
        )
        .await
        .unwrap();

    let response = delete_user(&resources, user_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let rows_removed = body["data"]["rows_removed"].as_object().unwrap();
    for table in [
        "messaging_messages",
        "messaging_delivery_receipts",
        "messaging_outbound_queue",
    ] {
        assert_eq!(rows_removed[table], 1, "{table} is cleared: {body}");
    }
    assert!(repos.users.get_global(user_id).await.unwrap().is_none());
    assert!(repos
        .messaging
        .get_session_messages(&session_id, tenant_id, 10, 0)
        .await
        .unwrap()
        .is_empty());
}

/// An agent its author shares with the tenant, or one another user talks to,
/// is somebody else's too: the delete refuses and names it. A private agent
/// only its author used goes with them, their own conversation with it too.
#[tokio::test]
async fn an_agent_other_users_rely_on_blocks_the_delete_and_a_private_one_goes() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (sharer, sharer_tenant, _) = seed_user(repos, "catalogue-admin").await;
    seed_agent(
        repos,
        sharer,
        sharer_tenant,
        "Club Marathon Coach",
        AgentVisibility::Tenant,
    )
    .await;

    let response = delete_user(&resources, sharer).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        blockers_of(&body),
        vec![(
            UserReferenceKind::AuthoredAgent.as_str().to_owned(),
            "Club Marathon Coach".to_owned()
        )]
    );
    assert!(repos.users.get_global(sharer).await.unwrap().is_some());

    let (author, author_tenant, _) = seed_user(repos, "private-author").await;
    let (other, other_tenant, _) = seed_user(repos, "borrower").await;
    let agent_id = seed_agent(
        repos,
        author,
        author_tenant,
        "Private Taper Coach",
        AgentVisibility::Private,
    )
    .await;
    repos
        .chat
        .create_conversation(
            &author.to_string(),
            author_tenant,
            "taper",
            "model",
            Some(&agent_id),
            None,
        )
        .await
        .unwrap();
    let borrowed = repos
        .chat
        .create_conversation(
            &other.to_string(),
            other_tenant,
            "borrowed",
            "model",
            Some(&agent_id),
            None,
        )
        .await
        .unwrap();

    let response = delete_user(&resources, author).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        blockers_of(&body),
        vec![(
            UserReferenceKind::AuthoredAgent.as_str().to_owned(),
            "Private Taper Coach".to_owned()
        )],
        "another user's conversation is bound to the agent"
    );

    repos
        .chat
        .delete_conversation(&borrowed.id, &other.to_string(), other_tenant)
        .await
        .unwrap();
    let response = delete_user(&resources, author).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a private agent only its author used goes with them"
    );
    let body = body_json(response).await;
    assert_eq!(
        body["data"]["rows_removed"]["chat_conversations"], 1,
        "{body}"
    );
    assert!(repos.users.get_global(author).await.unwrap().is_none());
}

/// The only owner of a tenant other users belong to cannot be deleted: every
/// tenant read joins its owner, so the tenant would vanish for its members.
#[tokio::test]
async fn the_only_owner_of_a_tenant_with_members_cannot_be_deleted() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (owner, club, _) = seed_user(repos, "club-founder").await;
    let (member, _, _) = seed_user(repos, "club-member").await;
    repos.users.update_tenant_id(member, club).await.unwrap();

    let response = delete_user(&resources, owner).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        blockers_of(&body),
        vec![(
            UserReferenceKind::OwnsTenant.as_str().to_owned(),
            "club-founder tenant".to_owned()
        )]
    );
    assert!(repos.users.get_global(owner).await.unwrap().is_some());
    assert!(
        repos
            .tenants
            .list_for_user(member)
            .await
            .unwrap()
            .iter()
            .any(|t| t.id == club),
        "the tenant stays visible to its member"
    );
}

/// A subscription the billing provider may still charge blocks the delete,
/// since removing the row cancels nothing there; a canceled one is the
/// user's own record and the delete clears it.
#[tokio::test]
async fn a_live_subscription_blocks_the_delete_and_a_canceled_one_is_cleared() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (user_id, tenant_id, _) = seed_user(repos, "subscriber").await;
    let now = Utc::now();
    let mut subscription = Subscription {
        id: Uuid::new_v4(),
        tenant_id,
        user_id,
        provider: "dummy".to_owned(),
        provider_customer_id: format!("cus_{}", Uuid::new_v4().simple()),
        provider_subscription_id: Some(format!("sub_{}", Uuid::new_v4().simple())),
        status: SubscriptionStatus::Active,
        plan_tier: UserTier::Professional,
        current_period_start: None,
        current_period_end: None,
        cancel_at_period_end: false,
        canceled_at: None,
        trial_end: None,
        metadata: None,
        created_at: now,
        updated_at: now,
    };
    repos
        .subscriptions
        .upsert_subscription(&subscription)
        .await
        .unwrap();

    let response = delete_user(&resources, user_id).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(
        blockers_of(&body),
        vec![(
            UserReferenceKind::BillingSubscription.as_str().to_owned(),
            format!("professional plan with dummy (active) in tenant {tenant_id}")
        )]
    );

    subscription.status = SubscriptionStatus::Canceled;
    subscription.canceled_at = Some(Utc::now());
    repos
        .subscriptions
        .upsert_subscription(&subscription)
        .await
        .unwrap();
    let response = delete_user(&resources, user_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["rows_removed"]["subscriptions"], 1, "{body}");
    assert!(repos
        .subscriptions
        .get_subscription_by_user(user_id)
        .await
        .unwrap()
        .is_none());
}

/// Mark a tenant inactive, which no repository path does for a test.
async fn deactivate_tenant(resources: &ServerContext, tenant_id: TenantId) {
    const DEACTIVATE: &str = "UPDATE tenants SET is_active = FALSE WHERE id = $1";
    let deactivated = match resources.agent.database.as_ref() {
        Database::SQLite(db) => sqlx::query(DEACTIVATE)
            .bind(tenant_id.to_string())
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => sqlx::query(DEACTIVATE)
            .bind(tenant_id.as_uuid())
            .execute(db.pool())
            .await
            .unwrap()
            .rows_affected(),
    };
    assert_eq!(deactivated, 1, "the tenant exists to deactivate");
}

/// A user who also belongs to an inactive tenant is held outside a token
/// scoped to their other tenant: the delete would clear their rows there too.
#[tokio::test]
async fn a_scoped_token_cannot_reach_a_user_who_also_belongs_to_an_inactive_tenant() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, home, _) = seed_user(repos, "two-homes").await;
    let (_, dormant, _) = seed_user(repos, "dormant-club").await;
    repos
        .users
        .update_tenant_id(user_id, dormant)
        .await
        .unwrap();
    deactivate_tenant(&resources, dormant).await;

    let error = handle_delete_user(
        State(context),
        Extension(scoped_manage_users_token(home)),
        Path(user_id.to_string()),
        delete_request(),
    )
    .await
    .expect_err("a membership outside the token's tenant is refused");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(repos.users.get_global(user_id).await.unwrap().is_some());
    assert_eq!(
        repos
            .tenants
            .list_membership_tenant_ids(user_id)
            .await
            .unwrap()
            .len(),
        2,
        "both memberships stand"
    );
}

/// A disconnect that fails as the first one may still have revoked its grant
/// upstream, so the delete answers with what it knows rather than a bare
/// error, and leaves the account.
#[tokio::test]
async fn a_first_disconnect_that_fails_is_reported_as_an_interrupted_removal() {
    struct AlwaysFails;

    #[async_trait]
    impl ProviderDisconnector for AlwaysFails {
        async fn disconnect(&self, _: Uuid, _: &str, _: TenantId) -> AppResult<RevocationOutcome> {
            Err(AppError::internal(
                "the token delete broke after the revocation",
            ))
        }

        fn supports(&self, _: &str) -> bool {
            true
        }
    }

    let resources = resources().await;
    let repos = &resources.common.repos;
    let context = admin_context_with(&resources, Arc::new(AlwaysFails));
    let (user_id, tenant_id, _) = seed_user(repos, "half-revoked").await;
    connect_strava(repos, user_id, tenant_id, None).await;
    let cause =
        AppError::internal("the token delete broke after the revocation").sanitized_message();
    let expected = format!(
        "Disconnecting strava (tenant {tenant_id}) failed: {cause}. Its grant may already have been revoked at the provider. No other provider was disconnected. The account was left in place."
    );

    let response = handle_delete_user(
        State(context.clone()),
        Extension(manage_users_token()),
        Path(user_id.to_string()),
        delete_request(),
    )
    .await
    .expect("a failed first disconnect is a response naming what may have happened");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(response).await;
    assert_eq!(body["message"], expected.as_str());
    assert_eq!(
        body["data"]["failed"]["tenant_id"],
        tenant_id.to_string().as_str()
    );
    assert_eq!(body["data"]["disconnected"].as_array().unwrap().len(), 0);
    assert_eq!(body["data"]["account_removed"], false);
    assert!(repos.users.get_global(user_id).await.unwrap().is_some());

    let response = handle_disconnect_user_provider(
        State(context),
        Extension(manage_users_token()),
        Path((user_id.to_string(), "strava".to_owned())),
    )
    .await
    .expect("the provider disconnect answers the same way");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body_json(response).await["message"], expected.as_str());
}

/// An A2A client's API key belongs to the client's system user, so the
/// cascade from the owner never reached it: the delete removes it with the
/// client, and the key stops authenticating.
#[tokio::test]
async fn delete_removes_the_api_key_of_an_a2a_client_the_user_registered() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let (user_id, _, _) = seed_user(repos, "a2a-owner").await;
    let credentials = resources
        .a2a
        .a2a_client_manager
        .register_client(
            ClientRegistrationRequest {
                name: "Removal test agent".to_owned(),
                description: "Registered by a user about to be removed".to_owned(),
                capabilities: vec!["fitness-data-analysis".to_owned()],
                redirect_uris: vec!["https://example.com/callback".to_owned()],
                contact_email: "owner@example.com".to_owned(),
            },
            user_id,
        )
        .await
        .unwrap();
    let system_user = repos
        .users
        .get_by_email(&format!("a2a-system-{}@pierre.ai", credentials.client_id))
        .await
        .unwrap()
        .expect("the client's system user exists");
    assert_eq!(
        repos
            .api_keys
            .get_for_user(system_user.id)
            .await
            .unwrap()
            .len(),
        1,
        "the client carries one key"
    );

    let response = delete_user(&resources, user_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["rows_removed"]["api_keys"], 1, "{body}");
    assert!(
        repos
            .api_keys
            .get_for_user(system_user.id)
            .await
            .unwrap()
            .is_empty(),
        "the deleted user's A2A key no longer exists to authenticate"
    );
}

/// A pool-app grant in a tenant that configured its own Strava app since is
/// still revoked under the pool app that issued it: the tenant's client
/// cannot withdraw another application's grant.
#[tokio::test]
async fn a_pool_grant_is_revoked_under_its_app_after_the_tenant_configured_its_own() {
    let resources = resources().await;
    let repos = &resources.common.repos;
    let mut stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);
    let (user_id, tenant_id, _) = seed_user(repos, "pool-then-club-app").await;
    repos
        .oauth_tokens
        .upsert_strava_pool_app(POOL_APP, POOL_SECRET, 10, Some("pool-2"))
        .await
        .unwrap();
    connect_strava(repos, user_id, tenant_id, Some(POOL_APP)).await;
    repos
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id,
            provider: "strava".to_owned(),
            client_id: "club-own-strava-app".to_owned(),
            client_secret: "club-own-strava-secret".to_owned(),
            redirect_uri: "http://localhost/api/oauth/callback/strava".to_owned(),
            scopes: vec!["read".to_owned()],
            rate_limit_per_day: 1000,
        })
        .await
        .unwrap();

    let response = handle_disconnect_user_provider(
        State(context),
        Extension(manage_users_token()),
        Path((user_id.to_string(), "strava".to_owned())),
    )
    .await
    .expect("disconnect handler");
    assert_eq!(response.status(), StatusCode::OK);
    assert_one_strava_revocation(&stub.received(), POOL_APP, POOL_SECRET);
}

/// The env app is the pool's implicit member; registering it again would make
/// one application read as two.
#[tokio::test]
async fn the_env_app_cannot_be_registered_as_a_pool_app() {
    let resources = resources().await;
    let stub = RevokeStub::start().await;
    let context = admin_context(&resources, &stub.url);

    let error = handle_upsert_strava_pool_app(
        State(context),
        Extension(super_admin_token()),
        Json(json!({
            "client_id": ENV_CLIENT_ID,
            "client_secret": POOL_SECRET,
            "seat_cap": 50,
        })),
    )
    .await
    .expect_err("the env app is refused");
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(resources
        .common
        .repos
        .oauth_tokens
        .list_strava_pool_apps(false)
        .await
        .unwrap()
        .iter()
        .all(|app| app.client_id != ENV_CLIENT_ID));
}

/// A foreign key as the catalog reports it: `(child, child column, parent,
/// on-delete action)`.
type ForeignKey = (String, String, String, String);

/// The aliases `table` is read under in `sql`, after a `FROM` or a `JOIN`.
fn aliases_in<'a>(sql: &'a str, table: &str) -> Vec<&'a str> {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .windows(3)
        .filter(|w| matches!(w[0], "FROM" | "JOIN") && w[1] == table)
        .map(|w| w[2])
        .collect()
}

/// Whether `sql` reads `alias.column` as a whole qualified name.
fn reads_column(sql: &str, alias: &str, column: &str) -> bool {
    let wanted = format!("{alias}.{column}");
    sql.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .any(|token| token == wanted)
}

/// Every non-cascading foreign key into a row the delete removes that
/// nothing handles, as `child.column -> parent`.
///
/// The delete removes a table's rows at a step: a table `purge` lists at its
/// position, `users` after every purge, and a table a `CASCADE` reaches at the
/// earliest step of a parent it cascades from (a listed conversation takes its
/// messages with it, at its own step). A reference is handled when its child's
/// rows go at a strictly earlier step than its parent's, or when `blockers`
/// reads the referencing column (`alias.column` for an alias it reads the
/// child under), so the refusal names the row before anything is revoked.
/// References into `users` are the blockers' and the user-table gate's.
fn unhandled_references(keys: &[ForeignKey], purge: &[&str], blockers: &str) -> Vec<String> {
    let mut steps: BTreeMap<&str, usize> = purge
        .iter()
        .enumerate()
        .rev()
        .map(|(at, table)| (*table, at))
        .collect();
    steps.insert("users", purge.len());
    loop {
        let mut changed = false;
        for (child, _, parent, on_delete) in keys {
            if on_delete != "CASCADE" {
                continue;
            }
            let Some(&parent_at) = steps.get(parent.as_str()) else {
                continue;
            };
            let child_at = steps.entry(child.as_str()).or_insert(usize::MAX);
            if parent_at < *child_at {
                *child_at = parent_at;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    keys.iter()
        .filter(|(child, _, parent, on_delete)| {
            !matches!(on_delete.as_str(), "CASCADE" | "SET NULL" | "SET DEFAULT")
                && parent != "users"
                && child != parent
        })
        .filter_map(|(child, column, parent, _)| {
            let parent_at = *steps.get(parent.as_str())?;
            let cleared_first = steps
                .get(child.as_str())
                .is_some_and(|child_at| *child_at < parent_at);
            let refused = aliases_in(blockers, child)
                .into_iter()
                .any(|alias| reads_column(blockers, alias, column));
            (!cleared_first && !refused).then(|| format!("{child}.{column} -> {parent}"))
        })
        .collect()
}

/// The reference gate below sees what a cascade from a cleared table removes
/// at that table's step, and holds a blocker to the column it reads, not to
/// the table's name appearing somewhere in the statement.
#[test]
fn the_reference_gate_follows_cascades_from_cleared_tables_and_reads_columns() {
    let key = |child: &str, column: &str, parent: &str, on_delete: &str| {
        (
            child.to_owned(),
            column.to_owned(),
            parent.to_owned(),
            on_delete.to_owned(),
        )
    };

    // A cleared conversation takes its messages with it at its own step, so a
    // table referencing a message with no ON DELETE and cleared after the
    // conversation still holds the reference when the message goes.
    let behind_a_cascade = [
        key(
            "chat_messages",
            "conversation_id",
            "chat_conversations",
            "CASCADE",
        ),
        key("message_marks", "message_id", "chat_messages", "NO ACTION"),
    ];
    assert_eq!(
        unhandled_references(
            &behind_a_cascade,
            &["chat_conversations", "message_marks"],
            DELETION_BLOCKERS_SQL
        ),
        vec!["message_marks.message_id -> chat_messages".to_owned()]
    );
    assert!(unhandled_references(
        &behind_a_cascade,
        &["message_marks", "chat_conversations"],
        DELETION_BLOCKERS_SQL
    )
    .is_empty());

    // The blockers read chat_conversations for its agent, not for any other
    // column: a second reference from it is not refused by that read.
    let beside_a_blocker = [
        key("agents", "user_id", "users", "CASCADE"),
        key("chat_conversations", "agent_id", "agents", "NO ACTION"),
        key(
            "chat_conversations",
            "pinned_agent_id",
            "agents",
            "NO ACTION",
        ),
    ];
    assert_eq!(
        unhandled_references(&beside_a_blocker, &[], DELETION_BLOCKERS_SQL),
        vec!["chat_conversations.pinned_agent_id -> agents".to_owned()]
    );
}

/// Every foreign key that does not cascade into a table the delete removes
/// rows from (by cascade from `users`, by cascade from a table it clears, or
/// by clearing it) comes from a table the delete clears first, or has its
/// column read by the blocker statement, so no such reference can fail the
/// delete unnamed after the grants were revoked. Each engine's own purge is
/// the one read.
#[tokio::test]
async fn every_reference_into_what_the_delete_removes_is_cleared_first_or_refused() {
    const SQLITE_FOREIGN_KEYS: &str = r#"
        SELECT m.name AS child, f."from" AS child_column, f."table" AS parent,
               UPPER(f.on_delete) AS on_delete
        FROM sqlite_master m, pragma_foreign_key_list(m.name) f
        WHERE m.type = 'table'"#;
    #[cfg(feature = "postgresql")]
    const POSTGRES_FOREIGN_KEYS: &str = r"
        SELECT CAST(c.relname AS TEXT) AS child, CAST(a.attname AS TEXT) AS child_column,
               CAST(p.relname AS TEXT) AS parent,
               CASE k.confdeltype WHEN 'c' THEN 'CASCADE' WHEN 'n' THEN 'SET NULL'
                    WHEN 'd' THEN 'SET DEFAULT' ELSE 'NO ACTION' END AS on_delete
        FROM pg_catalog.pg_constraint k
        JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
        JOIN pg_catalog.pg_class p ON p.oid = k.confrelid
        CROSS JOIN LATERAL pg_catalog.unnest(k.conkey) AS u(attnum)
        JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = u.attnum
        WHERE k.contype = 'f' AND pg_catalog.pg_table_is_visible(c.oid)";

    let resources = resources().await;
    let (keys, purge): (Vec<ForeignKey>, UserPurge) = match resources.agent.database.as_ref() {
        Database::SQLite(db) => (
            sqlx::query_as(SQLITE_FOREIGN_KEYS)
                .fetch_all(db.pool())
                .await
                .unwrap(),
            SQLITE_USER_PURGE,
        ),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => (
            sqlx::query_as(POSTGRES_FOREIGN_KEYS)
                .fetch_all(db.pool())
                .await
                .unwrap(),
            POSTGRES_USER_PURGE,
        ),
    };
    assert!(
        keys.len() > 50,
        "the catalog read must see the schema's foreign keys: {}",
        keys.len()
    );
    assert!(
        keys.iter().any(
            |(child, column, parent, on_delete)| child == "coaching_groups"
                && column == "agent_id"
                && parent == "agents"
                && on_delete == "NO ACTION"
        ),
        "the catalog read must name each reference's column: {keys:?}"
    );

    let unhandled = unhandled_references(&keys, purge.tables, DELETION_BLOCKERS_SQL);
    assert!(
        unhandled.is_empty(),
        "these references fail the account delete with nothing to name them: {unhandled:?}"
    );
}

/// An account delete that failed with nothing disconnected before it names
/// no failed provider: there was none, and "no other provider" would imply
/// one.
#[test]
fn a_failed_account_delete_with_nothing_disconnected_names_no_provider() {
    let interruption = Interruption {
        disconnected: Vec::new(),
        failed: None,
        error: AppError::internal("the account delete broke"),
    };

    assert_eq!(
        interruption.describe("the account delete broke"),
        "The account delete failed: the account delete broke. No provider was disconnected. The account, its memberships and its other rows were left in place."
    );
}

/// A provider this build cannot disconnect is refused before the chokepoint
/// is called: nothing reached the provider and nothing was removed, so the
/// answer says so instead of warning that a grant may have been revoked.
#[tokio::test]
async fn disconnecting_a_provider_this_build_cannot_revoke_changes_nothing() {
    struct CountsCalls {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ProviderDisconnector for CountsCalls {
        async fn disconnect(
            &self,
            _: Uuid,
            provider: &str,
            _: TenantId,
        ) -> AppResult<RevocationOutcome> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(AppError::invalid_input(format!(
                "Unsupported provider: {provider}"
            )))
        }

        fn supports(&self, provider: &str) -> bool {
            provider == "strava"
        }
    }

    let resources = resources().await;
    let repos = &resources.common.repos;
    let disconnector = Arc::new(CountsCalls {
        calls: AtomicUsize::new(0),
    });
    let context = admin_context_with(&resources, disconnector.clone());
    let (user_id, tenant_id, _) = seed_user(repos, "unrevocable").await;
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            "synthetic",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let response = handle_disconnect_user_provider(
        State(context),
        Extension(manage_users_token()),
        Path((user_id.to_string(), "synthetic".to_owned())),
    )
    .await
    .expect("a provider this build cannot disconnect is a response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert_eq!(
        body["message"],
        "This server cannot disconnect synthetic; nothing was revoked or removed. Deleting the user clears its rows."
    );
    assert_eq!(
        body["data"]["not_revocable"][0]["tenant_id"],
        tenant_id.to_string().as_str()
    );
    assert_eq!(
        disconnector.calls.load(Ordering::SeqCst),
        0,
        "the chokepoint was never called"
    );
    assert_eq!(
        repos
            .provider_connections
            .get_for_user(user_id, None)
            .await
            .unwrap()
            .len(),
        1,
        "the connection row stands"
    );
}
