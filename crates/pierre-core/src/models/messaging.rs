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

/// The sender ids a channel parser substituted when the platform named no
/// sender. dravr-canot up to 0.11 wrote `"unknown"` for a `WhatsApp` message
/// from a username sender with no shared phone number, and for Slack,
/// Messenger and Discord events with no author (carnet#1668).
const PLACEHOLDER_SENDER_IDS: [&str; 1] = ["unknown"];

/// Whether `sender_id` names one real sender on a channel.
///
/// A channel identity is the key an account is bound to, so a placeholder
/// shared by every author-less event must never be linked or routed: the
/// first athlete to link it would own every later message that carries it,
/// and read and write that account's coaching history. An empty or
/// whitespace-only id is refused for the same reason.
#[must_use]
pub fn is_routable_sender_id(sender_id: &str) -> bool {
    let trimmed = sender_id.trim();
    !trimmed.is_empty()
        && !PLACEHOLDER_SENDER_IDS
            .iter()
            .any(|placeholder| trimmed.eq_ignore_ascii_case(placeholder))
}

#[cfg(test)]
mod tests {
    use super::is_routable_sender_id;

    #[test]
    fn a_real_sender_id_is_routable() {
        for id in ["15149166448", "US.13491208655302741918", "U123", "42"] {
            assert!(is_routable_sender_id(id), "{id:?}");
        }
    }

    #[test]
    fn a_placeholder_or_blank_sender_id_is_not_routable() {
        for id in ["unknown", "UNKNOWN", " unknown ", "", "   "] {
            assert!(!is_routable_sender_id(id), "{id:?}");
        }
    }
}
