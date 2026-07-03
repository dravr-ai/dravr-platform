// ABOUTME: HTTP integration tests for MCP (Model Context Protocol) routes
// ABOUTME: Tests all MCP endpoints including tool discovery and JSON-RPC request handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! Comprehensive HTTP integration tests for MCP routes
//!
//! This test suite validates that all MCP endpoints are correctly registered
//! in the router and handle HTTP requests appropriately.

mod common;
mod helpers;

use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_mcp_server::{
    mcp::resources::{ServerContext, ServerContextOptions},
    routes::mcp::McpRoutes,
};
use serde_json::json;
use std::sync::Arc;

/// Test setup helper for MCP route testing
#[allow(dead_code)]
struct McpTestSetup {
    resources: Arc<ServerContext>,
    user_id: uuid::Uuid,
    jwt_token: String,
}

impl McpTestSetup {
    async fn new() -> anyhow::Result<Self> {
        common::init_server_config();
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();
        let cache = common::create_test_cache().await?;

        // Create test user
        let (user_id, user) = common::create_test_user(&database).await?;

        // Create ServerContext
        let temp_dir = tempfile::tempdir()?;
        let config = Arc::new(ServerConfig {
            http_port: 8081,
            database: DatabaseConfig {
                url: DatabaseUrl::Memory,
                backup: BackupConfig {
                    directory: temp_dir.path().to_path_buf(),
                    ..Default::default()
                },
                ..Default::default()
            },
            app_behavior: AppBehaviorConfig {
                ci_mode: true,
                auto_approve_users: false,
                ..Default::default()
            },
            security: SecurityConfig {
                headers: SecurityHeadersConfig {
                    environment: Environment::Testing,
                },
                ..Default::default()
            },
            ..Default::default()
        });

        let resources = Arc::new(
            ServerContext::new(
                (*database).clone(),
                (*auth_manager).clone(),
                "test_jwt_secret",
                config,
                cache,
                ServerContextOptions {
                    rsa_key_size_bits: Some(2048),
                    jwks_manager: Some(common::get_shared_test_jwks()),
                    llm_provider: None,
                    chat_provider: None,
                    extra_tools: Vec::new(),
                    billing_provider: None,
                },
            )
            .await,
        );

        // Generate JWT token for the user
        let jwt_token = auth_manager
            .generate_token(&user, &resources.auth.jwks_manager)
            .map_err(|e| anyhow::anyhow!("Failed to generate JWT: {}", e))?;

        Ok(Self {
            resources,
            user_id,
            jwt_token,
        })
    }

    fn routes(&self) -> axum::Router {
        McpRoutes::routes(self.resources.clone())
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.jwt_token)
    }
}

// ============================================================================
// GET /mcp/tools - Tool Discovery Tests
// ============================================================================

#[tokio::test]
async fn test_mcp_tools_discovery_success() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::get("/mcp/tools").send(routes).await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body["tools"].is_array());
}

#[tokio::test]
async fn test_mcp_tools_no_auth_required() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // Tools endpoint should work without authentication
    let response = AxumTestRequest::get("/mcp/tools").send(routes).await;

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_mcp_tools_response_structure() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::get("/mcp/tools").send(routes).await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body.is_object());
    assert!(body["tools"].is_array());

    // Verify each tool has required fields
    let tools = body["tools"].as_array().unwrap();
    for tool in tools {
        assert!(tool["name"].is_string());
        assert!(tool["description"].is_string());
    }
}

#[tokio::test]
async fn test_mcp_tools_returns_available_tools() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::get("/mcp/tools").send(routes).await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    let tools = body["tools"].as_array().unwrap();

    // Should have at least some tools registered
    assert!(!tools.is_empty(), "Should return at least one tool");
}

#[tokio::test]
async fn test_mcp_tools_concurrent_requests() {
    let setup = McpTestSetup::new().await.expect("Setup failed");

    // Make multiple tool discovery requests concurrently
    let mut handles = vec![];

    for _ in 0..5 {
        let routes = setup.routes();
        let handle =
            tokio::spawn(async move { AxumTestRequest::get("/mcp/tools").send(routes).await });

        handles.push(handle);
    }

    // All requests should succeed
    for handle in handles {
        let response = handle.await.expect("Task panicked");
        assert_eq!(response.status(), 200);
    }
}

// ============================================================================
// POST /mcp - JSON-RPC Request Tests
// ============================================================================

