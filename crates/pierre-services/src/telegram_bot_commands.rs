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
//! The registration is for Telegram's default scope, with no `language_code`:
//! the command catalogue carries one description per command (localization in
//! `commands/*.md` covers the reply strings, not the frontmatter descriptions),
//! and the default scope is what Telegram serves to every user locale.

use std::env;

use pierre_core::http_client::api_client;
use reqwest::StatusCode;
use serde_json::json;
use tracing::{info, warn};

/// Telegram's cap on a command description.
const MAX_DESCRIPTION_LEN: usize = 256;
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

/// Publish `commands` to Telegram's `setMyCommands` for the bot named by
/// `TELEGRAM_BOT_TOKEN`.
///
/// Returns without acting when the env var is unset or empty — the same
/// condition under which `messaging_seed` skips seeding the Telegram channel,
/// so a deployment without Telegram stays silent.
///
/// Best-effort: a Telegram outage at boot must not fail startup, so failures
/// are logged and swallowed. Neither the URL nor the response body is logged —
/// the token sits in the request path, and a rejection description can echo it.
pub async fn publish_telegram_commands(commands: &[(String, String)]) {
    let Ok(token) = env::var("TELEGRAM_BOT_TOKEN") else {
        return;
    };
    if token.is_empty() {
        return;
    }

    let payload = telegram_command_payload(commands);
    if payload.is_empty() {
        warn!("No Telegram-acceptable slash commands in the catalogue; menu not published");
        return;
    }
    let count = payload.len();

    let url = format!("https://api.telegram.org/bot{token}/setMyCommands");
    // The token sits in the request path and a reqwest error carries that URL,
    // so it is masked out here, before anything reaches a log line.
    let outcome = api_client()
        .post(&url)
        .json(&json!({ "commands": payload }))
        .send()
        .await
        .map(|res| res.status())
        .map_err(|e| e.to_string().replace(&token, "***"));

    log_publish_result(&outcome, count);
}

/// Report the outcome of one `setMyCommands` call.
fn log_publish_result(outcome: &Result<StatusCode, String>, count: usize) {
    match outcome {
        Ok(status) if status.is_success() => {
            info!(
                commands = count,
                "Published the Telegram slash-command menu"
            );
        }
        Ok(status) => {
            warn!(
                %status,
                "Telegram rejected setMyCommands; the / menu keeps its previous contents"
            );
        }
        Err(error) => {
            warn!(
                %error,
                "setMyCommands request failed; the / menu keeps its previous contents"
            );
        }
    }
}
