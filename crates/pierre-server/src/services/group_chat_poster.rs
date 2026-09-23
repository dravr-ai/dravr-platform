// ABOUTME: ServerGroupChatPoster — posts a group's weekly digest into the group's bound Telegram/Slack/Discord chat
// ABOUTME: Escapes the text, splits it to the channel's ceiling and sends the parts in order through the tenant's adapter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Binary-side implementation of [`GroupChatPoster`].
//!
//! The weekly-digest scheduler in `pierre-routes-groups` renders the digest a
//! group's chat reads and hands the text here, because the messaging adapters
//! live in this crate. Posting it is three decisions:
//!
//! - **Which adapter.** The tenant's stored channel config, through the same
//!   [`AdapterResolver`] the backfill notifier and the commitment reporter
//!   build their adapters with.
//! - **How the text is carried.** Member names and the group's title are data
//!   typed by people. The text is sent as rich text, whose per-channel
//!   renderers escape what each channel parses as markup: HTML on Telegram,
//!   `& < >` on Slack (so a name like `<!channel>` cannot ping the room), and
//!   `\ * _` and backticks on Discord. Slack's mrkdwn has no escape for
//!   `*`, `_` or `~`, so a name wrapped in them renders as formatting there.
//!   On Discord a name can also be a mention (`@everyone`, `<@&role>`), which
//!   markdown escaping does not touch, so every `@` is followed by a
//!   zero-width space before the text leaves ([`neutralise_mentions`]).
//! - **How long a message may be.** A large group's digest can outrun a
//!   channel's ceiling (2000 characters on Discord), so it is split with the
//!   egress splitter the proactive paths share and sent part by part, in
//!   order, stopping at the first part refused so the chat never shows a
//!   later part without an earlier one. The splitter measures the text before
//!   the channel's renderer escapes it, so on Discord, where each escape adds a
//!   backslash, the ceiling is lowered by the escapes the whole text will gain.
//!
//! Addressing is that of a room notice: the recipient is the chat itself, with
//! no reply or thread linkage. Nothing is queued for retry — the scheduler
//! closes the week after one attempt, since a resend would repeat parts
//! already delivered.

use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_messaging::rich_text::escape_markdown;
use pierre_routes_groups::GroupChatPoster;
use pierre_services::messaging_broadcast::proactive_rich_text;
use tracing::{info, warn};

use crate::services::backfill_notifier::{config_adapter_resolver, AdapterResolver};
use crate::services::messaging_ingress::block_render::{channel_ceiling, fan_out};

/// Posts into a group's bound chat through the tenant's channel adapter.
pub struct ServerGroupChatPoster {
    /// Adapter resolver (config-driven in production, faked in tests), shared
    /// with the other proactive senders so every outbound path builds its
    /// channel the same way.
    resolver: Arc<dyn AdapterResolver>,
}

impl ServerGroupChatPoster {
    /// Build the production poster over the stored channel configs.
    #[must_use]
    pub fn from_repos(repos: Arc<RepositoryRegistry>) -> Arc<dyn GroupChatPoster> {
        Arc::new(Self {
            resolver: config_adapter_resolver(repos),
        })
    }

    /// Build a poster with an explicit adapter resolver.
    ///
    /// Test seam: lets the escape, split and send path run against a fake
    /// adapter that captures the outgoing messages instead of calling a
    /// channel API.
    #[must_use]
    pub fn with_resolver(resolver: Arc<dyn AdapterResolver>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl GroupChatPoster for ServerGroupChatPoster {
    async fn post(
        &self,
        tenant_id: TenantId,
        channel_type: ChannelType,
        chat_id: &str,
        text: &str,
    ) -> usize {
        let channel = channel_type.to_string();
        let Some((adapter, config)) = self
            .resolver
            .resolve(tenant_id, &channel, channel_type)
            .await
        else {
            return 0;
        };

        let text = neutralise_mentions(channel_type, text);
        let message =
            proactive_rich_text(channel_type, chat_id.to_owned(), &escape_markdown(&text));
        let mut delivered = 0;
        for part in fan_out(message, part_ceiling(channel_type, &text)) {
            if adapter.send(&part, &config).await.is_err() {
                // The error is not logged: a transport failure renders the
                // request URL into it, and a Telegram URL carries the bot token.
                warn!(
                    channel = %channel,
                    delivered,
                    "group chat post refused; the remaining parts are not sent"
                );
                break;
            }
            delivered += 1;
        }
        if delivered > 0 {
            info!(channel = %channel, parts = delivered, "posted into a group chat");
        }
        delivered
    }
}

/// The characters Discord's renderer puts a backslash before.
const DISCORD_ESCAPED: [char; 4] = ['\\', '*', '_', '`'];

/// Zero-width space: placed after `@` it breaks a mention without changing
/// what the reader sees.
const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// `text` with every Discord mention made inert. `@everyone`, `@here`,
/// `<@user>` and `<@&role>` all start with `@` or contain it, and a
/// zero-width space after the `@` stops Discord resolving any of them. Other
/// channels get `text` unchanged.
fn neutralise_mentions(channel_type: ChannelType, text: &str) -> String {
    if channel_type != ChannelType::Discord {
        return text.to_owned();
    }
    text.replace('@', &format!("@{ZERO_WIDTH_SPACE}"))
}

/// The length each part may be measured at before the channel renders it.
///
/// Discord's renderer adds a backslash before every escaped character, after
/// the split, so the ceiling is lowered by as many escapes as the whole text
/// will gain: whichever part they fall in, it still fits once rendered. It
/// never drops below half the ceiling, where even a part made only of escaped
/// characters doubles to at most the ceiling.
fn part_ceiling(channel_type: ChannelType, text: &str) -> usize {
    let ceiling = channel_ceiling(channel_type);
    if channel_type != ChannelType::Discord {
        return ceiling;
    }
    let escapes = text.chars().filter(|c| DISCORD_ESCAPED.contains(c)).count();
    ceiling.saturating_sub(escapes).max(ceiling / 2)
}
