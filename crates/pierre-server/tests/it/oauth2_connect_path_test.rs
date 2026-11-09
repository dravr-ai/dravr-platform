// ABOUTME: The discovery a marketplace client reads before connecting: path-suffixed resource metadata, the 401 scope, RFC 9207 and the advertised endpoints
// ABOUTME: A resource published with the /mcp path keeps the bare origin answering, so tokens and clients from before keep working
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! claude.ai requires the RFC 9728 `resource` to equal the URL the user
//! entered (`https://mcp.example.test/mcp`), and fetches the metadata at the
//! path-suffixed well-known URL. A connection made while the resource was the
//! bare origin holds that origin as `resource`, as the audience of its tokens
//! and as the document it fetched, so all three keep being served.

use crate::common;
use crate::helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use common::{create_test_server_resources_with_config, create_test_tenant};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_auth::auth::DelegatedGrant;
use pierre_auth::config::{resolve_mcp_resource_aliases, OAuth2ServerConfig};
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::mcp::McpRoutes;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use serde_json::{json, Value};

const ISSUER: &str = "https://app.example.test";
const MCP_ORIGIN: &str = "https://mcp.example.test";
const MCP_RESOURCE: &str = "https://mcp.example.test/mcp";
const MCP_HOST: &str = "mcp.example.test";

/// The deployment of `MCP_RESOURCE_URL=https://mcp.example.test/mcp`, with
/// `BASE_URL` the issuer's host, resolved as startup resolves it.
fn path_bearing() -> OAuth2ServerConfig {
    OAuth2ServerConfig {
        issuer_url: ISSUER.to_owned(),
        mcp_resource_url: MCP_RESOURCE.to_owned(),
        mcp_resource_aliases: resolve_mcp_resource_aliases(MCP_RESOURCE, Some(ISSUER)),
        ..OAuth2ServerConfig::default()
    }
}

async fn resources_with(oauth2_server: OAuth2ServerConfig) -> Arc<ServerContext> {
    Box::pin(create_test_server_resources_with_config(ServerConfig {
        oauth2_server,
        activity_fetch_limit: 100,
        ..ServerConfig::default()
    }))
    .await
    .unwrap()
}

fn oauth2_routes(resources: &Arc<ServerContext>) -> axum::Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &resources.common.config.rate_limiting,
        )),
        refresh_token_expiry_days: 30,
        csrf_manager: resources.auth.csrf_manager.clone(),
        accounts: resources.account_service(),
        google_sign_in: None,
        strings: Arc::new(MessagingStringsRegistry::new()),
        first_party_only_sources: Arc::default(),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_901))))
}

async fn get_via(resources: &Arc<ServerContext>, uri: &str, host: &str) -> AxumTestResponse {
    AxumTestRequest::get(uri)
        .header("x-forwarded-host", host)
        .send(oauth2_routes(resources))
        .await
}

async fn tools_list(resources: &Arc<ServerContext>, bearer: Option<&str>) -> AxumTestResponse {
    let mut request = AxumTestRequest::post("/mcp")
        .header("x-forwarded-host", MCP_HOST)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}));
    if let Some(token) = bearer {
        request = request.header("authorization", &format!("Bearer {token}"));
    }
    request.send(McpRoutes::routes(resources.clone())).await
}

#[tokio::test]
async fn the_path_suffixed_document_names_the_path_bearing_resource() {
    let resources = resources_with(path_bearing()).await;

    let response = get_via(
        &resources,
        "/.well-known/oauth-protected-resource/mcp",
        MCP_HOST,
    )
    .await;
    assert_eq!(response.status(), 200);
    let metadata: Value = response.json();
    assert_eq!(metadata["resource"], MCP_RESOURCE);
    assert_eq!(metadata["authorization_servers"], json!([ISSUER]));
    assert!(
        metadata["scopes_supported"]
            .as_array()
            .unwrap()
            .contains(&json!("offline_access")),
        "a client that requests every supported scope asks for a refresh token: {metadata}"
    );
}

