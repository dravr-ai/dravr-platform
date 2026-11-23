// ABOUTME: An admin-only tool call from outside Dravr's own surfaces is one audit_events row, written before it runs
// ABOUTME: The row names the caller, the tool, the transport and the outcome; no row, no call; in-app calls write none
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! EFVP 2026-11-17, decision 9.4d. An admin's API key carries every scope,
//! so an outside client holding one runs the eight admin-only tools, five of
//! which write the agents and system prompts that answer users. A call over
//! `/mcp` or A2A wrote only a log line; it is now a row of the admin audit
//! trail, and a row that cannot be written refuses the call.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use dravr_tronc::dates::Timestamp;
use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::impersonation::{CallerCredential, CredentialKind};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::permissions::UserRole;
use pierre_core::transport::Transport;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{AuditEventFilter, AuditEventRecord};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::mcp::tool_handlers::ToolHandlers;
use pierre_tool_runtime::protocols::{
    ProtocolError, UniversalRequest, UniversalResponse, UniversalToolExecutor,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::support_session::CALLER_CREDENTIAL;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;

async fn resources() -> Arc<ServerContext> {
    common::init_server_config();
    common::init_test_http_clients();
    common::create_test_server_resources().await.unwrap()
}

/// An executor serving `transport`, under a grant holding every scope: an
/// admin's API key.
fn executor(resources: &Arc<ServerContext>, transport: Transport) -> UniversalToolExecutor {
    UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(transport)
}

/// An admin with a tenant of their own.
pub(crate) async fn admin(resources: &Arc<ServerContext>) -> (Uuid, String) {
    let email = format!("audit_admin_{}@example.com", Uuid::new_v4());
    let mut user = User::new(
        email.clone(),
        "not-a-hash".to_owned(),
        Some("Audit Admin".to_owned()),
    );
    user.role = UserRole::Admin;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Timestamp::now());
    let repos = &resources.common.repos;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("Audit tenant for {email}"),
            slug: format!("audit-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        })
        .await
        .unwrap();
    repos
        .users
        .update_tenant_id(user.id, tenant_id)
        .await
        .unwrap();
    (user.id, tenant_id.to_string())
}

fn request(tool: &str, params: Value, user: Uuid, tenant: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user.to_string(),
        protocol: "mcp".to_owned(),
        tenant_id: Some(tenant.to_owned()),
    }
}

fn new_agent(title: &str) -> Value {
    json!({
        "title": title,
        "system_prompt": "You are a helpful test coach.",
        "category": "training",
    })
}

async fn call(
    executor: &UniversalToolExecutor,
    tool: &str,
    params: Value,
    user: Uuid,
    tenant: &str,
) -> Result<UniversalResponse, ProtocolError> {
    executor
        .execute_tool(request(tool, params, user, tenant))
        .await
}

/// `actor`'s rows on the trail, newest first.
pub(crate) async fn trail(resources: &Arc<ServerContext>, actor: Uuid) -> Vec<AuditEventRecord> {
    resources
        .common
        .repos
        .admin_audit
        .list(
            &AuditEventFilter {
                actor_user_id: Some(actor),
                ..AuditEventFilter::default()
            },
            50,
        )
        .await
        .unwrap()
}

