// ABOUTME: Pure text detectors behind the persona conformance rules (lists, blocks, numbers, words)
// ABOUTME: No contract or I/O here — each takes the reply text and answers one shape question
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Text detectors for [`super`]: every function reads reply text and reports a
//! shape — a bullet run, a `Label: value` block, a hedged or unrounded number,
//! a standalone word — so the rule checks stay about the contract.

/// Whitespace-split word count. Matches the "word budget" the vault doc
/// uses — close enough to a tokenizer for soft caps.
#[must_use]
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Longest contiguous run of bullet/list lines anywhere in the reply.
/// Empty lines and indented continuations (lines starting with two
/// spaces) extend the current run; everything else breaks it.
#[must_use]
pub fn longest_bullet_run(text: &str) -> usize {
    let mut best = 0_usize;
    let mut current = 0_usize;
    for line in text.lines() {
        if is_bullet_line(line) {
            current += 1;
            best = best.max(current);
        } else if line.trim().is_empty() || line.starts_with("  ") {
            // Soft break — preserves the run across wrapped bullets.
        } else {
            current = 0;
        }
    }
    best
}

/// Markdown bullet markers, each with the space that makes it a list item.
const BULLET_MARKERS: &[&str] = &["- ", "* ", "+ "];

/// `true` when `line` is a markdown bullet — `-`, `*`, `+`, or a numbered
/// `N.` / `N)` followed by a space. Indentation is allowed.
#[must_use]
pub fn is_bullet_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    BULLET_MARKERS
        .iter()
        .any(|marker| trimmed.starts_with(marker))
        || numbered_list_prefix(trimmed)
}

fn numbered_list_prefix(s: &str) -> bool {
    let mut chars = s.chars();
    let mut saw_digit = false;
    for c in chars.by_ref() {
        if c.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        if saw_digit && (c == '.' || c == ')') {
            return matches!(chars.next(), Some(' '));
        }
        return false;
    }
    false
}

/// Two or more consecutive lines matching `Label: value`.
///
/// See [`is_label_value_line`] for the line shapes that count.
#[must_use]
pub fn detects_label_value_block(text: &str) -> bool {
    let mut consecutive = 0_usize;
    for line in text.lines() {
        if is_label_value_line(line) {
            consecutive += 1;
            if consecutive >= 2 {
                return true;
            }
        } else if !line.trim().is_empty() {
            consecutive = 0;
        }
    }
    false
}

/// Longest label, in characters, that still reads as a field name.
const LABEL_MAX_CHARS: usize = 32;

/// `true` when `line` is one `Label: value` row.
///
/// Accepts the shapes models actually emit, not only the bare ASCII one: a
/// list marker before the label (`- Distance: 42 km`), markdown emphasis
/// around it (`**Distance:** 42 km`), a non-ASCII label (`Durée`), French
/// typography's space before the colon (`Durée : 1 h 12`), and a quoted name
/// as the label (`« Sortie longue » : 32 km`). An ASCII-only bare-label
/// shape would miss every French block and every bolded one, reporting a
/// strict persona's correct reply as having none.
fn is_label_value_line(line: &str) -> bool {
    let row = strip_list_marker(line.trim());
    let Some((label, value)) = row.split_once(':') else {
        return false;
    };
    let label = label.trim_matches(|c: char| c.is_whitespace() || matches!(c, '*' | '_'));
    let value = value.trim_matches(|c: char| c.is_whitespace() || matches!(c, '*' | '_'));
    if value.is_empty() || label.is_empty() || label.chars().count() > LABEL_MAX_CHARS {
        return false;
    }
    let label_chars_ok = label.chars().all(|c| {
        c.is_alphanumeric()
            || c.is_whitespace()
            || matches!(
                c,
                '_' | '-' | '\'' | '\u{2019}' | '/' | '(' | ')' | '«' | '»' | '"'
            )
    });
    let starts_with_word = label
        .chars()
        .find(|c| !matches!(c, '«' | '"') && !c.is_whitespace())
        .is_some_and(char::is_alphabetic);
    label_chars_ok && starts_with_word
}

/// `line` without a leading markdown list marker (`- `, `* `, `+ `, `1. `).
fn strip_list_marker(line: &str) -> &str {
    if let Some(rest) = BULLET_MARKERS
        .iter()
        .find_map(|marker| line.strip_prefix(marker))
    {
        return rest;
    }
    if numbered_list_prefix(line) {
        return line
            .split_once(' ')
            .map_or(line, |(_, rest)| rest.trim_start());
    }
    line
}

