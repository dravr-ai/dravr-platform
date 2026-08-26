// ABOUTME: Resolves a messaging channel's SurfaceProfile from the canot descriptor and renderer
// ABOUTME: The one place transport capabilities cross from canot into the chat pipeline

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Messaging surface resolution.
//!
//! The chat pipeline decides what to render from
//! [`pierre_chat_pipeline::RenderCapabilities`], never from a channel name.
//! This module is where those capabilities are read off the real transport:
//! canot's [`ChannelDescriptor::max_message_length`] for the character
//! ceiling, and the channel renderer's `supports_media` / `supports_cards`
//! for whether a chart arrives as pixels and whether an action arrives as a
//! button.
//!
//! It lives in `pierre-server` rather than in the pipeline crate because
//! canot's channel adapters are feature-gated per channel and only the
//! composition root compiles them all in. Reading them here means
//! `max_message_length` finally has a production consumer: until this
//! module existed the descriptor's number was asserted by tests and read by
//! nothing, while `/plan` hard-coded the cross-channel floor instead.

use pierre_chat_pipeline::{MessagingTransportCaps, SurfaceId, SurfaceProfile, SurfaceRequest};
use pierre_core::models::messaging::ChannelType;
use pierre_messaging::channels::discord::renderer::DiscordRenderer;
use pierre_messaging::channels::discord::DiscordDescriptor;
use pierre_messaging::channels::messenger::renderer::MessengerRenderer;
use pierre_messaging::channels::messenger::MessengerDescriptor;
use pierre_messaging::channels::slack::renderer::SlackRenderer;
use pierre_messaging::channels::slack::SlackDescriptor;
use pierre_messaging::channels::telegram::renderer::TelegramRenderer;
use pierre_messaging::channels::telegram::TelegramDescriptor;
use pierre_messaging::channels::whatsapp::renderer::WhatsAppRenderer;
use pierre_messaging::channels::whatsapp::WhatsAppDescriptor;
use pierre_messaging::descriptor::ChannelDescriptor;
use pierre_messaging::ResponseRenderer;

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
/// Asking canot rather than keeping a table here means a channel that gains
/// media support, card support, or a longer message ceiling upstream reaches
/// athletes on the next dependency bump with no change in this repository.
#[must_use]
pub fn transport_caps(channel_type: ChannelType) -> MessagingTransportCaps {
    match channel_type {
        ChannelType::Telegram => MessagingTransportCaps {
            max_message_length: TelegramDescriptor.max_message_length(),
            renders_media_natively: TelegramRenderer.supports_media(),
            renders_cards_natively: TelegramRenderer.supports_cards(),
        },
        ChannelType::WhatsApp => MessagingTransportCaps {
            max_message_length: WhatsAppDescriptor.max_message_length(),
            renders_media_natively: WhatsAppRenderer.supports_media(),
            renders_cards_natively: WhatsAppRenderer.supports_cards(),
        },
        ChannelType::Discord => MessagingTransportCaps {
            max_message_length: DiscordDescriptor.max_message_length(),
            renders_media_natively: DiscordRenderer.supports_media(),
            renders_cards_natively: DiscordRenderer.supports_cards(),
        },
        ChannelType::Slack => MessagingTransportCaps {
            max_message_length: SlackDescriptor.max_message_length(),
            renders_media_natively: SlackRenderer.supports_media(),
            renders_cards_natively: SlackRenderer.supports_cards(),
        },
        ChannelType::Messenger => MessagingTransportCaps {
            max_message_length: MessengerDescriptor.max_message_length(),
            renders_media_natively: MessengerRenderer.supports_media(),
            renders_cards_natively: MessengerRenderer.supports_cards(),
        },
    }
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
        transport: Some(transport_caps(channel_type)),
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