#[tokio::test]
async fn test_mcp_request_with_auth() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .json(&mcp_request)
        .send(routes)
        .await;

    // Should process the request (200) or return accepted (202)
    assert!(response.status() == 200 || response.status() == 202);
}

#[tokio::test]
async fn test_mcp_request_without_auth() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    let response = AxumTestRequest::post("/mcp")
        .json(&mcp_request)
        .send(routes)
        .await;

    // MCP can work without auth for some methods, should not be 401
    // Might be 200, 202, or 400 depending on method requirements
    assert_ne!(response.status(), 500);
}

#[tokio::test]
async fn test_mcp_request_with_session() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let session_id = format!("session_{}", uuid::Uuid::new_v4());

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .header("mcp-session-id", &session_id)
        .json(&mcp_request)
        .send(routes)
        .await;

    // Should process and may return session ID header
    assert!(response.status() == 200 || response.status() == 202);
}

#[tokio::test]
async fn test_mcp_request_invalid_json() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // Empty/non-JSON body fails to parse into a JSON-RPC envelope.
    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .header("content-type", "application/json")
        .send(routes)
        .await;

    // The tronc Streamable-HTTP transport renders a parse failure as a JSON-RPC
    // error object (code -32700) carried in the HTTP body, returned with a 200
    // status. This is JSON-RPC-conformant: the MCP Streamable-HTTP spec mandates
    // HTTP 400 only for an invalid `MCP-Protocol-Version`, a required-but-missing
    // session id, or a notification/response the server cannot accept — not for a
    // malformed request body, which is surfaced as a JSON-RPC error response.
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json();
    assert_eq!(
        body["error"]["code"], -32700,
        "parse failure must carry the JSON-RPC parse-error code: {:?}",
        body["error"]
    );
}

#[tokio::test]
async fn test_mcp_request_invalid_jsonrpc_format() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let invalid_request = json!({
        "not_jsonrpc": "invalid"
    });

    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .json(&invalid_request)
        .send(routes)
        .await;

    // A body that is valid JSON but not a JSON-RPC envelope is missing the
    // required `jsonrpc`/`method` fields, so it fails to deserialize into the
    // request type. The tronc transport surfaces that as a JSON-RPC parse error
    // (-32700) in the HTTP body with a 200 status — the same JSON-RPC-conformant
    // posture as a syntactically malformed body (see test_mcp_request_invalid_json).
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json();
    assert_eq!(
        body["error"]["code"], -32700,
        "invalid JSON-RPC envelope must carry the parse-error code: {:?}",
        body["error"]
    );
}

#[tokio::test]
async fn test_mcp_request_empty_body() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .send(routes)
        .await;

    // Should handle empty body gracefully
    assert!(response.status() == 400 || response.status() == 200);
}

#[tokio::test]
async fn test_mcp_request_with_notification() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // JSON-RPC notification (no id field)
    let notification_request = json!({
        "jsonrpc": "2.0",
        "method": "notification/test",
        "params": {}
    });

    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .json(&notification_request)
        .send(routes)
        .await;

    // A notification (no `id`) yields no JSON-RPC response, so the tronc transport
    // returns an empty-body success. tronc renders this as 204 No Content; the MCP
    // Streamable-HTTP spec's letter is 202 Accepted for an accepted notification.
    // Both are empty-body 2xx accept codes; the 204-vs-202 gap is a tronc transport
    // detail (it cannot be changed from the platform — flagged for the tronc engine).
    assert_eq!(response.status(), 204);
}

#[tokio::test]
async fn test_mcp_request_tools_list_method() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .json(&mcp_request)
        .send(routes)
        .await;

    assert!(response.status() == 200 || response.status() == 202);

    if response.status() == 200 {
        let body: serde_json::Value = response.json();
        // JSON-RPC response should have id and result or error
        assert!(body["id"].is_number() || body["id"].is_string());
    }
}

// ============================================================================
// Additional Integration Tests
// ============================================================================

#[tokio::test]
async fn test_all_mcp_endpoints_registered() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let endpoints = vec![("/mcp/tools", "GET"), ("/mcp", "POST")];

    for (endpoint, method) in endpoints {
        let response = if method == "GET" {
            AxumTestRequest::get(endpoint).send(routes.clone()).await
        } else {
            AxumTestRequest::post(endpoint)
                .json(&json!({}))
                .send(routes.clone())
                .await
        };

        // Should not be 404 (endpoint not found)
        assert_ne!(
            response.status(),
            404,
            "{} {} should be registered",
            method,
            endpoint
        );
    }
}

