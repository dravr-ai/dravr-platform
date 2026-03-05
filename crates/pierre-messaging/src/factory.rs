// ABOUTME: Stateless adapter factory creating channel adapters from DB config values
// ABOUTME: Constructs on-demand adapters for webhook signature verification without a registry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

#[cfg(feature = "channel-discord")]
use crate::channels::discord::DiscordChannel;
#[cfg(feature = "channel-messenger")]
use crate::channels::messenger::MessengerChannel;
#[cfg(feature = "channel-slack")]
use crate::channels::slack::SlackChannel;
#[cfg(feature = "channel-telegram")]
use crate::channels::telegram::TelegramChannel;
#[cfg(feature = "channel-whatsapp")]
use crate::channels::whatsapp::WhatsAppChannel;

use pierre_core::errors::messaging::{MessagingError, MessagingResult};
use pierre_core::models::messaging::ChannelType;
use serde_json::Value;

use crate::channel::MessagingChannel;

/// Construct a channel adapter from a DB config row.
///
/// The adapter is stateless except for the webhook secret used for signature
/// verification. This allows creating adapters on-demand per request instead
/// of maintaining a long-lived registry that must stay in sync with the DB.
///
/// # Errors
///
/// Returns `MessagingError::ChannelNotConfigured` if the config row is missing
/// the required secret field for the given channel type.
pub fn create_adapter_from_config(
    channel_type: ChannelType,
    config: &Value,
) -> MessagingResult<Arc<dyn MessagingChannel>> {
    match channel_type {
        #[cfg(feature = "channel-slack")]
        ChannelType::Slack => {
            let secret = extract_string(config, "webhook_secret", "slack")?;
            Ok(Arc::new(SlackChannel::new(secret)))
        }
        #[cfg(feature = "channel-telegram")]
        ChannelType::Telegram => {
            let secret = extract_string(config, "webhook_secret", "telegram")?;
            Ok(Arc::new(TelegramChannel::new(secret)))
        }
        #[cfg(feature = "channel-discord")]
        ChannelType::Discord => {
            let public_key = extract_string(config, "webhook_secret", "discord")?;
            let app_id = extract_string(config, "account_id", "discord")?;
            Ok(Arc::new(DiscordChannel::new(public_key, app_id)))
        }
        #[cfg(feature = "channel-whatsapp")]
        ChannelType::WhatsApp => {
            let secret = extract_string(config, "webhook_secret", "whatsapp")?;
            Ok(Arc::new(WhatsAppChannel::new(secret)))
        }
        #[cfg(feature = "channel-messenger")]
        ChannelType::Messenger => {
            let secret = extract_string(config, "webhook_secret", "messenger")?;
            Ok(Arc::new(MessengerChannel::new(secret)))
        }
        #[allow(unreachable_patterns)]
        _ => Err(MessagingError::ChannelNotConfigured {
            channel: channel_type.to_string(),
        }),
    }
}

/// Extract a required string field from a JSON config row
fn extract_string(config: &Value, field: &str, channel: &str) -> MessagingResult<String> {
    config
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| MessagingError::ChannelNotConfigured {
            channel: format!("{channel}: missing {field}"),
        })
}