/// The titles of the system agents the admin's own app lists.
async fn agent_titles(resources: &Arc<ServerContext>, admin: Uuid, tenant: &str) -> Vec<String> {
    let listed = call(
        &executor(resources, Transport::WebApp),
        "admin_list_system_agents",
        json!({}),
        admin,
        tenant,
    )
    .await
    .unwrap();
    assert!(listed.success, "{:?}", listed.error);
    listed.result.unwrap()["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|agent| agent["title"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn an_admin_tool_write_over_mcp_is_one_row_naming_the_admin_tool_transport_and_outcome() {
    let resources = resources().await;
    let (admin, tenant) = admin(&resources).await;

    let created = call(
        &executor(&resources, Transport::McpHttp),
        "admin_create_system_agent",
        new_agent("Audited Coach"),
        admin,
        &tenant,
    )
    .await
    .unwrap();
    assert!(created.success, "{:?}", created.error);

    let rows = trail(&resources, admin).await;
    assert_eq!(rows.len(), 1, "bug: one call, one row: {rows:#?}");
    let row = &rows[0];
    assert_eq!(
        row.action, "tool.admin_create_system_agent",
        "bug: the row's action names the tool called"
    );
    assert_eq!(
        row.source, "tool_call",
        "bug: the row is sourced as a tool call"
    );
    assert_eq!(
        row.outcome, "success",
        "bug: the row is settled to how it ended"
    );
    assert_eq!(
        row.actor_user_id.as_deref(),
        Some(admin.to_string().as_str()),
        "bug: the row names the caller as its actor"
    );
    assert_eq!(
        row.tenant_id.as_deref(),
        Some(tenant.as_str()),
        "bug: the row names the tenant the call acted in"
    );
    assert_eq!(
        row.metadata["tool"], "admin_create_system_agent",
        "bug: the row's metadata names the tool"
    );
    assert_eq!(
        row.metadata["transport"], "mcp_http",
        "bug: the row's metadata names the transport the call came over"
    );
}

#[tokio::test]
async fn a_read_over_a2a_and_a_refused_call_are_each_recorded_with_their_outcome() {
    let resources = resources().await;
    let (admin, tenant) = admin(&resources).await;
    let a2a = executor(&resources, Transport::A2a);

    let listed = call(&a2a, "admin_list_system_agents", json!({}), admin, &tenant)
        .await
        .unwrap();
    assert!(listed.success, "{:?}", listed.error);
    let rows = trail(&resources, admin).await;
    assert_eq!(rows.len(), 1, "bug: a read is recorded too: {rows:#?}");
    assert_eq!(
        rows[0].outcome, "success",
        "bug: the row is settled to how the call ended"
    );
    assert_eq!(
        rows[0].metadata["transport"], "a2a",
        "bug: the row's metadata names the transport the call came over"
    );

    // A non-admin naming an admin tool is refused by the admin gate, and the
    // attempt is on the trail under their own id.
    let (regular, _, regular_tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        "audit_regular@example.com",
        "starter",
    )
    .await
    .unwrap();
    let refused = call(
        &executor(&resources, Transport::McpHttp),
        "admin_delete_system_agent",
        json!({ "agent_id": Uuid::new_v4().to_string() }),
        regular,
        &regular_tenant.to_string(),
    )
    .await;
    assert!(
        matches!(refused, Err(ProtocolError::PermissionDenied { .. })),
        "{refused:?}"
    );
    let attempts = trail(&resources, regular).await;
    assert_eq!(attempts.len(), 1, "{attempts:#?}");
    assert_eq!(
        attempts[0].action, "tool.admin_delete_system_agent",
        "bug: the row's action names the tool called"
    );
    assert_eq!(
        attempts[0].outcome, "refused",
        "bug: a refused call's row says refused"
    );
}

#[tokio::test]
async fn an_admin_tool_call_from_dravrs_own_app_writes_no_row() {
    let resources = resources().await;
    let (admin, tenant) = admin(&resources).await;

    let created = call(
        &executor(&resources, Transport::WebApp),
        "admin_create_system_agent",
        new_agent("In-app Coach"),
        admin,
        &tenant,
    )
    .await
    .unwrap();
    assert!(created.success, "{:?}", created.error);
    assert_eq!(
        agent_titles(&resources, admin, &tenant).await,
        vec!["In-app Coach".to_owned()],
        "bug: the in-app call ran"
    );
    assert!(
        trail(&resources, admin).await.is_empty(),
        "the admin's own session answers for an in-app call"
    );
}

#[tokio::test]
async fn an_admin_tool_write_whose_row_cannot_be_written_is_not_run() {
    let resources = resources().await;
    let (admin, tenant) = admin(&resources).await;
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query("DROP TABLE audit_events")
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query("DROP TABLE audit_events CASCADE")
                .execute(db.pool())
                .await
                .unwrap();
        }
    }

    let refused = call(
        &executor(&resources, Transport::McpHttp),
        "admin_create_system_agent",
        new_agent("Unrecorded Coach"),
        admin,
        &tenant,
    )
    .await;

    assert!(
        matches!(&refused, Err(ProtocolError::ExecutionFailed(message)) if message.contains("audit trail")),
        "bug: no row, no call: {refused:?}"
    );
    assert!(
        agent_titles(&resources, admin, &tenant).await.is_empty(),
        "bug: the refused write created nothing"
    );
}

#[tokio::test]
async fn an_admin_tool_call_that_declared_no_transport_is_recorded_as_undeclared() {
    let resources = resources().await;
    let (admin, tenant) = admin(&resources).await;
    let undeclared =
        UniversalToolExecutor::new(resources.clone()).with_scopes(OAuthScope::self_grant());

    let listed = call(
        &undeclared,
        "admin_list_system_agents",
        json!({}),
        admin,
        &tenant,
    )
    .await
    .unwrap();
    assert!(listed.success, "{:?}", listed.error);

    let rows = trail(&resources, admin).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: a call that declared no transport is read as outside, and recorded: {rows:#?}"
    );
    assert_eq!(
        rows[0].outcome, "success",
        "bug: the row is settled to how the call ended"
    );
    assert_eq!(
        rows[0].metadata["transport"], "undeclared",
        "bug: the row's metadata names the transport the call came over"
    );
}

