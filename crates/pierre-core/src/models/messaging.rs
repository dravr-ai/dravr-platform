// ABOUTME: Re-exports messaging types from dravr-canot standalone crate
// ABOUTME: Channel types, message content variants, delivery tracking, retry queue entries, channel names
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

use dravr_canot::channels::descriptor_for;
use dravr_canot::descriptor::ChannelDescriptor;

// All messaging models are canonical in dravr-canot
pub use dravr_canot::models::*;
/// Inline formatting: the dialect parsers and per-channel renderers, plus the
/// markdown reader and the plain-text renderer a surface with no formatting
/// of its own needs. Re-exported so a caller outside the messaging feature —
/// a chat list row, say — reads the same parser the channels do rather than
/// hand-rolling a second one.
pub use dravr_canot::rich_text;

/// What the platform calls the chat an athlete came from when its slug names
/// no channel this build compiled: an in-app surface (`web_chat`,
/// `mobile_chat`), or an empty or unknown query parameter.
const UNNAMED_CHANNEL_LABEL: &str = "your chat app";

/// The name an athlete knows a messaging channel by ("Telegram", "Slack").
///
/// `None` when the slug names no channel this build compiled: an in-app
/// surface (`web_chat`, `mobile_chat`), or an empty or unknown query
/// parameter.
///
/// Read from the channel's own canot descriptor, so no platform crate spells a
/// channel's name itself. A page rendered in the athlete's locale words the
/// unnamed case from its own catalogue; [`channel_label`] words it in English.
#[must_use]
pub fn channel_display_name(slug: &str) -> Option<&'static str> {
    ChannelType::from_str(slug)
        .ok()
        .and_then(descriptor_for)
        .map(ChannelDescriptor::display_name)
}

/// The name an athlete knows a messaging channel by ("Telegram", "Slack"),
/// for an email that names the chat they came from.
///
/// A slug that names no channel, or one whose adapter this build did not
/// compile, reads as "your chat app".
#[must_use]
pub fn channel_label(slug: &str) -> &'static str {
    channel_display_name(slug).unwrap_or(UNNAMED_CHANNEL_LABEL)
}
