// ABOUTME: Pins what each surface shape renders — the table the capability catalogue publishes
// ABOUTME: A block kind gaining or losing a surface changes a concrete list here, not a boolean

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What each surface shape renders.
//!
//! `RenderCapabilities::renders` is the one place a capability field becomes an
//! answer about a reply block: the envelope asks it before pushing a block and
//! the capability catalogue asks it to fill each surface's `blocks` column. A
//! block kind quietly gaining or losing a surface therefore changes a concrete
//! list here rather than flipping a boolean nobody reads.

use pierre_chat_pipeline::{
    MessagingTransportCaps, ReplyBlock, ReplyBlockKind, SurfaceId, SurfaceProfile, SurfaceRequest,
};

/// Resolve a profile the way the ingress boundary does.
fn profile(surface: SurfaceId, transport: Option<MessagingTransportCaps>) -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface,
        locale: "en".to_owned(),
        transport,
        prose_contract: None,
    })
}

/// A transport that publishes media but has no native card primitive.
///
/// The shapes here are transport shapes, not per-channel truth: the pipeline
/// never keeps a channel table, and which real channel answers which way is
/// canot's to say. The catalogue endpoint's own test pins the live numbers.
const MEDIA_ONLY: MessagingTransportCaps = MessagingTransportCaps {
    max_message_length: 4096,
    renders_media_natively: true,
    renders_cards_natively: false,
};

/// A transport that publishes both media and native cards.
const MEDIA_AND_CARDS: MessagingTransportCaps = MessagingTransportCaps {
    max_message_length: 40_000,
    renders_media_natively: true,
    renders_cards_natively: true,
};

/// Read a block-kind list as the wire tokens the catalogue publishes.
fn tokens(kinds: &[ReplyBlockKind]) -> Vec<&'static str> {
    kinds.iter().map(|kind| kind.as_str()).collect()
}

#[test]
fn in_app_renders_every_block_except_the_rasterised_chart() {
    let web = profile(SurfaceId::Web, None);
    assert_eq!(
        tokens(&web.render.renderable_blocks()),
        vec![
            "prose",
            "activity_list",
            "workout_plan",
            "scene",
            "verdicts",
            "reconnect",
            "actions",
            "notice",
        ],
        "the in-app surface draws its own charts, so it is never handed pixels"
    );
}

#[test]
fn both_in_app_surfaces_render_the_same_blocks() {
    // Web and Mobile are separate identities sharing one capability set. The
    // shared registry publishes a single `blocks` column for the chat surface
    // on the strength of this, so it is asserted rather than assumed.
    let web = profile(SurfaceId::Web, None);
    let mobile = profile(SurfaceId::Mobile, None);
    assert_eq!(
        web.render.renderable_blocks(),
        mobile.render.renderable_blocks()
    );
    assert_eq!(web.render.max_reply_chars, mobile.render.max_reply_chars);
}

#[test]
fn a_media_only_channel_gets_pixels_and_no_buttons() {
    let telegram = profile(SurfaceId::Telegram, Some(MEDIA_ONLY));
    assert_eq!(
        tokens(&telegram.render.renderable_blocks()),
        vec!["prose", "scene_image", "reconnect", "notice"],
        "no inline scene, no plan card, no activity panel, no chips, no buttons"
    );
    assert_eq!(telegram.render.max_reply_chars, 4096);
    assert!(!telegram.render.interactive);
}

#[test]
fn a_card_capable_channel_adds_buttons_and_stays_without_chips() {
    let slack = profile(SurfaceId::Slack, Some(MEDIA_AND_CARDS));
    assert_eq!(
        tokens(&slack.render.renderable_blocks()),
        vec!["prose", "scene_image", "reconnect", "actions", "notice"],
    );
    assert_eq!(slack.render.max_reply_chars, 40_000);
    assert!(slack.render.interactive);
}

#[test]
fn prose_and_notices_reach_every_surface() {
    for surface in SurfaceId::ALL {
        let transport = match surface {
            SurfaceId::Web | SurfaceId::Mobile => None,
            _ => Some(MEDIA_ONLY),
        };
        let resolved = profile(surface, transport);
        assert!(
            resolved.render.renders(ReplyBlockKind::Prose),
            "{} must render prose",
            surface.as_str()
        );
        assert!(
            resolved.render.renders(ReplyBlockKind::Notice),
            "{} must render notices",
            surface.as_str()
        );
    }
}

#[test]
fn every_block_kind_names_itself_once() {
    let mut tokens: Vec<&'static str> = ReplyBlockKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect();
    assert_eq!(tokens.len(), 9);
    tokens.sort_unstable();
    tokens.dedup();
    assert_eq!(tokens.len(), 9, "two kinds share a wire token");
}

#[test]
fn a_block_reports_its_own_kind() {
    // The coupling that makes the catalogue unskippable: a new ReplyBlock
    // variant cannot compile without a kind, and a new kind changes the
    // generated client file.
    assert_eq!(
        ReplyBlock::Prose {
            text: "hello".to_owned()
        }
        .kind(),
        ReplyBlockKind::Prose
    );
    assert_eq!(
        ReplyBlock::SceneImage {
            url: "https://example.test/chart.png".to_owned(),
            mime_type: "image/png".to_owned(),
            caption: None,
        }
        .kind(),
        ReplyBlockKind::SceneImage
    );
    assert_eq!(
        ReplyBlock::ActivityList {
            text: "1. Run".to_owned()
        }
        .kind()
        .as_str(),
        "activity_list"
    );
}
