// ABOUTME: Content tests for card_or_rich_text — emitters read the surface's action_buttons
// ABOUTME: capability and shape Card vs RichText fallback accordingly (registre#3 wiring)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_chat_pipeline::RenderCapabilities;
use pierre_core::models::messaging::{CardAction, ChannelType, MessageContent};
use pierre_mcp_server::services::messaging_ingress::card_or_rich_text;
use pierre_mcp_server::services::messaging_ingress::surface::messaging_render_profile;

/// What `channel`'s surface can render, resolved the way production resolves
/// it — through the canot renderer, never through a name match in this file.
fn render(channel: ChannelType) -> RenderCapabilities {
    messaging_render_profile(channel, "en").render
}

fn url_action() -> Vec<CardAction> {
    vec![CardAction {
        label: "Connect Strava".to_owned(),
        action_type: "url".to_owned(),
        value: "https://dravr.ai/connect/abc123".to_owned(),
    }]
}

#[test]
fn test_card_capable_channels_keep_native_card_content() {
    for channel in [
        ChannelType::Telegram,
        ChannelType::Slack,
        ChannelType::Discord,
        ChannelType::Messenger,
    ] {
        let caps = render(channel);
        assert!(
            caps.blocks.action_buttons,
            "{channel:?} renders native controls, so the capability must be true"
        );
        let content = card_or_rich_text(
            &caps,
            "Connect a provider".to_owned(),
            "Tap the button to link your account.".to_owned(),
            url_action(),
        );
        match content {
            MessageContent::Card {
                title,
                body,
                actions,
            } => {
                assert_eq!(title, "Connect a provider");
                assert_eq!(body, "Tap the button to link your account.");
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0].label, "Connect Strava");
                assert_eq!(actions[0].value, "https://dravr.ai/connect/abc123");
            }
            other => panic!("{channel:?} should render a native Card, got {other:?}"),
        }
    }
}

#[test]
fn test_whatsapp_degrades_to_rich_text_with_tappable_link() {
    let caps = render(ChannelType::WhatsApp);
    assert!(
        !caps.blocks.action_buttons,
        "WhatsApp degrades a Card, so the capability must be false"
    );
    let content = card_or_rich_text(
        &caps,
        "Connect a provider".to_owned(),
        "Tap the link to connect.".to_owned(),
        url_action(),
    );
    match content {
        MessageContent::RichText { body } => {
            assert!(
                body.starts_with("**Connect a provider**\n\n"),
                "title must lead as bold rich text: {body}"
            );
            assert!(
                body.contains("Tap the link to connect."),
                "body kept: {body}"
            );
            assert!(
                body.contains("Connect Strava: https://dravr.ai/connect/abc123"),
                "action must be a label: url line: {body}"
            );
        }
        other => panic!("WhatsApp has no native cards; expected RichText, got {other:?}"),
    }
}

#[test]
fn test_whatsapp_empty_title_yields_no_empty_bold_marker() {
    let content = card_or_rich_text(
        &render(ChannelType::WhatsApp),
        String::new(),
        "Just the body.".to_owned(),
        url_action(),
    );
    match content {
        MessageContent::RichText { body } => {
            assert!(
                body.starts_with("Just the body."),
                "no bold marker for an empty title: {body}"
            );
            assert!(!body.contains("****"), "no empty emphasis pair: {body}");
        }
        other => panic!("expected RichText, got {other:?}"),
    }
}
