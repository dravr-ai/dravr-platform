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

use pierre_core::http_client::api_client;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use tracing::{info, warn};

/// The Graph API's own host.
const GRAPH_API: &str = "https://graph.facebook.com";

/// The Messenger page a persistent menu is published to: its access token,
/// and the Graph API host the call goes to.
#[derive(Debug, Clone)]
pub struct MessengerPage {
    /// Scheme and host of the Graph API, with no trailing slash.
    graph_base: String,
    /// The page access token, which rides in the query string.
    token: SecretString,
}

impl MessengerPage {
    /// The page `token` names, on Meta's own Graph API.
    #[must_use]
    pub fn new(token: SecretString) -> Self {
        Self::at(GRAPH_API, token)
    }

    /// The page `token` names, on the Graph API served at `graph_base`: the
    /// stand-in a test listens on.
    #[must_use]
    pub fn at(graph_base: &str, token: SecretString) -> Self {
        Self {
            graph_base: graph_base.trim_end_matches('/').to_owned(),
            token,
        }
    }

    /// The URL the page's profile is written at. It carries the token in its
    /// query string, so it never reaches a log line.
    fn profile_url(&self) -> String {
        format!(
            "{}/{GRAPH_API_VERSION}/me/messenger_profile?access_token={}",
            self.graph_base,
            self.token.expose_secret()
        )
    }
}

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

/// Publish `commands` as `page`'s persistent menu.
///
/// The environment is not read here: the server binary builds `page` from the
/// configuration it read at startup, and a caller that holds no page cannot
/// publish (carnet#1600).
pub async fn publish_messenger_menu(page: &MessengerPage, commands: &[(String, String)]) {
    let token = page.token.expose_secret();

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

    // The token sits in the query string and a reqwest error carries that URL,
    // so it is masked out here, before anything reaches a log line.
    let outcome = api_client()
        .post(page.profile_url())
        .json(&body)
        .send()
        .await
        .map(|res| res.status())
        .map_err(|e| e.to_string().replace(token, "***"));

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A page built on a stand-in sends there and nowhere else: a test that
    /// hands `at` its local listener relies on this to stay off Meta.
    ///
    /// The literals are wire values: the Graph API's host, its version
    /// segment and the Messenger profile path are Meta's, not this codebase's.
    #[test]
    fn a_page_built_at_a_base_calls_that_base_and_new_calls_meta() {
        let token = || SecretString::from("test-page-token".to_owned());
        assert_eq!(
            MessengerPage::at("http://127.0.0.1:4010/", token()).profile_url(),
            "http://127.0.0.1:4010/v25.0/me/messenger_profile?access_token=test-page-token",
            "bug: a page built at a base must call that base, or a test's stand-in is bypassed"
        );
        assert_eq!(
            MessengerPage::new(token()).profile_url(),
            "https://graph.facebook.com/v25.0/me/messenger_profile?access_token=test-page-token",
            "bug: a page built with `new` must call Meta's own Graph API"
        );
    }
}
