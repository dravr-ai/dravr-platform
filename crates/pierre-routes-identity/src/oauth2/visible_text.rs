// ABOUTME: What a hosted page shows its reader, read from its HTML: the rendered words, and every mark that hides or reveals some
// ABOUTME: The consent text version digests it, so a markup-only change asks no one again and a visibility change does
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The visible text of a page
//!
//! [`visible_text`] reads an HTML page the way its reader does, for the
//! consent text version (EFVP 2026-11-17, decision 9.4a):
//!
//! - **Words as they render.** An inline tag (`<strong>`, `<em>`, `<a>`,
//!   `<span>` …) joins the words on either side, as a browser renders them,
//!   so wrapping a word that a full stop follows changes nothing. Any other
//!   tag separates the words around it, as a block or a line break does.
//! - **Entities decoded.** `&nbsp;`, `&#32;` and a plain space read alike,
//!   as do `&amp;` and `&`; whitespace runs read as one space.
//! - **Nothing that never renders.** Comments and the contents of `<style>`,
//!   `<script>`, `<template>` and `<noscript>` are left out.
//! - **Every mark that hides or reveals.** An element's `hidden`,
//!   `aria-hidden`, `inert`, `open` or `style` attribute is read, with its
//!   value, where the element starts: adding one hides a line, removing one
//!   shows it, and either is a change to what the reader sees. Classes and
//!   every other attribute say only how the page looks.
//!
//! What the stylesheet hides is not read here: a class the sheet hides would
//! change what is shown without changing this text. The consent page's tests
//! refuse a sheet rule that hides anything the page shows, so such a change
//! cannot ship unnoticed.

/// Elements whose contents never render as text on the page.
const UNRENDERED: [&str; 4] = ["style", "script", "template", "noscript"];

/// Elements that render inline: the words around them run together.
const INLINE: [&str; 26] = [
    "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "del", "dfn", "em", "i", "ins", "kbd",
    "label", "mark", "q", "s", "samp", "small", "span", "strong", "sub", "sup", "time", "u",
];

/// Attributes that hide an element or show it, whatever its classes.
const VISIBILITY_ATTRIBUTES: [&str; 5] = ["hidden", "aria-hidden", "inert", "open", "style"];

/// Marks a visibility attribute in the text, where no rendered word can.
const MARK: char = '\u{0}';

/// What `page` shows a reader, whitespace runs read as one space (see the
/// module documentation).
pub(super) fn visible_text(page: &str) -> String {
    let mut shown = String::with_capacity(page.len() / 4);
    let mut rest = page;
    while let Some(open) = rest.find('<') {
        decode_into(&mut shown, &rest[..open]);
        let markup = &rest[open..];
        if let Some(comment) = markup.strip_prefix("<!--") {
            shown.push(' ');
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        let Some(tag) = Tag::read(markup) else {
            // A `<` that opens no tag is a character of the text.
            shown.push('<');
            rest = &markup[1..];
            continue;
        };
        rest = &markup[tag.len..];
        if !INLINE.contains(&tag.name.as_str()) {
            shown.push(' ');
        }
        if tag.closing {
            continue;
        }
        for (name, value) in &tag.visibility {
            shown.push(MARK);
            shown.push_str(name);
            shown.push('=');
            shown.push_str(&value.split_whitespace().collect::<Vec<_>>().join(" "));
            shown.push(MARK);
        }
        if UNRENDERED.contains(&tag.name.as_str()) {
            let close = format!("</{}", tag.name);
            rest = rest
                .to_ascii_lowercase()
                .find(&close)
                .map_or("", |end| &rest[end..]);
        }
    }
    decode_into(&mut shown, rest);
    shown.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One tag, read from the `<` that opens it.
struct Tag {
    /// Its element name, lowercased; `!doctype` for the doctype.
    name: String,
    /// Whether it closes an element (`</p>`).
    closing: bool,
    /// Its visibility attributes, name lowercased, in order.
    visibility: Vec<(String, String)>,
    /// Bytes from its `<` through its `>`.
    len: usize,
}

impl Tag {
    /// The tag `markup` opens, or `None` when its `<` opens none: no name
    /// follows it.
    fn read(markup: &str) -> Option<Self> {
        let body = &markup[1..];
        let (closing, body) = body
            .strip_prefix('/')
            .map_or((false, body), |inner| (true, inner));
        let name_len = body
            .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
            .unwrap_or(body.len());
        let name = &body[..name_len];
        if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '!') {
            return None;
        }
        let (attributes, end) = attributes_of(&body[name_len..]);
        let consumed = markup.len() - body.len() + name_len + end;
        Some(Self {
            name: name.to_ascii_lowercase(),
            closing,
            visibility: attributes
                .into_iter()
                .filter(|(name, _)| VISIBILITY_ATTRIBUTES.contains(&name.as_str()))
                .collect(),
            len: consumed,
        })
    }
}

/// The attributes of a tag whose name is read, from `rest` (what follows
/// the name) through its `>`, and the bytes they span with the `>`. A `>`
/// inside a quoted value does not end the tag.
fn attributes_of(rest: &str) -> (Vec<(String, String)>, usize) {
    let mut attributes = Vec::new();
    let bytes = rest.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'>' => return (attributes, at + 1),
            b'/' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => at += 1,
            _ => {
                let start = at;
                while at < bytes.len() && !b" \t\n\r\x0c/>=".contains(&bytes[at]) {
                    at += 1;
                }
                let name = rest[start..at].to_ascii_lowercase();
                let mut value = String::new();
                if bytes.get(at) == Some(&b'=') {
                    at += 1;
                    if let Some(&quote @ (b'"' | b'\'')) = bytes.get(at) {
                        let from = at + 1;
                        let to = rest[from..]
                            .find(char::from(quote))
                            .map_or(rest.len(), |close| from + close);
                        decode_into(&mut value, &rest[from..to]);
                        at = (to + 1).min(rest.len());
                    } else {
                        let from = at;
                        while at < bytes.len() && !b" \t\n\r\x0c>".contains(&bytes[at]) {
                            at += 1;
                        }
                        decode_into(&mut value, &rest[from..at]);
                    }
                }
                attributes.push((name, value));
            }
        }
    }
    (attributes, rest.len())
}

