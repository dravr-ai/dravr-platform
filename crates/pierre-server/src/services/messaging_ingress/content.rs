// ABOUTME: The text and type label of a channel message's content, read one way by every messaging path
// ABOUTME: The inbound and outbound ledgers, the intake, the agent pick and the room transcript all read it here

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::messaging::{ChannelType, MessageContent};

/// Extract a content type label from the message content variant
pub(super) fn content_type_label(content: &MessageContent) -> &'static str {
    match content {
        MessageContent::Text { .. } => "text",
        MessageContent::RichText { .. } => "rich_text",
        MessageContent::Media { .. } => "media",
        MessageContent::Location { .. } => "location",
        MessageContent::Card { .. } => "card",
    }
}

/// The text canot puts in a `Text` message when the platform sent a type it does not parse
///
/// canot's `WhatsApp`, Telegram and Messenger transports collapse every such type into
/// this one literal, so the platform cannot tell them apart by content. On `WhatsApp`
/// that is a reaction, a sticker or a contact card; on Telegram a voice note, video,
/// document or sticker.
const UNSUPPORTED_PLACEHOLDER: &str = "[unsupported message type]";

/// `true` when `content` is canot's stand-in for a message type it does not parse
fn is_unsupported_placeholder(content: &MessageContent) -> bool {
    matches!(content, MessageContent::Text { body } if body == UNSUPPORTED_PLACEHOLDER)
}

/// `true` when the message carries nothing the agent can read and nothing the sender
/// should be told about: it is dropped before a link prompt, a session or a ledger row.
///
/// The placeholder on `WhatsApp` is most often a reaction (a thumbs-up on an earlier
/// message), which canot cannot tell from a sticker, so answering "text only" to it
/// would answer every emoji tap. The other channels make no such promise, see
/// [`needs_text_only_reply`].
pub(super) fn is_ignorable_placeholder(
    content: &MessageContent,
    channel_type: ChannelType,
) -> bool {
    is_unsupported_placeholder(content) && channel_type == ChannelType::WhatsApp
}

/// `true` when the sender sent something to read that the platform cannot read yet, and
/// is owed a reply saying so instead of silence.
///
/// That is a media message without a caption (voice note, photo, video, document), and,
/// on Telegram only, canot's placeholder: a Telegram reaction never arrives as a
/// message (see `reactions.rs`), so there the placeholder is a voice note, a video, a
/// document or a sticker.
///
/// Only a message to the agent is owed it (`addressed`: a direct message, or one that
/// addresses the bot). A photo posted into a shared room is for the people in it, and
/// answering it "text only" would talk over them in a room that answers every message.
///
/// Messenger is left out: canot parses its thumbs-up "like" as an uncaptioned image
/// attachment, indistinguishable from a photo, so answering media there would answer
/// every like.
pub(super) fn needs_text_only_reply(
    content: &MessageContent,
    channel_type: ChannelType,
    addressed: bool,
) -> bool {
    if !addressed {
        return false;
    }
    match content {
        MessageContent::Media { .. } => {
            channel_type != ChannelType::Messenger && content_body_text(content).is_none()
        }
        MessageContent::Text { .. } => {
            is_unsupported_placeholder(content) && channel_type == ChannelType::Telegram
        }
        MessageContent::RichText { .. }
        | MessageContent::Location { .. }
        | MessageContent::Card { .. } => false,
    }
}

