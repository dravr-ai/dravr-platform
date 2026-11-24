// ABOUTME: The ISO 8601 durations a grant's authorization details carry, read and written as exact seconds
// ABOUTME: Weeks, days, hours, minutes and seconds only: a year or a month has no fixed length to narrow by
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! ISO 8601 durations (`P30D`, `PT1H`, `P1W2DT3H`).
//!
//! A lifetime narrows a grant's expiry, which is compared second for second,
//! so only the units of a fixed length are read: weeks, days, hours, minutes
//! and seconds. `P1M` or `P1Y` is refused rather than guessed at.

use pierre_core::constants::time_constants;

/// Seconds in a day, signed as a duration's total is.
pub(super) const SECONDS_PER_DAY: i64 = time_constants::SECONDS_PER_DAY as i64;

/// Seconds in a week.
const SECONDS_PER_WEEK: i64 = time_constants::SECONDS_PER_WEEK as i64;

/// Seconds in an hour.
const SECONDS_PER_HOUR: i64 = time_constants::SECONDS_PER_HOUR as i64;

/// Read `text` as an ISO 8601 duration of fixed-length units, in seconds.
/// `None` for anything else: an empty or zero duration, a year or month part,
/// a fraction, a sign, or a value past `i64` seconds.
pub(super) fn parse_seconds(text: &str) -> Option<i64> {
    let rest = text.strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((date, time)) if !time.is_empty() => (date, Some(time)),
        Some(_) => return None,
        None => (rest, None),
    };
    let mut total: i64 = 0;
    let mut any = false;
    for (part, units) in [
        (date, &[('W', SECONDS_PER_WEEK), ('D', SECONDS_PER_DAY)][..]),
        (
            time.unwrap_or_default(),
            &[('H', SECONDS_PER_HOUR), ('M', 60), ('S', 1)][..],
        ),
    ] {
        let (seconds, read) = sum_units(part, units)?;
        total = total.checked_add(seconds)?;
        any |= read;
    }
    (any && total > 0).then_some(total)
}

/// The seconds `part` spells in `units`, each at most once and in order, and
/// whether it named any; `None` when it spells anything else.
fn sum_units(part: &str, units: &[(char, i64)]) -> Option<(i64, bool)> {
    let mut total: i64 = 0;
    let mut rest = part;
    let mut next_unit = 0;
    let mut read = false;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit())?;
        if digits == 0 {
            return None;
        }
        let (number, tail) = rest.split_at(digits);
        let mut chars = tail.chars();
        let designator = chars.next()?;
        let position = units
            .iter()
            .skip(next_unit)
            .position(|(unit, _)| *unit == designator)?;
        let (_, scale) = units.get(next_unit + position)?;
        next_unit += position + 1;
        let value: i64 = number.parse().ok()?;
        total = total.checked_add(value.checked_mul(*scale)?)?;
        read = true;
        rest = chars.as_str();
    }
    Some((total, read))
}

/// `seconds` as the duration the stored document carries: whole days as
/// `P{n}D`, anything else as `PT{n}S`, so one lifetime has one spelling.
pub(super) fn format_seconds(seconds: i64) -> String {
    if seconds % SECONDS_PER_DAY == 0 {
        format!("P{}D", seconds / SECONDS_PER_DAY)
    } else {
        format!("PT{seconds}S")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_length_units_read_as_seconds() {
        assert_eq!(
            parse_seconds("P30D"),
            Some(30 * SECONDS_PER_DAY),
            "bug: days read as the wrong number of seconds"
        );
        assert_eq!(
            parse_seconds("PT1H"),
            Some(3_600),
            "bug: an hour read as the wrong number of seconds"
        );
        assert_eq!(
            parse_seconds("P1W"),
            Some(7 * SECONDS_PER_DAY),
            "bug: a week read as the wrong number of seconds"
        );
        assert_eq!(
            parse_seconds("P1W2DT3H4M5S"),
            Some(9 * SECONDS_PER_DAY + 3 * 3_600 + 4 * 60 + 5),
            "bug: a duration mixing every fixed unit summed wrong"
        );
        assert_eq!(
            parse_seconds("PT90M"),
            Some(5_400),
            "bug: minutes past an hour summed wrong"
        );
    }

    #[test]
    fn anything_else_is_not_a_duration() {
        for text in [
            "",
            "P",
            "PT",
            "P0D",
            "PT0S",
            "P1M",
            "P1Y",
            "P1.5D",
            "-P1D",
            "P1DT",
            "1D",
            "P1H",
            "PT1D",
            "P2D1W",
            "P1D1D",
            "PTH",
            "P99999999999999999999D",
        ] {
            assert_eq!(
                parse_seconds(text),
                None,
                "bug: {text:?} read as a duration"
            );
        }
    }

    #[test]
    fn a_lifetime_has_one_spelling() {
        assert_eq!(
            format_seconds(30 * SECONDS_PER_DAY),
            "P30D",
            "bug: whole days are not spelled in days"
        );
        assert_eq!(
            format_seconds(3_600),
            "PT3600S",
            "bug: a part day is not spelled in seconds"
        );
        assert_eq!(
            parse_seconds(&format_seconds(3_600)),
            Some(3_600),
            "bug: a lifetime's spelling does not read back"
        );
    }
}
