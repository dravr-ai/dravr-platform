// ABOUTME: Lays out an assistant turn's ReplyBlocks as ordered channel messages for one surface
// ABOUTME: The messaging egress renders what the envelope decided; it never re-decides the reply

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Rendering a [`TurnEnvelope`]'s blocks onto a messaging channel.
//!
//! The pipeline already read this surface's
//! [`RenderCapabilities`] and produced an ordered block list: what the channel
//! cannot draw was folded into the prose before it got here, and what it can
//! draw arrived as its own block. This module's whole job is layout — turn
//! each block into the canot content the adapter sends, in order.
//!
//! # Splitting, not cutting
//!
//! A channel accepts a bounded message (Discord 2000 characters, Telegram
//! 4096, Slack 40000) and rejects anything longer outright. That used to be
//! answered by trimming the reply to the ceiling, which delivered a paragraph
//! that stopped mid-thought and told the athlete nothing about the missing
//! tail. The prose is split here instead — at sentence boundaries, into
//! ordered messages that each fit — so a long answer arrives whole
//! (registre#2). The ceiling is the channel's own, read from
//! [`RenderCapabilities::max_reply_chars`], never a cross-channel constant.
//!
//! [`TurnEnvelope`]: pierre_chat_pipeline::TurnEnvelope

use pierre_chat_pipeline::{
    AssistantTurn, NoticeKind, QuotaWarningState, RenderCapabilities, ReplyBlock,
};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PROVIDER_RECONNECT_BUTTON, KEY_QUOTA_WARNING,
};
use pierre_core::chunking::chunk_reply;
use pierre_core::models::messaging::{CardAction, MessageContent, OutgoingMessage};

use super::surface::messaging_render_profile;
use super::viz_delivery::strip_viz_markers;
use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
use pierre_core::models::messaging::ChannelType;

/// One assistant turn, laid out for one channel.
pub struct RenderedReply {
    /// The coaching text, split into messages the channel will accept. Empty
    /// when the turn produced nothing readable — which is what the caller's
    /// empty-reply guard tests.
    pub prose: Vec<String>,
    /// Everything that follows the prose, in reply order: charts the channel
    /// fetches, and the reconnect control where the channel draws controls.
    pub attachments: Vec<MessageContent>,
}