#[tokio::test]
async fn test_mcp_request_user_isolation() {
    let setup1 = McpTestSetup::new().await.expect("Setup 1 failed");
    let setup2 = McpTestSetup::new().await.expect("Setup 2 failed");

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    // User 1 makes a request
    let routes1 = setup1.routes();
    let response1 = AxumTestRequest::post("/mcp")
        .header("authorization", &setup1.auth_header())
        .json(&mcp_request)
        .send(routes1)
        .await;

    // User 2 makes a request
    let routes2 = setup2.routes();
    let response2 = AxumTestRequest::post("/mcp")
        .header("authorization", &setup2.auth_header())
        .json(&mcp_request)
        .send(routes2)
        .await;

    // Both should succeed independently
    assert!(response1.status() == 200 || response1.status() == 202);
    assert!(response2.status() == 200 || response2.status() == 202);
}

#[tokio::test]
async fn test_mcp_concurrent_requests() {
    let setup = McpTestSetup::new().await.expect("Setup failed");

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    // Make multiple MCP requests concurrently
    let mut handles = vec![];

    for i in 0..5 {
        let routes = setup.routes();
        let auth = setup.auth_header();
        let mut request = mcp_request.clone();
        request["id"] = json!(i);

        let handle = tokio::spawn(async move {
            AxumTestRequest::post("/mcp")
                .header("authorization", &auth)
                .json(&request)
                .send(routes)
                .await
        });

        handles.push(handle);
    }

    // All requests should succeed
    for handle in handles {
        let response = handle.await.expect("Task panicked");
        assert!(response.status() == 200 || response.status() == 202);
    }
}

#[tokio::test]
async fn test_mcp_session_persistence() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    // The tronc Streamable-HTTP transport is stateless: it carries no session
    // store and never issues an `Mcp-Session-Id`. The MCP spec makes session ids
    // OPTIONAL — a server MAY assign one only in the `InitializeResult` response,
    // and a stateless server simply omits it. Each request therefore stands on
    // its own bearer; there is no server-issued session to persist or replay.
    let response1 = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .json(&mcp_request)
        .send(routes.clone())
        .await;

    assert_eq!(response1.status(), 200);
    assert!(
        response1.header("mcp-session-id").is_none(),
        "the stateless transport must not issue an Mcp-Session-Id header"
    );

    // A second request without its own bearer is rejected: there is no stored
    // session to supply the credential. Auth is per-request, not session-based.
    let response2 = AxumTestRequest::post("/mcp")
        .json(&mcp_request)
        .send(routes)
        .await;

    assert_eq!(response2.status(), 401);
}

// ============================================================================
// tools/list Visibility Gating Tests
// ============================================================================

#[tokio::test]
async fn test_tools_list_unauthenticated_returns_401() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    // Send request WITHOUT auth header. Per the MCP authorization spec (RFC 9728),
    // a protected resource server rejects unauthenticated requests with a 401.
    let response = AxumTestRequest::post("/mcp")
        .json(&mcp_request)
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);

    // The 401 must carry a WWW-Authenticate challenge pointing at the protected
    // resource metadata so clients can discover the authorization server.
    let challenge = response
        .header("www-authenticate")
        .expect("401 must include a WWW-Authenticate header")
        .to_owned();
    assert!(
        challenge.contains("resource_metadata="),
        "WWW-Authenticate must carry resource_metadata: {challenge}"
    );
    assert!(
        challenge.contains("oauth-protected-resource"),
        "resource_metadata must point at the protected resource metadata: {challenge}"
    );

    // No tools are disclosed to unauthenticated callers.
    let body: serde_json::Value = response.json();
    assert!(
        body["result"].get("tools").is_none(),
        "Unauthenticated tools/list must not return any tools"
    );
}

