// ABOUTME: Post-LLM deterministic acronym expansion — glosses each catalogued acronym on first use
// ABOUTME: Data-driven from contremaitre persona_contracts.yaml `glossary`; the hard backstop behind the prompt rule
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! First-use acronym expansion.
//!
//! The system prompt instructs the model to define acronyms on first
//! use, but prompts only steer. This pass guarantees it for the curated
//! set of unambiguous sports-science acronyms: it rewrites the first
//! unglossed occurrence of each catalogued acronym into
//! `ACRONYM (full form)` in the active turn locale — e.g. `ATL` becomes
//! `ATL (acute training load)`.
//!
//! ## Relationship to `persona_conformance`
//!
//! [`super::persona_conformance`] *detects* unglossed acronyms and logs a
//! violation (per-persona, advisory). This module *remediates* them
//! (universal, deterministic) — a formatter to the conformance stage's
//! linter. The acronym data is single-source: both read the same
//! contremaitre `persona_contracts.yaml`.
//!
//! ## Safety
//!
//! Matching is exact, case-sensitive, and word-boundary-aligned, so
//! `ATL` never fires inside `ATLAS` and a lowercase `if` never matches
//! the (deliberately un-catalogued) power metric `IF`. The glossary
//! curation rule keeps ambiguous short tokens out entirely. The 30-char
//! gloss-detection window advances by characters, not bytes, so
//! multibyte locales (the French apostrophe `’`) never split a char
//! boundary.

use std::cmp::Reverse;

use pierre_contremaitre::persona_contracts::PersonaContractsSnapshot;

/// Characters after an acronym to scan for an existing `(` gloss before
/// deciding the occurrence is unglossed. Matches the window used by
/// [`super::persona_conformance::has_unglossed_acronym`] so detection and
/// remediation agree on what "already glossed" means.
const GLOSS_WINDOW_CHARS: usize = 30;

/// Expand the first unglossed occurrence of every catalogued acronym in
/// `text` with its `locale` full form.
///
/// Only the first word-boundary occurrence of each acronym is touched —
/// the first-use rule. If that first occurrence is already glossed (a `(`
/// within [`GLOSS_WINDOW_CHARS`]), the acronym is left untouched. Returns
/// `text` unchanged when the glossary is empty (boot before sync) or no
/// catalogued acronym appears.
#[must_use]
pub fn expand_acronyms_first_use(
    snapshot: &PersonaContractsSnapshot,
    text: &str,
    locale: &str,
) -> String {
    if snapshot.glossary.is_empty() {
        return text.to_owned();
    }

    // Resolve every insertion against the ORIGINAL text first, then splice
    // right-to-left so earlier byte offsets stay valid as we mutate.
    let mut inserts: Vec<(usize, String)> = Vec::new();
    for acronym in snapshot.glossary.keys() {
        let Some(expansion) = snapshot.acronym_gloss(acronym, locale) else {
            continue;
        };
        if let Some(end) = first_unglossed_occurrence_end(text, acronym) {
            inserts.push((end, format!(" ({expansion})")));
        }
    }

    if inserts.is_empty() {
        return text.to_owned();
    }

    inserts.sort_unstable_by_key(|&(pos, _)| Reverse(pos));
    let mut out = text.to_owned();
    for (pos, gloss) in inserts {
        out.insert_str(pos, &gloss);
    }
    out
}

/// Byte index just past the first word-boundary occurrence of `acronym`
/// that is not already glossed. Returns `None` when no boundary-aligned
/// occurrence is unglossed — either the acronym is absent, every match is
/// a substring of a larger word, or the first real occurrence already
/// carries a gloss (first-use satisfied).
fn first_unglossed_occurrence_end(text: &str, acronym: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = text[from..].find(acronym) {
        let start = from + rel;
        let end = start + acronym.len();
        if is_word_boundary(text, start, end) {
            // First genuine occurrence decides the outcome: gloss it if
            // bare, otherwise stop (the model already defined it).
            return if is_glossed_within_window(&text[end..]) {
                None
            } else {
                Some(end)
            };
        }
        from = end;
    }
    None
}

/// `true` when `[start, end)` is flanked by non-alphanumeric characters
/// (or the string edge) on both sides, so the match is a standalone token
/// rather than part of a larger word. Unicode-aware, so accented letters
/// in `es`/`de`/`fr`/`pt` count as word characters.
fn is_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let before_ok = text[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric());
    let after_ok = text[end..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric());
    before_ok && after_ok
}

/// `true` when a `(` appears within [`GLOSS_WINDOW_CHARS`] characters of
/// the start of `after`. Advances by characters, not bytes, so the window
/// boundary never splits a multibyte sequence.
fn is_glossed_within_window(after: &str) -> bool {
    let window_end = after
        .char_indices()
        .nth(GLOSS_WINDOW_CHARS)
        .map_or(after.len(), |(idx, _)| idx);
    after[..window_end].contains('(')
}
