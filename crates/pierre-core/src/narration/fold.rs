// ABOUTME: Separator folding — the normal form every matcher compares against
// ABOUTME: Split out of narration/mod.rs so each file stays legible
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Dashes that join words: ASCII hyphen, U+2010 hyphen, U+2011 non-breaking
/// hyphen, U+2012 figure dash. Inside a word (`terminal-based`) they carry no
/// clause boundary.
pub(super) const fn is_word_dash(ch: char) -> bool {
    matches!(ch, '-' | '\u{2010}' | '\u{2011}' | '\u{2012}')
}

/// Dashes that punctuate rather than join: en dash, em dash, horizontal bar.
/// These break a clause wherever they appear, spaced or not.
pub(super) const fn is_clause_dash(ch: char) -> bool {
    matches!(ch, '\u{2013}' | '\u{2014}' | '\u{2015}')
}

/// Apostrophes, ASCII and typographic.
pub(super) const fn is_apostrophe(ch: char) -> bool {
    matches!(ch, '\'' | '\u{2018}' | '\u{2019}')
}

/// Every character [`fold_into`] collapses to a space.
pub(super) fn is_separator(ch: char) -> bool {
    ch.is_whitespace() || is_word_dash(ch) || is_clause_dash(ch) || is_apostrophe(ch)
}

/// Fold a string for separator-insensitive matching, reporting the clause
/// breaks the fold erases.
///
/// Lowercases, then collapses every run of ASCII/Unicode hyphens, dashes,
/// apostrophes and whitespace to a single space. So `« prompt-injection »`,
/// `prompt — injection` and `prompt injection` all compare equal, and the
/// ASCII apostrophe in a pattern (`test d'injection`) matches the typographic
/// apostrophe LLMs emit in French (`test d’injection`).
///
/// Erasing dashes is what makes that equality work, but a dash is also how a
/// reply breaks a clause without punctuation — « I'm not a fitness coach — I'm
/// GitHub Copilot CLI » — so `on_clause_break` receives the byte offset, in
/// the returned string, of every collapsed run that carried one. A run counts
/// when it holds a punctuating dash, or a word dash next to whitespace
/// (`coach - I'm`, the plain-text em-dash substitute messaging clients emit);
/// a bare intra-word hyphen does not.
///
/// A run at either end collapses to nothing, so the result needs no trim and
/// the reported offsets index the string as returned.
pub(super) fn fold_into(s: &str, mut on_clause_break: impl FnMut(usize)) -> String {
    let mut out = String::with_capacity(s.len());
    // Start inside a run so a leading separator emits no space.
    let mut in_run = true;
    let mut space_at: Option<usize> = None;
    let mut run_word_dash = false;
    let mut run_clause_dash = false;
    let mut run_space = false;

    for ch in s.to_lowercase().chars() {
        if is_separator(ch) {
            run_word_dash |= is_word_dash(ch);
            run_clause_dash |= is_clause_dash(ch);
            run_space |= ch.is_whitespace();
            if !in_run {
                space_at = Some(out.len());
                out.push(' ');
                in_run = true;
            }
        } else {
            if in_run {
                // `space_at` is None for a leading run, which has no clause
                // in front of it to break.
                let breaks_clause = run_clause_dash || (run_word_dash && run_space);
                if let Some(at) = space_at.filter(|_| breaks_clause) {
                    on_clause_break(at);
                }
                space_at = None;
                run_word_dash = false;
                run_clause_dash = false;
                run_space = false;
                in_run = false;
            }
            out.push(ch);
        }
    }
    // A trailing run pushed a space and never closed; drop it rather than
    // trimming, so the offsets already reported stay valid.
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// [`fold_into`] for callers that need only the folded text. Applied to both
/// the patterns (once, at first use) and every candidate sentence/reply.
pub(super) fn fold_separators(s: &str) -> String {
    fold_into(s, |_| {})
}
