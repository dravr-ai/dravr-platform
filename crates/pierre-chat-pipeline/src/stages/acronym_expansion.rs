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
//! ## Relationship to the persona style stage
//!
//! dravr-colleur's [`persona_style`](dravr_colleur::persona_style) *detects*
//! unglossed acronyms and logs a violation (per-persona, advisory). This
//! module *remediates* them (universal, deterministic) — a formatter to the conformance stage's
//! linter. The acronym data is single-source: both read the same
//! contremaitre `persona_contracts.yaml`.
//!
//! ## Safety
//!
//! Matching is exact, case-sensitive, and word-boundary-aligned, so
//! `ATL` never fires inside `ATLAS` and a lowercase `if` never matches
//! the (deliberately un-catalogued) power metric `IF`. The glossary
//! curation rule keeps ambiguous short tokens out entirely. Where the first
//! use is and whether it is already glossed is colleur's
//! [`first_unglossed_use_end`], the check its first-use rule runs, so
//! detection and remediation agree on what "already glossed" means.

use std::cmp::Reverse;

use dravr_colleur::persona_style::first_unglossed_use_end;
use pierre_contremaitre::persona_contracts::PersonaContractsSnapshot;

/// Expand the first unglossed occurrence of every catalogued acronym in
/// `text` with its `locale` full form.
///
/// Only the first word-boundary occurrence of each acronym is touched —
/// the first-use rule. If that first occurrence is already glossed (a `(`
/// within colleur's gloss window, [`first_unglossed_use_end`]), the acronym
/// is left untouched. Returns `text` unchanged when the glossary is empty
/// (boot before sync) or no catalogued acronym appears.
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
        if let Some(end) = first_unglossed_use_end(text, acronym) {
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
