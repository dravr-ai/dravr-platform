// ABOUTME: Extracts inline ```dravr-viz fences from a coach reply into ordered, validated blocks
// ABOUTME: Prose keeps a positional marker where each block sat, so clients interleave the two
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Inline visual blocks.
//!
//! The workout-plan path in [`super::structured_output`] is *whole-reply
//! replacement*: the coach emits one JSON object and nothing else. Visual
//! blocks are the other content model — embedded in prose, several per reply —
//! so they need their own extraction rather than a second schema on that path.
//!
//! A block is a fenced code span with the `dravr-viz` info string. The outer
//! fence here is four backticks so the three-backtick fences it contains stay
//! content — with a matching outer fence the inner one closes it early and
//! rustdoc compiles the remainder as a Rust doctest.
//!
//! ````text
//! Ta charge grimpe depuis trois semaines.
//!
//! ```dravr-viz
//! {"type":"chart", ...}
//! ```
//!
//! C'est pourquoi on coupe jeudi.
//! ````
//!
//! Markdown-native on purpose: a block that fails extraction degrades to a
//! visible code fence rather than vanishing. Ugly is recoverable; silent loss
//! is not.

use std::borrow::Cow;

use serde_json::Value;
use tracing::warn;

use super::structured_output::{validator_for, SchemaTexts, DRAVR_VIZ};

/// Info string that marks a fenced block as a Dravr visual.
const FENCE_INFO: &str = "dravr-viz";

/// Placeholder left in the prose where a block was lifted out.
///
/// Clients split on it to interleave prose and rendered blocks. The brackets
/// are U+27E6/U+27E7 rather than ASCII so ordinary coach prose — including
/// markdown, LaTeX-ish notation and code — cannot collide with it.
const MARKER_OPEN: &str = "⟦viz:";
/// Closing half of [`MARKER_OPEN`].
const MARKER_CLOSE: &str = "⟧";

/// Render the marker a client looks for at block index `index`.
#[must_use]
pub fn marker(index: usize) -> String {
    format!("{MARKER_OPEN}{index}{MARKER_CLOSE}")
}

/// The result of lifting visual blocks out of a reply.
pub struct VizExtraction {
    /// Reply text with each block replaced by its positional marker.
    pub text: String,
    /// Validated blocks in the order they appeared.
    pub blocks: Vec<Value>,
}

/// Lift every valid `dravr-viz` block out of `reply`.
///
/// Returns `None` when the reply contains no such fence, so the caller can keep
/// the reply untouched rather than round-tripping it through this stage.
///
/// A fence that is malformed, unparseable, or fails schema validation is
/// **removed** from the reply and logged at WARN with its reason.
///
/// It used to be left in place as literal text, on the reasoning that a visible
/// broken fence is a bug someone reports rather than one that hides. In
/// practice it hid better that way: the athlete got a screenful of raw JSON —
/// which reads as a broken product, not a bug report — and the same text was
/// persisted as the assistant message, so on every later turn the coach read
/// its own transcript, saw a chart spec it had "emitted", and refused to draw
/// again ("le graphique est déjà juste au-dessus", Telegram 2026-08-21). One
/// refusal poisoned the whole conversation.
///
/// The reason it was kept visible is still honoured — the WARN names the fence
/// and why it failed — but the athlete sees the prose, which the visual
/// contract already requires to carry the interpretation on its own.
#[must_use]
pub fn extract_viz_blocks(
    schemas: &SchemaTexts,
    granted: &[String],
    tools_called: &[String],
    reply: &str,
) -> Option<VizExtraction> {
    if !reply.contains(FENCE_INFO) {
        return None;
    }

    let mut text = String::with_capacity(reply.len());
    let mut blocks: Vec<Value> = Vec::new();
    let mut dropped = 0_usize;
    let mut rest = reply;

    while let Some(fence) = next_fence(rest) {
        text.push_str(&rest[..fence.start]);
        match parse_block(schemas, granted, tools_called, fence.body) {
            Some(block) => {
                text.push_str(&marker(blocks.len()));
                blocks.push(block);
            }
            // Refused: drop the fence entirely. `parse_block` has already
            // logged which rule it broke, so the failure stays legible to us
            // without being spelled out to the athlete in JSON.
            None => dropped += 1,
        }
        rest = &rest[fence.end..];
    }

    if blocks.is_empty() && dropped == 0 {
        return None;
    }
    text.push_str(rest);

    if dropped > 0 {
        warn!(
            dropped,
            kept = blocks.len(),
            "viz-blocks: refused block(s) removed from the reply; the prose stands alone"
        );
    }

    Some(VizExtraction {
        text: text.trim().to_owned(),
        blocks,
    })
}

