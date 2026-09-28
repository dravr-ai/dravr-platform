// ABOUTME: Pins the MCP endpoint's Origin allowlist wiring (DNS-rebinding protection)
// ABOUTME: Asserts MCP_ALLOWED_ORIGINS reaches the tronc engine and that POST /mcp 403s an unlisted Origin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dravr_tronc::mcp::transport::http::mcp_router;
use pierre_config::environment::ServerConfig;
use pierre_config::mcp::McpConfig;
use pierre_config::network::CorsConfig;
use pierre_mcp_server::mcp::host_seams::build_mcp_server;
use tower::ServiceExt;

mod common;

/// Drive a real `POST /mcp` through the router the server actually mounts.
///
/// `origin` is sent as the `Origin` header when `Some`; `None` reproduces a
/// native or CLI MCP client, which sends no `Origin` at all.
async fn post_mcp(allowlist: &[&str], origin: Option<&str>) -> StatusCode {
    let resources = Box::pin(common::create_test_server_resources_with_config(
        config_with_origins(allowlist, "*"),
    ))
    .await
    .expect("Should build test resources");

    let app = mcp_router(build_mcp_server(resources));
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json");
    if let Some(origin) = origin {
        builder = builder.header("origin", origin);
    }
    let request = builder
        .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
        .expect("request builds");

    app.oneshot(request)
        .await
        .expect("router responds")
        .status()
}

/// Build a `ServerConfig` whose MCP and CORS lists disagree.
///
/// Letting them differ is what allows a test to tell which of the two the
/// engine actually received.
fn config_with_origins(mcp_origins: &[&str], cors: &str) -> ServerConfig {
    ServerConfig {
        activity_fetch_limit: 100,
        mcp: McpConfig {
            allowed_origins: mcp_origins.iter().map(|o| (*o).to_owned()).collect(),
            ..McpConfig::default()
        },
        cors: CorsConfig {
            allowed_origins: cors.to_owned(),
            allow_localhost_dev: true,
        },
        ..ServerConfig::default()
    }
}

/// The configured allowlist must reach the engine.
///
/// Before this wiring existed the list was always empty, which the tronc of the
/// day treated as permit-any — so every browser origin reached `POST /mcp`.
#[tokio::test]
async fn test_configured_origins_reach_the_engine() {
    common::init_server_config();

    let resources = Box::pin(common::create_test_server_resources_with_config(
        config_with_origins(&["https://app.dravr.ai", "https://admin.dravr.ai"], "*"),
    ))
    .await
    .expect("Should build test resources");

    let server = build_mcp_server(resources);

    assert_eq!(
        server.allowed_origins(),
        ["https://app.dravr.ai", "https://admin.dravr.ai"],
        "MCP_ALLOWED_ORIGINS must reach the engine"
    );
}

/// The MCP allowlist is deliberately not the CORS list.
///
/// Deployed environments wildcard CORS so proxied web and mobile clients work,
/// while the MCP endpoint has no legitimate browser caller. Reusing the CORS
/// value would make the guard inert exactly where it is needed.
#[tokio::test]
async fn test_mcp_allowlist_is_independent_of_cors() {
    common::init_server_config();

    let resources = Box::pin(common::create_test_server_resources_with_config(
        config_with_origins(&["https://app.dravr.ai"], "*"),
    ))
    .await
    .expect("Should build test resources");

    let server = build_mcp_server(resources);

    assert!(
        !server.allowed_origins().iter().any(|o| o == "*"),
        "a wildcard CORS list must not leak into the MCP allowlist"
    );
    assert_eq!(server.allowed_origins(), ["https://app.dravr.ai"]);
}

/// The env var is a comma-separated list.
///
/// Entries are trimmed and blanks dropped, so a trailing comma or a padded
/// value does not produce an origin that can never match.
#[tokio::test]
async fn test_env_list_is_split_and_trimmed() {
    env::set_var(
        "MCP_ALLOWED_ORIGINS",
        " https://a.example , https://b.example ,",
    );
    let parsed = McpConfig::from_env().allowed_origins;
    env::remove_var("MCP_ALLOWED_ORIGINS");

    assert_eq!(parsed, ["https://a.example", "https://b.example"]);
}

/// An unset `MCP_ALLOWED_ORIGINS` admits only loopback browser origins.
///
/// The engine reads an empty allowlist as loopback-only, so a page on another
/// site that rebinds its name to 127.0.0.1 cannot drive a local server, while
/// a local web client on `localhost` keeps working. A deployment that serves a
/// browser client from a real hostname must list it.
#[tokio::test]
async fn test_unset_allowlist_admits_only_loopback_origins() {
    common::init_server_config();

    let remote = Box::pin(post_mcp(&[], Some("https://evil.example.com"))).await;
    assert_eq!(
        remote,
        StatusCode::FORBIDDEN,
        "an unset MCP_ALLOWED_ORIGINS must refuse a non-loopback browser origin"
    );

    let local = Box::pin(post_mcp(&[], Some("http://localhost:3000"))).await;
    assert_ne!(
        local,
        StatusCode::FORBIDDEN,
        "an unset MCP_ALLOWED_ORIGINS must admit a loopback origin"
    );
}

/// An unlisted browser Origin is rejected with 403 before authentication.
///
/// This is the behaviour the whole change exists for. The wiring assertions
/// above prove the allowlist reaches the engine; this proves the composed
/// system actually refuses a request, which is what DNS-rebinding protection
/// means.
#[tokio::test]
async fn test_unlisted_origin_is_refused_with_403() {
    common::init_server_config();

    let status = Box::pin(post_mcp(
        &["https://app.dravr.ai"],
        Some("https://evil.example.com"),
    ))
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an Origin outside MCP_ALLOWED_ORIGINS must be refused with 403"
    );
}

/// A listed Origin passes the gate and reaches the auth layer.
///
/// Asserting "not 403" rather than a specific code keeps the test about the
/// Origin gate: the unauthenticated request is then refused by the auth hook,
/// and that code is the auth layer's business, not this test's.
#[tokio::test]
async fn test_listed_origin_passes_the_gate() {
    common::init_server_config();

    let status = Box::pin(post_mcp(
        &["https://app.dravr.ai"],
        Some("https://app.dravr.ai"),
    ))
    .await;

    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "a listed Origin must pass the gate and reach authentication"
    );
}

/// A request with no `Origin` header is never gated.
///
/// Native and CLI MCP clients and server-to-server connectors send no Origin;
/// gating them would break every real consumer of this endpoint. Pinned so a
/// future stricter default cannot silently lock them out.
#[tokio::test]
async fn test_absent_origin_is_never_gated() {
    common::init_server_config();

    let status = Box::pin(post_mcp(&["https://app.dravr.ai"], None)).await;

    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "a request with no Origin (native/CLI client) must never be gated"
    );
}
