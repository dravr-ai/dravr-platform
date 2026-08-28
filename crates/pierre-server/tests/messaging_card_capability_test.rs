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
        // WhatsApp joined this set when its renderer learned reply buttons
        // and list menus; before that it was the one channel that degraded.
        ChannelType::WhatsApp,
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
fn test_whatsapp_now_declares_native_card_support() {
    // WhatsApp used to be the single channel with action_buttons = false, so
    // every card reached the athlete as a "Label: value" line with nothing to
    // tap. Its renderer now carries reply buttons and list menus, so the
    // platform stops degrading at this layer.
    let caps = render(ChannelType::WhatsApp);
    assert!(
        caps.blocks.action_buttons,
        "WhatsApp renders native controls now, so the capability must be true"
    );
    assert!(
        caps.interactive,
        "a tapped WhatsApp control reaches the platform, so interactive must be true"
    );
}

#[test]
fn test_whatsapp_url_card_keeps_its_action_for_the_renderer() {
    // A url action has no interactive equivalent on WhatsApp — reply buttons
    // cannot hold a link — but the degrading belongs to the RENDERER, which
    // knows the channel's payload rules. This layer must hand it a Card with
    // the action intact rather than pre-flattening and losing the structure.
    let content = card_or_rich_text(
        &render(ChannelType::WhatsApp),
        "Connect a provider".to_owned(),
        "Tap the link to connect.".to_owned(),
        url_action(),
    );
    match content {
        MessageContent::Card {
            title,
            body,
            actions,
        } => {
            assert_eq!(title, "Connect a provider");
            assert_eq!(body, "Tap the link to connect.");
            assert_eq!(actions.len(), 1);
            assert_eq!(actions[0].label, "Connect Strava");
            assert_eq!(actions[0].value, "https://dravr.ai/connect/abc123");
        }
        other => panic!("WhatsApp must receive a Card now, got {other:?}"),
    }
}

#[test]
fn test_a_card_incapable_surface_yields_no_empty_bold_marker() {
    // Exercised through a capability with action_buttons off rather than
    // through WhatsApp, which now renders cards natively: the fallback path
    // still exists for any surface that cannot, and an empty title must not
    // leave a dangling emphasis pair in it.
    let mut caps = render(ChannelType::WhatsApp);
    caps.blocks.action_buttons = false;
    let content = card_or_rich_text(
        &caps,
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