#[tokio::test]
async fn test_tools_list_authenticated_returns_tools() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes_unauth = setup.routes();
    let routes_auth = setup.routes();

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    // Unauthenticated requests are rejected with 401.
    let response_unauth = AxumTestRequest::post("/mcp")
        .json(&mcp_request)
        .send(routes_unauth)
        .await;
    assert_eq!(response_unauth.status(), 401);

    // Authenticated requests receive the tool set.
    let response_auth = AxumTestRequest::post("/mcp")
        .header("authorization", &setup.auth_header())
        .json(&mcp_request)
        .send(routes_auth)
        .await;

    assert_eq!(response_auth.status(), 200);
    let body_auth: serde_json::Value = response_auth.json();
    let tools_auth = body_auth["result"]["tools"]
        .as_array()
        .expect("tools should be an array");

    assert!(
        !tools_auth.is_empty(),
        "Authenticated tools/list should return tools"
    );
    assert!(
        tools_auth
            .iter()
            .filter_map(|t| t["name"].as_str())
            .any(|n| n == "get_activities"),
        "Authenticated tools/list should include get_activities"
    );
}

#[tokio::test]
async fn test_tools_list_invalid_token_returns_401() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    // A present-but-invalid token must be rejected with 401, not silently
    // downgraded to a public tool subset (MCP authorization spec / RFC 6750).
    let response = AxumTestRequest::post("/mcp")
        .header("authorization", "Bearer invalid_token_12345")
        .json(&mcp_request)
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);

    let challenge = response
        .header("www-authenticate")
        .expect("401 must include a WWW-Authenticate header")
        .to_owned();
    assert!(
        challenge.contains("error=\"invalid_token\""),
        "Invalid-token challenge must carry error=invalid_token: {challenge}"
    );

    let body: serde_json::Value = response.json();
    assert!(
        body["result"].get("tools").is_none(),
        "Invalid-token tools/list must not return any tools"
    );
}

#[tokio::test]
async fn test_tools_list_admin_user_sees_all_tools_including_admin() {
    // Admin tool visibility is gated on the GLOBAL `User.is_admin` flag, not the
    // per-tenant Owner/Admin role — so this needs a genuine global admin, which
    // the default tenant-owner test user is not.
    let (resources, jwt) = setup_with_admin_flag("tools_list_global_admin@example.com", true)
        .await
        .expect("Setup failed");
    let routes = McpRoutes::routes(resources.clone());

    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {jwt}"))
        .json(&mcp_request)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools should be an array");

    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    // Admin user should see admin tools
    assert!(
        tool_names.iter().any(|n| n.starts_with("admin_")),
        "Admin user should see admin tools in tools/list, got: {:?}",
        tool_names
    );

    // Admin user should also see all non-admin tools
    assert!(
        tool_names.contains(&"get_activities"),
        "Admin should see get_activities"
    );
    assert!(
        tool_names.contains(&"connect_provider"),
        "Admin should see connect_provider"
    );

    // Admin should also see write/mutation tools, proving it's the full set, not
    // just the read-only core.
    assert!(
        tool_names.contains(&"set_goal"),
        "Admin should see write tool set_goal"
    );
}

#[tokio::test]
async fn test_tools_list_params_token_in_body_is_not_an_auth_path() {
    let setup = McpTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // Carrying the bearer in the JSON-RPC `params.token` field — instead of the
    // `Authorization` header — was a non-spec platform extension. The MCP
    // authorization spec (RFC 9728) defines bearer auth via the `Authorization`
    // header only, which the tronc transport's auth hook reads. `params.token` is
    // not honored on the `/mcp` HTTP route, and no production HTTP client sends it
    // (the SDK authenticates via the OAuth `authProvider` Authorization header).
    let mcp_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {
            "token": setup.jwt_token
        }
    });

    // No Authorization header — the body-embedded token is ignored, so the request
    // is unauthenticated and rejected with a 401 + WWW-Authenticate challenge.
    let response = AxumTestRequest::post("/mcp")
        .json(&mcp_request)
        .send(routes)
        .await;

    assert_eq!(
        response.status(),
        401,
        "params.token must not authenticate the request; auth requires the Authorization header"
    );
    assert!(
        response.header("www-authenticate").is_some(),
        "the 401 must carry a WWW-Authenticate challenge"
    );
}

// ============================================================================
// ADMIN_ONLY gate — global User.is_admin, NOT tenant Owner/Admin role
// ============================================================================

