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

use dravr_canot::channel::MessagingChannel;
use dravr_canot::factory::create_adapter_from_config;
use dravr_canot::rich_text::{parse_markdown, render_rich_text};
use dravr_canot::turn::ConversationTurnId;
use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
use pierre_core::models::messaging::{ChannelConfig, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
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
/// make opposite promises about the body. A `RichText` body is parsed, so
/// markup in it becomes formatting, and each channel's renderer then escapes
/// that channel's own metacharacters (Telegram's HTML, Slack's `& < >`,
/// Discord's markdown). A `Text` body is not parsed, and it is escaped on the
/// way out on Telegram only — its renderer runs `encode_text` over it, which
/// keeps agent prose like "HR <100 bpm" from mangling the parse. Slack and
/// Discord pass a `Text` body through as native markup, so a value
/// interpolated into one is inert on Telegram alone. Routing every proactive
/// push through this one would turn a stored value that happens to contain a
/// marker into live formatting.
///
/// Reach for it when the *string* owns the markup, as the intake questions do:
/// they ship `**1**` in all five locales, and in a `Text` envelope the athlete
/// reads the asterisks instead of a bold numeral. Reach for it too, over a body
/// passed through `escape_markdown` first, when text people typed — a member's
/// name, a chat's title — goes into a chat: the escape stops their `*` and `_`
/// being parsed as markup here, and each renderer escapes what its channel
/// would parse (Telegram's HTML, Slack's `& < >`, Discord's markdown), which is
/// how the group digest is posted into a group's chat. Slack's mrkdwn has no
/// escape for `*`, `_` or `~`, so there a name wrapped in them still renders as
/// formatting.
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
/// there, which locale that link speaks, and which tenant's bot holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedChannelTarget {
    /// Channel the link belongs to.
    pub channel_type: ChannelType,
    /// The user's identifier *on that channel* (chat id, Slack user id, …).
    pub recipient_id: String,
    /// BCP-47 locale recorded on the link, or [`DEFAULT_LOCALE`] when it
    /// records none.
    pub locale: String,
    /// The tenant that stores the link: the tenant of the bot that holds this
    /// chat, whose channel config is the one that can post into it. For a link
    /// made through the deployment bot this is the bot's tenant, not the
    /// athlete's own.
    pub tenant_id: TenantId,
}

/// Resolve every channel `user_id` can be reached on, whichever tenant's bot
/// holds each link.
///
/// Reads the user's links by `user_id` across tenants
/// (`MessagingRepository::list_channel_links_for_user`), not under one tenant:
/// ingress stores a link made through the deployment bot under the bot's
/// tenant, while the athlete lives in a personal tenant of their own, so a read
/// scoped to the athlete's tenant never reaches it. A user id is strictly
/// narrower than a tenant — each row names the person it belongs to — and the
/// platform is messaging that same person about themselves, so the result is
/// only ever the recipient's own chats. Callers pass the id of the user being
/// notified, never one taken from a request.
///
/// Split out from [`send_to_linked_channels`] because this half is the whole
/// decision — who gets told, where, in what language, and through which bot —
/// while the half after it is a network call to a channel host. Links with an
/// unreadable shape, an unknown channel type, or no valid tenant are skipped,
/// so a single malformed row cannot silence the athlete's other channels.
pub async fn resolve_linked_targets(
    messaging: &dyn MessagingRepository,
    user_id: Uuid,
) -> Vec<LinkedChannelTarget> {
    let links = match messaging
        .list_channel_links_for_user(&user_id.to_string())
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
        let Some(tenant_id) = link
            .get("tenant_id")
            .and_then(Value::as_str)
            .and_then(|raw| TenantId::parse_str(raw).ok())
        else {
            warn!(channel = %channel_str, "Channel link carries no valid tenant; skipping proactive send");
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
            tenant_id,
        });
    }
    targets
}

/// Send a localized text on every messaging channel `user_id` has linked, and
/// report how many were delivered.
///
/// Each link is delivered by the bot that holds it: the channel config is read
/// under the link's own tenant ([`LinkedChannelTarget::tenant_id`]), because
/// that is the bot the chat was opened with and the only one that can post
/// into it.
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
    user_id: Uuid,
    body_for_locale: F,
) -> usize
where
    F: Fn(&str) -> String,
{
    let targets = resolve_linked_targets(messaging, user_id).await;

    // One config lookup per bot, not per link: a user linked twice through the
    // same bot would otherwise re-read and re-parse the same row.
    let mut senders: HashMap<(TenantId, ChannelType), (Arc<dyn MessagingChannel>, ChannelConfig)> =
        HashMap::new();
    let mut delivered = 0;

    for target in &targets {
        let channel_str = target.channel_type.to_string();
        let bot = (target.tenant_id, target.channel_type);
        if let Entry::Vacant(slot) = senders.entry(bot) {
            let Some(sender) = resolve_target_sender(messaging, target).await else {
                continue;
            };
            slot.insert(sender);
        }
        let Some((adapter, config)) = senders.get(&bot) else {
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

/// Resolve the adapter and config of the bot that posts into `target`'s chat.
///
/// That is the bot the link's own tenant ([`LinkedChannelTarget::tenant_id`])
/// holds on the link's channel. `None` (logged) when that bot is unconfigured
/// or its config cannot be read.
///
/// The half of [`send_to_linked_channels`] that picks the sending bot, so the
/// choice can be checked without posting to a channel host.
pub async fn resolve_target_sender(
    messaging: &dyn MessagingRepository,
    target: &LinkedChannelTarget,
) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)> {
    let channel_type = target.channel_type;
    let channel_str = channel_type.to_string();
    let raw_config = match messaging
        .get_channel_config(target.tenant_id, &channel_str)
        .await
    {
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
