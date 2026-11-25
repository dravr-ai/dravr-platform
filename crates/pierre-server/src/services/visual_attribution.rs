// ABOUTME: The attribution a reply's charts and tables are drawn with, read off the line the platform closed the reply with
// ABOUTME: Every renderer of a resolved block — the app, the press — draws it beneath the title, so an image carries it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin attribution on charts and tables (carnet#1154).
//!
//! A reply whose turn drew on Garmin device-sourced data closes with the
//! platform's attribution line (`messaging.attribution.garmin`, carnet#521):
//! the platform appends it, once, and drops any copy the model writes, so the
//! line ending a stored reply is the turn's provenance as the platform read
//! it. Garmin's brand guidelines ask every image showing that data to carry
//! the attribution too, wherever the image is forwarded, so each visual block
//! the reply carries is drawn with the same line beneath its title
//! ([`RenderBlock::attribute`]): in the app, and in the PNG the press makes
//! for a messaging channel.
//!
//! Read off the reply rather than stored beside its blocks, so a chart
//! already sitting in history is attributed on its next read, as its geometry
//! is re-resolved on every read.

use photograveur::RenderBlock;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_REPLY_GARMIN_ATTRIBUTION,
};
use pierre_core::models::SUPPORTED_LOCALES;

/// The attribution line `content` closes with, when the platform closed it
/// with one, in whichever supported locale it was written.
#[must_use]
pub fn reply_attribution(registry: &MessagingStringsRegistry, content: &str) -> Option<String> {
    let last = content
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())?
        .trim();
    SUPPORTED_LOCALES
        .iter()
        .filter_map(|locale| registry.get_exact(KEY_REPLY_GARMIN_ATTRIBUTION, locale))
        .map(|line| line.trim().to_owned())
        .find(|line| !line.is_empty() && line == last)
}

/// Draw each of `blocks` with `line` beneath its title, when there is one.
pub fn attribute_blocks(blocks: &mut [RenderBlock], line: Option<&str>) {
    if let Some(line) = line {
        for block in blocks {
            block.attribute(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use photograveur::{resolve, Locale, RenderBlock};
    use pierre_contremaitre::messaging_strings::{
        MessagingStringsRegistry, KEY_REPLY_GARMIN_ATTRIBUTION,
    };
    use serde_json::json;

    use super::{attribute_blocks, reply_attribution};

    fn chart() -> RenderBlock {
        resolve(
            &json!({
                "type": "chart", "kind": "line", "source_tool": "analyze_training_load",
                "title": "CTL",
                "x": { "label": "Date", "type": "time" },
                "series": [{ "label": "CTL", "points": [["2026-11-01", 40.0], ["2026-11-02", 41.0]] }]
            }),
            Locale::En,
        )
        .unwrap_or_else(|e| panic!("fixture resolves: {e}"))
    }

    /// A reply the platform closed with the Garmin line, in any locale,
    /// attributes its blocks; one that only mentions Garmin does not.
    #[test]
    fn the_line_closing_a_reply_attributes_its_charts() {
        let registry = MessagingStringsRegistry::new();
        let french = registry.get(KEY_REPLY_GARMIN_ATTRIBUTION, "fr");
        let reply = format!("Ta charge monte doucement.\n\n{french}\n");
        let line = reply_attribution(&registry, &reply);
        assert_eq!(line.as_deref(), Some(french.trim()));

        let mut blocks = vec![chart()];
        attribute_blocks(&mut blocks, line.as_deref());
        assert_eq!(blocks[0].attribution(), Some(french.trim()));

        assert_eq!(
            reply_attribution(&registry, "Your Garmin data shows a steady build."),
            None
        );
        let mut plain = vec![chart()];
        attribute_blocks(&mut plain, None);
        assert_eq!(plain[0].attribution(), None);
    }
}
