// ABOUTME: The one doubling backoff: a first wait doubled per earlier failure, never past a ceiling
// ABOUTME: Pure and saturating: every retry ledger spells one schedule from its own constants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The wait before a failed job's next attempt.
//!
//! Every retry ledger here holds a failed row for a wait that doubles with
//! each failure and stops at a ceiling. [`doubling_backoff`] is that rule;
//! a ledger brings its first wait, its ceiling and how many failures came
//! before the one it is settling.

use std::time::Duration;

/// The wait after a failure that `earlier_failures` others came before:
/// `first` for the first one, doubled for each earlier failure, never past
/// `ceiling`.
///
/// Saturating throughout: a count no schedule reaches gives `ceiling`, and a
/// negative one, which only a damaged counter holds, reads as zero. A ledger
/// with no ceiling of its own passes [`Duration::MAX`].
#[must_use]
pub fn doubling_backoff(first: Duration, ceiling: Duration, earlier_failures: i64) -> Duration {
    let doublings = u32::try_from(earlier_failures.max(0)).unwrap_or(u32::MAX);
    first
        .saturating_mul(2_u32.saturating_pow(doublings))
        .min(ceiling)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: Duration = Duration::from_secs(60);
    const CEILING: Duration = Duration::from_secs(3600);

    #[test]
    fn the_first_failure_waits_the_first_wait() {
        assert_eq!(doubling_backoff(FIRST, CEILING, 0), FIRST);
    }

    #[test]
    fn each_earlier_failure_doubles_the_wait() {
        assert_eq!(doubling_backoff(FIRST, CEILING, 1), FIRST * 2);
        assert_eq!(doubling_backoff(FIRST, CEILING, 2), FIRST * 4);
        assert_eq!(doubling_backoff(FIRST, CEILING, 5), FIRST * 32);
    }

    #[test]
    fn the_wait_stops_at_its_ceiling() {
        assert_eq!(doubling_backoff(FIRST, CEILING, 6), CEILING);
        assert_eq!(doubling_backoff(FIRST, CEILING, 31), CEILING);
        assert_eq!(doubling_backoff(FIRST, CEILING, 32), CEILING);
        assert_eq!(doubling_backoff(FIRST, CEILING, i64::MAX), CEILING);
    }

    #[test]
    fn a_ceiling_under_the_first_wait_wins() {
        let ceiling = Duration::from_secs(10);
        assert_eq!(doubling_backoff(FIRST, ceiling, 0), ceiling);
    }

    #[test]
    fn a_negative_count_reads_as_no_earlier_failure() {
        assert_eq!(doubling_backoff(FIRST, CEILING, -1), FIRST);
        assert_eq!(doubling_backoff(FIRST, CEILING, i64::MIN), FIRST);
    }

    #[test]
    fn nothing_overflows_without_a_ceiling() {
        assert_eq!(doubling_backoff(FIRST, Duration::MAX, 16), FIRST * 65_536);
        assert_eq!(
            doubling_backoff(Duration::MAX, Duration::MAX, i64::MAX),
            Duration::MAX
        );
    }
}
