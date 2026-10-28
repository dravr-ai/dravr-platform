// ABOUTME: Real MCP server E2E tests with SSE streaming integration
// ABOUTME: Tests actual Axum server with MCP protocol over HTTP POST + SSE transport
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use anyhow::Result;
use dravr_tronc::testkit::assert::assert_rpc_error;
use dravr_tronc::testkit::McpTestClient;
use pierre_auth::auth::{AuthManager, DelegatedGrant};
use pierre_cache::{Cache, CacheConfig};
use pierre_config::environment::{
    AppBehaviorConfig, AuthConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment,
    ExternalServicesConfig, HttpClientConfig, LogLevel, LoggingConfig, OAuth2ServerConfig,
    OAuthConfig, PostgresPoolConfig, ProtocolConfig, SecurityConfig, SecurityHeadersConfig,
    ServerConfig, SseConfig, TlsConfig,
};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_mcp_server::mcp::{
    multitenant::ProviderToolRouter,
    resources::{ServerContext, ServerContextOptions},
};
use rand::Rng;
use reqwest::Client;
use serde_json::{json, Value};
use std::{net::TcpListener, path::PathBuf, sync::Arc, time::Duration};
use tokio::{task::JoinHandle, time::sleep};
use uuid::Uuid;

const TEST_JWT_SECRET: &str = "test_jwt_secret_for_sse_e2e_tests";

/// Check if a port is available
fn is_port_available(port: u16) -> bool {
    TcpListener::bind(format!("127.0.0.1:{port}")).is_ok()
}

/// Find an available port
fn find_available_port() -> u16 {
    let mut rng = rand::rng();
    for _ in 0..100 {
        let port = rng.random_range(40000..50000);
        if is_port_available(port) {
            return port;
        }
    }
    panic!("Could not find an available port after 100 attempts");
}

/// Test server setup
struct TestServer {
    port: u16,
    database: Arc<Database>,
    auth_manager: Arc<AuthManager>,
    jwt_secret: String,
    _temp_dir: tempfile::TempDir,
}

impl TestServer {
    async fn new() -> Result<Self> {
        let port = find_available_port();
        let temp_dir = tempfile::tempdir()?;
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();
        let jwt_secret = TEST_JWT_SECRET.to_owned();

        Ok(Self {
            port,
            database,
            auth_manager,
            jwt_secret,
            _temp_dir: temp_dir,
        })
    }

    async fn start(&self) -> Result<JoinHandle<()>> {
        let config = self.create_config();
        let cache = Cache::new(CacheConfig {
            max_entries: 1000,
            redis_url: None,
            cleanup_interval: Duration::from_mins(1),
            enable_background_cleanup: false,
            ..Default::default()
        })
        .await?;

        let resources = Arc::new(
            ServerContext::new(
                (*self.database).clone(),
                (*self.auth_manager).clone(),
                &self.jwt_secret,
                config,
                cache,
                ServerContextOptions {
                    rsa_key_size_bits: Some(2048),
                    jwks_manager: Some(common::get_shared_test_jwks()),
                    llm_provider: None,
                    chat_provider: None,
                    extra_tools: Vec::new(),
                    billing_provider: None,
                    turn_runner: None,
                    archive_imports: None,
                },
            )
            .await
            .unwrap(),
        );

        let server = ProviderToolRouter::new(resources);
        let port = self.port;

        let handle = tokio::spawn(async move {
            if let Err(e) = server.run(port).await {
                panic!("the MCP server exited with an error: {e}");
            }
        });

        // Wait for server to be ready
        sleep(Duration::from_millis(500)).await;

        Ok(handle)
    }

