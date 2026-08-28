// ABOUTME: Pins the resolved capabilities of every chat surface against the real canot descriptors
// ABOUTME: The proof that decisions read capabilities, not a channel name — and that no number drifted

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_chat_pipeline::{
    ModelPolicy, ProgressiveSupport, ProseFormat, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnBudget,
};
use pierre_core::models::messaging::ChannelType;
use pierre_mcp_server::services::messaging_ingress::surface::{
    messaging_surface_request, transport_caps,
};
use pierre_messaging::channels::discord::DiscordDescriptor;
use pierre_messaging::channels::messenger::MessengerDescriptor;
use pierre_messaging::channels::slack::SlackDescriptor;
use pierre_messaging::channels::telegram::TelegramDescriptor;
use pierre_messaging::channels::whatsapp::WhatsAppDescriptor;
use pierre_messaging::descriptor::ChannelDescriptor;

fn messaging(channel_type: ChannelType) -> SurfaceProfile {
    SurfaceProfile::resolve(&messaging_surface_request(
        channel_type,
        "fr".to_owned(),
        None,
    ))
}

fn in_app(surface: SurfaceId) -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

#[test]
fn telegram_resolves_its_real_transport_ceiling_and_block_set() {
    let profile = messaging(ChannelType::Telegram);

    assert_eq!(profile.surface, SurfaceId::Telegram);
    assert_eq!(profile.surface.as_str(), "telegram");
    assert_eq!(profile.render.max_reply_chars, 4096);
    assert_eq!(profile.render.prose, ProseFormat::PlainText);
    assert!(!profile.render.blocks.workout_plan_card);
    assert!(!profile.render.blocks.activity_list_card);
    assert!(!profile.render.blocks.scene_inline);
    assert!(profile.render.blocks.scene_raster);
    assert!(profile.render.blocks.action_buttons);
    assert_eq!(profile.render.progressive, ProgressiveSupport::Complete);
    assert!(!profile.render.progressive.has_delta_channel());
    assert_eq!(profile.budget, TurnBudget::Fixed(5));
    assert_eq!(profile.model_policy, ModelPolicy::OverrideWithEnv);
    assert_eq!(profile.locale, "fr");
}

#[test]
fn discord_and_messenger_resolve_the_two_thousand_character_floor() {
    assert_eq!(messaging(ChannelType::Discord).render.max_reply_chars, 2000);
    assert_eq!(
        messaging(ChannelType::Messenger).render.max_reply_chars,
        2000
    );
}

#[test]
fn slack_resolves_its_forty_thousand_character_headroom() {
    let profile = messaging(ChannelType::Slack);
    assert_eq!(profile.render.max_reply_chars, 40_000);
    assert_eq!(profile.surface.as_str(), "slack");
    assert_eq!(profile.surface.call_type(), "messaging");
}

#[test]
fn whatsapp_has_media_and_native_cards() {
    let profile = messaging(ChannelType::WhatsApp);
    assert_eq!(profile.render.max_reply_chars, 4096);
    assert!(profile.render.blocks.scene_raster);
    // WhatsApp was the one channel that degraded a Card to text. Its renderer
    // now carries reply buttons and list menus, and this profile reads that
    // straight off the renderer rather than from a name match, so the
    // capability follows the dependency bump with no edit here.
    assert!(
        profile.render.blocks.action_buttons,
        "WhatsApp lays out native controls now"
    );
    assert!(
        profile.render.interactive,
        "a tapped WhatsApp control reaches the platform"
    );
}

#[test]
fn in_app_surface_renders_markdown_cards_and_inline_scenes() {
    let profile = in_app(SurfaceId::Web);

    assert_eq!(profile.surface, SurfaceId::Web);
    assert_eq!(profile.surface.as_str(), "web_chat");
    assert_eq!(profile.surface.call_type(), "chat");
    assert_eq!(profile.render.prose, ProseFormat::Markdown);
    assert!(profile.render.blocks.workout_plan_card);
    assert!(profile.render.blocks.activity_list_card);
    assert!(profile.render.blocks.scene_inline);
    assert!(!profile.render.blocks.scene_raster);
    assert!(profile.render.blocks.action_buttons);
    assert_eq!(profile.render.progressive, ProgressiveSupport::DeltaChannel);
    assert_eq!(profile.render.max_reply_chars, usize::MAX);
    assert_eq!(profile.budget, TurnBudget::CoachOrAdminDefault);
    assert_eq!(profile.model_policy, ModelPolicy::UseStored);
    assert_eq!(profile.prose_contract, None);
}

