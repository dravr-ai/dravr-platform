// ABOUTME: Drives the Slack/Discord OAuth code exchange against a local stub speaking their real shapes
// ABOUTME: Covers the round trip that could not be verified without real OAuth apps — including every failure branch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # Verifying the OAuth exchange without OAuth apps
//!
//! The Slack and Discord link flow ends in a code-for-identity exchange, and that
//! round trip shipped unverified: testing it appeared to need real apps on both
//! platforms.
//!
//! It does not. The exchange takes its endpoints as parameters, so a local stub
//! speaking the same response shapes exercises the real code — the real form
//! encoding, the real bearer auth, the real `sub`-or-`id` extraction, the real
//! error branches. What a live app would additionally prove is that Slack and
//! Discord still return those shapes, which is a contract-drift question rather
//! than a correctness one, and one no amount of local testing settles.
//!
//! The stub deliberately mirrors the documented payloads rather than a
//! convenient minimum: Slack `OpenID` Connect returns the stable user id as `sub`,
//! Discord's `users/@me` returns it as `id`, and both wrap the token in
//! `access_token`. Getting those wrong is the failure this suite exists to catch.

use axum::routing::{get, post};
use axum::{Json, Router};
use pierre_core::models::messaging::ChannelType;
use pierre_mcp_server::routes::messaging::{exchange_code_for_identity, oauth_endpoints};
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// Stand up a stub on an ephemeral port and return its base URL.
///
/// `token_body` and `identity_body` are returned verbatim, so a test can shape
/// either response — including malformed ones — without touching the stub.
async fn spawn_provider_stub(token_body: Value, identity_body: Value) -> String {
    let app = Router::new()
        .route("/token", post(move || async move { Json(token_body) }))
        .route("/identity", get(move || async move { Json(identity_body) }));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Run the exchange against a stub returning these two bodies.
async fn exchange_against(token_body: Value, identity_body: Value) -> Result<String, String> {
    let base = spawn_provider_stub(token_body, identity_body).await;
    exchange_code_for_identity(
        &format!("{base}/token"),
        &format!("{base}/identity"),
        "client-id",
        "client-secret",
        "auth-code",
        "https://dravr.test/api/messaging/link/callback/slack",
        "slack",
    )
    .await
    .map_err(|e| e.to_string())
}

/// Slack `OpenID` Connect returns the stable user id as `sub`.
#[tokio::test]
async fn a_slack_shaped_response_yields_the_sub_claim() {
    let identity = exchange_against(
        json!({"ok": true, "access_token": "xoxp-stub", "token_type": "Bearer"}),
        json!({
            "ok": true,
            "sub": "U02ABCDEF",
            "https://slack.com/user_id": "U02ABCDEF",
            "email": "maya@example.com"
        }),
    )
    .await
    .expect("a well-formed Slack exchange must succeed");

    assert_eq!(
        identity, "U02ABCDEF",
        "Slack's stable identifier is `sub`, not the email or the namespaced claim"
    );
}

/// Discord's `users/@me` returns the snowflake as `id`.
#[tokio::test]
async fn a_discord_shaped_response_yields_the_id_field() {
    let identity = exchange_against(
        json!({"access_token": "discord-stub", "token_type": "Bearer", "scope": "identify"}),
        json!({"id": "80351110224678912", "username": "maya", "discriminator": "0"}),
    )
    .await
    .expect("a well-formed Discord exchange must succeed");

    assert_eq!(
        identity, "80351110224678912",
        "Discord's stable identifier is `id`, not the username"
    );
}

/// A rejected exchange must fail rather than link the wrong person — and must
/// not leak the provider's body, which echoes back the client secret.
#[tokio::test]
async fn a_rejected_token_exchange_fails_without_leaking_the_body() {
    let err = exchange_against(
        json!({"ok": false, "error": "invalid_client_id", "client_secret": "super-secret-value"}),
        json!({"sub": "U02ABCDEF"}),
    )
    .await
    .expect_err("a response with no access_token must fail");

    assert!(
        !err.contains("super-secret-value"),
        "the error must not carry the provider body — a rejected exchange echoes the secret back: {err}"
    );
}

/// A token that works but an identity payload carrying no id must fail loudly.
/// Falling through would create a channel link keyed on an empty string, which
/// the next sender would collide with.
#[tokio::test]
async fn an_identity_without_a_user_id_fails() {
    let err = exchange_against(
        json!({"access_token": "stub"}),
        json!({"ok": true, "team": {"id": "T123"}}),
    )
    .await
    .expect_err("an identity with neither sub nor id must fail");

    assert!(
        !err.is_empty(),
        "the failure must carry a message an operator can act on"
    );
}

/// An empty id is not an id. Treating it as one would key a link on "" and let
/// the next sender inherit the account.
#[tokio::test]
async fn an_empty_user_id_is_rejected() {
    let result = exchange_against(json!({"access_token": "stub"}), json!({"sub": ""})).await;

    assert!(
        result.is_err(),
        "an empty sub must be rejected, not linked — it would key the link on an empty string"
    );
}

/// The endpoint pairing itself: the right hosts, and a clear refusal for the
/// channels that do not link by OAuth.
#[test]
fn endpoints_are_paired_per_provider_and_refused_for_deep_link_channels() {
    let (slack_token, slack_identity) = oauth_endpoints(ChannelType::Slack).unwrap();
    assert!(slack_token.contains("slack.com"));
    assert!(slack_identity.contains("userInfo"));

    let (discord_token, discord_identity) = oauth_endpoints(ChannelType::Discord).unwrap();
    assert!(discord_token.contains("discord.com"));
    assert!(discord_identity.contains("users/@me"));

    for deep in [
        ChannelType::Telegram,
        ChannelType::WhatsApp,
        ChannelType::Messenger,
    ] {
        assert!(
            oauth_endpoints(deep).is_err(),
            "{deep} links by deep link; asking for OAuth endpoints is a bug, not a fallback"
        );
    }
}
