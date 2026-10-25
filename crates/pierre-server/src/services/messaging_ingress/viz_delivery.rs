// ABOUTME: Turns a reply's visual blocks into per-channel media messages or a prose fallback
// ABOUTME: The fidelity negotiator — this is what finally gives MessageContent::Media a producer

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-channel fidelity negotiation.
//!
//! Every channel renders a different subset of what a reply can carry, so the
//! emitter picks the shape up front rather than letting canot degrade it
//! generically — the emitter knows what the content *means* and can do better.
//! Cards and charts both negotiate here, beside each other, because they are
//! the same decision asked twice.
//!
//! The app renders a Scene inline. A messaging channel cannot, but most of them
//! accept a media URL and fetch it server-side at send time. This module decides
//! which of those a given reply gets, and mints the URLs.
//!
//! # The fallback is prose, not a placeholder
//!
//! An agent writes the interpretation in the sentences around a chart — that is
//! the contract the visual-blocks prompt sets. So a channel that cannot show an
//! image loses the picture and keeps the point, which is why the degraded path
//! simply strips the markers and sends the text. Nothing is substituted in, and
//! nothing says "chart unavailable"; the reply reads as ordinary coaching.

use std::fmt::Write as _;

use dravr_canot::rich_text::{escape_markdown, parse_markdown, render_rich_text};
use pierre_chat_pipeline::stages::viz_blocks::strip_markers;
use pierre_chat_pipeline::suggestions::is_postback;
use pierre_chat_pipeline::RenderCapabilities;
use pierre_core::models::messaging::{CardAction, MessageContent};
use pierre_core::models::{ColorScheme, TenantId, ACTIONS_BLOCK_TYPE};
use serde_json::Value;
use tracing::debug;

use crate::routes::viz::{VizTarget, VizToken};

/// What the egress needs to turn one reply's blocks into media.
#[derive(Debug)]
pub struct VizDelivery<'a> {
    /// Conversation, user, tenant and message ids the tokens point at.
    pub target: VizTarget,
    /// The blocks as stored on the message — specs, not scenes.
    pub stored_blocks: Option<&'a str>,
    /// What the surface the reply is going to can render. `scene_raster`
    /// decides whether this reply gets pixels or the prose fallback.
    pub render: &'a RenderCapabilities,
    /// Locale the axis labels must resolve in.
    pub locale: &'a str,
    /// Colour scheme the press service paints the chart in, resolved from the
    /// athlete's `users.theme` pin. The channel's servers fetch the PNG, not
    /// the athlete's device, so the pin is the only thing that can speak for
    /// the athlete here — and an athlete who pinned nothing gets dark, which
    /// is what messaging clients draw media bubbles on.
    pub theme: ColorScheme,
    /// Public base URL the channel's fetcher will resolve.
    pub base_url: &'a str,
    /// Whether a press service is configured at all.
    pub press_enabled: bool,
}

/// One chart, ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VizMedia {
    /// Signed, short-TTL URL the channel fetches.
    pub url: String,
    /// Always `image/png`; the press service emits nothing else.
    pub mime_type: String,
    /// The block's own title, when it carried one.
    pub caption: Option<String>,
}

/// Decide what the channel gets, and mint the URLs if it gets pixels.
///
/// Returns an empty vec when the channel cannot render media, no press service
/// is configured, or the reply carried no blocks — all three degrade to the
/// prose the caller already has.
#[must_use]
pub fn plan_media(delivery: &VizDelivery<'_>, secret: &str) -> Vec<VizMedia> {
    if !delivery.press_enabled {
        debug!("viz media: no press service configured; sending prose only");
        return Vec::new();
    }
    if !delivery.render.blocks.scene_raster {
        debug!("viz media: surface cannot show a rasterised chart; sending prose only");
        return Vec::new();
    }
    let Some(raw) = delivery.stored_blocks else {
        return Vec::new();
    };
    let specs: Vec<Value> = serde_json::from_str(raw).unwrap_or_default();

    // A token addresses its block by its index in the stored array, so the
    // reply's stored controls — the one actions entry, which is no chart —
    // are skipped after the indexes are taken.
    specs
        .iter()
        .enumerate()
        .filter(|(_, spec)| spec.get("type").and_then(Value::as_str) != Some(ACTIONS_BLOCK_TYPE))
        .map(|(index, spec)| {
            let token = VizToken::for_block(
                delivery.target.clone(),
                index,
                delivery.theme.as_str(),
                delivery.locale,
            )
            .mint(secret);
            VizMedia {
                url: format!(
                    "{}/api/viz/{token}.png",
                    delivery.base_url.trim_end_matches('/')
                ),
                mime_type: "image/png".to_owned(),
                caption: spec.get("title").and_then(Value::as_str).map(str::to_owned),
            }
        })
        .collect()
}