#[tokio::test]
async fn the_root_document_keeps_naming_the_origin_an_existing_connection_holds() {
    let resources = resources_with(path_bearing()).await;

    let metadata: Value = get_via(
        &resources,
        "/.well-known/oauth-protected-resource",
        MCP_HOST,
    )
    .await
    .json();
    assert_eq!(metadata["resource"], MCP_ORIGIN);
    assert_eq!(metadata["authorization_servers"], json!([ISSUER]));
}

#[tokio::test]
async fn a_path_this_server_publishes_no_resource_under_is_not_found() {
    let resources = resources_with(path_bearing()).await;

    for (uri, host) in [
        ("/.well-known/oauth-protected-resource/other", MCP_HOST),
        ("/.well-known/oauth-protected-resource/mcp/deeper", MCP_HOST),
        // BASE_URL's host publishes the bare origin only.
        (
            "/.well-known/oauth-protected-resource/mcp",
            "app.example.test",
        ),
    ] {
        let response = get_via(&resources, uri, host).await;
        assert_eq!(response.status(), 404, "{uri} via {host}");
    }
}

#[tokio::test]
async fn the_401_points_at_the_suffixed_document_and_asks_for_the_default_grant() {
    let resources = resources_with(path_bearing()).await;
    let expected_pointer =
        format!("resource_metadata=\"{MCP_ORIGIN}/.well-known/oauth-protected-resource/mcp\"");

    let missing = tools_list(&resources, None).await;
    assert_eq!(missing.status(), 401);
    assert_eq!(
        missing.header("www-authenticate"),
        Some(format!("Bearer {expected_pointer}, scope=\"fitness:read profile:read\"").as_str()),
        "the scope is what keeps a first connection read-only"
    );

    let invalid = tools_list(&resources, Some("not-a-token")).await;
    assert_eq!(invalid.status(), 401);
    assert_eq!(
        invalid.header("www-authenticate"),
        Some(
            format!(
                "Bearer {expected_pointer}, error=\"invalid_token\", \
                 scope=\"fitness:read profile:read\""
            )
            .as_str()
        )
    );
}

#[tokio::test]
async fn a_token_bound_to_the_origin_or_the_path_is_served_at_mcp() {
    let resources = resources_with(path_bearing()).await;
    let (user, _session) = create_test_tenant(&resources, "connect-path@example.test")
        .await
        .unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .map(|tenant| tenant.id.to_string());
    let mint = |audience: &str| {
        resources
            .auth
            .auth_manager
            .generate_oauth_access_token(
                &resources.auth.jwks_manager,
                &DelegatedGrant {
                    client_id: "mcp_client_test",
                    scopes: &["fitness:read".to_owned()],
                    audience: Some(audience),
                },
                &user.id,
                &[],
                tenant.clone(),
            )
            .unwrap()
    };

    for audience in [MCP_ORIGIN, MCP_RESOURCE, ISSUER] {
        let served = tools_list(&resources, Some(&mint(audience))).await;
        assert_eq!(served.status(), 200, "a token bound to {audience}");
    }
    let elsewhere = tools_list(&resources, Some(&mint("https://elsewhere.example.test"))).await;
    assert_eq!(elsewhere.status(), 401, "a third resource is still refused");
}

#[tokio::test]
async fn the_authorization_server_advertises_what_a_marketplace_client_reads() {
    let resources = resources_with(path_bearing()).await;

    let discovery: Value = get_via(
        &resources,
        "/.well-known/oauth-authorization-server",
        MCP_HOST,
    )
    .await
    .json();
    assert_eq!(
        discovery["token_endpoint_auth_methods_supported"],
        json!(["none", "client_secret_basic", "client_secret_post"])
    );
    assert_eq!(discovery["client_id_metadata_document_supported"], true);
    assert_eq!(
        discovery["authorization_response_iss_parameter_supported"], true,
        "RFC 9207: the flag a client reads before it insists on iss"
    );
    assert_eq!(
        discovery["revocation_endpoint"],
        format!("{ISSUER}/oauth2/revoke")
    );
    assert!(
        discovery["scopes_supported"]
            .as_array()
            .unwrap()
            .contains(&json!("offline_access")),
        "{discovery}"
    );
}
