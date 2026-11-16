// ABOUTME: The messaging channel credentials the deployment's environment names, read once at startup
// ABOUTME: Telegram, WhatsApp, Messenger and Discord: what the channel seed stores and the menu publishers send with
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

use pierre_core::constant_time::configured_secret;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// The messaging channels the environment configured at startup.
///
/// The channel seed and the command-menu publishers are handed this value;
/// neither reads the environment. A config that was not built by
/// [`Self::from_env`] therefore names no channel, and a server built on it
/// seeds nothing and publishes nothing, whatever the process exports: a test
/// run on a developer's machine cannot reach the deployment's real bot
/// (carnet#1600).
///
/// Slack's bot token and signing secret are in
/// [`WebhookSecrets`](crate::webhooks::WebhookSecrets), which the interactive
/// route reads too. Every secret here is a [`SecretString`]: never
/// serialized, and `Debug` prints a redaction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelCredentials {
    /// `TELEGRAM_BOT_TOKEN`: the bot the Telegram channel is seeded with, and
    /// the one the `/` menu and the bot description are published to.
    #[serde(skip)]
    pub telegram_bot_token: Option<SecretString>,
    /// `TELEGRAM_WEBHOOK_SECRET`: what a Telegram webhook must present.
    #[serde(skip)]
    pub telegram_webhook_secret: Option<SecretString>,
    /// `META_WHATSAPP_APP_SECRET`: the key Meta signs `WhatsApp` webhooks with.
    #[serde(skip)]
    pub whatsapp_app_secret: Option<SecretString>,
    /// `META_WHATSAPP_ACCESS_TOKEN`: the token `WhatsApp` messages are sent with.
    #[serde(skip)]
    pub whatsapp_access_token: Option<SecretString>,
    /// `META_WHATSAPP_PHONE_NUMBER_ID`: Meta's id for the business number,
    /// as the variable states it.
    pub whatsapp_phone_number_id: Option<String>,
    /// `META_WHATSAPP_VERIFY_TOKEN`: what Meta's subscription handshake
    /// presents back; `None` when blank.
    #[serde(skip)]
    pub whatsapp_verify_token: Option<SecretString>,
    /// `META_MESSENGER_APP_SECRET`: the key Meta signs Messenger webhooks with.
    #[serde(skip)]
    pub messenger_app_secret: Option<SecretString>,
    /// `META_MESSENGER_PAGE_ACCESS_TOKEN`: the page the Messenger channel is
    /// seeded with, and the one the persistent menu is published to.
    #[serde(skip)]
    pub messenger_page_access_token: Option<SecretString>,
    /// `META_MESSENGER_VERIFY_TOKEN`: what Meta's subscription handshake
    /// presents back; `None` when blank.
    #[serde(skip)]
    pub messenger_verify_token: Option<SecretString>,
    /// `DISCORD_BOT_TOKEN`: the bot the Discord channel is seeded with.
    #[serde(skip)]
    pub discord_bot_token: Option<SecretString>,
    /// `DISCORD_PUBLIC_KEY`: the key Discord interactions are verified with.
    #[serde(skip)]
    pub discord_public_key: Option<SecretString>,
    /// `DISCORD_APPLICATION_ID`: the Discord application, as the variable
    /// states it.
    pub discord_application_id: Option<String>,
}

impl ChannelCredentials {
    /// Read the channel credentials from the environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            telegram_bot_token: named("TELEGRAM_BOT_TOKEN"),
            telegram_webhook_secret: named("TELEGRAM_WEBHOOK_SECRET"),
            whatsapp_app_secret: named("META_WHATSAPP_APP_SECRET"),
            whatsapp_access_token: named("META_WHATSAPP_ACCESS_TOKEN"),
            whatsapp_phone_number_id: env::var("META_WHATSAPP_PHONE_NUMBER_ID").ok(),
            whatsapp_verify_token: configured_secret("META_WHATSAPP_VERIFY_TOKEN")
                .map(SecretString::from),
            messenger_app_secret: named("META_MESSENGER_APP_SECRET"),
            messenger_page_access_token: named("META_MESSENGER_PAGE_ACCESS_TOKEN"),
            messenger_verify_token: configured_secret("META_MESSENGER_VERIFY_TOKEN")
                .map(SecretString::from),
            discord_bot_token: named("DISCORD_BOT_TOKEN"),
            discord_public_key: named("DISCORD_PUBLIC_KEY"),
            discord_application_id: env::var("DISCORD_APPLICATION_ID").ok(),
        }
    }
}

/// The credential `var` names, or `None` when it is unset or empty: the rule
/// the channel seed and the menu publishers have always skipped a channel on.
fn named(var: &str) -> Option<SecretString> {
    env::var(var)
        .ok()
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}