/// Remove every ```` ```dravr-viz ```` fence from replayed transcript text.
///
/// Nothing written today persists a raw fence — a lifted block leaves a marker
/// and a refused one is stripped — so any fence still sitting in stored history
/// predates that and is pure poison. Left in place it is read back to the coach
/// as its own prior work: on 2026-08-21 a coach read one and answered "le
/// graphique est déjà juste au-dessus", refusing to draw a chart the athlete
/// had never actually been shown.
///
/// Shares [`next_fence`] with the extractor on purpose. A second opinion about
/// what a fence looks like is how the two drift apart.
#[must_use]
pub fn strip_fences(text: &str) -> Cow<'_, str> {
    if !text.contains(FENCE_INFO) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut found = false;
    while let Some(fence) = next_fence(rest) {
        found = true;
        out.push_str(&rest[..fence.start]);
        rest = &rest[fence.end..];
    }
    if !found {
        return Cow::Borrowed(text);
    }
    out.push_str(rest);
    Cow::Owned(out.trim().to_owned())
}

/// A located fence: byte range in the haystack plus the body between delimiters.
struct Fence<'a> {
    start: usize,
    end: usize,
    body: &'a str,
}

/// Find the next ```` ```dravr-viz ```` fence, if any.
///
/// Matches only when the info string stands alone on the opening line, so a
/// prose mention of the word cannot open a block.
fn next_fence(haystack: &str) -> Option<Fence<'_>> {
    let mut search = 0usize;
    loop {
        let open = haystack[search..].find("```")? + search;
        let after_ticks = open + 3;
        let line_end = haystack[after_ticks..]
            .find('\n')
            .map_or(haystack.len(), |offset| after_ticks + offset);
        let info = haystack[after_ticks..line_end].trim();

        let Some(close_rel) = haystack[line_end..].find("```") else {
            // Unterminated fence — nothing more to find in this reply.
            return None;
        };
        let close = line_end + close_rel;
        let end = (close + 3).min(haystack.len());

        if info == FENCE_INFO {
            return Some(Fence {
                start: open,
                end,
                body: &haystack[line_end..close],
            });
        }
        // A different fenced language (```json, ```text). Skip past it whole so
        // a `dravr-viz` mention inside it cannot be misread as an opener.
        search = end;
    }
}

/// Parse, schema-validate, and attribution-check one block body.
fn parse_block(
    schemas: &SchemaTexts,
    granted: &[String],
    tools_called: &[String],
    body: &str,
) -> Option<Value> {
    let block: Value = match serde_json::from_str(body.trim()) {
        Ok(value) => value,
        Err(e) => {
            warn!(error = %e, "viz-blocks: fence body is not valid JSON; leaving it in the reply");
            return None;
        }
    };

    if !schema_valid(schemas, &block) {
        return None;
    }

    if !kind_granted(&block, granted) {
        return None;
    }

    // Attribution. The schema can require `source_tool` to be present; only
    // the pipeline knows whether that tool actually ran this turn. Without this
    // check the field is decoration, and a chart of invented numbers would
    // carry a citation that makes it look measured — strictly worse than the
    // same invention in a sentence, because the rendering lends it authority.
    //
    // An empty `tools_called` rejects every block, which is the correct
    // outcome: no tool ran, so there is no data any visual could be built from.
    if !source_tool_ran(&block, tools_called) {
        return None;
    }

    // JSON Schema cannot express "every row has as many cells as there are
    // columns", so the one relational invariant is checked here.
    if !table_rows_match_columns(&block) {
        warn!("viz-blocks: table row arity does not match its columns; leaving it in the reply");
        return None;
    }

    Some(block)
}

