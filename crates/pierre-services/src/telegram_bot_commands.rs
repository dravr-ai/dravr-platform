// ABOUTME: Publishes the slash-command catalogue to Telegram's setMyCommands so the / menu is real
// ABOUTME: canot builds the list; without this caller it was built at startup and thrown away

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Telegram bot command menu.
//!
//! Telegram shows a `/` menu built from whatever the bot last registered via
//! `setMyCommands`. `dravr-canot` has always been able to shape that list —
//! `CommandRegistry::bot_command_list` returns exactly the
//! `(command, description)` pairs the API wants — but nothing on the platform
//! ever called it, so the menu was whatever had been set by hand, or empty.
//!
//! [`publish_telegram_commands`] closes that: it runs at startup, next to the
//! channel seeding that reads the same `TELEGRAM_BOT_TOKEN`, and pushes the
//! catalogue the server actually dispatches. A command added to `commands/*.md`
//! therefore appears in the Telegram menu on the next boot rather than needing
//! a manual BotFather edit.
//!
//! Each scope is published once per locale the strings registry speaks, with
//! Telegram's `language_code` naming the locale, plus one list with no
//! `language_code` as the fallback for a viewer whose language has no list
//! of its own. Telegram serves the nearest match, so a French athlete's menu
//! reads in French while an unknown locale still gets a menu.
//!
//! The default scope already reaches groups — Telegram's resolution chain for
//! a group ends on it — so the second, `all_group_chats` list exists to
//! *re-describe* what a shared room offers, not to give it reach it lacked. A
//! nearer scope wins, so the group list is the one a group member sees.
//!
//! Both lists carry every command. A menu that hides rows costs discovery and
//! lies by omission, because the command still runs when typed. What the
//! group list changes is the description: a command marked `personal` in the
//! catalogue is prefixed with [`PERSONAL_MARKER`], so a member reading the
//! menu in a shared room can tell at a glance which entries act on them alone.
//! `BotCommand` has exactly two settable fields, `command` and `description`
//! — there is no disabled or greyed state to set — so the description is the
//! only lever a scope has.
//!
//! There is no menu button to set alongside these. Telegram's menu button is
//! a private-chat surface: `setChatMenuButton`'s `chat_id` is documented
//! "Unique identifier for the target private chat", the MTProto call beneath
//! it is `bots.setBotMenuButton user_id:InputUser` with no peer parameter at
//! all, and the live API answers a real group id with "Bad Request: invalid
//! chat_id specified". A group's entry point is the `/` composer icon, which
//! these lists fill.

use std::env;

use pierre_core::http_client::api_client;
use reqwest::StatusCode;
use serde_json::json;
use tracing::{info, warn};

/// Telegram's cap on a command description.
const MAX_DESCRIPTION_LEN: usize = 256;
/// Prefixed to a personal command's description in a shared room's menu.
///
/// An emoji rather than a word: catalogue descriptions are not localized (the
/// `commands/*.md` frontmatter carries one English line per command, while
/// localization covers the reply strings), so a word would be wrong for four
/// of the five locales. A single-person glyph reads the same in all of them.
pub const PERSONAL_MARKER: &str = "👤 ";
/// Telegram's cap on a command name (excluding the leading slash).
const MAX_COMMAND_LEN: usize = 32;

/// The `setMyCommands` payload: the commands Telegram will accept, with
/// descriptions trimmed to its limit.
///
/// Telegram rejects the whole call if any entry is malformed, so one command
/// with an uppercase letter or a hyphen would silently cost the entire menu.
/// Names must be 1-32 characters of lowercase letters, digits and underscores;
/// descriptions 1-256 characters.
#[must_use]
pub fn telegram_command_payload(commands: &[(String, String)]) -> Vec<serde_json::Value> {
    commands
        .iter()
        .filter(|(name, description)| {
            !name.is_empty()
                && name.len() <= MAX_COMMAND_LEN
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && !description.trim().is_empty()
        })
        .map(|(name, description)| {
            let trimmed: String = description
                .trim()
                .chars()
                .take(MAX_DESCRIPTION_LEN)
                .collect();
            json!({ "command": name, "description": trimmed })
        })
        .collect()
}

/// Which chats a published command list serves.
///
/// Telegram resolves a viewer's menu through a per-scope chain that ends at
/// the default scope, so the default list already reaches groups. Publishing
/// `AllGroupChats` as well does not add reach — it re-describes what a shared
/// room shows, because a nearer scope wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandScope {
    /// Every chat with no narrower list set. The fallback the chain ends on.
    Default,
    /// Group and supergroup chats only.
    AllGroupChats,
}

impl CommandScope {
    /// The `scope` object Telegram expects, or `None` for the default scope,
    /// which is expressed by omitting the field entirely.
    fn payload(self) -> Option<serde_json::Value> {
        match self {
            Self::Default => None,
            Self::AllGroupChats => Some(json!({ "type": "all_group_chats" })),
        }
    }

    /// Stable label for logs.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AllGroupChats => "all_group_chats",
        }
    }
}

/// Publish `commands` to Telegram's `setMyCommands` for the bot named by
/// `TELEGRAM_BOT_TOKEN`, under `scope`.
///
/// Returns without acting when the env var is unset or empty — the same
/// condition under which `messaging_seed` skips seeding the Telegram channel,
/// so a deployment without Telegram stays silent.
///
/// Best-effort: a Telegram outage at boot must not fail startup, so failures
/// are logged and swallowed. Neither the URL nor the response body is logged —
/// the token sits in the request path, and a rejection description can echo it.
pub async fn publish_telegram_commands(
    commands: &[(String, String)],
    scope: CommandScope,
    language_code: Option<&str>,
) {
    let Ok(token) = env::var("TELEGRAM_BOT_TOKEN") else {
        return;
    };
    if token.is_empty() {
        return;
    }

    let payload = telegram_command_payload(commands);
    if payload.is_empty() {
        warn!(
            scope = scope.as_str(),
            "No Telegram-acceptable slash commands in the catalogue; menu not published"
        );
        return;
    }
    let count = payload.len();

    let mut body = json!({ "commands": payload });
    if let Some(scope_payload) = scope.payload() {
        body["scope"] = scope_payload;
    }
    if let Some(code) = language_code {
        body["language_code"] = json!(code);
    }

    let url = format!("https://api.telegram.org/bot{token}/setMyCommands");
    // The token sits in the request path and a reqwest error carries that URL,
    // so it is masked out here, before anything reaches a log line.
    let outcome = api_client()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map(|res| res.status())
        .map_err(|e| e.to_string().replace(&token, "***"));

    log_publish_result(&outcome, count, scope);
}

/// Report the outcome of one `setMyCommands` call.
fn log_publish_result(outcome: &Result<StatusCode, String>, count: usize, scope: CommandScope) {
    match outcome {
        Ok(status) if status.is_success() => {
            info!(
                commands = count,
                scope = scope.as_str(),
                "Published the Telegram slash-command menu"
            );
        }
        Ok(status) => {
            warn!(
                %status,
                scope = scope.as_str(),
                "Telegram rejected setMyCommands; the / menu keeps its previous contents"
            );
        }
        Err(error) => {
            warn!(
                %error,
                scope = scope.as_str(),
                "setMyCommands request failed; the / menu keeps its previous contents"
            );
        }
    }
}
