// ABOUTME: The Garmin attribution line in every supported locale, as the messaging catalogue words it
// ABOUTME: One line set for every reader: the chat pipeline appends and strips it, the visual renderers read it back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Garmin attribution line set (carnet#521, carnet#1154).
//!
//! dravr-colleur recognises the line whatever locale or apostrophe a copy
//! carries ([`AttributionLines`]); the catalogue that words it is the
//! platform's. The chat pipeline appends the line to a reply and strips the
//! model's copies, and the chart renderers read which line a stored reply
//! closes with, all from this one set.

use dravr_colleur::AttributionLines;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_REPLY_GARMIN_ATTRIBUTION,
};
use pierre_core::models::SUPPORTED_LOCALES;

/// The Garmin attribution line in every supported locale the catalogue words.
#[must_use]
pub fn garmin_attribution_lines(registry: &MessagingStringsRegistry) -> AttributionLines {
    AttributionLines::new(
        SUPPORTED_LOCALES
            .iter()
            .filter_map(|locale| registry.get_exact(KEY_REPLY_GARMIN_ATTRIBUTION, locale)),
    )
}
