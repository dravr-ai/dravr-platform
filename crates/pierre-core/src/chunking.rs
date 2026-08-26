// ABOUTME: Splits an over-limit reply into ordered messages at sentence boundaries, never mid-word
// ABOUTME: The one chunker — every messaging send path packs against the channel's own ceiling here

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reply chunking.
//!
//! A chat transport accepts a bounded message: Discord 2000 characters,
//! Telegram 4096, Slack 40000. A reply longer than that used to be cut at the
//! ceiling — the athlete read a paragraph that stopped mid-thought and never
//! learned there was more. Cutting is not delivering, so this splits instead.
//!
//! # Where the split lands
//!
//! In descending order of preference: after a sentence, after a line, after a
//! word, and — only for a single token longer than the whole ceiling — between
//! two characters. A coach's paragraph therefore arrives as whole sentences in
//! order, and a URL is never broken in half by the split itself.
//!
//! # What is preserved
//!
//! Every non-whitespace character, in its original order. The whitespace that
//! sat exactly on a split point is consumed by the split: a chunk never starts
//! with a space or ends with a newline, because a transport renders that as an
//! empty first line. Nothing else is dropped, nothing is added, and no marker
//! is appended — the next message *is* the continuation.

/// Characters that end a sentence when whitespace (or the text) follows.
///
/// The trailing-whitespace requirement is what keeps `10.5 km` and
/// `dravr.ai` intact: a period inside a token is not a sentence end.
const SENTENCE_END: [char; 4] = ['.', '!', '?', '…'];

/// Closing marks that belong to the sentence they follow, so the split lands
/// after them rather than orphaning them onto the next message.
const SENTENCE_CLOSERS: [char; 6] = ['"', '\'', '»', ')', ']', '”'];

/// Split `text` into ordered messages, each at most `max_chars` characters.
///
/// Returns an empty vector for blank input — a caller that has nothing to say
/// sends nothing, which is what makes the empty-reply guard a length check
/// rather than a second emptiness test.
///
/// The returned chunks are trimmed, so each is a message a transport will
/// accept as-is. Concatenating them recovers every non-whitespace character of
/// `text` in order.
#[must_use]
pub fn chunk_reply(text: &str, max_chars: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // A ceiling of zero would admit no character at all and loop forever; a
    // transport that accepts nothing is not one we can chunk for, so the reply
    // travels whole and the transport rejects it visibly.
    if max_chars == 0 || trimmed.chars().count() <= max_chars {
        return vec![trimmed.to_owned()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;

    for unit in split_sentences(trimmed) {
        for piece in fit_unit(unit, max_chars) {
            let piece_chars = piece.chars().count();
            if current_chars > 0 && current_chars + piece_chars > max_chars {
                push_trimmed(&mut chunks, &current);
                current.clear();
                current_chars = 0;
            }
            if current_chars == 0 {
                // A new message never opens on the whitespace the previous one
                // ended at.
                let opening = piece.trim_start();
                current.push_str(opening);
                current_chars = opening.chars().count();
            } else {
                current.push_str(piece);
                current_chars += piece_chars;
            }
        }
    }
    push_trimmed(&mut chunks, &current);
    chunks
}

/// Append `buffer` as a finished message when it holds anything readable.
fn push_trimmed(chunks: &mut Vec<String>, buffer: &str) {
    let finished = buffer.trim();
    if !finished.is_empty() {
        chunks.push(finished.to_owned());
    }
}

/// Split `text` into sentence-sized units, each carrying the whitespace that
/// followed it so the packer can drop that whitespace at a split point and
/// keep it everywhere else.
fn split_sentences(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut units = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;

    while index < chars.len() {
        let (_, ch) = chars[index];
        let break_at = if ch == '\n' {
            // A line end is always a legitimate place to split: a coach's
            // bulleted list has no sentence punctuation to look for.
            Some(index + 1)
        } else if SENTENCE_END.contains(&ch) {
            sentence_break(&chars, index)
        } else {
            None
        };
        let Some(after_mark) = break_at else {
            index += 1;
            continue;
        };
        let mut end_index = after_mark;
        while end_index < chars.len() && chars[end_index].1.is_whitespace() {
            end_index += 1;
        }
        let end_byte = chars
            .get(end_index)
            .map_or_else(|| text.len(), |&(byte, _)| byte);
        units.push(&text[start..end_byte]);
        start = end_byte;
        index = end_index;
    }

    if start < text.len() {
        units.push(&text[start..]);
    }
    units
}

/// Index just past the terminator run starting at `index`, when that run
/// really ends a sentence — that is, when whitespace or the end of the text
/// follows it. `None` when the mark sits inside a token (`10.5`, `dravr.ai`).
fn sentence_break(chars: &[(usize, char)], index: usize) -> Option<usize> {
    let mut after = index;
    while after < chars.len()
        && (SENTENCE_END.contains(&chars[after].1) || SENTENCE_CLOSERS.contains(&chars[after].1))
    {
        after += 1;
    }
    match chars.get(after) {
        None => Some(after),
        Some(&(_, next)) if next.is_whitespace() => Some(after),
        Some(_) => None,
    }
}

/// Break one unit down until every piece fits `max_chars`.
///
/// A sentence longer than the whole ceiling is split between words; a single
/// word longer than the ceiling (a pathological token, not prose) is split
/// between characters, because the alternative is a message the transport
/// refuses.
fn fit_unit(unit: &str, max_chars: usize) -> Vec<&str> {
    if unit.chars().count() <= max_chars {
        return vec![unit];
    }
    split_words(unit)
        .into_iter()
        .flat_map(|word| split_chars(word, max_chars))
        .collect()
}

/// Split a unit into word-sized pieces, each carrying its trailing whitespace.
fn split_words(unit: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = unit.char_indices().collect();
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;

    while index < chars.len() {
        if !chars[index].1.is_whitespace() {
            index += 1;
            continue;
        }
        let mut end_index = index;
        while end_index < chars.len() && chars[end_index].1.is_whitespace() {
            end_index += 1;
        }
        let end_byte = chars
            .get(end_index)
            .map_or_else(|| unit.len(), |&(byte, _)| byte);
        pieces.push(&unit[start..end_byte]);
        start = end_byte;
        index = end_index;
    }

    if start < unit.len() {
        pieces.push(&unit[start..]);
    }
    pieces
}

/// Last resort: cut `piece` on character boundaries so nothing exceeds the
/// ceiling.
fn split_chars(piece: &str, max_chars: usize) -> Vec<&str> {
    if piece.chars().count() <= max_chars {
        return vec![piece];
    }
    let mut cuts = Vec::new();
    let mut start = 0usize;
    let mut taken = 0usize;
    for (byte, _) in piece.char_indices() {
        if taken == max_chars {
            cuts.push(&piece[start..byte]);
            start = byte;
            taken = 0;
        }
        taken += 1;
    }
    if start < piece.len() {
        cuts.push(&piece[start..]);
    }
    cuts
}