/// Append `text` to `shown` with its character references decoded. A
/// reference that names no character is kept as written.
fn decode_into(shown: &mut String, text: &str) {
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        shown.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let reference = after
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| character_of(&after[..end]).map(|c| (c, end)));
        if let Some((character, end)) = reference {
            shown.push(character);
            rest = &after[end + 1..];
        } else {
            shown.push('&');
            rest = after;
        }
    }
    shown.push_str(rest);
}

/// The character a reference's name (between `&` and `;`) stands for.
fn character_of(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code);
    }
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_visible_text_drops_tags_comments_and_styles() {
        assert_eq!(
            visible_text(
                "<!-- note --><html><head><style>p { color: red }</style>\
                 <title>{{T}} - Dravr</title></head><body><p class=\"x\">\n  Hi   \
                 <b>there</b></p><script>var a = 1;</script></body></html>"
            ),
            "{{T}} - Dravr Hi there",
            "bug: tags, comments, styles and scripts are not read as text"
        );
    }

    /// Inline markup renders its words where they stand: a word wrapped in
    /// one reads as it did, the full stop after it included.
    #[test]
    fn an_inline_tag_joins_the_words_around_it() {
        let plain = visible_text("<p>Your fitness data.</p>");
        for marked in [
            "<p>Your fitness <strong>data</strong>.</p>",
            "<p>Your <em>fitness</em> data.</p>",
            "<p>Your fitness <a href=\"/x\">data</a>.</p>",
            "<p><span class=\"lead\">Your</span> fitness data.</p>",
        ] {
            assert_eq!(visible_text(marked), plain, "{marked}");
        }
        assert_ne!(
            visible_text("<p>Your fitness</p><p>data.</p>"),
            visible_text("<p>Your fitnessdata.</p>"),
            "a block separates the words around it"
        );
        assert_ne!(
            visible_text("<p>fit<span>ness</span></p>"),
            visible_text("<p>fit ness</p>"),
            "words an inline tag runs together read together"
        );
    }

    /// A reference reads as the character it names.
    #[test]
    fn a_character_reference_reads_as_its_character() {
        let plain = visible_text("<p>Data you share &amp; keep.</p>");
        for written in [
            "<p>Data&nbsp;you share &amp; keep.</p>",
            "<p>Data&#32;you share &#38; keep.</p>",
            "<p>Data&#x20;you share & keep.</p>",
        ] {
            assert_eq!(visible_text(written), plain, "{written}");
        }
        assert_eq!(
            visible_text("a &unknown; b"),
            "a &unknown; b",
            "bug: a reference naming no character is kept as written"
        );
        assert_eq!(
            visible_text("1 < 2"),
            "1 < 2",
            "bug: a `<` that opens no tag is a character of the text"
        );
    }

    /// Nothing inside an element that never renders is read.
    #[test]
    fn what_never_renders_is_not_read() {
        let plain = visible_text("<p>Shown.</p>");
        for hidden in [
            "<p>Shown.</p><template><p>Not shown.</p></template>",
            "<p>Shown.</p><noscript>Not shown.</noscript>",
            "<p>Shown.</p><SCRIPT>let x = '<p>';</SCRIPT>",
        ] {
            assert_eq!(visible_text(hidden), plain, "{hidden}");
        }
    }

    /// An attribute that hides an element, or shows one, is what the reader
    /// sees; a class or any other attribute is not.
    #[test]
    fn a_visibility_attribute_is_read_and_a_class_is_not() {
        let plain = visible_text("<div><p>Health data.</p></div>");
        for hiding in [
            "<div hidden><p>Health data.</p></div>",
            "<div><p aria-hidden=\"true\">Health data.</p></div>",
            "<div style=\"display:none\"><p>Health data.</p></div>",
            "<div inert><p>Health data.</p></div>",
        ] {
            assert_ne!(visible_text(hiding), plain, "{hiding}");
        }
        assert_ne!(
            visible_text("<div style='display: none'><p>a</p></div>"),
            visible_text("<div style='display: block'><p>a</p></div>"),
            "bug: a style's value is read, so changing it is a change"
        );
        assert_eq!(
            visible_text("<div class=\"x\" data-id=\"1\" title=\"t\"><p>Health data.</p></div>"),
            plain,
            "bug: a class and other attributes say only how the page looks"
        );
        // A `>` inside a quoted value does not end the tag.
        assert_eq!(
            visible_text("<div data-rule=\"a > b\"><p>Health data.</p></div>"),
            plain,
            "bug: a `>` inside a quoted value does not end the tag"
        );
        // An input of type `hidden` is a form field, not a hidden element.
        assert_eq!(
            visible_text("<input type=\"hidden\" value=\"x\"><div><p>Health data.</p></div>"),
            plain,
            "bug: a hidden input is a form field, not a hidden element"
        );
    }
}