/// A real `/mcp` dispatch of `tool` by `user` in `tenant`, under every scope.
async fn mcp_dispatch(
    resources: &Arc<ServerContext>,
    user: Uuid,
    tenant: TenantId,
    tool: &str,
    args: Value,
) -> ToolResponse {
    let state: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
    let tool_context = ToolContext {
        user_id: Some(user.to_string()),
        tenant_id: Some(tenant.to_string()),
        auth_method: Some("api_key".to_owned()),
        request_id: Some(json!(1)),
        is_admin: true,
        scopes: OAuthScope::self_grant()
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect(),
        ..Default::default()
    };
    ToolHandlers::dispatch_tool_call(resources, &state, &tool_context, user, tenant, tool, args)
        .await
}

#[tokio::test]
async fn an_admin_tool_call_mcp_refuses_before_the_executor_is_one_refused_row() {
    let resources = resources().await;
    let (admin, tenant) = admin(&resources).await;
    // The quota ladder fails closed on counters it cannot read: `/mcp`
    // refuses the call before it reaches the executor.
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query("DROP TABLE usage_counters")
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query("DROP TABLE usage_counters CASCADE")
                .execute(db.pool())
                .await
                .unwrap();
        }
    }

    // The `/mcp` dispatcher declares the call's credential around the
    // dispatch; this one is an API key.
    let key = CallerCredential {
        kind: CredentialKind::ApiKey,
        client_id: None,
        binding: None,
    };
    let refused = CALLER_CREDENTIAL
        .scope(
            Some(key),
            mcp_dispatch(
                &resources,
                admin,
                TenantId::parse_str(&tenant).unwrap(),
                "admin_create_system_agent",
                new_agent("Door-refused Coach"),
            ),
        )
        .await;
    assert!(refused.is_error, "{refused:?}");

    let rows = trail(&resources, admin).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: an admin-only call refused at the door is on the trail: {rows:#?}"
    );
    assert_eq!(
        rows[0].action, "tool.admin_create_system_agent",
        "bug: the row's action names the tool called"
    );
    assert_eq!(
        rows[0].outcome, "refused",
        "bug: a refused call's row says refused"
    );
    assert_eq!(
        rows[0].tenant_id.as_deref(),
        Some(tenant.as_str()),
        "bug: the row names the tenant the call acted in"
    );
    assert_eq!(
        rows[0].metadata["transport"], "mcp_http",
        "bug: the row's metadata names the transport the call came over"
    );
    assert_eq!(
        rows[0].metadata["protocol"], "mcp",
        "bug: the row's metadata names the protocol the call came in on"
    );
    assert_eq!(
        rows[0].metadata["credential_kind"], "api_key",
        "bug: a call refused before the executor names its credential's kind"
    );

    // Any other tool refused at the same door writes nothing.
    let other = mcp_dispatch(
        &resources,
        admin,
        TenantId::parse_str(&tenant).unwrap(),
        "get_connection_status",
        json!({}),
    )
    .await;
    assert!(other.is_error, "{other:?}");
    assert_eq!(
        trail(&resources, admin).await.len(),
        1,
        "bug: a call naming no admin-only tool writes no row"
    );
}

#[tokio::test]
async fn an_admin_tool_call_a2a_refuses_before_the_executor_is_one_refused_row() {
    let resources = resources().await;
    // An admin who belongs to no tenant, holding a grant that names one: A2A
    // refuses the call before it reaches the executor.
    let user = untenanted_admin(&resources).await;
    let grant = format!(
        "Bearer {}",
        common::delegated_test_token_for_tenant(
            &resources,
            user.id,
            Some(TenantId::generate().to_string())
        )
    );
    let app = ProviderToolRouter::build_http_app(&resources);

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": format!("m-{}", Uuid::new_v4()),
                "role": "ROLE_USER",
                "parts": [{ "data": {
                    "tool_name": "admin_list_system_agents",
                    "parameters": {},
                } }]
            }
        },
    });
    let request = Request::post("/a2a/jsonrpc")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("A2A-Version", "1.0")
        .header("authorization", grant)
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let answer: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("{status}: {}", String::from_utf8_lossy(&bytes)));
    assert_eq!(
        answer["result"]["task"]["status"]["state"], "TASK_STATE_FAILED",
        "{answer}"
    );

    let rows = trail(&resources, user.id).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: an admin-only call refused at the A2A door is on the trail: {rows:#?}"
    );
    assert_eq!(
        rows[0].action, "tool.admin_list_system_agents",
        "bug: the row's action names the tool called"
    );
    assert_eq!(
        rows[0].outcome, "refused",
        "bug: a refused call's row says refused"
    );
    assert_eq!(
        rows[0].metadata["transport"], "a2a",
        "bug: the row's metadata names the transport the call came over"
    );
}