/// `true` when `acronym` appears in the text WITHOUT a `(...)` gloss
/// within the next 30 characters of any occurrence.
#[must_use]
pub fn has_unglossed_acronym(text: &str, acronym: &str) -> bool {
    let mut search_from = 0;
    while let Some(rel_idx) = text[search_from..].find(acronym) {
        let abs_idx = search_from + rel_idx;
        let after = &text[abs_idx + acronym.len()..];
        // Advance by characters, not bytes, so the window never splits a
        // multibyte char. A raw byte offset can land inside a multibyte
        // sequence (e.g. the French apostrophe `’`), and slicing there panics.
        let lookahead_end = after
            .char_indices()
            .nth(30)
            .map_or(after.len(), |(idx, _)| idx);
        let window = &after[..lookahead_end];
        if !window.contains('(') {
            return true;
        }
        search_from = abs_idx + acronym.len();
    }
    false
}

/// `true` when the FIRST occurrence of `acronym` carries no `(...)` gloss
/// within the following 30 characters. Later occurrences are ignored, which is
/// what separates this from [`has_unglossed_acronym`].
///
/// Returns `false` when the acronym is absent — nothing to gloss.
#[must_use]
pub fn first_use_is_unglossed(text: &str, acronym: &str) -> bool {
    let Some(idx) = find_standalone_word(text, acronym) else {
        return false;
    };
    let after = &text[idx + acronym.len()..];
    let lookahead_end = after.char_indices().nth(30).map_or(after.len(), |(i, _)| i);
    !after[..lookahead_end].contains('(')
}

/// Numeric tokens containing a decimal point. Integers are excluded on
/// purpose — [`super::check_round_numbers`] only judges fractional precision.
#[must_use]
pub fn decimal_tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        let numeric = ch.is_ascii_digit() || (ch == '.' && start.is_some());
        if numeric {
            if start.is_none() {
                start = Some(idx);
            }
        } else if let Some(s) = start.take() {
            push_decimal(text, s, idx, &mut tokens);
        }
    }
    if let Some(s) = start {
        push_decimal(text, s, text.len(), &mut tokens);
    }
    tokens
}

/// Trim a candidate to a real decimal and keep it only if it has a fractional
/// part. A trailing `.` is sentence punctuation, not precision.
fn push_decimal<'a>(text: &'a str, start: usize, end: usize, out: &mut Vec<&'a str>) {
    let token = text[start..end].trim_end_matches('.');
    if token.contains('.') {
        out.push(token);
    }
}

/// Significant digits in a decimal token: leading zeros carry no precision, so
/// `0.5` is one significant digit while `12.34` is four.
#[must_use]
pub fn significant_digits(token: &str) -> usize {
    token
        .chars()
        .filter(char::is_ascii_digit)
        .skip_while(|c| *c == '0')
        .count()
}

/// `true` when `modifier` appears within [`super::VAGUE_MODIFIER_WINDOW`] characters
/// of an ASCII digit, in either direction. Both texts are expected lowercased.
#[must_use]
pub fn modifier_adjacent_to_digit(lowered: &str, modifier: &str) -> bool {
    let mut search_from = 0;
    while let Some(rel) = lowered[search_from..].find(modifier) {
        let start = search_from + rel;
        let end = start + modifier.len();
        let before = &lowered[..start];
        let lead_start = before
            .char_indices()
            .rev()
            .nth(super::VAGUE_MODIFIER_WINDOW - 1)
            .map_or(0, |(i, _)| i);
        let after = &lowered[end..];
        let trail_end = after
            .char_indices()
            .nth(super::VAGUE_MODIFIER_WINDOW)
            .map_or(after.len(), |(i, _)| i);
        if before[lead_start..].chars().any(|c| c.is_ascii_digit())
            || after[..trail_end].chars().any(|c| c.is_ascii_digit())
        {
            return true;
        }
        search_from = end;
    }
    false
}

/// Byte index of the first standalone occurrence of `word` — one not glued to
/// an adjacent alphanumeric, so `P1` does not match inside `P10` and `Go` does
/// not match inside `Going`.
#[must_use]
pub fn find_standalone_word(text: &str, word: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find(word) {
        let start = search_from + rel;
        let end = start + word.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = text[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return Some(start);
        }
        search_from = end;
    }
    None
}

/// `true` when `word` appears as a standalone token.
#[must_use]
pub fn contains_standalone_word(text: &str, word: &str) -> bool {
    find_standalone_word(text, word).is_some()
}

