// ABOUTME: Pins the transport half of "does this athlete watch the reply appear": which surfaces read frames
// ABOUTME: The provider half is the tool loop's own answer, so a surface flag here never claims streamed text

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_chat_pipeline::{
    MessagingTransportCaps, ProgressiveSupport, SurfaceId, SurfaceProfile, SurfaceRequest,
};

fn in_app(surface: SurfaceId) -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

fn messaging() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Telegram,
        locale: "fr".to_owned(),
        transport: Some(MessagingTransportCaps {
            max_message_length: 4096,
            renders_media_natively: true,
            renders_cards_natively: true,
        }),
        prose_contract: None,
    })
}

/// The channel is opened on the surface's capability, because the terminal
/// frame rides the same body whether or not a delta preceded it. Whether text
/// reaches it is the tool loop's own answer (`ToolLoopParams::streams_text`,
/// pinned in pierre-server's tool-stream suites), never a surface property.
#[test]
fn both_in_app_clients_declare_the_delta_channel_and_messaging_declares_none() {
    for surface in [SurfaceId::Web, SurfaceId::Mobile] {
        let progressive = in_app(surface).render.progressive;
        assert_eq!(progressive, ProgressiveSupport::DeltaChannel, "{surface:?}");
        assert!(progressive.has_delta_channel(), "{surface:?}");
    }

    // A webhook sends one message per turn; no provider makes it progressive.
    let telegram = messaging().render.progressive;
    assert_eq!(telegram, ProgressiveSupport::Complete);
    assert!(!telegram.has_delta_channel());
}
