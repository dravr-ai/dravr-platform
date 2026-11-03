// ABOUTME: The secrets inbound provider and messaging webhooks are verified against, read once at startup
// ABOUTME: Strava, Wahoo, Garmin and WHOOP webhook secrets, and Slack's signing secret, bot token and allowed bots
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

use pierre_core::constant_time::configured_secret;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// What an inbound webhook must present, as the environment configured it at
/// startup.
///
/// Every secret is `None` when its variable is unset or blank
/// ([`configured_secret`]): the verifier then refuses every request rather
/// than accept one signed or matched against an empty value. The secrets are
/// [`SecretString`]s: never serialized, and `Debug` prints a redaction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookSecrets {
    /// `STRAVA_WEBHOOK_VERIFY_TOKEN`: what Strava's subscription handshake
    /// presents back.
    #[serde(skip)]
    pub strava_verify_token: Option<SecretString>,
    /// `STRAVA_WEBHOOK_SUBSCRIPTION_ID`: the comma-separated subscription ids
    /// this deployment registered; an event naming another is refused, and
    /// every event is refused while it is empty.
    pub strava_subscription_ids: Vec<u64>,
    /// `WAHOO_WEBHOOK_TOKEN`: the token a Wahoo workout summary carries.
    #[serde(skip)]
    pub wahoo_token: Option<SecretString>,
    /// `GARMIN_WEBHOOK_TOKEN`: the unguessable path segment every Garmin push
    /// URL configured in Garmin's developer portal carries. Garmin signs
    /// nothing, so this, the `garmin-client-id` header and a known `userId`
    /// are all a genuine push has that a forger must guess.
    #[serde(skip)]
    pub garmin_token: Option<SecretString>,
    /// `WHOOP_WEBHOOK_SECRET`: the WHOOP app's client secret its webhooks
    /// are signed with. dravr-enforme's WHOOP provider reads the same
    /// variable at startup to key its HMAC; the route refuses every event
    /// while this is `None`, so a blank one never reaches it.
    #[serde(skip)]
    pub whoop_secret: Option<SecretString>,
    /// `SLACK_SIGNING_SECRET`: the key Slack signs interactive actions with.
    #[serde(skip)]
    pub slack_signing_secret: Option<SecretString>,
    /// `SLACK_BOT_TOKEN`: the token the interactive route redraws the card a
    /// tap came from with, and answers a command postback with.
    #[serde(skip)]
    pub slack_bot_token: Option<SecretString>,
    /// `SLACK_ALLOWED_BOT_IDS`: comma-separated Slack bot ids whose messages
    /// pass canot's bot-loop filter as user input (QA drivers, trusted
    /// integration bots; never the workspace's own Pierre bot). Empty when
    /// unset.
    pub slack_allowed_bot_ids: Vec<String>,
}

impl WebhookSecrets {
    /// Read the webhook secrets from the environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            strava_verify_token: configured_secret("STRAVA_WEBHOOK_VERIFY_TOKEN")
                .map(SecretString::from),
            strava_subscription_ids: env::var("STRAVA_WEBHOOK_SUBSCRIPTION_ID")
                .map(|raw| parse_subscription_ids(&raw))
                .unwrap_or_default(),
            wahoo_token: configured_secret("WAHOO_WEBHOOK_TOKEN").map(SecretString::from),
            garmin_token: configured_secret("GARMIN_WEBHOOK_TOKEN").map(SecretString::from),
            whoop_secret: configured_secret("WHOOP_WEBHOOK_SECRET").map(SecretString::from),
            slack_signing_secret: configured_secret("SLACK_SIGNING_SECRET").map(SecretString::from),
            slack_bot_token: configured_secret("SLACK_BOT_TOKEN").map(SecretString::from),
            slack_allowed_bot_ids: env::var("SLACK_ALLOWED_BOT_IDS")
                .map(|raw| parse_allowed_bot_ids(&raw))
                .unwrap_or_default(),
        }
    }
}

/// The bot ids a comma-separated list names, trimmed, with empty entries
/// (stray, doubled or trailing commas) dropped.
#[must_use]
fn parse_allowed_bot_ids(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The subscription ids a comma-separated list names; an entry that is not a
/// number (the secret manager's placeholder, a blank) names none.
#[must_use]
fn parse_subscription_ids(raw: &str) -> Vec<u64> {
    raw.split(',')
        .filter_map(|id| id.trim().parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_allowed_bot_ids;

    #[test]
    fn allowed_bot_ids_are_trimmed_and_empty_entries_dropped() {
        assert_eq!(
            parse_allowed_bot_ids(" B0ABC123 ,,B0DEF456, "),
            ["B0ABC123", "B0DEF456"],
            "bug: padding or a doubled/trailing comma reached the allow-list"
        );
        assert_eq!(
            parse_allowed_bot_ids("B0ABC123"),
            ["B0ABC123"],
            "bug: a single id with no comma was not kept"
        );
    }

    #[test]
    fn a_list_of_only_separators_names_no_bot() {
        for raw in ["", ",,,", "  ,  ,  "] {
            assert_eq!(
                parse_allowed_bot_ids(raw),
                Vec::<String>::new(),
                "bug: {raw:?} names no bot, yet one was allowed"
            );
        }
    }
}
