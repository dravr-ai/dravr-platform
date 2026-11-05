// ABOUTME: The range a provider's stated UTC offset must fall in to be stored on an activity
// ABOUTME: Civil zones run from -12:00 to +14:00, so anything outside is a bad reading, not a clock
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::ops::RangeInclusive;

/// Seconds east of UTC a civil clock can read: -12:00 to +14:00.
const UTC_OFFSETS: RangeInclusive<i64> = -43_200..=50_400;

/// A provider's stated UTC offset as an activity carries it, if it is civil.
///
/// `None` when the value is outside every civil zone: an activity that does
/// not say reads as today, while a wrong clock would be shown as the athlete's
/// own.
#[must_use]
pub fn stated_utc_offset(seconds: i64) -> Option<i32> {
    UTC_OFFSETS
        .contains(&seconds)
        .then(|| i32::try_from(seconds).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::stated_utc_offset;

    #[test]
    fn a_civil_offset_stands_and_the_range_ends_are_inclusive() {
        for seconds in [-43_200, -14_400, -12_600, 0, 19_800, 20_700, 50_400] {
            assert_eq!(
                stated_utc_offset(seconds),
                i32::try_from(seconds).ok(),
                "bug: a civil zone can have {seconds}"
            );
        }
    }

    #[test]
    fn an_offset_outside_every_civil_zone_is_no_offset() {
        for seconds in [-43_201, 50_401, 86_400, i64::MAX, i64::MIN] {
            assert_eq!(
                stated_utc_offset(seconds),
                None,
                "bug: no civil zone has {seconds}"
            );
        }
    }
}
