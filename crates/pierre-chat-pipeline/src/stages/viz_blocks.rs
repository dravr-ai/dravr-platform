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
use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;
use tracing::warn;

use super::structured_output::{validator_for, SchemaTexts, DRAVR_VIZ};

/// What a conversation with no coach persona bound may draw.
///
/// The `visuals:` grant is a *coach author's* choice, so it only exists when a
/// coach is bound. A group chat binds none — the platform itself is answering —
/// and reading "no coach" as "no grant" left the visual contract out of the
/// prompt entirely. The model then reported, accurately, that it had no way to
/// draw: "Je peux pas générer de graphique dans ce chat — pas d'outil pour ça
/// de mon côté" (Telegram group, 2026-08-21), having already fetched the data.
///
/// So absence of a coach means no author expressed a preference, and the
/// platform baseline applies. A coach that IS bound still governs its own
/// reply, including a deliberately empty grant meaning "this persona does not
/// draw".
pub const DEFAULT_VISUALS: &[&str] = &["chart", "table"];

/// The visual kinds this turn may emit.
///
/// One rule, two readers: the prompt-assembly stage decides whether to teach the
/// contract, and the post-process stage decides whether to honour a fence. They
/// must agree — a coach told it may draw whose blocks are then refused produces
/// exactly the raw-JSON reply this pipeline works to avoid.
#[must_use]
pub fn granted_visuals(coach_visuals: Option<&[String]>) -> Vec<String> {
    coach_visuals.map_or_else(
        || DEFAULT_VISUALS.iter().map(|k| (*k).to_owned()).collect(),
        <[String]>::to_vec,
    )
}

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