#[test]
fn every_messaging_ceiling_comes_from_the_canot_descriptor() {
    // The point of the whole resolution path: the number the egress enforces
    // is the number canot publishes, not a table in this repository. A canot
    // bump that changes a ceiling changes the profile with no edit here.
    let pairs: [(ChannelType, usize); 5] = [
        (
            ChannelType::Telegram,
            TelegramDescriptor.max_message_length(),
        ),
        (
            ChannelType::WhatsApp,
            WhatsAppDescriptor.max_message_length(),
        ),
        (ChannelType::Discord, DiscordDescriptor.max_message_length()),
        (ChannelType::Slack, SlackDescriptor.max_message_length()),
        (
            ChannelType::Messenger,
            MessengerDescriptor.max_message_length(),
        ),
    ];
    for (channel_type, descriptor_limit) in pairs {
        assert_eq!(
            messaging(channel_type).render.max_reply_chars,
            descriptor_limit,
            "{channel_type:?} must resolve the descriptor's own ceiling"
        );
        assert_eq!(
            transport_caps(channel_type).max_message_length,
            descriptor_limit
        );
    }
}

#[test]
fn the_prose_contract_round_trips_the_live_contremaitre_string() {
    // The contract is contremaitre configuration — a push reaches production
    // in about a minute, while a compiled-in constant needs a full deploy. The
    // resolver must carry every word of it through; deriving the contract in
    // code would downgrade the most-tuned knob on the primary delivery
    // surface.
    let contract = "Réponds en texte brut, sans markdown. Reste concis.";
    let profile = SurfaceProfile::resolve(&messaging_surface_request(
        ChannelType::Telegram,
        "fr".to_owned(),
        Some(contract.to_owned()),
    ));

    let resolved = profile
        .prose_contract
        .expect("messaging carries a contract");
    assert!(
        resolved.starts_with(contract),
        "the live contract must lead, unedited: {resolved}"
    );
}

#[test]
fn the_hard_ceiling_the_model_is_told_is_the_one_the_egress_enforces() {
    // The one number derived in code. It must equal max_reply_chars — the
    // same field the guardrails stage trims against — or the coach writes to
    // a budget nobody enforces and the athlete reads a cut-off reply.
    for channel_type in [
        ChannelType::Telegram,
        ChannelType::WhatsApp,
        ChannelType::Discord,
        ChannelType::Slack,
        ChannelType::Messenger,
    ] {
        let profile = SurfaceProfile::resolve(&messaging_surface_request(
            channel_type,
            "en".to_owned(),
            Some("Be brief.".to_owned()),
        ));
        let enforced = profile.render.max_reply_chars;
        let resolved = profile
            .prose_contract
            .expect("messaging carries a contract");
        assert!(
            resolved.contains(&format!("at most {enforced} characters")),
            "{channel_type:?} must be told its own enforced ceiling, got: {resolved}"
        );
    }

    // Discord's 2000 and Slack's 40000 are an order of magnitude apart: a
    // single sentence shared by both would be wrong for one of them.
    let discord = SurfaceProfile::resolve(&messaging_surface_request(
        ChannelType::Discord,
        "en".to_owned(),
        Some("Be brief.".to_owned()),
    ));
    assert!(discord
        .prose_contract
        .is_some_and(|c| c.contains("at most 2000 characters")));
}

#[test]
fn the_in_app_surface_is_told_no_ceiling_because_none_is_enforced() {
    // usize::MAX is the absence of a transport ceiling, not a ceiling of
    // 18 quintillion — printing it would be nonsense in a prompt.
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: Some("Write markdown.".to_owned()),
    });
    assert_eq!(profile.prose_contract.as_deref(), Some("Write markdown."));
}

#[test]
fn web_and_mobile_are_separate_identities_over_one_capability_set() {
    // The regression this pins: `SurfaceId::Web` used to mean "web browser OR
    // mobile app". One identity for two clients meant a per-client capability
    // had nowhere to live and a per-surface coverage catalogue could not name
    // the mobile row at all. Splitting the identity while sharing the
    // capability set is the whole point — so assert BOTH halves.
    let web = in_app(SurfaceId::Web);
    let mobile = in_app(SurfaceId::Mobile);

    // Half one: distinct identities, each with its own telemetry label.
    assert_ne!(web.surface, mobile.surface);
    assert_eq!(web.surface.as_str(), "web_chat");
    assert_eq!(mobile.surface.as_str(), "mobile_chat");
    // Both are in-app turns, so both bill as `chat`, not `messaging`.
    assert_eq!(web.surface.call_type(), "chat");
    assert_eq!(mobile.surface.call_type(), "chat");

    // Half two: proven-identical capabilities, field by field. `RenderCapabilities`
    // is `PartialEq`, so a capability added to one client and not the other
    // reds this line rather than hiding behind a shared variant.
    assert_eq!(web.render, mobile.render);
    assert_eq!(web.budget, mobile.budget);
    assert_eq!(web.model_policy, mobile.model_policy);
    assert_eq!(mobile.render.prose, ProseFormat::Markdown);
    assert!(mobile.render.blocks.scene_inline);
    assert!(mobile.render.blocks.workout_plan_card);
    assert!(mobile.render.blocks.activity_list_card);
    assert_eq!(mobile.render.max_reply_chars, usize::MAX);
}
