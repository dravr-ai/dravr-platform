// ABOUTME: Publishes the slash-command catalogue as Messenger's persistent menu at startup
// ABOUTME: Messenger's hamburger menu is the one always-on menu surface a bot can actually set

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Messenger persistent menu.
//!
//! Of the channels Dravr speaks, Messenger is the only one offering a bot a
//! genuine *always-on* menu it can set itself: a hamburger beside the
//! composer, populated through the Messenger Profile API and shown until the
//! page changes it. Telegram's equivalent is read-only to us (its menu button
//! is a private-chat surface with no API to aim at a group, and in a direct
//! message it already opens the command list), Slack's is the workspace's
//! slash-command registry, and `WhatsApp` has none at all.
//!
//! The menu is built from the same `commands/*.md` catalogue the `/` menu and
//! `/help` are built from, so there is one vocabulary and no second list to
//! keep in step.
//!
//! No personal marker is applied here. The Messenger Platform addresses a
//! `recipient.id` that is a page-scoped id for one person and has no group
//! thread at all, so every command in this menu is inherently the reader's
//! own — the same reason `/help` leaves a direct message unmarked.
//!
//! Best-effort, like the Telegram publisher: a Meta outage at boot must not
//! fail startup. The page access token rides in the query string, so neither
//! the URL nor a response body reaches a log line.

use std::env;

use pierre_core::http_client::api_client;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tracing::{info, warn};

/// Graph API version the payload shape below was verified against.
const GRAPH_API_VERSION: &str = "v25.0";
/// Meta's cap on top-level menu entries.
const MAX_MENU_ITEMS: usize = 20;
/// Meta's cap on a menu entry's title.
const MAX_TITLE_LEN: usize = 30;
/// Meta's cap on a postback payload.
const MAX_PAYLOAD_LEN: usize = 1000;

/// Build the `persistent_menu` body from `commands`.
///
/// Each entry is a `postback` whose payload is the command text, so tapping
/// the menu is exactly typing the command — the same contract the card
/// buttons and the `WhatsApp` list rows use, which is what lets one inbound
/// path serve all three.
///
/// Only the `postback` and `web_url` types appear in Meta's current
/// documentation; the `nested` submenu type does not, so this stays one flat
/// level rather than shipping a shape that cannot be checked against the
/// reference.
///
/// The array must carry an entry whose locale is `default` — Meta rejects a
/// menu without one. Command titles are the command text itself and are not
/// localized (the catalogue carries one description per command), so
/// `default` is the only locale published.
#[must_use]
pub fn persistent_menu_payload(commands: &[(String, String)]) -> Value {
    let actions: Vec<Value> = commands
        .iter()
        .filter(|(name, _)| !name.trim().is_empty())
        .take(MAX_MENU_ITEMS)
        .map(|(name, _)| {
            let command = format!("/{name}");
            let title: String = command.chars().take(MAX_TITLE_LEN).collect();
            let payload: String = command.chars().take(MAX_PAYLOAD_LEN).collect();
            json!({ "type": "postback", "title": title, "payload": payload })
        })
        .collect();

    json!({
        "persistent_menu": [{
            "locale": "default",
            "composer_input_disabled": false,
            "call_to_actions": actions,
        }]
    })
}

/// Publish `commands` as the Messenger page's persistent menu.
///
/// Returns without acting when `META_MESSENGER_PAGE_ACCESS_TOKEN` is unset or
/// empty — the same condition under which `messaging_seed` skips seeding the
/// Messenger channel, so a deployment without Messenger stays silent.
pub async fn publish_messenger_menu(commands: &[(String, String)]) {
    let Ok(token) = env::var("META_MESSENGER_PAGE_ACCESS_TOKEN") else {
        return;
    };
    if token.is_empty() {
        return;
    }

    let body = persistent_menu_payload(commands);
    let count = body
        .pointer("/persistent_menu/0/call_to_actions")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if count == 0 {
        warn!("No commands available for the Messenger persistent menu; not published");
        return;
    }
    // A silent cap reads as "published everything" when it did not.
    if commands.len() > MAX_MENU_ITEMS {
        warn!(
            published = count,
            available = commands.len(),
            "Messenger caps the persistent menu at 20 entries; the rest are reachable by typing"
        );
    }

    let url = format!(
        "https://graph.facebook.com/{GRAPH_API_VERSION}/me/messenger_profile?access_token={token}"
    );
    // The token sits in the query string and a reqwest error carries that URL,
    // so it is masked out here, before anything reaches a log line.
    let outcome = api_client()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map(|res| res.status())
        .map_err(|e| e.to_string().replace(&token, "***"));

    log_publish_result(&outcome, count);
}

/// Report the outcome of one `messenger_profile` write.
fn log_publish_result(outcome: &Result<StatusCode, String>, count: usize) {
    match outcome {
        Ok(status) if status.is_success() => {
            info!(entries = count, "Published the Messenger persistent menu");
        }
        Ok(status) => {
            warn!(
                %status,
                "Meta rejected the persistent menu; it keeps its previous contents"
            );
        }
        Err(error) => {
            warn!(
                %error,
                "messenger_profile request failed; the menu keeps its previous contents"
            );
        }
    }
}