/// An admin who belongs to no tenant.
async fn untenanted_admin(resources: &Arc<ServerContext>) -> User {
    let mut user = User::new(
        format!("audit_untenanted_{}@example.com", Uuid::new_v4()),
        "not-a-hash".to_owned(),
        Some("Untenanted Admin".to_owned()),
    );
    user.role = UserRole::Admin;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Timestamp::now());
    resources.common.repos.users.create(&user).await.unwrap();
    user
}

/// `tools/call` of `tool` posted to `/mcp` with `credential`; the HTTP status.
async fn post_mcp_call(resources: &Arc<ServerContext>, credential: &str, tool: &str) -> u16 {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {credential}"))
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": {} },
            })
            .to_string(),
        ))
        .unwrap();
    ProviderToolRouter::build_http_app(resources)
        .oneshot(request)
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn an_admin_tool_call_the_mcp_auth_hook_refuses_is_one_refused_row() {
    let resources = resources().await;

    // No tenant: the hook answers 403 before anything is dispatched.
    let untenanted = untenanted_admin(&resources).await;
    let grant = common::delegated_test_token_for_tenant(
        &resources,
        untenanted.id,
        Some(TenantId::generate().to_string()),
    );
    let status = post_mcp_call(&resources, &grant, "admin_list_system_agents").await;
    assert_eq!(status, 403, "bug: the /mcp auth hook refuses the call");
    let rows = trail(&resources, untenanted.id).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: a call the /mcp auth hook refuses for want of a tenant is on the trail: {rows:#?}"
    );
    assert_eq!(
        rows[0].action, "tool.admin_list_system_agents",
        "bug: the row's action names the tool called"
    );
    assert_eq!(
        rows[0].outcome, "refused",
        "bug: a refused call's row says refused"
    );
    assert_eq!(
        rows[0].tenant_id, None,
        "bug: a call refused for want of a tenant names none"
    );
    assert_eq!(
        rows[0].metadata["transport"], "mcp_http",
        "bug: the row's metadata names the transport the call came over"
    );
    assert_eq!(
        rows[0].metadata["credential_kind"], "oauth_grant",
        "bug: a call the hook refused after admitting its credential names its kind"
    );
    assert_eq!(
        rows[0].metadata["client_id"], "mcp_client_test",
        "bug: a call the hook refused after admitting its credential names its client"
    );

    // A delegated grant never carries `admin`: the hook answers 403
    // insufficient_scope, and the attempt is on the trail in its tenant.
    let (admin, tenant) = admin(&resources).await;
    let grant = common::delegated_test_token_for_tenant(&resources, admin, Some(tenant.clone()));
    let status = post_mcp_call(&resources, &grant, "admin_list_system_agents").await;
    assert_eq!(status, 403, "bug: the /mcp auth hook refuses the call");
    let rows = trail(&resources, admin).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: a call the /mcp auth hook refuses for want of scope is on the trail: {rows:#?}"
    );
    assert_eq!(
        rows[0].outcome, "refused",
        "bug: a refused call's row says refused"
    );
    assert_eq!(
        rows[0].tenant_id.as_deref(),
        Some(tenant.as_str()),
        "bug: the row names the tenant the call acted in"
    );
    assert_eq!(
        rows[0].metadata["protocol"], "mcp",
        "bug: the row's metadata names the protocol the call came in on"
    );
    assert_eq!(
        rows[0].metadata["credential_kind"], "oauth_grant",
        "bug: a call refused for want of scope names its credential's kind"
    );
    assert_eq!(
        rows[0].metadata["client_id"], "mcp_client_test",
        "bug: a call refused for want of scope names its credential's client"
    );

    // A request that names no admin-only tool writes nothing.
    post_mcp_call(&resources, &grant, "no_such_tool").await;
    assert_eq!(
        trail(&resources, admin).await.len(),
        1,
        "bug: a call naming no admin-only tool writes no row"
    );
}