    fn create_config(&self) -> Arc<ServerConfig> {
        Arc::new(ServerConfig {
            http_port: self.port,
            log_level: LogLevel::Info,
            logging: LoggingConfig::default(),
            http_client: HttpClientConfig::default(),
            database: DatabaseConfig {
                url: DatabaseUrl::Memory,
                auto_migrate: true,
                backup: BackupConfig {
                    enabled: false,
                    interval_seconds: 3600,
                    retention_count: 7,
                    directory: PathBuf::from("test_backups"),
                },
                postgres_pool: PostgresPoolConfig::default(),
            },
            auth: AuthConfig {
                jwt_expiry_hours: 24,
                refresh_token_expiry_days: 30,
            },
            oauth: OAuthConfig::default(),
            security: SecurityConfig {
                cors_origins: vec!["*".to_owned()],
                allowed_mobile_redirect_origins: vec![],
                tls: TlsConfig {
                    enabled: false,
                    cert_path: None,
                    key_path: None,
                },
                headers: SecurityHeadersConfig {
                    environment: Environment::Testing,
                },
            },
            external_services: ExternalServicesConfig::default(),
            app_behavior: AppBehaviorConfig {
                max_activities_fetch: 100,
                default_activities_limit: 20,
                ci_mode: true,
                auto_approve_users: false,
                auto_approve_users_from_env: false,
                auto_approve_domains: vec![],
                protocol: ProtocolConfig {
                    mcp_version: "2025-06-18".to_owned(),
                    server_name: "pierre-mcp-server-test".to_owned(),
                    server_version: env!("CARGO_PKG_VERSION").to_owned(),
                },
            },
            sse: SseConfig::default(),
            oauth2_server: OAuth2ServerConfig::default(),
            ..Default::default()
        })
    }

    async fn create_test_user(&self, email: &str, password: &str) -> Result<(Uuid, String)> {
        let user_id = Uuid::new_v4();
        let password_hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)?;

        let user = User {
            id: user_id,
            email: email.to_owned(),
            display_name: Some("Test User".to_owned()),
            password_hash,
            tier: UserTier::Starter,
            strava_token: None,
            is_active: true,
            user_status: UserStatus::Active,
            is_admin: false,
            role: UserRole::User,
            approved_by: Some(user_id),
            approved_at: Some(chrono::Utc::now()),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            firebase_uid: None,
            auth_provider: String::new(),
            analytics_consent: false,
            analytics_consent_at: None,
            locale: "fr".to_owned(),
            coaching_persona: CoachingPersona::Casual,
            manages_roster: false,
            timezone: None,
            theme: None,
        };

        let repos = self.database.repositories();
        repos.users.create(&user).await?;

        // E3: the tronc-backed `PierreAuthHook` resolves a tenant context for
        // every MCP request and rejects a tenant-less caller with 403. Create the
        // tenant with this user as owner (which also inserts the owner into
        // `tenant_users`) so the auth hook can resolve the default tenant.
        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: "Test Tenant".to_owned(),
            slug: format!("test-tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        repos.tenants.create(&tenant).await?;
        repos.users.update_tenant_id(user_id, tenant_id).await?;

        // An MCP client's delegated grant: `/mcp` refuses a first-party
        // session token (carnet#768).
        let scopes: Vec<String> = OAuthScope::delegable_as_str()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let jwt_token = self.auth_manager.generate_oauth_access_token(
            &common::get_shared_test_jwks(),
            &DelegatedGrant {
                client_id: "mcp_client_test",
                scopes: &scopes,
                audience: None,
            },
            &user.id,
            &[],
            None,
        )?;

        Ok((user_id, jwt_token))
    }
}

/// tronc's MCP test client for `/mcp` on the server at `port`, presenting
/// `jwt_token` — the client every dravr MCP host's tests drive
/// (carnet#1000), here over a real socket.
fn mcp_client(port: u16, jwt_token: String) -> McpTestClient {
    McpTestClient::http(format!("http://127.0.0.1:{port}/mcp")).with_bearer(jwt_token)
}

// ============================================================================
// TEST 1: Real Server with MCP HTTP POST Requests
// ============================================================================

