// ABOUTME: The language a sender with no account is asked in, from what their channel says about them
// ABOUTME: A WhatsApp number or BSUID country, a Telegram app language; the default only when nothing places them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
use pierre_core::models::messaging::{ChannelType, IncomingMessage};
use pierre_core::models::{locale_from_language_tag, whatsapp_sender_locale};
use serde_json::Value;

/// Metadata key under which dravr-canot puts a `WhatsApp` sender's phone number
/// (Meta's `wa_id`) whenever the webhook carried one. canot writes it from
/// 8f2bd6c on, the commit that keys senders on their business-scoped user id
/// (carnet#1668); v0.11.0 keys them on the number and sends no metadata.
const WHATSAPP_PHONE_NUMBER_KEY: &str = "wa_id";

/// The language a sender who has not linked an account reads, from what the
/// channel tells us about them in `message`; [`DEFAULT_LOCALE`] only when it
/// places them nowhere (carnet#1669).
///
/// A `WhatsApp` sender is placed by their phone number, down to the area code,
/// whether it is the sender id or rides in the metadata beside a
/// business-scoped user id, else by that id's country. A Telegram update names
/// the language of the sender's Telegram app. The messages Messenger, Slack
/// and Discord deliver carry no language of their sender.
pub(super) fn prelink_locale(message: &IncomingMessage) -> &'static str {
    let signal = match message.channel_type {
        ChannelType::WhatsApp => whatsapp_sender_locale(
            &message.sender_id,
            message
                .metadata
                .get(WHATSAPP_PHONE_NUMBER_KEY)
                .and_then(Value::as_str),
        ),
        ChannelType::Telegram => {
            telegram_sender_language(&message.raw_payload).and_then(locale_from_language_tag)
        }
        _ => None,
    };
    signal.unwrap_or(DEFAULT_LOCALE)
}

/// The IETF language tag a Telegram update gives its sender (`from.language_code`),
/// which the sender's Telegram app sets. dravr-canot hands the update over whole
/// as the message's raw payload; a post signed as a channel has no sender and
/// no tag.
fn telegram_sender_language(update: &Value) -> Option<&str> {
    ["message", "edited_message", "callback_query"]
        .into_iter()
        .find_map(|kind| update.get(kind))?
        .pointer("/from/language_code")?
        .as_str()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use dravr_canot::turn::ConversationTurnId;
    use pierre_core::models::messaging::MessageContent;
    use serde_json::json;

    use super::*;

    /// An inbound text from `sender_id` on `channel_type`, its webhook body `update`.
    fn inbound(channel_type: ChannelType, sender_id: &str, update: Value) -> IncomingMessage {
        IncomingMessage {
            channel_type,
            sender_id: sender_id.to_owned(),
            sender_name: None,
            content: MessageContent::Text {
                body: "Hello".to_owned(),
            },
            conversation_id: Some(sender_id.to_owned()),
            chat_title: None,
            channel_message_id: "m-1669".to_owned(),
            timestamp: Utc::now(),
            raw_payload: update,
            turn_id: ConversationTurnId::new(),
            is_direct_message: true,
            addressed_to_bot: true,
            metadata: Value::Null,
        }
    }

    /// A Telegram update of `kind` whose sender's app speaks `language_code`.
    fn telegram_update(kind: &str, language_code: &str) -> Value {
        let mut update = json!({ "update_id": 1669 });
        update[kind] = json!({
            "from": { "id": 4242, "first_name": "Ana", "language_code": language_code },
        });
        update
    }

    #[test]
    fn a_whatsapp_sender_is_asked_in_the_language_of_their_number() {
        for (number, expected) in [
            ("15145550123", "fr"),
            ("12125550123", "en"),
            ("34612345678", "es"),
            ("31612345678", "en"), // a country whose language the platform does not speak
            ("unknown", DEFAULT_LOCALE),
        ] {
            let message = inbound(ChannelType::WhatsApp, number, json!({}));
            assert_eq!(
                prelink_locale(&message),
                expected,
                "bug: WhatsApp sender {number} is not asked in {expected}"
            );
        }
    }

    /// The message dravr-canot builds from 8f2bd6c on (carnet#1668): the sender
    /// id is the business-scoped user id and the number, when Meta sent one,
    /// rides in the metadata. canot v0.11.0 cannot produce it from a webhook,
    /// so it is built here.
    #[test]
    fn a_whatsapp_sender_keyed_on_their_bsuid_is_still_placed() {
        for (sender_id, metadata, expected) in [
            (
                "US.13491208655302741918",
                json!({ "wa_id": "15145550123", "user_id": "US.13491208655302741918" }),
                "fr",
            ),
            (
                "CH.13491208655302741918",
                json!({ "user_id": "CH.13491208655302741918" }),
                "de",
            ),
            ("CA.ENT.11815799212886844830", Value::Null, "en"),
            // A wa_id that is not a string is no number: the BSUID's country decides.
            (
                "FR.13491208655302741918",
                json!({ "wa_id": 15_145_550_123_u64 }),
                "fr",
            ),
        ] {
            let mut message = inbound(ChannelType::WhatsApp, sender_id, json!({}));
            message.metadata = metadata;
            assert_eq!(
                prelink_locale(&message),
                expected,
                "bug: WhatsApp sender {sender_id} is not asked in {expected}"
            );
        }
    }

    #[test]
    fn a_telegram_sender_is_asked_in_the_language_of_their_app() {
        for (kind, language_code, expected) in [
            ("message", "en", "en"),
            ("message", "pt-br", "pt"),
            ("edited_message", "es", "es"),
            ("callback_query", "de", "de"),
            ("message", "nl", "en"),
        ] {
            let message = inbound(
                ChannelType::Telegram,
                "4242",
                telegram_update(kind, language_code),
            );
            assert_eq!(
                prelink_locale(&message),
                expected,
                "bug: a Telegram {kind} in {language_code} is not asked in {expected}"
            );
        }
        let unsigned_post = inbound(
            ChannelType::Telegram,
            "4242",
            json!({ "channel_post": { "sender_chat": { "id": -100 } } }),
        );
        assert_eq!(
            prelink_locale(&unsigned_post),
            DEFAULT_LOCALE,
            "bug: an update naming no sender language was given one"
        );
    }

    /// A Telegram or Slack id is digits too, but it is no phone number:
    /// reading one as a calling code would guess a language from an account
    /// counter.
    #[test]
    fn a_numeric_id_on_another_channel_is_not_read_as_a_phone_number() {
        for channel in [
            ChannelType::Telegram,
            ChannelType::Messenger,
            ChannelType::Slack,
            ChannelType::Discord,
        ] {
            let message = inbound(channel, "12125550123", json!({}));
            assert_eq!(
                prelink_locale(&message),
                DEFAULT_LOCALE,
                "bug: a {channel} id was read as a phone number"
            );
        }
    }
}