/// `true` when the block's kind appears in the coach's grant.
///
/// The grant is kind-level, not boolean: `visuals: [table]` permits tables and
/// nothing else. Without this the frontmatter list would be decoration — any
/// non-empty grant would accept every schema-valid kind.
fn kind_granted(block: &Value, granted: &[String]) -> bool {
    let kind = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if granted.iter().any(|g| g == kind) {
        return true;
    }
    warn!(
        kind,
        "viz-blocks: block kind is outside this coach's visuals grant; leaving it in the reply"
    );
    false
}

/// `true` when the block validates against the dravr-viz schema.
///
/// Logs the first violation on failure; a schema that failed to compile is
/// treated as invalid, which conservatively refuses every block rather than
/// rendering unvalidated ones.
fn schema_valid(schemas: &SchemaTexts, block: &Value) -> bool {
    let Some(validator) = validator_for(schemas, DRAVR_VIZ) else {
        return false;
    };
    if validator.is_valid(block) {
        return true;
    }
    if let Some(error) = validator.iter_errors(block).next() {
        warn!(
            error = %error,
            path = %error.instance_path(),
            "viz-blocks: block failed schema validation; leaving it in the reply"
        );
    }
    false
}

/// `true` when the block's `source_tool` names a tool that actually ran.
///
/// Compared case-sensitively — a near-miss is a miss, because the point is that
/// the citation be checkable rather than plausible.
///
/// The one normalisation is the MCP server prefix. On the native ACP path the
/// loop records a tool as `dravr-get_activities`, because that is the name the
/// `/mcp` server exposes it under, while the model cites it as
/// `get_activities` — the name in its own tool catalogue and in the block
/// schema. Comparing those literally rejects a chart whose attribution is
/// exactly right, which is not the fabrication this gate exists to catch. It
/// cost the first genuine chart the coach ever produced (2026-08-18): correct
/// data, correct source, refused on a prefix.
/// Drop the MCP server prefix a natively-called tool is recorded under.
///
/// `dravr-get_activities` -> `get_activities`. Only the platform's own prefix
/// is stripped: a tool from some other MCP server is a different tool and must
/// not satisfy a citation for ours.
fn strip_mcp_prefix(recorded: &str) -> &str {
    recorded.strip_prefix("dravr-").unwrap_or(recorded)
}

fn source_tool_ran(block: &Value, tools_called: &[String]) -> bool {
    let claimed = block
        .get("source_tool")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if tools_called
        .iter()
        .any(|called| called == claimed || strip_mcp_prefix(called) == claimed)
    {
        return true;
    }
    warn!(
        source_tool = claimed,
        "viz-blocks: source_tool did not run this turn; leaving the block in the reply"
    );
    false
}

/// `true` unless this is a table whose rows disagree with its column count.
fn table_rows_match_columns(block: &Value) -> bool {
    if block.get("type").and_then(Value::as_str) != Some("table") {
        return true;
    }
    let Some(columns) = block.get("columns").and_then(Value::as_array) else {
        return true;
    };
    let Some(rows) = block.get("rows").and_then(Value::as_array) else {
        return true;
    };
    rows.iter()
        .all(|row| row.as_array().is_some_and(|r| r.len() == columns.len()))
}

/// `true` when every one of `count` block markers is still present in `text`.
///
/// The marker is what places a block; a block whose marker is gone is not
/// merely mispositioned, it is unrenderable. Several later stages replace or
/// truncate the reply wholesale — the guardrail blocked-topic substitution, the
/// too-long truncation, the verification block-fallback — and any of them drops
/// markers while leaving the blocks behind. Checking parity at the end catches
/// all of them, including stages added later, which patching each site would
/// not.
#[must_use]
pub fn markers_intact(text: &str, count: usize) -> bool {
    (0..count).all(|index| text.contains(&marker(index)))
}
