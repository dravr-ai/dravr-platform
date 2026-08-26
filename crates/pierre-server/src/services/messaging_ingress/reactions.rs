// ABOUTME: Turns an inbound emoji reaction on a channel message into the shared per-message feedback
// ABOUTME: One feedback system: web thumbs, mobile thumbs and a channel emoji all write the same row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reactions as message feedback.
//!
//! The web app and the mobile app both rate a coach reply with a thumb, and
//! both write `chat_message_feedback`. A messaging channel has no thumb — it
//! has an emoji on the bubble — so this module maps that emoji onto the same
//! write. There is one feedback system with three surfaces, not three.
//!
//! # What the platform is handed
//!
//! A reaction webhook names the channel's own message (Telegram `message_id`,
//! Slack `ts`, Discord snowflake), never a chat message. The outbound persist
//! stamps `messaging_messages.chat_message_id` on the row it sends, so the
//! repository can walk back from the channel id to the assistant message.
//!
//! # Reach
//!
//! Telegram delivers reactions on its webhook, Slack on both its webhook and
//! Socket Mode, and both paths call in here.
//!
//! LIMITATION(registre#106): Discord reactions never reach this module.
//! `DiscordChannel::parse_reactions` reads a Gateway dispatch envelope
//! correctly and the webhook path would hand it one, but Discord posts only
//! interactions to a webhook, and dravr-canot v0.4.23's Gateway client
//! forwards `MESSAGE_CREATE` alone and asks for neither reaction intent — so
//! the frames are never received to forward. The gate below is the channel's
//! own predicate, so Discord starts working the moment canot surfaces those
//! frames; nothing here has to change.
//!
//! # Who is allowed to rate
//!
//! In a group room every member who speaks gets their own session and their
//! own conversation; the assistant reply belongs to the member who asked.
//! A reaction from anyone else is a bystander's applause, not that athlete's
//! rating, so it is dropped rather than written under their name. The
//! repository's own ownership gate would refuse the write anyway; the guard
//! here is what keeps a group room from turning every refusal into a logged
//! error.

use std::sync::Arc;

use pierre_core::models::messaging::{ChannelType, InboundReaction, ReactionAction};
use pierre_core::models::UpsertMessageFeedbackParams;
use pierre_database::repositories::{MessagingRepository, ReactionFeedbackTarget};
use pierre_messaging::channels::discord::DiscordDescriptor;
use pierre_messaging::channels::messenger::MessengerDescriptor;
use pierre_messaging::channels::slack::SlackDescriptor;
use pierre_messaging::channels::telegram::TelegramDescriptor;
use pierre_messaging::channels::whatsapp::WhatsAppDescriptor;
use pierre_messaging::descriptor::ChannelDescriptor;
use tracing::{debug, info, warn};

use crate::mcp::resources::ServerContext;

/// Emoji and channel-native reaction names that read as approval.
///
/// Telegram sends the Unicode character, Slack sends its own name
/// (`thumbsup`), Discord sends `emoji.name` — which is the Unicode character
/// for a standard emoji and the custom name otherwise. All three shapes are
/// listed so the mapping does not depend on which channel asked.
const POSITIVE_REACTIONS: &[&str] = &[
    "👍",
    "👏",
    "❤",
    "🔥",
    "💯",
    "🙏",
    "🎉",
    "✅",
    "💪",
    "😍",
    "🤩",
    "😁",
    "🥰",
    "thumbsup",
    "+1",
    "clap",
    "heart",
    "heart_eyes",
    "fire",
    "100",
    "pray",
    "tada",
    "white_check_mark",
    "muscle",
    "star-struck",
    "raised_hands",
    "ok_hand",
    "smile",
];

/// Emoji and channel-native reaction names that read as disapproval.
const NEGATIVE_REACTIONS: &[&str] = &[
    "👎",
    "💩",
    "😡",
    "🤮",
    "🤬",
    "😢",
    "thumbsdown",
    "-1",
    "poop",
    "rage",
    "face_vomiting",
    "cry",
    "x",
];

/// Whether a channel's webhook API delivers inbound reaction events at all.
///
/// Answers from the channel's own [`ChannelDescriptor`] rather than from its
/// name: the predicate belongs to the channel adapter, and a host that
/// re-derives it from a slug drifts the first time a platform gains or loses
/// the capability.
#[must_use]
pub fn channel_delivers_reactions(channel_type: ChannelType) -> bool {
    let descriptor: &dyn ChannelDescriptor = match channel_type {
        ChannelType::Telegram => &TelegramDescriptor,
        ChannelType::Slack => &SlackDescriptor,
        ChannelType::Discord => &DiscordDescriptor,
        ChannelType::WhatsApp => &WhatsAppDescriptor,
        ChannelType::Messenger => &MessengerDescriptor,
    };
    descriptor.delivers_inbound_reactions()
}

