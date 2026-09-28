// ABOUTME: Resolves a messaging channel's SurfaceProfile from canot's declared channel capabilities
// ABOUTME: The one place transport capabilities cross from canot into the chat pipeline

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Messaging surface resolution.
//!
//! The chat pipeline decides what to render from
//! [`pierre_chat_pipeline::RenderCapabilities`], never from a channel name.
//! This module is where those capabilities are read off the real transport:
//! canot's [`capabilities_for`] carries the descriptor's character ceiling
//! and its `supports_media` / `supports_cards`, which decide whether a chart
//! arrives as pixels and whether an action arrives as a button.
//!
//! It lives in `pierre-server` rather than in the pipeline crate because
//! canot's channel adapters are feature-gated per channel and only the
//! composition root compiles them all in. Reading them here means
//! `max_message_length` finally has a production consumer: until this
//! module existed the descriptor's number was asserted by tests and read by
//! nothing, while `/plan` hard-coded the cross-channel floor instead.

use pierre_chat_pipeline::{MessagingTransportCaps, SurfaceId, SurfaceProfile, SurfaceRequest};
use pierre_core::models::messaging::ChannelType;
use pierre_messaging::channels::capabilities_for;

/// The [`SurfaceId`] a channel type reports on every pipeline span.
#[must_use]
pub const fn surface_id(channel_type: ChannelType) -> SurfaceId {
    match channel_type {
        ChannelType::Telegram => SurfaceId::Telegram,
        ChannelType::WhatsApp => SurfaceId::WhatsApp,
        ChannelType::Discord => SurfaceId::Discord,
        ChannelType::Slack => SurfaceId::Slack,
        ChannelType::Messenger => SurfaceId::Messenger,
    }
}

/// Read what `channel_type`'s transport will actually carry.
///
/// canot's [`capabilities_for`] is a projection of its `descriptor_for`, the
/// one `ChannelType` → descriptor lookup, which canot's own `list_channels`
/// tool reads too, so the platform and canot cannot disagree about a channel. A channel that
/// gains media support, card support, or a longer message ceiling upstream
/// reaches athletes on the next dependency bump with no change here.
///
/// `None` when this build did not compile the channel's adapter, so no turn
/// can arrive on it; `client-messaging` enables every canot channel.
#[must_use]
pub fn transport_caps(channel_type: ChannelType) -> Option<MessagingTransportCaps> {
    capabilities_for(channel_type).map(|caps| MessagingTransportCaps {
        max_message_length: caps.max_message_length,
        renders_media_natively: caps.supports_media,
        renders_cards_natively: caps.supports_cards,
    })
}

/// Build the surface request for one messaging turn.
///
/// `prose_contract` carries contremaitre's `messaging_context` system prompt
/// — live configuration a contremaitre push reaches production with in about
/// a minute. Paths that render a reply without prompting a model (slash
/// commands, the connect card) pass `None`.
#[must_use]
pub fn messaging_surface_request(
    channel_type: ChannelType,
    locale: String,
    prose_contract: Option<String>,
) -> SurfaceRequest {
    SurfaceRequest {
        surface: surface_id(channel_type),
        locale,
        transport: transport_caps(channel_type),
        prose_contract,
    }
}

/// Resolve the full profile for a messaging turn that has no model call
/// behind it — a slash-command reply or a connect prompt.
#[must_use]
pub fn messaging_render_profile(channel_type: ChannelType, locale: &str) -> SurfaceProfile {
    SurfaceProfile::resolve(&messaging_surface_request(
        channel_type,
        locale.to_owned(),
        None,
    ))
}
