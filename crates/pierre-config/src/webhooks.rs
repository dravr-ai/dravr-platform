// ABOUTME: The secrets inbound provider and messaging webhooks are verified against, read once at startup
// ABOUTME: Strava's verify token and subscription ids, Wahoo's webhook token and Slack's signing secret
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
    /// `SLACK_SIGNING_SECRET`: the key Slack signs interactive actions with.
    #[serde(skip)]
    pub slack_signing_secret: Option<SecretString>,
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
            slack_signing_secret: configured_secret("SLACK_SIGNING_SECRET").map(SecretString::from),
        }
    }
}

/// The subscription ids a comma-separated list names; an entry that is not a
/// number (the secret manager's placeholder, a blank) names none.
#[must_use]
fn parse_subscription_ids(raw: &str) -> Vec<u64> {
    raw.split(',')
        .filter_map(|id| id.trim().parse().ok())
        .collect()
}
