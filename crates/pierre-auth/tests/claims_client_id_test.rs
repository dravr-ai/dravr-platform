// ABOUTME: Tests the RFC 9068 client_id claim on pierre-auth's JWT claims
// ABOUTME: An older token decodes without it, it round-trips, and only sub == client_id marks a client's token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The `client_id` claim names the OAuth client a token was issued to, and is
//! what the tool-call audit event records as the client. A client-credentials
//! token names that client as its subject as well, which is the one shape
//! [`Claims::client_credentials_client`] reads as a client's own token.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_auth::auth::Claims;
use pierre_core::constants::service_names::{MCP, PIERRE_MCP_SERVER};
use serde_json::{json, Value};
use uuid::Uuid;

/// A token's claims as JSON, naming `sub` and, when given, `client_id`.
fn claims_json(sub: &str, client_id: Option<&str>) -> Value {
    let mut claims = json!({
        "sub": sub,
        "email": "athlete@example.com",
        "iat": 1_700_000_000,
        "exp": 1_700_003_600,
        "iss": PIERRE_MCP_SERVER,
        "jti": "jti-1",
        "providers": [],
        "aud": MCP,
        "scope": "fitness:read",
    });
    if let Some(client_id) = client_id {
        claims["client_id"] = json!(client_id);
    }
    claims
}

#[test]
fn a_token_minted_before_the_client_id_claim_decodes_without_one() {
    let claims: Claims =
        serde_json::from_value(claims_json(&Uuid::new_v4().to_string(), None)).unwrap();
    assert_eq!(claims.client_id, None);
    assert_eq!(claims.client_credentials_client(), None);
}

#[test]
fn the_client_id_claim_round_trips_and_is_omitted_when_absent() {
    let user = Uuid::new_v4().to_string();
    let named: Claims = serde_json::from_value(claims_json(&user, Some("mcp_client_abc"))).unwrap();
    let rendered = serde_json::to_value(&named).unwrap();
    assert_eq!(rendered["client_id"], "mcp_client_abc");

    let unnamed: Claims = serde_json::from_value(claims_json(&user, None)).unwrap();
    let rendered = serde_json::to_value(&unnamed).unwrap();
    assert!(rendered.get("client_id").is_none(), "{rendered}");
}

#[test]
fn only_a_token_whose_subject_is_its_client_is_a_client_credentials_token() {
    let client: Claims =
        serde_json::from_value(claims_json("a2a_client_1", Some("a2a_client_1"))).unwrap();
    assert_eq!(client.client_credentials_client(), Some("a2a_client_1"));

    // A user's token through an integration names the client, not as its
    // subject: it acts for the user.
    let delegated: Claims = serde_json::from_value(claims_json(
        &Uuid::new_v4().to_string(),
        Some("a2a_client_1"),
    ))
    .unwrap();
    assert_eq!(delegated.client_credentials_client(), None);

    // A prefixed subject naming no `client_id` is not a client-credentials
    // token: only the two claims together mark one.
    let prefixed: Claims =
        serde_json::from_value(claims_json("client:a2a_client_1", None)).unwrap();
    assert_eq!(prefixed.client_credentials_client(), None);
}