/// The rating an emoji stands for, or `None` when it says nothing about the
/// reply.
///
/// A trailing variation selector (`U+FE0F`) is stripped first: `❤️` and `❤`
/// are the same reaction wearing different clothes, and only one of them
/// arrives from any given client.
#[must_use]
pub fn rating_for_emoji(emoji: &str) -> Option<&'static str> {
    let normalized = emoji.trim().trim_end_matches('\u{fe0f}');
    if POSITIVE_REACTIONS.contains(&normalized) {
        return Some("up");
    }
    if NEGATIVE_REACTIONS.contains(&normalized) {
        return Some("down");
    }
    None
}

/// Apply every reaction in one webhook payload to the feedback store.
///
/// Removals are applied before additions. One Telegram `message_reaction`
/// update reports the reactor's whole reaction set before and after, so
/// swapping 👍 for 👎 arrives as an addition and a removal together —
/// clearing first is what leaves the athlete on the rating they just chose
/// rather than on nothing.
pub async fn apply_reactions(resources: &Arc<ServerContext>, reactions: &[InboundReaction]) {
    for reaction in reactions
        .iter()
        .filter(|r| matches!(r.action, ReactionAction::Removed))
        .chain(
            reactions
                .iter()
                .filter(|r| matches!(r.action, ReactionAction::Added)),
        )
    {
        apply_one(resources, reaction).await;
    }
}

/// Resolve one reaction to a chat message and write (or clear) the rating.
///
/// Every way this can decline — an emoji that is not a rating, a channel
/// message the platform never sent, a reactor who is not the conversation's
/// athlete — is a logged no-op. A reaction is unsolicited input on a message
/// that may be months old; none of those cases is a fault to surface.
async fn apply_one(resources: &Arc<ServerContext>, reaction: &InboundReaction) {
    let Some(rating) = rating_for_emoji(&reaction.emoji) else {
        debug!(
            channel = %reaction.channel_type,
            emoji = %reaction.emoji,
            "reaction carries no rating; nothing to record"
        );
        return;
    };
    let Some(target) = resolve_target(resources, reaction).await else {
        return;
    };

    if reaction.reactor_id != target.channel_user_id {
        info!(
            channel = %reaction.channel_type,
            conversation_id = %target.conversation_id,
            "reaction came from a room member who is not the conversation's athlete; not recorded"
        );
        return;
    }

    match reaction.action {
        ReactionAction::Added => record_rating(resources, &target, rating, reaction).await,
        ReactionAction::Removed => clear_rating(resources, &target, reaction).await,
    }
}

/// Walk from the channel message the reaction quotes back to the assistant
/// message it delivered.
///
/// `None` covers both "the platform never sent that message" and "the lookup
/// itself failed"; the caller treats them the same, because neither is
/// something the athlete can be told about.
async fn resolve_target(
    resources: &Arc<ServerContext>,
    reaction: &InboundReaction,
) -> Option<ReactionFeedbackTarget> {
    let messaging: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    match messaging
        .find_reaction_feedback_target(
            &reaction.channel_type.to_string(),
            &reaction.channel_message_id,
            reaction.conversation_id.as_deref(),
        )
        .await
    {
        Ok(Some(target)) => Some(target),
        Ok(None) => {
            debug!(
                channel = %reaction.channel_type,
                channel_message_id = %reaction.channel_message_id,
                "reaction targets no assistant message the platform sent; ignored"
            );
            None
        }
        Err(e) => {
            warn!(error = %e, "reaction target lookup failed");
            None
        }
    }
}

/// Write the athlete's rating on the assistant message the reaction targeted.
async fn record_rating(
    resources: &Arc<ServerContext>,
    target: &ReactionFeedbackTarget,
    rating: &str,
    reaction: &InboundReaction,
) {
    let result = resources
        .common
        .repos
        .chat
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id: target.tenant_id,
            conversation_id: &target.conversation_id,
            message_id: &target.chat_message_id,
            user_id: &target.user_id,
            rating,
            // The emoji is the whole statement; a channel reaction carries no
            // "what went wrong?" text the way the web thumbs-down does.
            comment: None,
        })
        .await;

    match result {
        // Deliberately not a `notify` event: the web and mobile thumbs write
        // the same row without emitting one, and a reaction is the same act on
        // a third surface.
        Ok(_) => info!(
            channel = %reaction.channel_type,
            rating = %rating,
            "message feedback recorded from a channel reaction"
        ),
        Err(e) => warn!(error = %e, "reaction feedback write failed"),
    }
}

/// Clear the athlete's rating when they take their reaction back.
async fn clear_rating(
    resources: &Arc<ServerContext>,
    target: &ReactionFeedbackTarget,
    reaction: &InboundReaction,
) {
    match resources
        .common
        .repos
        .chat
        .delete_message_feedback(&target.chat_message_id, &target.user_id, target.tenant_id)
        .await
    {
        Ok(removed) => debug!(
            channel = %reaction.channel_type,
            removed,
            "reaction removed; rating cleared"
        ),
        Err(e) => warn!(error = %e, "reaction feedback clear failed"),
    }
}
