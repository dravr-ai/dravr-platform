// ABOUTME: Lightweight markdown emphasis-stripper for messaging output
// ABOUTME: Removes `CommonMark` `*x*`, `**x**`, `_x_`, `__x__` so platform-agnostic plain text reaches every channel
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Strip `CommonMark` emphasis markers (`*x*`, `**x**`, `_x_`, `__x__`) while
/// preserving the inner text.
///
/// Handles only the four emphasis forms — not code spans, links, headings, or
/// list markers. Coach and tool descriptions seeded from `.md` files are
/// short single paragraphs where emphasis is the only `CommonMark` feature
/// that leaks when the text is relayed as plain-text messaging output
/// (Telegram HTML mode, Discord plain, `WhatsApp` template body, Messenger
/// generic template). Slack's `mrkdwn` renderer coincidentally honors
/// asterisks, but running them through this stripper still yields correct
/// plain output for everyone.
///
/// Escaped asterisks (`\*`) are emitted as literal `*`. Unbalanced markers
/// (opening without a closing match before end-of-input) are emitted
/// verbatim so we never silently swallow arbitrary text.
#[must_use]
pub fn strip_emphasis(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() && matches!(bytes[i + 1], b'*' | b'_') {
            out.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if (b == b'*' || b == b'_') && i + 1 < bytes.len() && bytes[i + 1] == b {
            if let Some(end) = find_double_marker(bytes, i + 2, b) {
                out.push_str(&input[i + 2..end]);
                i = end + 2;
                continue;
            }
        }
        if b == b'*' || b == b'_' {
            if let Some(end) = find_single_marker(bytes, i + 1, b) {
                out.push_str(&input[i + 1..end]);
                i = end + 1;
                continue;
            }
        }
        let ch_len = utf8_char_len(b);
        out.push_str(&input[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn find_single_marker(bytes: &[u8], start: usize, marker: u8) -> Option<usize> {
    let mut j = start;
    while j < bytes.len() {
        if bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 2;
            continue;
        }
        if bytes[j] == marker {
            if j + 1 < bytes.len() && bytes[j + 1] == marker {
                j += 2;
                continue;
            }
            if j > start {
                return Some(j);
            }
            return None;
        }
        j += 1;
    }
    None
}

fn find_double_marker(bytes: &[u8], start: usize, marker: u8) -> Option<usize> {
    let mut j = start;
    while j + 1 < bytes.len() {
        if bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 2;
            continue;
        }
        if bytes[j] == marker && bytes[j + 1] == marker && j > start {
            return Some(j);
        }
        j += 1;
    }
    None
}

const fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b < 0xC0 {
        // continuation byte at char boundary shouldn't happen in valid UTF-8;
        // advancing 1 byte keeps the loop finite on malformed input
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}