/// Extract the text body from the message content (if applicable)
///
/// A blank caption and canot's unsupported-type placeholder read as no text, so neither
/// reaches the agent, the intake or a slash-command parse.
///
/// LIMITATION(registre#1670): media content is never read — a voice note, photo, video or document is not downloaded, transcribed or analysed, so it only draws the text-only reply, and a caption is all the agent sees.
pub(super) fn content_body_text(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::Text { .. } if is_unsupported_placeholder(content) => None,
        MessageContent::Text { body }
        | MessageContent::RichText { body }
        | MessageContent::Card { body, .. } => Some(body.clone()),
        MessageContent::Media { caption, .. } => caption
            .as_ref()
            .filter(|caption| !caption.trim().is_empty())
            .cloned(),
        MessageContent::Location { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{content_body_text, is_ignorable_placeholder, needs_text_only_reply};
    use pierre_core::models::messaging::{ChannelType, MessageContent};

    fn media(caption: Option<&str>) -> MessageContent {
        MessageContent::Media {
            url: "media-id".to_owned(),
            mime_type: "audio/ogg".to_owned(),
            caption: caption.map(str::to_owned),
        }
    }

    fn placeholder() -> MessageContent {
        MessageContent::Text {
            body: "[unsupported message type]".to_owned(),
        }
    }

    #[test]
    fn a_captioned_media_message_is_answered_from_its_caption() {
        let content = media(Some("Tempo run this morning"));
        assert_eq!(
            content_body_text(&content).as_deref(),
            Some("Tempo run this morning"),
            "the caption is what the athlete asked, so it is the text the agent reads"
        );
        assert!(
            !needs_text_only_reply(&content, ChannelType::WhatsApp, true),
            "a captioned photo has text to answer and must not be told the agent reads text only"
        );
    }

    #[test]
    fn an_uncaptioned_or_blank_captioned_media_message_is_owed_the_text_only_reply() {
        for content in [media(None), media(Some("   "))] {
            assert_eq!(
                content_body_text(&content),
                None,
                "a missing or blank caption carries no text for the agent: {content:?}"
            );
            for channel in [ChannelType::WhatsApp, ChannelType::Telegram] {
                assert!(
                    needs_text_only_reply(&content, channel, true),
                    "{content:?} sent to the agent on {channel:?} is owed the reply"
                );
            }
        }
    }

    #[test]
    fn the_unsupported_placeholder_never_reads_as_text() {
        assert_eq!(
            content_body_text(&placeholder()),
            None,
            "canot's placeholder is not the athlete's words and must never reach the agent as text"
        );
    }

    #[test]
    fn on_whatsapp_the_placeholder_is_dropped_without_a_reply() {
        assert!(
            is_ignorable_placeholder(&placeholder(), ChannelType::WhatsApp),
            "on WhatsApp the placeholder is mostly a reaction and must be dropped"
        );
        assert!(
            !needs_text_only_reply(&placeholder(), ChannelType::WhatsApp, true),
            "a WhatsApp reaction must not be answered that the agent reads text only"
        );
    }

    #[test]
    fn on_telegram_the_placeholder_is_a_message_to_answer() {
        assert!(
            !is_ignorable_placeholder(&placeholder(), ChannelType::Telegram),
            "a Telegram reaction never arrives as a message, so the placeholder is not dropped"
        );
        assert!(
            needs_text_only_reply(&placeholder(), ChannelType::Telegram, true),
            "on Telegram the placeholder is a voice note, video or sticker and is owed the reply"
        );
    }

    #[test]
    fn ordinary_text_and_a_location_are_neither() {
        let text = MessageContent::Text {
            body: "hello".to_owned(),
        };
        assert!(
            !is_ignorable_placeholder(&text, ChannelType::WhatsApp),
            "ordinary WhatsApp text must never be dropped as a placeholder"
        );
        assert!(
            !needs_text_only_reply(&text, ChannelType::WhatsApp, true),
            "ordinary text is answered by the agent, not by the text-only reply"
        );
        let pin = MessageContent::Location {
            latitude: 1.0,
            longitude: 2.0,
        };
        assert!(
            !needs_text_only_reply(&pin, ChannelType::Telegram, true),
            "a shared location pin is stored silently, not told the agent reads text only"
        );
    }

    #[test]
    fn an_unaddressed_room_message_is_never_owed_the_text_only_reply() {
        for channel in [ChannelType::WhatsApp, ChannelType::Telegram] {
            assert!(
                !needs_text_only_reply(&media(None), channel, false),
                "a photo posted into a {channel:?} room for its people must draw no reply"
            );
            assert!(
                !needs_text_only_reply(&placeholder(), channel, false),
                "an unaddressed {channel:?} placeholder must draw no reply"
            );
        }
    }

    #[test]
    fn a_messenger_image_draws_no_reply_because_a_like_arrives_as_one() {
        assert!(
            !needs_text_only_reply(&media(None), ChannelType::Messenger, true),
            "bug: a Messenger thumbs-up arrives as an uncaptioned image, so answering it would answer every like"
        );
        assert_eq!(
            content_body_text(&media(Some("Ma sortie de dimanche"))).as_deref(),
            Some("Ma sortie de dimanche"),
            "bug: a captioned Messenger image must still reach the agent through its caption"
        );
    }
}
