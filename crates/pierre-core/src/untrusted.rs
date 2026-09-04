// ABOUTME: Neutralizes untrusted text before it is interpolated into a structured destination
// ABOUTME: Shared flatten/cap mechanics, plus the defang for text a client renders
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Collapse every whitespace and control character run to a single space, and trim.
///
/// Untrusted text reaches a structured destination — an LLM prompt, a rendered
/// tool result — as one field inside a line-oriented format. The newline in
/// that field is what forges the second line: a markdown heading, an extra row
/// in a numbered list, a second `<user_fact>` fence. Flattening removes the
/// line-start every one of those forgeries needs while keeping every word the
/// author actually wrote.
///
/// Control characters are folded too, not only whitespace. `\u{0}` and friends
/// are not `is_whitespace`, and a bidirectional override (`\u{202E}`) reorders
/// the rendering of everything after it — neither has a legitimate place in a
/// single-line field.
#[must_use]
pub fn flatten_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() || ch.is_control() {
            // Only a separator once something precedes it, so leading runs are
            // dropped rather than becoming a leading space.
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    // `pending_space` is deliberately never flushed: a trailing run trims.
    out
}

/// Cap to `max_chars`, marking the cut with an ellipsis.
///
/// Counts characters rather than bytes, so a multi-byte name is cut where a
/// reader would expect and never mid-codepoint.
#[must_use]
pub fn cap(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let head: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{head}…")
}

/// Leading characters that would let a field open a markdown block of its own.
const STRUCTURAL_LEAD: [char; 6] = ['#', '>', '*', '-', '`', '|'];

/// Defang untrusted text that a **client will render**, as opposed to text a
/// model will read.
///
/// The distinction matters because the threat is different. A prompt
/// destination worries about forged instructions; a rendering destination
/// worries about forged *interface* — a heading that looks like the server
/// said it, a table row that invents an activity, an image whose URL carries
/// the surrounding text off to whoever authored the field.
///
/// Provider payloads are the untrusted input here: an activity title comes
/// from Strava or Garmin and is whatever the athlete — or anyone who can write
/// to their account — typed. It arrives verbatim in the text an MCP client
/// renders.
///
/// Four neutralizations, each aimed at one forgery:
///
/// - angle brackets become guillemets, so raw HTML cannot open a tag. Replaced
///   outright rather than matched as tag names, which is what defeats the
///   case- and whitespace-variant spellings that substring matching misses.
/// - backticks become apostrophes, so a field cannot open or close a code fence
///   and swallow the lines after it.
/// - a leading structural character is stripped, so a field cannot begin a
///   heading, blockquote, list item, or table row. Only the lead is touched;
///   a hyphen inside a name is ordinary punctuation.
/// - the `](` joint is spaced apart, which breaks `[text](url)` and
///   `![alt](url)` without deleting either character. An image is the
///   exfiltration case: renderers fetch it, so the URL leaves with whatever
///   the author put in it. No real title contains that two-character sequence.
///
/// Call [`flatten_line`] first; this assumes it is working on one line.
#[must_use]
pub fn defang_for_display(s: &str) -> String {
    s.trim_start_matches(STRUCTURAL_LEAD)
        .trim_start()
        .replace('<', "‹")
        .replace('>', "›")
        .replace('`', "'")
        .replace("](", "] (")
}