/// Build a `ServerContext` and create a tenant-owning user with an explicit
/// global-admin flag, returning a tenant-scoped JWT for the wire.
///
/// `global_admin` controls only `User.is_admin`; the user is always created as
/// the **owner** of its own tenant. This separates the two privilege axes the
/// `ADMIN_ONLY` gate must distinguish: a tenant owner (`TenantContext::is_admin`)
/// is admin *of their tenant*, which must not grant system-wide admin powers —
/// only the global `User.is_admin` flag does.
async fn setup_with_admin_flag(
    email: &str,
    global_admin: bool,
) -> anyhow::Result<(Arc<ServerContext>, String)> {
    common::init_server_config();
    let database = common::create_test_database().await?;
    let auth_manager = common::create_test_auth_manager();
    let cache = common::create_test_cache().await?;

    let temp_dir = tempfile::tempdir()?;
    let config = Arc::new(ServerConfig {
        http_port: 8081,
        database: DatabaseConfig {
            url: DatabaseUrl::Memory,
            backup: BackupConfig {
                directory: temp_dir.path().to_path_buf(),
                ..Default::default()
            },
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            ci_mode: true,
            auto_approve_users: false,
            ..Default::default()
        },
        security: SecurityConfig {
            headers: SecurityHeadersConfig {
                environment: Environment::Testing,
            },
            ..Default::default()
        },
        ..Default::default()
    });

    let resources = Arc::new(
        ServerContext::new(
            (*database).clone(),
            (*auth_manager).clone(),
            "test_jwt_secret",
            config,
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
            },
        )
        .await,
    );

    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Wire Test User".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    user.is_admin = global_admin;
    let user_id = user.id;

    let repos = database.repositories();
    repos.users.create(&user).await?;

    // Always create the user as the OWNER of its own tenant.
    let tenant_id = TenantId::new();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Wire Tenant for {email}"),
        slug: format!("wire-tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    repos.tenants.create(&tenant).await?;
    repos.users.update_tenant_id(user_id, tenant_id).await?;

    let jwt_token = auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .map_err(|e| anyhow::anyhow!("Failed to generate JWT: {}", e))?;

    Ok((resources, jwt_token))
}

/// Issue a `tools/call` for an `ADMIN_ONLY` tool over the `/mcp` wire and return
/// the parsed JSON-RPC response body.
async fn call_admin_only_tool(
    resources: &Arc<ServerContext>,
    jwt_token: &str,
) -> serde_json::Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "admin_list_system_coaches",
            "arguments": {}
        }
    });

    let response = AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {jwt_token}"))
        .json(&request)
        .send(McpRoutes::routes(resources.clone()))
        .await;

    assert_eq!(
        response.status(),
        200,
        "an authenticated tools/call returns the JSON-RPC result in-band"
    );
    response.json()
}

/// Regression guard for the `/mcp` wire privilege-escalation: a tenant OWNER who
/// is NOT a global admin must be DENIED an `ADMIN_ONLY` tool. The auth hook
/// resolves the gate flag from the global `User.is_admin`, not the per-tenant
/// `TenantContext::is_admin` (which is true for any Owner/Admin role).
#[tokio::test]
async fn test_tenant_owner_without_global_admin_is_denied_admin_tool() {
    let (resources, jwt) = setup_with_admin_flag("wire_owner_not_admin@example.com", false)
        .await
        .expect("setup failed");

    let body = call_admin_only_tool(&resources, &jwt).await;

    assert_eq!(
        body["result"]["isError"],
        json!(true),
        "tenant owner without global admin must be denied the ADMIN_ONLY tool, got: {body}"
    );
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .expect("error result must carry text content");
    assert!(
        text.contains("Admin access required"),
        "denial must come from the ADMIN_ONLY gate, got: {text}"
    );
}

/// Companion to the escalation guard: a user with the global `User.is_admin`
/// flag set IS allowed past the `ADMIN_ONLY` gate (reaches the tool body), so
/// the fix denies tenant owners without over-denying genuine system admins.
#[tokio::test]
async fn test_global_admin_user_is_allowed_admin_tool() {
    let (resources, jwt) = setup_with_admin_flag("wire_global_admin@example.com", true)
        .await
        .expect("setup failed");

    let body = call_admin_only_tool(&resources, &jwt).await;

    // The global admin clears the gate and runs the tool body, which lists the
    // (empty) system-coach set — a successful, non-error result.
    assert_eq!(
        body["result"]["isError"],
        json!(false),
        "global admin must clear the ADMIN_ONLY gate, got: {body}"
    );
    if let Some(text) = body["result"]["content"][0]["text"].as_str() {
        assert!(
            !text.contains("Admin access required"),
            "global admin must not hit the admin-privileges denial, got: {text}"
        );
    }
}
