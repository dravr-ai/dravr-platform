// ABOUTME: Garmin Connect athlete self-report mapping — RPE written ×10 (10..100) and the 0..100 "How did you feel?" score
// ABOUTME: Pure functions the sciotte conversion calls so a Garmin encoding never leaves this module
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Garmin Connect athlete self-report
//!
//! After an activity the watch (or Connect) asks for a self-evaluation: a
//! perceived effort and how the athlete felt (Garmin manuals, "Evaluating an
//! Activity"). Garmin Connect's activity JSON keeps both in `summaryDTO`:
//! `directWorkoutRpe` and `directWorkoutFeel`. dravr-sciotte carries them
//! unconverted — the RPE as its decimal string, the feel as its rank — and
//! this module turns them into the platform's scales.
//!
//! ## Perceived effort: 10–100, the CR-10 rating ×10
//!
//! Connect stores the 1–10 effort the athlete picked multiplied by ten. Its own
//! labels for the stored values are Borg's CR-10 anchors ×10 — 10 Very Light,
//! 20 Light, 30 Moderate, 40 Somewhat Hard, 50 Hard, 70 Very Hard, 90
//! Extremely Hard, 100 Maximum (`tcgoetz/GarminDB` @ `06d9d524`,
//! `garmindb/garmin_json_data.py`, `get_self_eval_effort`) — so dividing by ten
//! is exact, not an approximation. Other clients read it the same way
//! (`paufregi/GarminConnectFeed` @ `b0598d20`, `Summary.kt`: "10.0-100.0 (step
//! 10.0)"; `bpauli/gccli` @ `091377ba`, `activity_fields.go`). A value that does
//! not parse, or falls outside 10–100, is not a rating and maps to `None`.
//!
//! ## Feel: 0–100 in five steps, **100 is the best**
//!
//! | wire | Connect label | [`Feel`] |
//! |------|---------------|----------|
//! | 100  | Very Strong   | [`Feel::Strong`] |
//! | 75   | Strong        | [`Feel::Good`] |
//! | 50   | Normal        | [`Feel::Normal`] |
//! | 25   | Weak          | [`Feel::Poor`] |
//! | 0    | Very Weak     | [`Feel::Weak`] |
//!
//! The steps and labels are GarminDB's `get_self_eval_feel` (same file as
//! above), and writers store exactly those five values (`tamcore/garmin-mcp` @
//! `a18a5860`, `activitywrite.go`, "feel must be 0, 25, 50, 75 or 100"). The
//! labels are the five `TrainingPeaks` uses, mapped to the same variants
//! ([`crate::trainingpeaks_self_report`]), but the direction is the reverse of
//! `TrainingPeaks` and COROS: here the larger number is the better day. A
//! score between steps goes to the nearest one (0–12, 13–37, 38–62, 63–87,
//! 88–100); past 100 it is not a score and maps to `None`.

use crate::models::Feel;

/// Garmin's `directWorkoutRpe`, carried as its decimal string, as a CR-10
/// rating, or `None` when the string is not a number inside 10–100.
#[must_use]
pub fn rpe_from_garmin(rpe: &str) -> Option<f32> {
    let stored: f32 = rpe.trim().parse().ok()?;
    (10.0..=100.0).contains(&stored).then(|| stored / 10.0)
}

/// Map Garmin's `directWorkoutFeel` score onto the platform's named scale:
/// the nearest of its five steps, with **100 as the best** (Very Strong). Any
/// score past 100 maps to `None`.
#[must_use]
pub const fn feel_from_garmin(score: u8) -> Option<Feel> {
    match score {
        0..=12 => Some(Feel::Weak),
        13..=37 => Some(Feel::Poor),
        38..=62 => Some(Feel::Normal),
        63..=87 => Some(Feel::Good),
        88..=100 => Some(Feel::Strong),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_effort_is_the_rating_times_ten() {
        assert_eq!(rpe_from_garmin("10"), Some(1.0));
        assert_eq!(rpe_from_garmin("60"), Some(6.0));
        assert_eq!(rpe_from_garmin("100"), Some(10.0));
        assert_eq!(rpe_from_garmin(" 70 "), Some(7.0));
        assert_eq!(rpe_from_garmin("65"), Some(6.5));
        assert_eq!(rpe_from_garmin("60.0"), Some(6.0));
    }

    #[test]
    fn an_effort_that_is_not_a_stored_rating_is_none() {
        for rpe in ["", "0", "5", "9", "101", "110", "-60", "Hard", "NaN", "inf"] {
            assert_eq!(rpe_from_garmin(rpe), None, "{rpe:?}");
        }
    }

    #[test]
    fn each_feel_step_maps_to_the_feel_at_its_position() {
        // 100 is Garmin's best (Very Strong), 0 its worst (Very Weak).
        assert_eq!(feel_from_garmin(100), Some(Feel::Strong));
        assert_eq!(feel_from_garmin(75), Some(Feel::Good));
        assert_eq!(feel_from_garmin(50), Some(Feel::Normal));
        assert_eq!(feel_from_garmin(25), Some(Feel::Poor));
        assert_eq!(feel_from_garmin(0), Some(Feel::Weak));
    }

    #[test]
    fn a_score_between_steps_goes_to_the_nearest() {
        assert_eq!(feel_from_garmin(12), Some(Feel::Weak));
        assert_eq!(feel_from_garmin(13), Some(Feel::Poor));
        assert_eq!(feel_from_garmin(37), Some(Feel::Poor));
        assert_eq!(feel_from_garmin(38), Some(Feel::Normal));
        assert_eq!(feel_from_garmin(62), Some(Feel::Normal));
        assert_eq!(feel_from_garmin(63), Some(Feel::Good));
        assert_eq!(feel_from_garmin(87), Some(Feel::Good));
        assert_eq!(feel_from_garmin(88), Some(Feel::Strong));
    }

    #[test]
    fn a_score_past_the_scale_is_not_a_feel() {
        for score in [101, 128, u8::MAX] {
            assert_eq!(feel_from_garmin(score), None, "{score}");
        }
    }
}
