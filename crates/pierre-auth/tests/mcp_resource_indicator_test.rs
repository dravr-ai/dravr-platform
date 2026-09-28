// ABOUTME: The MCP resource identifier (MCP_RESOURCE_URL) and the RFC 8707 resource parameters matched against it
// ABOUTME: Pins its BASE_URL default, the origin-only rule, and which requested resources bind which audience
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `mcp_resource_url` is what a client dialling `/mcp` is told the resource
//! is (RFC 9728 §3.3 has it reject a mismatch), and the only resource a
//! `resource` parameter may name (RFC 8707). Before carnet#484 the published
//! `resource` was the issuer, so the endpoint could be served under one
//! hostname only.

use pierre_auth::config::{resolve_mcp_resource_url, OAuth2ServerConfig};
use pierre_auth::oauth2_server::resource::{bound_audience, token_audience};

const BASE: &str = "https://app.example.test";
const MCP: &str = "https://mcp.example.test";

fn serving(mcp_resource_url: &str) -> OAuth2ServerConfig {
    OAuth2ServerConfig {
        mcp_resource_url: mcp_resource_url.to_owned(),
        ..OAuth2ServerConfig::default()
    }
}

#[test]
fn mcp_resource_url_wins_over_base_url() {
    assert_eq!(resolve_mcp_resource_url(Some(MCP), Some(BASE), 8081), MCP);
}

#[test]
fn an_unset_mcp_resource_url_is_base_url() {
    assert_eq!(
        resolve_mcp_resource_url(None, Some(BASE), 8081),
        BASE,
        "unset is a no-op: the resource stays the address the deployment answers to"
    );
    assert_eq!(
        resolve_mcp_resource_url(Some("  "), Some(BASE), 8081),
        BASE,
        "a blank value is not an address"
    );
    assert_eq!(
        resolve_mcp_resource_url(None, None, 8097),
        "http://localhost:8097",
        "only a deployment that knows no address gets the local form"
    );
}

#[test]
fn a_trailing_slash_is_dropped() {
    // The value is published verbatim and has the well-known path appended to
    // it, so `https://mcp.example.test/` would publish a `//.well-known` URL.
    assert_eq!(
        resolve_mcp_resource_url(Some("https://mcp.example.test/"), None, 8081),
        MCP
    );
    assert_eq!(
        resolve_mcp_resource_url(None, Some("https://app.example.test/"), 8081),
        BASE
    );
}

#[test]
fn an_origin_is_a_valid_resource_identifier() {
    serving(MCP).validate_mcp_resource_url(true).unwrap();
    serving("http://localhost:8081")
        .validate_mcp_resource_url(false)
        .unwrap();
    serving("http://127.0.0.1:8091")
        .validate_mcp_resource_url(false)
        .unwrap();
}

#[test]
fn anything_but_an_origin_is_refused() {
    for value in [
        "https://mcp.example.test/mcp",
        "https://mcp.example.test/?tenant=a",
        "https://mcp.example.test/#frag",
        "https://user:pass@mcp.example.test",
        "mcp.example.test",
        "ftp://mcp.example.test",
        "",
    ] {
        let error = serving(value)
            .validate_mcp_resource_url(false)
            .expect_err(value);
        assert!(
            error.message.contains("MCP_RESOURCE_URL"),
            "the refusal names the variable to fix: {}",
            error.message
        );
    }
}

#[test]
fn production_requires_https() {
    let error = serving("http://mcp.example.test")
        .validate_mcp_resource_url(true)
        .unwrap_err();
    assert!(error.message.contains("HTTPS"), "{}", error.message);
}

#[test]
fn the_published_identifier_binds_itself() {
    assert_eq!(bound_audience(MCP, MCP).unwrap(), MCP);
}

#[test]
fn the_forms_mcp_clients_send_bind_the_published_identifier() {
    // The MCP TypeScript SDK sends `new URL(resource).href`, which adds `/`
    // to a bare origin; clients that skip the metadata send the endpoint URL.
    // Host case and an explicit default port are the same origin.
    for requested in [
        "https://mcp.example.test/",
        "https://mcp.example.test/mcp",
        "https://MCP.example.test",
        "https://mcp.example.test:443/mcp",
    ] {
        assert_eq!(
            bound_audience(MCP, requested).unwrap(),
            MCP,
            "{requested} names the MCP resource server"
        );
    }
}

#[test]
fn another_resource_is_invalid_target() {
    for requested in [
        "https://app.example.test",
        "http://mcp.example.test",
        "https://mcp.example.test:8443",
        "https://mcp.example.test.evil.test",
        "https://evil.test/https://mcp.example.test",
    ] {
        let error = bound_audience(MCP, requested).unwrap_err();
        assert_eq!(error.error, "invalid_target", "{requested}");
        assert_eq!(
            error.error_uri.as_deref(),
            Some("https://datatracker.ietf.org/doc/html/rfc8707#section-2")
        );
    }
}

#[test]
fn a_malformed_resource_is_invalid_target() {
    for requested in [
        "mcp.example.test",
        "/mcp",
        "https://mcp.example.test/#fragment",
        "https://mcp.example.test/?q=1",
        "https://someone@mcp.example.test",
        "urn:example:mcp",
    ] {
        assert_eq!(
            bound_audience(MCP, requested).unwrap_err().error,
            "invalid_target",
            "{requested}"
        );
    }
}

#[test]
fn a_served_resource_with_a_path_binds_only_at_or_below_it() {
    let served = "https://api.example.test/mcp";
    assert_eq!(
        bound_audience(served, "https://api.example.test/mcp/").unwrap(),
        served
    );
    assert_eq!(
        bound_audience(served, "https://api.example.test/mcp/v2").unwrap(),
        served
    );
    assert!(bound_audience(served, "https://api.example.test/").is_err());
    assert!(bound_audience(served, "https://api.example.test/mcpx").is_err());
}

#[test]
fn a_token_request_without_resource_gets_the_grants() {
    assert_eq!(token_audience(MCP, None, None).unwrap(), None);
    assert_eq!(
        token_audience(MCP, None, Some(MCP)).unwrap().as_deref(),
        Some(MCP)
    );
}

#[test]
fn a_token_request_narrows_an_unbound_grant_and_repeats_a_bound_one() {
    assert_eq!(
        token_audience(MCP, Some("https://mcp.example.test/"), None)
            .unwrap()
            .as_deref(),
        Some(MCP)
    );
    assert_eq!(
        token_audience(MCP, Some(MCP), Some(MCP))
            .unwrap()
            .as_deref(),
        Some(MCP)
    );
}

#[test]
fn a_token_request_for_another_resource_is_invalid_target() {
    assert_eq!(
        token_audience(MCP, Some(BASE), Some(MCP))
            .unwrap_err()
            .error,
        "invalid_target"
    );
    assert_eq!(
        token_audience(MCP, Some(BASE), None).unwrap_err().error,
        "invalid_target"
    );
}

#[test]
fn a_grant_bound_to_a_resource_no_longer_served_is_invalid_grant() {
    // MCP_RESOURCE_URL moved after the grant was made: minting would hand the
    // client a token no resource server accepts, so it re-authorizes instead.
    assert_eq!(
        token_audience(MCP, None, Some(BASE)).unwrap_err().error,
        "invalid_grant"
    );
}
