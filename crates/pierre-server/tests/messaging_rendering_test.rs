// ABOUTME: Golden-file rendering snapshots per channel — catches format-drift regressions
// ABOUTME: Tier L3 from the messaging-eval plan, implemented as inline Rust JSON assertions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-channel outbound rendering snapshots.
//!
//! Realizes Tier L3 of the messaging-eval plan's channel-fidelity safety net:
//! a canonical `OutgoingMessage` is fed through each of the five
//! channel renderers (`SlackRenderer`, `TelegramRenderer`,
//! `DiscordRenderer`, `WhatsAppRenderer`, `MessengerRenderer`) and the
//! resulting JSON payload is compared byte-for-byte against the
//! expected platform-native shape. Any drift — a renamed field, a
//! nesting change, a silently-changed parse mode — fails the test.
//!
//! This runs in-process against the real renderers (no mock) because
//! rendering is pure: `fn render(&OutgoingMessage) -> MessagingResult<Value>`
//! with no network, DB, or auth. It does not exercise the full
//! webhook → pipeline → outbound-delivery path; that belongs in the
//! integration tests under `messaging_eval_phase_1_integration_test.rs`.
//!
//! ## What a failing test means
//!
//! - Expected JSON shape **is** the platform's documented schema. A
//!   change here is a channel-API deviation that should ship together
//!   with a schema-version bump, not silently.
//! - Telegram renders with `parse_mode: "HTML"` and HTML-escapes the
//!   body. If the escape logic changes, the Telegram test catches it.
//! - Slack builds Block Kit `section`/`mrkdwn` blocks. If a block type
//!   or `text.type` changes, the Slack test catches it.
//!
//! ## When to extend
//!
//! Add a case when:
//! - You ship a new channel — add its renderer + a `_renders_short_plain`
//!   test here.
//! - A bug report surfaces a platform-specific format mishandling
//!   (code fence stripping, mention escaping, long-reply truncation) —
//!   add a case with the minimum reproducer and the corrected expected
//!   JSON.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "client-messaging")]
mod rendering_snapshots {
    use pierre_contremaitre::messaging_strings::{
        MessagingStringsRegistry, KEY_INTAKE_PERSONA, KEY_INTAKE_YESNO_HINT,
    };
    use pierre_core::models::ConversationTurnId;
    use pierre_messaging::channels::discord::renderer::DiscordRenderer;
    use pierre_messaging::channels::messenger::renderer::MessengerRenderer;
    use pierre_messaging::channels::slack::renderer::SlackRenderer;
    use pierre_messaging::channels::telegram::renderer::TelegramRenderer;
    use pierre_messaging::channels::whatsapp::renderer::WhatsAppRenderer;
    use pierre_messaging::models::{ChannelType, MessageContent, OutgoingMessage};
    use pierre_messaging::renderer::ResponseRenderer;
    use pierre_services::messaging_broadcast::proactive_rich_text;
    use serde_json::json;

    /// Build a canonical short-plain outgoing message for channel `ct`
    /// with the given recipient id. Keeps the body constant across
    /// channels so any difference in the rendered payload reflects
    /// the renderer's behavior, not the input.
    fn short_plain(ct: ChannelType, recipient: &str) -> OutgoingMessage {
        OutgoingMessage {
            channel_type: ct,
            recipient_id: recipient.to_owned(),
            content: MessageContent::Text {
                body: "Your recent training load is on track.".to_owned(),
            },
            turn_id: ConversationTurnId::nil().into(),
            reply_to: None,
            thread_id: None,
        }
    }

    #[test]
    fn slack_renders_short_plain() {
        let msg = short_plain(ChannelType::Slack, "C_SNAP");
        let rendered = SlackRenderer.render(&msg).expect("slack render succeeds");

        let expected = json!({
            "channel": "C_SNAP",
            "blocks": [{
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": "Your recent training load is on track."
                }
            }]
        });

