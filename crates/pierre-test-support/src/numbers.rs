// ABOUTME: Finds a number in serialized text as a number of its own, never as digits inside a timestamp or id
// ABOUTME: Replaces `contains("<digits>")`, which timestamps' nanosecond fractions and UUID segments match by chance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Whole-number search over serialized replies.
//!
//! A reply that carries a clock (`2026-11-13T03:24:31.708009814+00:00`) or an
//! id (`3f9a-9001-…`) holds digit runs the test never wrote. A plain
//! `text.contains("31.7")` reads the seconds of that clock as the number, so a
//! negative check ("the athlete's 31.7 never reached the coach") fails whenever
//! the clock lands there, and a positive check passes for the wrong reason.
//! carnet#1821: it reddened `33.75`, `120` and `187` in one week.

/// Whether `number` appears in `text` as a number of its own.
///
/// A match is refused when it continues a longer token:
///
/// - a digit, letter, `_` or `.` before it, or a digit or `_` after it
///   (`1120` is not `120`; `…31.708` has no `708` of its own). A letter
///   after it is a unit (`7.2h`, `187km`) and does not continue the number;
/// - a `.` and a digit after a needle that has its own decimals (`31.75` is
///   not `31.7`). A whole-number needle still finds its figure written with
///   decimals (`42000` in `42000.0`), and a sentence's full stop ends the
///   number either way (`"HRV was 31.7."`);
/// - a `:` before it that follows a digit: the seconds of a clock
///   (`03:24:31.708`), while JSON's `"hrv":31.7` puts the colon after a quote;
/// - a `-` before it that follows a letter or digit (a date's or a UUID's
///   segment), and a `-` after it that precedes a digit (`2026-11`). A
///   needle that carries its own sign (`-77`) is refused after a digit or
///   letter for the same reason.
///
/// A range written without spaces (`50-60`) therefore reads as one token.
#[must_use]
pub fn mentions_number(text: &str, number: &str) -> bool {
    let bytes = text.as_bytes();
    let signed = number.starts_with('-');
    let decimal = number.contains('.');
    text.match_indices(number).any(|(start, _)| {
        let end = start + number.len();
        let before = start.checked_sub(1).map(|i| bytes[i]);
        let before_that = start.checked_sub(2).map(|i| bytes[i]);
        let continues_before = match before {
            Some(byte) if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' => true,
            Some(b':') => before_that.is_some_and(|byte| byte.is_ascii_digit()),
            Some(b'-') if !signed => before_that.is_some_and(|byte| byte.is_ascii_alphanumeric()),
            _ => false,
        };
        let after = bytes.get(end).copied();
        let continues_after = match after {
            Some(byte) if byte.is_ascii_digit() || byte == b'_' => true,
            Some(b'.') => decimal && bytes.get(end + 1).is_some_and(u8::is_ascii_digit),
            Some(b'-') => bytes.get(end + 1).is_some_and(u8::is_ascii_digit),
            _ => false,
        };
        !continues_before && !continues_after
    })
}

#[cfg(test)]
mod tests {
    use super::mentions_number;

    #[test]
    fn a_timestamps_seconds_are_not_the_number() {
        let text = r#"{"end":"2026-11-13T03:24:31.708009814+00:00"}"#;
        assert!(!mentions_number(text, "31.7"));
        assert!(!mentions_number(text, "31"));
        assert!(!mentions_number(text, "708009814"));
        assert!(!mentions_number(text, "13"));
        assert!(!mentions_number(text, "2026"));
        assert!(!mentions_number(text, "-11"));
    }

    #[test]
    fn a_longer_number_is_not_the_shorter_one() {
        assert!(!mentions_number(r#"{"value":31.75}"#, "31.7"));
        assert!(!mentions_number(r#"{"value":1120}"#, "120"));
        assert!(!mentions_number(r#"{"value":31.75}"#, "31.7"));
    }

    #[test]
    fn an_id_segment_is_not_the_number() {
        let text = r#"{"id":"3f9a2c10-9001-4b7e-8d2a-0c1f5e6b7a89"}"#;
        assert!(!mentions_number(text, "9001"));
        assert!(!mentions_number(text, "4"));
        assert!(!mentions_number(r#"{"id":"act_9001_x"}"#, "9001"));
    }

    #[test]
    fn a_number_of_its_own_is_found() {
        assert!(mentions_number(r#"{"hrv":31.7}"#, "31.7"));
        assert!(mentions_number(r#"{"values":[120,121]}"#, "120"));
        assert!(mentions_number(r#"{"distance":42000.0}"#, "42000"));
        assert!(mentions_number(r#"{"values":[119,120]}"#, "120"));
        assert!(mentions_number(r#"{"v":-77.5}"#, "-77.5"));
        assert!(mentions_number(r#"{"v":-77}"#, "-77"));
        assert!(mentions_number("Your HRV was 31.7.", "31.7"));
        assert!(mentions_number("a 42-day block", "42"));
        assert!(mentions_number("187 km", "187"));
        assert!(mentions_number("Sleep duration (7.2h) is short", "7.2"));
        assert!(mentions_number("(187)", "187"));
    }

    #[test]
    fn a_later_occurrence_is_found_past_an_earlier_false_one() {
        let text = r#"{"at":"2026-11-13T03:24:31.708Z","hrv":31.7}"#;
        assert!(mentions_number(text, "31.7"));
    }

    #[test]
    fn a_signed_needle_is_not_a_date_separator() {
        assert!(!mentions_number("2026-11-24", "-11"));
        assert!(!mentions_number("a-80", "-80"));
        assert!(mentions_number("[0, -80]", "-80"));
    }
}
