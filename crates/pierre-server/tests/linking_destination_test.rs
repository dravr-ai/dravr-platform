// ABOUTME: A linking URL must name a destination we resolved, on every messaging channel
// ABOUTME: Defaulting one sends the athlete's account-binding code somewhere nobody chose
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guard for linking destinations, through the route an athlete calls.
//!
//! `build_linking_url` used to default the bot handle to `PierreBot` whenever
//! the channel config carried no `bot_username`. It always did: the config
//! write path persists `api_key`, `api_secret`, `webhook_secret`,
//! `verify_token`, `account_id`, `phone_number` and `bot_token` — there is no
//! `bot_username` column, so the key could never be present and every link
//! resolved to `https://t.me/PierreBot`.
//!
//! That is a real bot belonging to somebody else, and the damage is not a
//! broken link. `detect_linking_code` binds whoever sends a valid code to the
//! requesting athlete's account inside the TTL, so pressing Start on that bot
//! hands an account-binding credential to a third party.
//!
//! The rule these tests hold `POST /api/messaging/link/init/{channel}` to: a
//! channel whose config names no destination answers with an error and hands
//! out neither a URL nor a pending link, and one that does name a destination links to
//! exactly that. How each channel resolves its destination, the Telegram
//! `getMe` answer included, is unit-tested beside `build_linking_url`.
//!
//! An environment variable was the first fix attempted for the Telegram handle.
//! It removes the hardcoded default and keeps its shape — a name a person types
//! that nothing verifies — so the last test here sets one and requires the
//! route to ignore it. That covers a config with no bot token; the ways out
//! past the token — a cached answer, an unreachable `getMe`, an answer that is
//! not JSON, an answer naming no bot — need a stand-in Telegram origin and are
//! tested beside `build_linking_url`.

#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::Arc;

use axum::Router;
use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use serde_json::{json, Value};
use serial_test::serial;

/// The handle the old fallback used. Somebody else's bot.
const THIRD_PARTY_BOT: &str = "PierreBot";

/// The environment variable an earlier draft of the fix read the Telegram
/// handle from. Named here to keep it out, not to support it.
const REJECTED_ENV_VAR: &str = "PIERRE_TELEGRAM_BOT_USERNAME";

/// Sets [`REJECTED_ENV_VAR`] for one test and puts back whatever was there
/// before, on a panic too.
struct TypedBotName {
    previous: Option<String>,
}

impl TypedBotName {
    fn set(value: &str) -> Self {
        let previous = env::var(REJECTED_ENV_VAR).ok();
        env::set_var(REJECTED_ENV_VAR, value);
        Self { previous }
    }
}

impl Drop for TypedBotName {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => env::set_var(REJECTED_ENV_VAR, value),
            None => env::remove_var(REJECTED_ENV_VAR),
        }
    }
}

async fn messaging_router() -> (Arc<ServerContext>, Router, String) {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = generate_test_token(&resources, &user).await;
    let router = MessagingRoutes::routes(Arc::clone(&resources));
    (resources, router, format!("Bearer {token}"))
}

/// Enable `channel` with exactly `credentials`, then ask for a link to it.
async fn init_link(
    router: &Router,
    token: &str,
    channel: &str,
    credentials: Value,
) -> (u16, String) {
    let configured = AxumTestRequest::put(&format!("/api/messaging/channels/{channel}"))
        .header("authorization", token)
        .json(&json!({ "enabled": true, "credentials": credentials }))
        .send(router.clone())
        .await;
    assert_eq!(configured.status(), 200, "{channel} config is stored");

    let response = AxumTestRequest::post(&format!("/api/messaging/link/init/{channel}"))
        .header("authorization", token)
        .send(router.clone())
        .await;
    (response.status(), response.text())
}

/// A channel whose config names no destination must refuse, never guess.
///
/// Each config below is enabled and carries a credential — just not the one
/// that says where the code goes. That is the shape that used to produce
/// `t.me/PierreBot`, and `wa.me/?text=LINK+CODE` for an absent number: a link
/// that opens a contact picker with the binding code already typed.
#[tokio::test]
async fn a_channel_with_no_resolvable_destination_refuses_to_link() {
    let (_resources, router, token) = messaging_router().await;

    for (channel, credentials) in [
        ("telegram", json!({ "webhook_secret": "tg-secret" })),
        ("whatsapp", json!({ "api_key": "wa-key" })),
        ("messenger", json!({ "api_key": "page-token" })),
        ("slack", json!({ "webhook_secret": "signing-secret" })),
        ("discord", json!({ "webhook_secret": "public-key" })),
    ] {
        let (status, body) = init_link(&router, &token, channel, credentials).await;

        assert!(
            status >= 400,
            "{channel} has no destination, so init must fail, got {status}: {body}"
        );
        assert!(
            !body.contains("linking_url") && !body.contains("expires_at"),
            "{channel} must hand out neither a URL nor a pending link: {body}"
        );
        assert!(
            !body.contains(THIRD_PARTY_BOT),
            "{channel} must never point at somebody else's bot: {body}"
        );
    }
}

/// A channel that does name its destination links to exactly that, and the code
/// in the URL is the one stored for the bot to redeem.
#[tokio::test]
async fn a_channel_links_to_the_destination_its_config_names() {
    let (resources, router, token) = messaging_router().await;

    for (channel, credentials, prefix) in [
        (
            "whatsapp",
            json!({ "phone_number": "15551234567" }),
            "https://wa.me/15551234567?text=LINK%20",
        ),
        (
            "messenger",
            json!({ "account_id": "page-1" }),
            "https://m.me/page-1?ref=",
        ),
    ] {
        let (status, body) = init_link(&router, &token, channel, credentials).await;
        assert_eq!(status, 200, "{channel}: {body}");
        let init: Value = serde_json::from_str(&body).unwrap();
        let code = init["code"].as_str().expect("deep-link code");

        assert_eq!(
            init["linking_url"],
            format!("{prefix}{code}"),
            "{channel} links to its configured destination"
        );
        let pending = resources
            .common
            .repos
            .messaging
            .get_link_state(code)
            .await
            .unwrap();
        assert!(
            pending.is_some(),
            "{channel}: the code in the URL is the one the bot can redeem"
        );
    }
}

/// A bot name somebody typed into the server's environment is not a
/// destination. With no token there is no `getMe` answer, so there is no bot,
/// whatever the environment says.
///
/// `#[serial]` because the variable is process-global, and it is the only test
/// in this binary that writes it.
#[tokio::test]
#[serial]
async fn a_bot_name_typed_into_the_environment_is_never_a_destination() {
    const TYPED: &str = "TypedIntoTheEnvironmentBot";
    let _typed = TypedBotName::set(TYPED);
    assert_eq!(
        env::var(REJECTED_ENV_VAR).as_deref(),
        Ok(TYPED),
        "premise: the variable is set while the route runs"
    );
    let (_resources, router, token) = messaging_router().await;

    let (status, body) = init_link(
        &router,
        &token,
        "telegram",
        json!({ "webhook_secret": "tg-secret" }),
    )
    .await;

    assert!(
        status >= 400,
        "no bot token means no known bot, so init must fail, got {status}: {body}"
    );
    assert!(
        !body.contains(TYPED) && !body.contains("t.me/"),
        "{REJECTED_ENV_VAR} must not become where the binding code is sent: {body}"
    );
    assert!(
        !body.contains("linking_url") && !body.contains("expires_at"),
        "a refused link hands out neither a URL nor a pending link: {body}"
    );
}