/// Lay out `assistant`'s blocks for a surface with `render` capabilities.
///
/// `registry` and `locale` resolve the one string this layout adds — the
/// reconnect button's label. Everything else is text the pipeline already
/// wrote in the athlete's language.
#[must_use]
pub fn render_reply(
    render: &RenderCapabilities,
    assistant: &AssistantTurn,
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> RenderedReply {
    let mut prose = Vec::new();
    let mut attachments = Vec::new();

    for block in &assistant.blocks {
        match block {
            ReplyBlock::Prose { text } => {
                // Positional viz markers are for a client that interleaves
                // prose and charts. A channel gets an image or nothing, so the
                // markers are stripped before the split — they are characters
                // the athlete never sees and must not spend the ceiling on.
                prose.extend(chunk_reply(
                    &strip_viz_markers(text),
                    render.max_reply_chars,
                ));
            }
            ReplyBlock::SceneImage {
                url,
                mime_type,
                caption,
            } => attachments.push(MessageContent::Media {
                url: url.clone(),
                mime_type: mime_type.clone(),
                caption: caption.clone(),
            }),
            ReplyBlock::Reconnect {
                display_name, url, ..
            } => attachments.extend(reconnect_control(
                render,
                registry,
                locale,
                display_name,
                url,
            )),
            // The envelope emits this block only where controls render; on a
            // channel without them it folded the `label: value` lines into the
            // prose instead, so there is no text fallback to repeat here.
            ReplyBlock::Actions { title, actions } => attachments.push(MessageContent::Card {
                title: title.clone().unwrap_or_default(),
                body: String::new(),
                actions: actions
                    .iter()
                    .map(|action| CardAction {
                        label: action.label.clone(),
                        action_type: action.kind.as_str().to_owned(),
                        value: action.value.clone(),
                    })
                    .collect(),
            }),
            // A usage cap the athlete is close to. There is no notice element
            // on a chat channel, so it arrives as its own short message after
            // the coaching text — the same standing the in-app client draws as
            // a notice, said in words.
            ReplyBlock::Notice {
                kind: NoticeKind::QuotaWarning(warning),
            } => prose.push(quota_warning_line(registry, locale, warning)),
            // An inline Scene, a plan card, an activity panel and verdict
            // chips are all gated on a `BlockSupport` field that is false for
            // every messaging surface, so the pipeline folded their content
            // into the prose above rather than emitting them.
            ReplyBlock::Scene { .. }
            | ReplyBlock::WorkoutPlan { .. }
            | ReplyBlock::ActivityList { .. }
            | ReplyBlock::Verdicts { .. } => {}
        }
    }

    RenderedReply { prose, attachments }
}

/// The one-line form of a quota standing, for a surface with no notice
/// element to draw it in.
///
/// Reads the same counters the in-app notice renders, so an athlete who chats
/// on both surfaces is told the same thing about the same budget.
fn quota_warning_line(
    registry: &MessagingStringsRegistry,
    locale: &str,
    warning: &QuotaWarningState,
) -> String {
    registry.render(
        KEY_QUOTA_WARNING,
        locale,
        &[
            &warning.current.to_string(),
            &warning.limit.to_string(),
            &warning.resets_at,
        ],
    )
}

/// The reconnect affordance for a channel that draws controls.
///
/// The reauth sentence — with the URL in it — is already in the prose, which
/// is what a channel without buttons gives the athlete: an autolinked line
/// they can tap. Where buttons render, the same URL also rides a `url`
/// action so the reconnect is one tap instead of a copy. Returning nothing on
/// a button-less channel is what keeps the link to a single affordance rather
/// than printing it twice.
fn reconnect_control(
    render: &RenderCapabilities,
    registry: &MessagingStringsRegistry,
    locale: &str,
    display_name: &str,
    url: &str,
) -> Option<MessageContent> {
    if !render.blocks.action_buttons {
        return None;
    }
    Some(MessageContent::Card {
        title: display_name.to_owned(),
        body: String::new(),
        actions: vec![CardAction {
            label: registry.render(KEY_PROVIDER_RECONNECT_BUTTON, locale, &[display_name]),
            action_type: "url".to_owned(),
            value: url.to_owned(),
        }],
    })
}

/// How many characters one message on `channel_type` may carry.
///
/// Read off [`RenderCapabilities`] rather than off canot's descriptor
/// directly, so the number the egress splits at is the same field the coach
/// was told about in its prose contract. The locale plays no part in a
/// character ceiling; the profile needs one, so it gets the default.
#[must_use]
pub fn channel_ceiling(channel_type: ChannelType) -> usize {
    messaging_render_profile(channel_type, DEFAULT_LOCALE)
        .render
        .max_reply_chars
}

/// Split one outbound message into as many as the channel's ceiling requires.
///
/// The counterpart to [`render_reply`] for the paths that build an
/// `OutgoingMessage` without running the pipeline — a slash-command answer, a
/// proactive push. Text and rich text are split at sentence boundaries;
/// everything else (a card, a chart, a location) is a single indivisible
/// object and travels unchanged.
///
/// Every message keeps the original's addressing, turn id and thread, so a
/// split reply stays one turn in the transcript and one thread in the room.
#[must_use]
pub fn fan_out(message: OutgoingMessage, max_chars: usize) -> Vec<OutgoingMessage> {
    let body = match &message.content {
        MessageContent::Text { body } | MessageContent::RichText { body } => body,
        MessageContent::Media { .. }
        | MessageContent::Location { .. }
        | MessageContent::Card { .. } => return vec![message],
    };
    let parts = chunk_reply(body, max_chars);
    if parts.len() <= 1 {
        return vec![message];
    }
    let rich = matches!(message.content, MessageContent::RichText { .. });
    parts
        .into_iter()
        .map(|body| OutgoingMessage {
            channel_type: message.channel_type,
            recipient_id: message.recipient_id.clone(),
            content: if rich {
                MessageContent::RichText { body }
            } else {
                MessageContent::Text { body }
            },
            turn_id: message.turn_id,
            reply_to: message.reply_to.clone(),
            thread_id: message.thread_id.clone(),
        })
        .collect()
}
