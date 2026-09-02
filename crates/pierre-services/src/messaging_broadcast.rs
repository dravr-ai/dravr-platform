// ABOUTME: The proactive outbound path — send a localized text on every channel a user has linked
// ABOUTME: One implementation for account notices, notification fan-out, and any other platform push

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Platform-initiated messaging.
//!
//! "Proactive" means the platform starts the turn — an account-approved
//! notice, a dispatched notification, a backfill-ready ping — rather than
//! replying to something the athlete said. Every such send resolves the same
//! three things: which channels the user has linked, what locale each link
//! speaks, and which adapter and channel config deliver to it.
//!
//! [`send_to_linked_channels`] is that resolution, written once. Callers supply
//! only the body, as a closure over the link's locale, so a user linked on two
//! channels with two locales gets each in their own language from one call.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
use pierre_core::models::messaging::{ChannelConfig, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use pierre_messaging::rich_text::{parse_markdown, render_rich_text};
use pierre_messaging::turn::ConversationTurnId;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

/// Build a proactive text message: a fresh conversation turn with no reply or
/// thread linkage.
///
/// So `turn_id` is a fresh [`ConversationTurnId`] and both `reply_to` and
/// `thread_id` are `None`. Reply messages, which carry the inbound turn id, a
/// `reply_to`, or a `thread_id`, construct [`OutgoingMessage`] inline.
#[must_use]
pub fn proactive_text(
    channel_type: ChannelType,
    recipient_id: String,
    body: String,
) -> OutgoingMessage {
    OutgoingMessage {
        channel_type,
        recipient_id,
        content: MessageContent::Text { body },
        turn_id: ConversationTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}

/// Build a proactive message from a body authored in inline markdown.
///
/// The `**bold**`, `*italic*` and `` `code` `` runs are converted here into
/// the rich-text dialect each channel's renderer translates into its native
/// formatting.
///
/// Separate from [`proactive_text`] rather than replacing it, because the two
/// make opposite promises about the body. A `Text` body is escaped on the way
/// out — Telegram's renderer runs `encode_text` over it — which is what keeps
/// coach prose like "HR <100 bpm" from mangling the parse, and what makes
/// interpolated values (coach titles, provider names) inert. A `RichText` body
/// is parsed, so markup in it becomes formatting. Routing every proactive push
/// through this one would turn a stored value that happens to contain a marker
/// into live formatting.
///
/// Reach for it when the *string* owns the markup, as the intake questions do:
/// they ship `**1**` in all five locales, and in a `Text` envelope the athlete
/// reads the asterisks instead of a bold numeral.
#[must_use]
pub fn proactive_rich_text(
    channel_type: ChannelType,
    recipient_id: String,
    body: &str,
) -> OutgoingMessage {
    OutgoingMessage {
        channel_type,
        recipient_id,
        content: MessageContent::RichText {
            body: render_rich_text(&parse_markdown(body)),
        },
        turn_id: ConversationTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}

/// One channel a user can be reached on: which adapter, which recipient id
/// there, and which locale that link speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedChannelTarget {
    /// Channel the link belongs to.
    pub channel_type: ChannelType,
    /// The user's identifier *on that channel* (chat id, Slack user id, …).
    pub recipient_id: String,
    /// BCP-47 locale recorded on the link, or [`DEFAULT_LOCALE`] when it
    /// records none.
    pub locale: String,
}

/// Resolve every channel `user_id` can be reached on within `tenant_id`.
///
/// Split out from [`send_to_linked_channels`] because this half is the whole
/// decision — who gets told, where, and in what language — while the half after
/// it is a network call to a channel host. Links with an unreadable shape or an
/// unknown channel type are skipped, so a single malformed row cannot silence
/// the athlete's other channels.
pub async fn resolve_linked_targets(
    messaging: &dyn MessagingRepository,
    tenant_id: TenantId,
    user_id: Uuid,
) -> Vec<LinkedChannelTarget> {
    let links = match messaging
        .list_user_channel_links(tenant_id, &user_id.to_string())
        .await
    {
        Ok(links) => links,
        Err(e) => {
            warn!(error = %e, "Failed to list channel links for proactive send");
            return Vec::new();
        }
    };

    let mut targets = Vec::with_capacity(links.len());
    for link in &links {
        let Some(channel_str) = link.get("channel_type").and_then(Value::as_str) else {
            continue;
        };
        let Some(recipient) = link.get("channel_user_id").and_then(Value::as_str) else {
            continue;
        };
        let Ok(channel_type) = ChannelType::from_str(channel_str) else {
            warn!(channel = %channel_str, "Unknown channel type on link; skipping proactive send");
            continue;
        };
        targets.push(LinkedChannelTarget {
            channel_type,
            recipient_id: recipient.to_owned(),
            locale: link
                .get("locale")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_LOCALE)
                .to_owned(),
        });
    }
    targets
}

/// Send a localized text on every messaging channel `user_id` has linked
/// within `tenant_id`, and report how many were delivered.
///
/// `body_for_locale` is called once per link with that link's BCP-47 locale, so
/// the caller renders through the messaging-strings registry rather than
/// passing a pre-baked string in one language.
///
/// Best-effort by contract: a link whose channel is unconfigured, whose adapter
/// cannot be built, or whose send fails is logged and skipped. A user with no
/// links delivers to zero channels, which is not an error — it is an athlete
/// who only uses the app.
pub async fn send_to_linked_channels<F>(
    messaging: &dyn MessagingRepository,
    tenant_id: TenantId,
    user_id: Uuid,
    body_for_locale: F,
) -> usize
where
    F: Fn(&str) -> String,
{
    let targets = resolve_linked_targets(messaging, tenant_id, user_id).await;

    // One config lookup per channel type, not per link: a user linked twice on
    // the same channel would otherwise re-read and re-parse the same row.
    let mut senders: HashMap<ChannelType, (Arc<dyn MessagingChannel>, ChannelConfig)> =
        HashMap::new();
    let mut delivered = 0;

    for target in &targets {
        let channel_str = target.channel_type.to_string();
        if let Entry::Vacant(slot) = senders.entry(target.channel_type) {
            let Some(sender) =
                resolve_sender(messaging, tenant_id, &channel_str, target.channel_type).await
            else {
                continue;
            };
            slot.insert(sender);
        }
        let Some((adapter, config)) = senders.get(&target.channel_type) else {
            continue;
        };

        let outgoing = proactive_text(
            target.channel_type,
            target.recipient_id.clone(),
            body_for_locale(&target.locale),
        );
        if let Err(e) = adapter.send(&outgoing, config).await {
            warn!(error = %e, channel = %channel_str, "Proactive channel send failed");
        } else {
            info!(channel = %channel_str, "Proactive message sent on linked channel");
            delivered += 1;
        }
    }

    delivered
}

/// Resolve the channel adapter and its config for an outbound send, or `None`
/// (logged) when the channel is unconfigured or its config cannot be read.
async fn resolve_sender(
    messaging: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel_str: &str,
    channel_type: ChannelType,
) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)> {
    let raw_config = match messaging.get_channel_config(tenant_id, channel_str).await {
        Ok(Some(cfg)) => cfg,
        Ok(None) => {
            warn!(channel = %channel_str, "No channel config for proactive send");
            return None;
        }
        Err(e) => {
            warn!(error = %e, "Failed to load channel config for proactive send");
            return None;
        }
    };
    let adapter = create_adapter_from_config(channel_type, &raw_config)
        .inspect_err(|e| {
            warn!(error = %e, channel = %channel_str, "Failed to build adapter for proactive send");
        })
        .ok()?;
    let config: ChannelConfig = serde_json::from_value(raw_config)
        .inspect_err(|e| {
            warn!(error = %e, "Failed to deserialize channel config for proactive send");
        })
        .ok()?;
    Some((adapter, config))
}
