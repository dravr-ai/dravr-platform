// ABOUTME: Pins the two halves of "does this athlete watch the reply appear" — transport and provider
// ABOUTME: A surface flag alone cannot answer it, and the truth table here is what keeps the claim honest

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_chat_pipeline::{
    MessagingTransportCaps, ProgressiveSupport, ProviderStreaming, SurfaceId, SurfaceProfile,
    SurfaceRequest,
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

#[test]
fn the_provider_half_reads_the_sdk_tool_calling_flag() {
    // `TurnEvent::ProseDelta` has exactly one producer in the workspace:
    // `run_headless_streaming`, inside the SDK-tool-calling (Copilot ACP)
    // branch. Every function-calling provider — Gemini, Cohere, Groq,
    // OpenRouter, the OpenAI-compatible gateway — runs the API loop, which
    // never touches a stream sink. That is the flag this reads.
    assert_eq!(
        ProviderStreaming::from_sdk_tool_calling(true),
        ProviderStreaming::TextDeltas
    );
    assert_eq!(
        ProviderStreaming::from_sdk_tool_calling(false),
        ProviderStreaming::WholeReply
    );
}

#[test]
fn partial_text_needs_a_delta_channel_and_a_delta_producing_provider() {
    // All four combinations, spelled out. Three of them are false, and the
    // one that used to be read as true regardless of provider — an in-app
    // turn on the documented Cohere/Gemini fallback chain — is the claim this
    // table retires.
    let table = [
        (
            ProgressiveSupport::DeltaChannel,
            ProviderStreaming::TextDeltas,
            true,
        ),
        (
            ProgressiveSupport::DeltaChannel,
            ProviderStreaming::WholeReply,
            false,
        ),
        (
            ProgressiveSupport::Complete,
            ProviderStreaming::TextDeltas,
            false,
        ),
        (
            ProgressiveSupport::Complete,
            ProviderStreaming::WholeReply,
            false,
        ),
    ];
    for (support, provider, expected) in table {
        assert_eq!(
            support.delivers_partial_text(provider),
            expected,
            "{support:?} × {provider:?}"
        );
    }

    // The transport half stands on its own: the channel is opened on the
    // surface's capability, because the terminal frame rides the same body
    // whether or not a delta preceded it.
    assert!(ProgressiveSupport::DeltaChannel.has_delta_channel());
    assert!(!ProgressiveSupport::Complete.has_delta_channel());
}

#[test]
fn both_in_app_clients_declare_the_delta_channel_and_messaging_declares_none() {
    let acp = ProviderStreaming::TextDeltas;
    let function_calling = ProviderStreaming::WholeReply;

    for surface in [SurfaceId::Web, SurfaceId::Mobile] {
        let progressive = in_app(surface).render.progressive;
        assert_eq!(progressive, ProgressiveSupport::DeltaChannel, "{surface:?}");
        assert!(progressive.delivers_partial_text(acp), "{surface:?}");
        assert!(
            !progressive.delivers_partial_text(function_calling),
            "{surface:?} must not claim partial text on a function-calling provider"
        );
    }

    // A webhook sends one message per turn; no provider makes it progressive.
    let telegram = messaging().render.progressive;
    assert_eq!(telegram, ProgressiveSupport::Complete);
    assert!(!telegram.delivers_partial_text(acp));
    assert!(!telegram.delivers_partial_text(function_calling));
}
