// ABOUTME: How often the coach checks in with an athlete on its own — the cadence the athlete chose (carnet#831)
// ABOUTME: Daily, three times a week, weekly or off; the local mornings each posts on, and the quiet hours that hold it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The check-in cadence.
//!
//! The athlete chooses how often the coach writes to them first: asked during
//! onboarding on every surface, and changeable in Settings. `Off` is a real
//! answer, never a default overridden later: an athlete who chose it, or who
//! never chose, is never checked in on.
//!
//! A cadence posts on fixed weekdays of the athlete's own calendar, so the
//! spacing is the calendar's: every day, Monday-Wednesday-Friday, or Monday.
//! On such a day the check-in is due from [`CHECK_IN_HOUR`] local time for
//! [`CHECK_IN_WINDOW_HOURS`] hours — a morning note, never a late-evening one
//! for a morning the platform missed — and held while the athlete's quiet
//! hours cover the moment ([`within_quiet_hours`]). The routine engine applies
//! both: the cadence is the athlete's check-in routine
//! ([`super::routine::Schedule::of_check_in`]), its window is the routine's
//! (ADR-036).

use dravr_tronc::dates::{CivilTime, Weekday};
use serde::{Deserialize, Serialize};

/// The local hour a check-in becomes due on a day its cadence posts on.
pub const CHECK_IN_HOUR: u32 = 8;

/// How many hours after [`CHECK_IN_HOUR`] a due check-in may still go out.
/// Past it the day's check-in is skipped rather than sent late.
pub const CHECK_IN_WINDOW_HOURS: u32 = 3;

/// How often the coach checks in with the athlete on its own initiative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckInCadence {
    /// Every morning.
    Daily,
    /// Monday, Wednesday and Friday mornings.
    ThreePerWeek,
    /// Monday mornings.
    Weekly,
    /// Never: the athlete starts every conversation.
    Off,
}

impl CheckInCadence {
    /// Every cadence, in the order the choice is offered.
    pub const ALL: [Self; 4] = [Self::Daily, Self::ThreePerWeek, Self::Weekly, Self::Off];

    /// The wire and stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::ThreePerWeek => "three_per_week",
            Self::Weekly => "weekly",
            Self::Off => "off",
        }
    }

    /// Read the wire spelling back; anything else is `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|cadence| cadence.as_str() == value)
    }

    /// Whether a check-in is due on `weekday` of the athlete's calendar.
    #[must_use]
    pub const fn posts_on(self, weekday: Weekday) -> bool {
        match self {
            Self::Daily => true,
            Self::ThreePerWeek => matches!(weekday, Weekday::Mon | Weekday::Wed | Weekday::Fri),
            Self::Weekly => matches!(weekday, Weekday::Mon),
            Self::Off => false,
        }
    }
}

/// Whether `at` falls inside the quiet hours `start..end`, on one clock.
///
/// A window whose end is earlier than its start runs over midnight; one whose
/// ends are equal is empty. The rule the notification pipeline suppresses a
/// push by, so a check-in is held exactly when a push would be.
#[must_use]
pub fn within_quiet_hours(start: CivilTime, end: CivilTime, at: CivilTime) -> bool {
    if start <= end {
        at >= start && at < end
    } else {
        at >= start || at < end
    }
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Elapsed;

    use super::*;

    const WEEK: [Weekday; 7] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];

    fn days(cadence: CheckInCadence) -> Vec<Weekday> {
        WEEK.into_iter()
            .filter(|day| cadence.posts_on(*day))
            .collect()
    }

    #[test]
    fn each_cadence_posts_on_its_own_weekdays() {
        assert_eq!(days(CheckInCadence::Daily), WEEK);
        assert_eq!(
            days(CheckInCadence::ThreePerWeek),
            [Weekday::Mon, Weekday::Wed, Weekday::Fri]
        );
        assert_eq!(days(CheckInCadence::Weekly), [Weekday::Mon]);
        assert!(days(CheckInCadence::Off).is_empty(), "off never posts");
    }

    fn time(hour: u32, minute: u32) -> CivilTime {
        CivilTime::MIDNIGHT + Elapsed::hours(i64::from(hour)) + Elapsed::minutes(i64::from(minute))
    }

    #[test]
    fn quiet_hours_hold_inside_their_window_and_across_midnight() {
        // 22:00 to 09:00 runs over midnight and covers the morning hour.
        let (start, end) = (time(22, 0), time(9, 0));
        assert!(within_quiet_hours(start, end, time(8, 30)));
        assert!(within_quiet_hours(start, end, time(23, 15)));
        assert!(
            !within_quiet_hours(start, end, time(9, 0)),
            "the end is open"
        );
        assert!(!within_quiet_hours(start, end, time(12, 0)));
        // 13:00 to 15:00 within one day leaves the morning free.
        assert!(!within_quiet_hours(time(13, 0), time(15, 0), time(8, 30)));
        assert!(within_quiet_hours(time(13, 0), time(15, 0), time(13, 0)));
        // Equal ends hold nothing.
        assert!(!within_quiet_hours(time(8, 0), time(8, 0), time(8, 0)));
    }

    #[test]
    fn the_wire_spelling_reads_back_and_nothing_else_does() {
        for cadence in CheckInCadence::ALL {
            assert_eq!(CheckInCadence::parse(cadence.as_str()), Some(cadence));
            assert_eq!(
                serde_json::to_string(&cadence).ok(),
                Some(format!("\"{}\"", cadence.as_str()))
            );
        }
        assert_eq!(CheckInCadence::parse("Daily"), None);
        assert_eq!(CheckInCadence::parse("monthly"), None);
        assert_eq!(CheckInCadence::parse(""), None);
    }
}