/// Remove the positional markers from prose.
///
/// Every channel path calls this, whether or not it sends media: `⟦viz:0⟧` in
/// a Telegram message is noise at best and looks like a bug at worst. The
/// marker's job ends once the client that can interleave has interleaved.
/// The strip itself lives beside the marker's definition
/// ([`strip_markers`]); this adds the blank-run collapse a message body wants.
#[must_use]
pub fn strip_viz_markers(text: &str) -> String {
    // Markers usually sit alone between blank lines, so removing one leaves a
    // triple newline behind.
    collapse_blank_runs(&strip_markers(text))
}

/// Collapse three or more consecutive newlines into two.
fn collapse_blank_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(ch);
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    out.trim().to_owned()
}

/// Build the target a token points at.
#[must_use]
pub fn target(
    conversation_id: String,
    user_id: String,
    tenant_id: TenantId,
    message_id: String,
) -> VizTarget {
    VizTarget {
        conversation_id,
        user_id,
        tenant_id,
        message_id,
    }
}

/// Shape a card-intent reply for a surface.
///
/// `body` is inline markdown, as every command reply and catalogue row is
/// authored; it leaves here in the rich-text dialect canot's renderers
/// translate. Returns a native [`MessageContent::Card`] where the surface lays
/// actions out as controls ([`pierre_chat_pipeline::BlockSupport::action_buttons`]),
/// otherwise a [`MessageContent::RichText`] fallback: bold title (when
/// non-empty), the body, then one `label: value` line per action — a bare URL
/// value stays tappable as autolinked text where buttons do not render, and a
/// suggestion's opaque postback is left out, its label standing alone.
///
/// Card-emitting paths consult the capability to pick the content shape up
/// front: canot's renderers do degrade an unsupported Card, but only
/// generically — the emitter can do better because it knows what the card
/// means.
#[must_use]
pub fn card_or_rich_text(
    render: &RenderCapabilities,
    title: String,
    body: String,
    actions: Vec<CardAction>,
) -> MessageContent {
    if render.blocks.action_buttons {
        return MessageContent::Card {
            title,
            body: render_rich_text(&parse_markdown(&body)),
            actions,
        };
    }
    let mut text = if title.is_empty() {
        body
    } else {
        format!("**{}**\n\n{body}", escape_markdown(&title))
    };
    for action in &actions {
        // Writing to a String is infallible; the discarded Result is fmt noise.
        let _ = if is_postback(&action.value) {
            write!(text, "\n\n{}", escape_markdown(&action.label))
        } else {
            write!(
                text,
                "\n\n{}: {}",
                escape_markdown(&action.label),
                escape_markdown(&action.value)
            )
        };
    }
    MessageContent::RichText {
        body: render_rich_text(&parse_markdown(&text)),
    }
}

#[cfg(test)]
mod tests {
    use pierre_chat_pipeline::{SurfaceId, SurfaceProfile, SurfaceRequest};
    use uuid::Uuid;

    use super::*;

    const SECRET: &str = "viz-delivery-test-secret";

    /// A surface that fetches rasterised charts.
    fn rasterising() -> RenderCapabilities {
        let mut render = SurfaceProfile::resolve(&SurfaceRequest {
            surface: SurfaceId::Web,
            locale: "en".to_owned(),
            transport: None,
            prose_contract: None,
        })
        .render;
        render.blocks.scene_raster = true;
        render
    }

    /// A reply's stored next steps sit in the same array as its charts; they
    /// are no chart, and the chart after them keeps the index its token names
    /// (carnet#830).
    #[test]
    fn a_stored_actions_entry_mints_no_chart_and_shifts_no_index() {
        let stored = r#"[
            {"type":"line","title":"Weekly load"},
            {"type":"actions","title":null,"actions":[{"label":"My next 2 weeks","action_type":"postback","value":"nx:0:fortnight"}]},
            {"type":"bar","title":"Sleep"}
        ]"#;
        let render = rasterising();
        let media = plan_media(
            &VizDelivery {
                target: target(
                    "conv-1".to_owned(),
                    Uuid::new_v4().to_string(),
                    TenantId::from_uuid(Uuid::new_v4()),
                    "msg-1".to_owned(),
                ),
                stored_blocks: Some(stored),
                render: &render,
                locale: "en",
                theme: ColorScheme::resolve(None),
                base_url: "https://dravr.test",
                press_enabled: true,
            },
            SECRET,
        );
        let minted: Vec<(usize, Option<&str>)> = media
            .iter()
            .map(|media| {
                let raw = media
                    .url
                    .strip_prefix("https://dravr.test/api/viz/")
                    .and_then(|rest| rest.strip_suffix(".png"))
                    .expect("a signed viz URL");
                let token = VizToken::verify(raw, SECRET).expect("the token verifies");
                (token.block_index, media.caption.as_deref())
            })
            .collect();
        assert_eq!(minted, [(0, Some("Weekly load")), (2, Some("Sleep"))]);
    }
}
