// ABOUTME: Strava's own active streak, read off the signed-in athlete's profile and carried on the athlete
// ABOUTME: A figure the provider reports, never one Dravr derives; a profile that reports none leaves it absent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's active streak, as Strava states it on their dashboard
//! (sciotte 2.1 reads it with the profile).
//!
//! It is Strava's own figure: nothing here derives it from activities, and
//! a streak Strava did not report is `None`, never zero days. It rides the
//! one profile read `get_athlete` makes; nothing reads the profile for it.

use dravr_sciotte::models::ActiveStreak as SciotteStreak;
use pierre_core::models::ActiveStreak;

/// Strava's streak as the platform's.
pub(super) fn convert(streak: SciotteStreak) -> ActiveStreak {
    ActiveStreak {
        length: streak.length,
        status: streak.status,
        valid_activity_count: streak.valid_activity_count,
    }
}

#[cfg(test)]
mod tests {
    use dravr_sciotte::models::ActiveStreak as SciotteStreak;

    use super::convert;

    #[test]
    fn strava_reports_cross_as_they_stand() {
        let out = convert(SciotteStreak {
            length: 21,
            status: "Ongoing".to_owned(),
            valid_activity_count: 640,
        });
        assert_eq!(
            (out.length, out.status.as_str(), out.valid_activity_count),
            (21, "Ongoing", 640),
            "bug: Strava's own figures were recomputed or reworded"
        );
    }
}