/// `SendMessage` of `message` posted to A2A with `grant`; the answer.
async fn post_a2a_message(resources: &Arc<ServerContext>, grant: &str, message: Value) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "SendMessage",
        "params": { "message": message },
    });
    let request = Request::post("/a2a/jsonrpc")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("A2A-Version", "1.0")
        .header("authorization", format!("Bearer {grant}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = ProviderToolRouter::build_http_app(resources)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("{status}: {}", String::from_utf8_lossy(&bytes)))
}

/// A message naming the admin-only `admin_list_system_agents`, continuing
/// `task_id` when one is named.
fn admin_tool_message(task_id: Option<&str>) -> Value {
    let mut message = json!({
        "messageId": format!("m-{}", Uuid::new_v4()),
        "role": "ROLE_USER",
        "parts": [{ "data": {
            "tool_name": "admin_list_system_agents",
            "parameters": {},
        } }]
    });
    if let Some(task_id) = task_id {
        message["taskId"] = json!(task_id);
    }
    message
}

/// Turn `user` away at the account-status gate. Another admin is made first:
/// the last active one cannot be suspended.
async fn suspend(resources: &Arc<ServerContext>, user: Uuid) {
    admin(resources).await;
    resources
        .common
        .repos
        .users
        .update_status(user, UserStatus::Suspended, None)
        .await
        .unwrap();
}

/// A credential the /mcp authenticator refuses once it has named its caller
/// — here an admin's API key whose owner the account-status gate turns away
/// — leaves the admin-only attempt on the trail, as the hook's tenant and
/// scope refusals do.
#[tokio::test]
async fn an_admin_tool_call_a_named_caller_is_refused_at_mcp_auth_is_one_refused_row() {
    let resources = resources().await;
    let (admin, _) = admin(&resources).await;
    let key = common::issue_test_api_key(&resources.agent.database, admin, "admin script")
        .await
        .unwrap();
    suspend(&resources, admin).await;

    let status = post_mcp_call(&resources, &key, "admin_list_system_agents").await;
    assert!(
        (400..500).contains(&status),
        "the suspended admin's key is refused: {status}"
    );
    let rows = trail(&resources, admin).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: an admin-only call the /mcp authenticator refuses for a named caller is on the trail: {rows:#?}"
    );
    assert_eq!(
        rows[0].action, "tool.admin_list_system_agents",
        "bug: the row's action names the tool called"
    );
    assert_eq!(
        rows[0].outcome, "refused",
        "bug: a refused call's row says refused"
    );
    assert_eq!(
        rows[0].metadata["transport"], "mcp_http",
        "bug: the row's metadata names the transport the call came over"
    );

    // A request that names no admin-only tool writes nothing.
    post_mcp_call(&resources, &key, "no_such_tool").await;
    assert_eq!(
        trail(&resources, admin).await.len(),
        1,
        "bug: a call naming no admin-only tool writes no row"
    );
}

/// The same over A2A: a grant whose owner the account-status gate turns
/// away, and a message continuing a task the caller does not hold, are
/// refused before the executor and leave the admin-only attempt on the
/// trail.
#[tokio::test]
async fn an_admin_tool_call_a2a_refuses_at_auth_or_send_is_one_refused_row() {
    let resources = resources().await;

    let (suspended, tenant) = admin(&resources).await;
    let grant = common::delegated_test_token_for_tenant(&resources, suspended, Some(tenant));
    suspend(&resources, suspended).await;
    let answer = post_a2a_message(&resources, &grant, admin_tool_message(None)).await;
    assert!(answer["error"].is_object(), "the call is refused: {answer}");
    let rows = trail(&resources, suspended).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: an admin-only call A2A authentication refuses for a named caller is on the trail: {rows:#?}"
    );
    assert_eq!(
        rows[0].action, "tool.admin_list_system_agents",
        "bug: the row's action names the tool called"
    );
    assert_eq!(
        rows[0].outcome, "refused",
        "bug: a refused call's row says refused"
    );
    assert_eq!(
        rows[0].metadata["transport"], "a2a",
        "bug: the row's metadata names the transport the call came over"
    );

    let (admin, tenant) = admin(&resources).await;
    let grant = common::delegated_test_token_for_tenant(&resources, admin, Some(tenant));
    let nowhere = Uuid::new_v4().to_string();
    let answer = post_a2a_message(&resources, &grant, admin_tool_message(Some(&nowhere))).await;
    assert!(answer["error"].is_object(), "the call is refused: {answer}");
    let rows = trail(&resources, admin).await;
    assert_eq!(
        rows.len(),
        1,
        "bug: an admin-only call A2A refuses before its task runs is on the trail: {rows:#?}"
    );
    assert_eq!(
        rows[0].outcome, "refused",
        "bug: a refused call's row says refused"
    );
    assert_eq!(
        rows[0].metadata["protocol"], "a2a",
        "bug: the row's metadata names the protocol the call came in on"
    );
}