/// Remove every positional marker from prose.
///
/// The marker's job ends once a client that can interleave has interleaved:
/// in a Telegram message or a list-row preview, `⟦viz:0⟧` is noise at best and
/// looks like a bug at worst. An unterminated marker is not a marker and is
/// kept, rather than swallowing the rest of the text after it.
#[must_use]
pub fn strip_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(MARKER_OPEN) {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find(MARKER_CLOSE) {
            rest = &rest[start + end + MARKER_CLOSE.len()..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

/// The result of lifting visual blocks out of a reply.
pub struct VizExtraction {
    /// Reply text with each block replaced by its positional marker.
    pub text: String,
    /// Validated blocks in the order they appeared.
    pub blocks: Vec<Value>,
    /// Why each refused fence was refused, in the order they appeared.
    ///
    /// A refusal used to exist only as a WARN line, which meant the reason was
    /// unavailable to anything that could act on it: the athlete asked for a
    /// chart, the block was dropped, and the prose shipped alone with nothing
    /// able to say what went wrong. Carrying the reasons out lets a caller
    /// re-ask the model with the actual fault — and lets a test assert the
    /// fault text rather than merely that extraction failed.
    pub refusals: Vec<String>,
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
    let mut refusals: Vec<String> = Vec::new();
    let mut rest = reply;

    while let Some(fence) = next_fence(rest) {
        text.push_str(&rest[..fence.start]);
        match parse_block(schemas, granted, tools_called, fence.body) {
            Ok(block) => {
                text.push_str(&marker(blocks.len()));
                blocks.push(block);
            }
            // Refused: drop the fence entirely. `parse_block` has already logged
            // which rule it broke, so the failure stays legible to us without
            // being spelled out to the athlete in JSON — and the reason travels
            // out in `refusals` so a caller can act on it.
            Err(reason) => refusals.push(reason),
        }
        rest = &rest[fence.end..];
    }

    if blocks.is_empty() && refusals.is_empty() {
        return None;
    }
    text.push_str(rest);

    if !refusals.is_empty() {
        warn!(
            dropped = refusals.len(),
            kept = blocks.len(),
            "viz-blocks: refused block(s) removed from the reply; the prose stands alone"
        );
    }

    Some(VizExtraction {
        text: text.trim().to_owned(),
        blocks,
        refusals,
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
) -> Result<Value, String> {
    let mut block: Value = match serde_json::from_str(body.trim()) {
        Ok(value) => value,
        Err(e) => {
            warn!(error = %e, "viz-blocks: fence body is not valid JSON; leaving it in the reply");
            return Err(format!("the block is not valid JSON: {e}"));
        }
    };

    drop_unknown_accents(&mut block);

    schema_check(schemas, &block)?;

    if !kind_granted(&block, granted) {
        return Err(format!(
            "this conversation may not draw a {} block",
            block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("block of that kind")
        ));
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
        return Err(format!(
            "source_tool \"{}\" did not run in this turn, so its numbers are unattributable",
            block
                .get("source_tool")
                .and_then(Value::as_str)
                .unwrap_or("")
        ));
    }

    // JSON Schema cannot express "every row has as many cells as there are
    // columns", so the one relational invariant is checked here.
    if !table_rows_match_columns(&block) {
        warn!("viz-blocks: table row arity does not match its columns; leaving it in the reply");
        return Err(
            "every row must have exactly as many cells as the table has columns".to_owned(),
        );
    }

    Ok(block)
}

/// The series accents the dravr-viz schema accepts — mirrors the schema's
/// enum and photograveur's `Accent`.
const KNOWN_ACCENTS: [&str; 4] = ["activity", "nutrition", "recovery", "mobility"];

/// Remove model-invented accent values so a styling slip cannot reject a
/// valid chart.
///
/// Live 2026-08-23 (Telegram group): the model wrote `"accent": "neutral"`
/// on one series of a two-athlete comparison — a plausible word, not a
/// schema value — and the whole otherwise-valid block failed validation and
/// was stripped; the athlete asked for a graph and got prose. An accent is a
/// styling hint, and the renderer already assigns distinct cycle colours to
/// unpinned series, so the correct handling of an unknown value is to drop
/// the field, not the chart.
fn drop_unknown_accents(block: &mut Value) {
    let Some(series) = block.get_mut("series").and_then(Value::as_array_mut) else {
        return;
    };
    for entry in series {
        let unknown = entry
            .get("accent")
            .and_then(Value::as_str)
            .is_some_and(|accent| !KNOWN_ACCENTS.contains(&accent));
        if unknown {
            if let Some(fields) = entry.as_object_mut() {
                warn!(
                    accent = fields.get("accent").and_then(|v| v.as_str()).unwrap_or(""),
                    "viz-blocks: unknown series accent dropped; the renderer's cycle colours apply"
                );
                fields.remove("accent");
            }
        }
    }
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
fn schema_check(schemas: &SchemaTexts, block: &Value) -> Result<(), String> {
    let Some(validator) = validator_for(schemas, DRAVR_VIZ) else {
        return Err("the dravr-viz schema is unavailable, so no block can be validated".to_owned());
    };
    if validator.is_valid(block) {
        return Ok(());
    }
    let faults = schema_faults(schemas, block).join("; ");
    warn!(
        faults = %faults,
        "viz-blocks: block failed schema validation; leaving it in the reply"
    );
    Err(faults)
}

/// Per-branch validators keyed by the `type` const of each `oneOf` arm.
///
/// The dravr-viz schema is a `oneOf` over Chart and Table, and a whole-schema
/// failure reports only that the block "is not valid under any of the schemas
/// listed in the 'oneOf' keyword" — true, unactionable, and identical for every
/// possible mistake. Validating against the arm the block *claims* to be yields
/// the real fault instead.
static BRANCH_VALIDATORS: OnceLock<BTreeMap<String, jsonschema::Validator>> = OnceLock::new();

/// Compile one validator per `oneOf` arm, keyed by that arm's `type` const.
fn branch_validators(schemas: &SchemaTexts) -> &'static BTreeMap<String, jsonschema::Validator> {
    BRANCH_VALIDATORS.get_or_init(|| {
        let mut compiled = BTreeMap::new();
        let Some(text) = schemas.get(DRAVR_VIZ) else {
            return compiled;
        };
        let Ok(mut schema) = serde_json::from_str::<Value>(text) else {
            return compiled;
        };
        // Same reason as structured_output::compile: leaving these in makes the
        // validator reach for the draft meta-schema over the network.
        if let Some(obj) = schema.as_object_mut() {
            obj.remove("$schema");
            obj.remove("$id");
        }
        let Some(arms) = schema.get("oneOf").and_then(Value::as_array) else {
            return compiled;
        };
        for arm in arms {
            let Some(kind) = arm
                .pointer("/properties/type/const")
                .and_then(Value::as_str)
            else {
                continue;
            };
            if let Ok(validator) = jsonschema::validator_for(arm) {
                compiled.insert(kind.to_owned(), validator);
            }
        }
        compiled
    })
}

/// Human- and model-readable faults for a block that failed the whole schema.
///
/// Each entry is `path: message` against the arm named by the block's own
/// `type`, so "series/0/points: [[\"Toi\", 472.0]] is too short" reaches the log
/// and the repair prompt instead of the bare `oneOf` refusal. A block whose
/// `type` matches no arm gets that stated plainly rather than a fault list from
/// an arm it never claimed.
fn schema_faults(schemas: &SchemaTexts, block: &Value) -> Vec<String> {
    let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
    let Some(validator) = branch_validators(schemas).get(kind) else {
        return vec![format!(
            "type: \"{kind}\" is not one of the block kinds this schema defines"
        )];
    };
    let faults: Vec<String> = validator
        .iter_errors(block)
        .map(|e| {
            let path = e.instance_path().to_string();
            if path.is_empty() {
                e.to_string()
            } else {
                format!("{}: {e}", path.trim_start_matches('/'))
            }
        })
        .collect();
    if faults.is_empty() {
        // The arm accepts it but the whole schema did not: the block satisfies
        // more than one arm, which `oneOf` forbids. Rare, and worth saying
        // exactly rather than reporting no fault at all.
        return vec![format!(
            "matches the {kind} shape but is ambiguous under oneOf — it satisfies more than one block kind"
        )];
    }
    faults
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