#[tokio::test]
async fn test_real_server_mcp_http_post() -> Result<()> {
    let server = TestServer::new().await?;
    let _handle = server.start().await?;

    let (_user_id, jwt_token) = server
        .create_test_user("test@example.com", "password123")
        .await?;
    let client = mcp_client(server.port, jwt_token);

    // Test 1: Initialize
    let init_result = client.initialize().await?;
    assert!(init_result.is_object());
    // Protocol version can be any supported version (backward compat)
    let protocol_version = init_result["protocolVersion"].as_str().unwrap();
    assert!(
        protocol_version == "2025-11-25"
            || protocol_version == "2025-06-18"
            || protocol_version == "2024-11-05",
        "Expected supported MCP protocol version, got: {protocol_version}"
    );

    // Test 2: List tools — decoded as tool definitions, named as registered
    let tools = client.list_tools().await?;
    assert!(!tools.is_empty(), "Should have at least one tool");
    assert!(
        tools
            .iter()
            .any(|tool| tool.name == "get_connection_status"),
        "the provider-connection tool is listed: {:?}",
        tools.iter().map(|tool| &tool.name).collect::<Vec<_>>()
    );

    println!(
        "✅ Test passed: Real server MCP HTTP POST - {} tools available",
        tools.len()
    );

    Ok(())
}

// ============================================================================
// TEST 2: SSE Connection to Real Server
// ============================================================================

// ============================================================================
// TEST 3: MCP POST Request + Verify Session Created
// ============================================================================

#[tokio::test]
async fn test_mcp_post_creates_session() -> Result<()> {
    let server = TestServer::new().await?;
    let _handle = server.start().await?;

    let (_user_id, jwt_token) = server
        .create_test_user("session@example.com", "password123")
        .await?;
    let client = mcp_client(server.port, jwt_token);

    // Make MCP request which should create a session
    let init_result = client.initialize().await?;
    assert!(init_result.is_object());

    // Make another request to verify session persists
    let tools = client.list_tools().await?;
    assert!(!tools.is_empty());

    println!("✅ Test passed: MCP POST requests create and maintain sessions");

    Ok(())
}

// ============================================================================
// TEST 4: Concurrent MCP Clients
// ============================================================================

#[tokio::test]
async fn test_concurrent_mcp_clients() -> Result<()> {
    let server = TestServer::new().await?;
    let _handle = server.start().await?;

    // Create multiple users and clients
    let mut handles = vec![];

    for i in 0..5 {
        let (_user_id, jwt_token) = server
            .create_test_user(&format!("user{i}@example.com"), "password123")
            .await?;
        let client = mcp_client(server.port, jwt_token);

        let handle = tokio::spawn(async move {
            // Each client initializes and lists tools
            let init_result = client.initialize().await;
            assert!(init_result.is_ok(), "{init_result:?}");

            let tools = client.list_tools().await.unwrap();
            assert!(!tools.is_empty());

            i
        });

        handles.push(handle);
    }

    // Wait for all clients to complete
    for handle in handles {
        let client_id = handle.await?;
        println!("✅ Client {client_id} completed successfully");
    }

    println!("✅ Test passed: 5 concurrent MCP clients handled successfully");

    Ok(())
}

// ============================================================================
// TEST 5: MCP Authentication Required
// ============================================================================

