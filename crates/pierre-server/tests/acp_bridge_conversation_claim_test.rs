// ABOUTME: The ACP MCP bridge's per-turn token must carry the turn's conversation, signed
// ABOUTME: Round-trips through the real signing and validation path, not just the struct
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::uninlined_format_args)]

//! A tool the model calls natively over the ACP MCP bridge runs in its OWN HTTP
//! request to `/mcp`, not inside the chat turn's task. The pipeline scopes the
//! conversation id into a task-local, which that separate request cannot see, so
//! detached follow-up work — the background activity backfill and its completion
//! push — had no channel to return to. That is one of the two native-path
//! defects the 2026-06-22 mitigation cited when it disabled native tool calling.
//!
//! The conversation now travels as a claim on the per-turn token the bridge
//! mints. Signed, not a header: this value routes a message to a chat, and a
//! caller-settable route would let one push a notice into a conversation it does
//! not own.

mod common;

use pierre_auth::auth::AuthManager;
use pierre_core::models::{User, UserTier};

fn test_user() -> User {
    let mut user = User::new(
        "acp_bridge_claim@example.com".to_owned(),
        "hash".to_owned(),
        Some("ACP Bridge".to_owned()),
    );
    user.tier = UserTier::Starter;
    user
}

/// The bridge's per-turn token carries the conversation through a real sign →
/// verify round trip, and is marked turn-scoped by the same argument.
#[tokio::test]
async fn a_turn_scoped_token_round_trips_its_conversation() {
    common::init_server_config();
    let auth_manager: AuthManager = (*common::create_test_auth_manager()).clone();
    let jwks = common::get_shared_test_jwks();
    let user = test_user();

    let token = auth_manager
        .generate_token_with_tenant_and_ttl(
            &user,
            &jwks,
            Some("11111111-1111-1111-1111-111111111111".to_owned()),
            chrono::Duration::minutes(15),
            Some("conversation-under-test"),
        )
        .expect("the bridge mints a per-turn token");

    let claims = auth_manager
        .validate_token(&token, &jwks)
        .expect("the /mcp endpoint validates it");

    assert_eq!(
        claims.turn_conversation_id(),
        Some("conversation-under-test".to_owned()),
        "the conversation must survive signing — without it a natively-called \
         tool cannot route its backfill completion push back to the channel"
    );
    // Both claims come from the one argument, so they cannot disagree.
    assert_eq!(
        claims.guardian_turn_token(),
        Some(claims.jti),
        "a token carrying a conversation is by construction turn-scoped"
    );
}

/// A reused session token is not one turn: it gets neither the Guardian turn key
/// nor a conversation to route to.
#[tokio::test]
async fn a_session_token_carries_no_conversation() {
    common::init_server_config();
    let auth_manager: AuthManager = (*common::create_test_auth_manager()).clone();
    let jwks = common::get_shared_test_jwks();
    let user = test_user();

    let token = auth_manager
        .generate_token(&user, &jwks)
        .expect("a normal session token mints");

    let claims = auth_manager
        .validate_token(&token, &jwks)
        .expect("token validates");

    assert_eq!(claims.turn_conversation_id(), None);
    assert_eq!(claims.guardian_turn_token(), None);
    assert_eq!(
        claims.turn_conversation_id, None,
        "the claim must be absent on the wire, not merely gated on read"
    );
}