/// Split into sentences on `.`, `!`, `?` and newlines, without breaking
/// decimals: a `.` flanked by digits belongs to the number, not the sentence.
#[must_use]
pub fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (pos, (idx, ch)) in chars.iter().enumerate() {
        let terminator = match ch {
            '.' => {
                let prev_digit = pos
                    .checked_sub(1)
                    .is_some_and(|p| chars[p].1.is_ascii_digit());
                let next_digit = chars.get(pos + 1).is_some_and(|(_, c)| c.is_ascii_digit());
                !(prev_digit && next_digit)
            }
            '!' | '?' | '\n' => true,
            _ => false,
        };
        if terminator {
            let piece = text[start..*idx].trim();
            if !piece.is_empty() {
                out.push(piece);
            }
            start = idx + ch.len_utf8();
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Longest contiguous run of `Label: value` lines. Blank lines and indented
/// continuations extend the run, matching [`longest_bullet_run`]'s treatment of
/// wrapped content.
#[must_use]
pub fn longest_label_value_run(text: &str) -> usize {
    let mut best = 0_usize;
    let mut current = 0_usize;
    for line in text.lines() {
        if is_label_value_line(line) {
            current += 1;
            best = best.max(current);
        } else if line.trim().is_empty() || line.starts_with("  ") {
            // Soft break — a wrapped value does not end the block.
        } else {
            current = 0;
        }
    }
    best
}

/// Athlete identifiers cited in the reply.
///
/// The four-character token following a `·` separator, per the
/// `<display_name> · <last4uuid>` contract shape. Lowercased so comparison
/// against [`super::RosterScope`] is case-insensitive.
#[must_use]
pub fn athlete_citations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, _) in text.match_indices('·') {
        let after = &text[idx + '·'.len_utf8()..];
        let token: String = after
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        if token.len() == 4 {
            let lowered = token.to_lowercase();
            if !out.contains(&lowered) {
                out.push(lowered);
            }
        }
    }
    out
}

/// Clip a sentence for a log line, on a character boundary.
pub(super) fn truncate_for_log(sentence: &str) -> String {
    const LIMIT: usize = 80;
    if sentence.chars().count() <= LIMIT {
        return sentence.to_owned();
    }
    let clipped: String = sentence.chars().take(LIMIT).collect();
    format!("{clipped}…")
}

/// Units that mark a number as a measurement rather than a date, a count or an
/// ordinal. Lowercase; compared against the letters that follow a number,
/// attached (`42km`, `88%`) or as the next word (`42 km`).
const MEASUREMENT_UNITS: &[&str] = &[
    "km", "m", "mi", "ft", "h", "min", "mn", "s", "sec", "ms", "bpm", "w", "watts", "kj", "kcal",
    "cal", "%", "rpm", "spm", "kg", "lb", "lbs", "km/h", "mph", "/km", "/mi",
];

/// Words a metric's value may sit behind its name: `CTL 62`, `TSB: -8`,
/// `CTL est à 62`, `TSB sits at -8`.
const METRIC_LOOKBACK_TOKENS: usize = 3;

/// Measured values in `text`: a number carrying a [`MEASUREMENT_UNITS`] unit,
/// or a number within [`METRIC_LOOKBACK_TOKENS`] words after one of
/// `metric_acronyms`.
/// A date (`3 octobre`), a window (`12 semaines`) or an ordinal counts as
/// none — they are what a reply with no data yet still says.
pub(super) fn measured_value_count(text: &str, metric_acronyms: &[&str]) -> usize {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut count = 0;
    for (idx, token) in tokens.iter().enumerate() {
        let token = token.trim_start_matches(['(', '*', '«', '"']);
        let numeric_len = token
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | ',' | ':' | '-' | '+')))
            .unwrap_or(token.len());
        if !token[..numeric_len].chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        let attached = &token[numeric_len..];
        let unit_source = if attached.is_empty() {
            tokens.get(idx + 1).copied().unwrap_or_default()
        } else {
            attached
        };
        let after_metric = tokens[idx.saturating_sub(METRIC_LOOKBACK_TOKENS)..idx]
            .iter()
            .map(|prev| prev.trim_matches(|c: char| !c.is_alphanumeric()))
            .any(|prev| metric_acronyms.contains(&prev));
        if is_measurement_unit(unit_source) || after_metric {
            count += 1;
        }
    }
    count
}

/// `true` when the leading run of letters (plus `%` and `/`) in `word` is
/// exactly a [`MEASUREMENT_UNITS`] unit: `km,` and `h30` qualify, while
/// `minutes`, `mardi` and `semaines` do not.
fn is_measurement_unit(word: &str) -> bool {
    let lowered = word.to_lowercase();
    let unit: String = lowered
        .chars()
        .take_while(|c| c.is_alphabetic() || matches!(c, '%' | '/'))
        .collect();
    !unit.is_empty() && MEASUREMENT_UNITS.contains(&unit.as_str())
}