        assert_eq!(
            rendered, expected,
            "Slack Block Kit shape drifted; update the snapshot only after verifying the platform-API schema is still current"
        );
    }

    #[test]
    fn telegram_renders_short_plain_with_html_parse_mode() {
        let msg = short_plain(ChannelType::Telegram, "123456789");
        let rendered = TelegramRenderer
            .render(&msg)
            .expect("telegram render succeeds");

        // Body contains no HTML-special chars; encoded body equals input.
        let expected = json!({
            "chat_id": "123456789",
            "text": "Your recent training load is on track.",
            "parse_mode": "HTML"
        });

        assert_eq!(
            rendered, expected,
            "Telegram sendMessage shape drifted. `parse_mode: HTML` serves the RichText and Card arms; on this Text arm it is inert, because the body is escaped before Telegram ever parses it — which is the point, so coach prose cannot inject markup."
        );
    }

    /// The intake questions ship their own markup, so they must ride a
    /// `RichText` envelope — in a `Text` one the athlete reads the angle
    /// brackets.
    ///
    /// Shipped broken to production on 2026-08-28: the persona question
    /// rendered as a literal `<b>1</b> — Je m'entraîne pour moi` on Telegram,
    /// because `proactive_text` hardcodes `MessageContent::Text` and the Text
    /// arm escapes. The opener paragraph above it carries no tags and rendered
    /// fine, which is what made the break look cosmetic rather than structural.
    ///
    /// Asserted against the real catalogue in all five locales rather than a
    /// fixture string: the tags live in the strings, so a locale added without
    /// them — or a sixth locale added later — has to fail here.
    #[test]
    fn telegram_renders_intake_questions_as_formatting_not_visible_tags() {
        let reg = MessagingStringsRegistry::new();

        for locale in ["fr", "en", "es", "de", "pt"] {
            for key in [KEY_INTAKE_PERSONA, KEY_INTAKE_YESNO_HINT] {
                let body = reg.get(key, locale);
                assert!(
                    body.contains("<b>"),
                    "{key}/{locale} lost its markup; this test guards the envelope, so it is \
                     vacuous once the string is plain: {body}"
                );

                let msg = proactive_rich_text(ChannelType::Telegram, "123456789".to_owned(), body);
                let rendered = TelegramRenderer
                    .render(&msg)
                    .expect("telegram render succeeds");
                let text = rendered["text"]
                    .as_str()
                    .expect("telegram payload must have a text field");

                assert!(
                    !text.contains("&lt;b&gt;"),
                    "{key}/{locale} reached the wire escaped, so the athlete sees the tags: {text}"
                );
                assert!(
                    text.contains("<b>"),
                    "{key}/{locale} lost its bold on the way to the wire: {text}"
                );
            }
        }
    }

    /// The same strings in the envelope they used to ship in — proof this test
    /// pair fails on the old code rather than passing either way.
    #[test]
    fn a_text_envelope_would_still_escape_those_tags() {
        let reg = MessagingStringsRegistry::new();
        let msg = OutgoingMessage {
            channel_type: ChannelType::Telegram,
            recipient_id: "123456789".to_owned(),
            content: MessageContent::Text {
                body: reg.get(KEY_INTAKE_PERSONA, "fr"),
            },
            turn_id: ConversationTurnId::nil().into(),
            reply_to: None,
            thread_id: None,
        };
        let rendered = TelegramRenderer
            .render(&msg)
            .expect("telegram render succeeds");
        let text = rendered["text"]
            .as_str()
            .expect("telegram payload must have a text field");

        assert!(
            text.contains("&lt;b&gt;"),
            "the Text arm must keep escaping — that is what protects coach prose: {text}"
        );
    }

    #[test]
    fn telegram_escapes_html_special_characters_in_body() {
        // Explicit escape test: <, >, & must be HTML-encoded so the
        // body can't open unclosed tags or inject markup. If this
        // breaks, coach replies containing "<100 bpm" or "A & B" will
        // render mangled or fail Telegram's parse_mode=HTML validator.
        let msg = OutgoingMessage {
            channel_type: ChannelType::Telegram,
            recipient_id: "123456789".to_owned(),
            content: MessageContent::Text {
                body: "HR <100 bpm & pace > threshold".to_owned(),
            },
            turn_id: ConversationTurnId::nil().into(),
            reply_to: None,
            thread_id: None,
        };
        let rendered = TelegramRenderer
            .render(&msg)
            .expect("telegram render succeeds");

        let text = rendered["text"]
            .as_str()
            .expect("telegram payload must have a text field");
        assert!(
            !text.contains("<100") && !text.contains("> threshold"),
            "raw < and > leaked to the wire: {text}"
        );
        assert!(
            text.contains("&lt;100") && text.contains("&gt;"),
            "HTML entities missing: {text}"
        );
    }

    #[test]
    fn discord_renders_short_plain() {
        let msg = short_plain(ChannelType::Discord, "987654321");
        let rendered = DiscordRenderer
            .render(&msg)
            .expect("discord render succeeds");

        let expected = json!({
            "content": "Your recent training load is on track.",
            "channel_id": "987654321"
        });

        assert_eq!(rendered, expected, "Discord content shape drifted");
    }

    #[test]
    fn whatsapp_renders_short_plain() {
        let msg = short_plain(ChannelType::WhatsApp, "15551234567");
        let rendered = WhatsAppRenderer
            .render(&msg)
            .expect("whatsapp render succeeds");

        let expected = json!({
            "messaging_product": "whatsapp",
            "to": "15551234567",
            "type": "text",
            "text": { "body": "Your recent training load is on track." }
        });

        assert_eq!(
            rendered, expected,
            "WhatsApp Cloud API shape drifted; messaging_product is required by Meta's API"
        );
    }

    #[test]
    fn messenger_renders_short_plain() {
        let msg = short_plain(ChannelType::Messenger, "fb-user-42");
        let rendered = MessengerRenderer
            .render(&msg)
            .expect("messenger render succeeds");

        let expected = json!({
            "recipient": { "id": "fb-user-42" },
            "message": { "text": "Your recent training load is on track." }
        });

        assert_eq!(rendered, expected, "Messenger Graph API shape drifted");
    }

    /// Build a canonical `RichText` outgoing message for channel `ct`.
    /// Same body across all channels so any per-channel difference
    /// reflects only the renderer's HTML-subset translation.
    fn short_rich(ct: ChannelType, recipient: &str) -> OutgoingMessage {
        OutgoingMessage {
            channel_type: ct,
            recipient_id: recipient.to_owned(),
            content: MessageContent::RichText {
                body: "Status is <b>enabled</b>. Use <code>/privacy off</code> to opt out."
                    .to_owned(),
            },
            turn_id: ConversationTurnId::nil().into(),
            reply_to: None,
            thread_id: None,
        }
    }

    #[test]
    fn telegram_renders_richtext_as_native_html() {
        // The /privacy reply ships as RichText. Telegram is the channel
        // that originally broke (literal `<b>` showed up to the user
        // because the tags were escaped through MessageContent::Text);
        // this snapshot pins the fix so a regression would be caught.
        let msg = short_rich(ChannelType::Telegram, "123456789");
        let rendered = TelegramRenderer
            .render(&msg)
            .expect("telegram render succeeds");

        let expected = json!({
            "chat_id": "123456789",
            "text": "Status is <b>enabled</b>. Use <code>/privacy off</code> to opt out.",
            "parse_mode": "HTML"
        });
        assert_eq!(
            rendered, expected,
            "Telegram RichText must pass `<b>` and `<code>` through verbatim, not escape them"
        );
    }

    #[test]
    fn slack_renders_richtext_as_mrkdwn() {
        let msg = short_rich(ChannelType::Slack, "C_RICH");
        let rendered = SlackRenderer.render(&msg).expect("slack render succeeds");

        let expected = json!({
            "channel": "C_RICH",
            "blocks": [{
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": "Status is *enabled*. Use `/privacy off` to opt out."
                }
            }]
        });
        assert_eq!(
            rendered, expected,
            "Slack RichText must translate `<b>` -> `*` and `<code>` -> `` ` `` (mrkdwn)"
        );
    }

    #[test]
    fn whatsapp_renders_richtext_with_native_formatting() {
        let msg = short_rich(ChannelType::WhatsApp, "15551234567");
        let rendered = WhatsAppRenderer
            .render(&msg)
            .expect("whatsapp render succeeds");

        let expected = json!({
            "messaging_product": "whatsapp",
            "to": "15551234567",
            "type": "text",
            "text": { "body": "Status is *enabled*. Use `/privacy off` to opt out." }
        });
        assert_eq!(
            rendered, expected,
            "WhatsApp RichText must translate `<b>` -> `*` and `<code>` -> `` ` ``"
        );
    }

    #[test]
    fn discord_renders_richtext_as_markdown() {
        let msg = short_rich(ChannelType::Discord, "987654321");
        let rendered = DiscordRenderer
            .render(&msg)
            .expect("discord render succeeds");

        let expected = json!({
            "content": "Status is **enabled**. Use `/privacy off` to opt out.",
            "channel_id": "987654321"
        });
        assert_eq!(
            rendered, expected,
            "Discord RichText must translate `<b>` -> `**` and `<code>` -> `` ` ``"
        );
    }

    #[test]
    fn messenger_renders_richtext_as_plain_text() {
        let msg = short_rich(ChannelType::Messenger, "fb-user-42");
        let rendered = MessengerRenderer
            .render(&msg)
            .expect("messenger render succeeds");

        let expected = json!({
            "recipient": { "id": "fb-user-42" },
            "message": { "text": "Status is enabled. Use /privacy off to opt out." }
        });
        assert_eq!(
            rendered, expected,
            "Messenger RichText must strip tags — Messenger has no native rich-text format"
        );
    }
}