#[tokio::test]
async fn test_mcp_authentication_required() -> Result<()> {
    let server = TestServer::new().await?;
    let _handle = server.start().await?;

    let http_client = Client::new();

    // Test 1: unauthenticated tools/list is rejected at the transport with 401 (RFC 9728)
    let response = http_client
        .post(format!("http://127.0.0.1:{}/mcp", server.port))
        .header("Content-Type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        }))
        .send()
        .await?;

    // All /mcp methods require auth post-RFC-9728; unauthenticated requests are rejected.
    assert_eq!(
        response.status(),
        401,
        "unauthenticated tools/list must be rejected with 401"
    );
    assert!(
        response.headers().contains_key("www-authenticate"),
        "401 must carry a WWW-Authenticate challenge (RFC 9728)"
    );

    // Test 2: unauthenticated tools/call is likewise rejected with 401 + a JSON-RPC auth error
    let response2 = http_client
        .post(format!("http://127.0.0.1:{}/mcp", server.port))
        .header("Content-Type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "get_activities",
                "arguments": {}
            }
        }))
        .send()
        .await?;

    // The 401 carries a JSON-RPC auth error body (RFC 9728 resource-server challenge).
    assert_eq!(
        response2.status(),
        401,
        "unauthenticated tools/call must be rejected with 401"
    );

    let json: Value = response2.json().await?;
    // E3: the tronc transport renders the 401 as a JSON-RPC auth error object
    // with code -32001 and the generic message "Unauthorized" (the descriptive
    // platform-specific "Authentication required" message was retired). Assert the
    // structured auth-error shape: an error object with the auth error code and a
    // message that still signals auth ("unauthorized" satisfies a case-insensitive
    // "auth" check).
    assert!(json["error"].is_object(), "Should have error object");
    assert_eq!(
        json["error"]["code"], -32001,
        "401 body must carry the JSON-RPC auth error code -32001: {:?}",
        json["error"]
    );
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("auth"),
        "Error message should signal authentication: {:?}",
        json["error"]
    );

    println!("✅ Test passed: unauthenticated /mcp is rejected with 401 (RFC 9728)");

    Ok(())
}

// ============================================================================
// TEST 6: Multiple Sequential MCP Requests
// ============================================================================

#[tokio::test]
async fn test_multiple_sequential_mcp_requests() -> Result<()> {
    let server = TestServer::new().await?;
    let _handle = server.start().await?;

    let (_user_id, jwt_token) = server
        .create_test_user("seq@example.com", "password123")
        .await?;
    let client = mcp_client(server.port, jwt_token);

    // Make 10 sequential requests
    for i in 0..10 {
        let tools = client.list_tools().await?;
        assert!(!tools.is_empty());
        println!("Request {}/10 completed", i + 1);
    }

    println!("✅ Test passed: 10 sequential MCP requests completed successfully");

    Ok(())
}

// ============================================================================
// TEST 7: MCP Error Handling
// ============================================================================

#[tokio::test]
async fn test_mcp_error_handling() -> Result<()> {
    let server = TestServer::new().await?;
    let _handle = server.start().await?;

    let (_user_id, jwt_token) = server
        .create_test_user("error@example.com", "password123")
        .await?;
    let client = mcp_client(server.port, jwt_token);

    // Test: Invalid method should return JSON-RPC error
    let response = client.request("invalid/method", None).await?;
    let message = assert_rpc_error(&response, -32601);
    println!("MCP error response: {message}");

    println!("✅ Test passed: MCP error handling works correctly");

    Ok(())
}

// ============================================================================
// TEST 8: MCP Tool Call with Parameters
// ============================================================================

#[tokio::test]
async fn test_mcp_tool_call_with_params() -> Result<()> {
    let server = TestServer::new().await?;
    let _handle = server.start().await?;

    let (_user_id, jwt_token) = server
        .create_test_user("tool@example.com", "password123")
        .await?;
    let client = mcp_client(server.port, jwt_token);

    // Initialize first
    let init_result = client.initialize().await?;
    assert!(
        init_result["protocolVersion"].is_string(),
        "initialize negotiates a protocol version: {init_result}"
    );

    // Try to call get_connection_status tool (this should work even without OAuth)
    let result = client
        .call_tool(
            "get_connection_status",
            json!({
                "provider": "strava"
            }),
        )
        .await;

    // The tool runs with no provider linked: its answer is a tool result the
    // client decodes (a failure would be `is_error`, never a protocol error).
    let response = result?;
    println!("✅ Tool call answered: is_error={:?}", response.is_error);

    println!("✅ Test passed: MCP tool calls can be made with parameters");

    Ok(())
}
