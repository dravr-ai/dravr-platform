// ABOUTME: The OAuth issuer a deployment publishes — the localhost default must not survive into one
// ABOUTME: Precedence is OAUTH2_ISSUER_URL, then BASE_URL, then the local form
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `issuer_url` is published verbatim in `/.well-known/oauth-authorization-server`
//! and `/.well-known/oauth-protected-resource`, so it is not an internal
//! default: it is the address every external MCP client is told to send its
//! authorization, token and JWKS requests to. `OAUTH2_ISSUER_URL` is set
//! nowhere in infra, so before carnet#358 the deployed dev backend advertised
//! `http://localhost:8081` and no external client could complete OAuth.

use pierre_auth::config::resolve_issuer_url;

const DEPLOYED: &str = "https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app";

#[test]
fn an_explicit_issuer_wins() {
    assert_eq!(
        resolve_issuer_url(Some("https://auth.example.test"), Some(DEPLOYED), 8081),
        "https://auth.example.test",
        "an operator who names the issuer outright is not second-guessed"
    );
}

#[test]
fn a_deployment_falls_back_to_the_address_it_knows_itself_by() {
    // This is the carnet#358 case: nothing sets OAUTH2_ISSUER_URL, but dev
    // does set BASE_URL from frontend_base_url, so the well-known documents
    // now name a host an external client can actually reach.
    assert_eq!(
        resolve_issuer_url(None, Some(DEPLOYED), 8081),
        DEPLOYED,
        "with no explicit issuer, BASE_URL is what the deployment is reachable at"
    );
}

#[test]
fn only_a_deployment_that_knows_no_address_gets_localhost() {
    assert_eq!(
        resolve_issuer_url(None, None, 8081),
        "http://localhost:8081",
        "the local form is for the local case, which is the only one left"
    );
    assert_eq!(
        resolve_issuer_url(None, None, 8097),
        "http://localhost:8097",
        "and it follows the port the server actually listens on"
    );
}

#[test]
fn an_empty_value_is_not_an_address() {
    // An unset variable and one set to the empty string reach this differently
    // — `env::var` yields Err for the first and Ok("") for the second — and a
    // published issuer of "" is worse than localhost, because it is not even
    // a URL to diagnose.
    assert_eq!(
        resolve_issuer_url(Some(""), Some(DEPLOYED), 8081),
        DEPLOYED,
        "an empty explicit issuer falls through rather than being published"
    );
    assert_eq!(
        resolve_issuer_url(Some("   "), None, 8081),
        "http://localhost:8081",
        "and whitespace is not an address either"
    );
    assert_eq!(
        resolve_issuer_url(None, Some(""), 8081),
        "http://localhost:8081",
        "the same holds for BASE_URL"
    );
}
