// ABOUTME: TrainingPeaks athlete self-report mapping — the feeling rank (1 is the best face) and RPE as its decimal string
// ABOUTME: Pure functions the sciotte conversion calls so a TrainingPeaks encoding never leaves this module
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `TrainingPeaks` athlete self-report
//!
//! What the athlete said about a workout, as opposed to what the sensors
//! recorded: the "How did you feel?" rating (`feeling` on the wire) and the
//! rating of perceived exertion (`rpe`). The scraper carries both as
//! `TrainingPeaks` encodes them — the feeling rank unconverted, the RPE as its
//! decimal string — and this module turns them into the platform's scales.
//!
//! ## Feeling: 0–10 on the wire, five faces, **1 is the best**
//!
//! The athlete picks one of five faces. The wire field is an integer the API
//! documents as 0–10, and the faces are written at the odd values, ordered
//! from best to worst:
//!
//! | wire | face (`TrainingPeaks` label) | [`Feel`] |
//! |------|------------------------------|----------|
//! | 1    | Very Strong                  | [`Feel::Strong`] |
//! | 3    | Strong                       | [`Feel::Good`] |
//! | 5    | Normal                       | [`Feel::Normal`] |
//! | 7    | Weak                         | [`Feel::Poor`] |
//! | 9    | Very Weak                    | [`Feel::Weak`] |
//!
//! Both scales have five steps ordered best to worst, so each face maps to
//! the step at the same position; the names differ because `TrainingPeaks`
//! spends two of its five on "very". Sources, which agree:
//!
//! - The `TrainingPeaks` Partners API documentation, `Workouts-Object`
//!   (`Feeling | int | scale of 0 to 10 | 1:strong - 3 - 5:normal - 7 -
//!   9:weak`) and `Strength-Workouts-Object` (`Feel`, the same wording),
//!   github.com/TrainingPeaks/PartnersAPI wiki @ `0f6e586f` (2026-09-22).
//! - The `TrainingPeaks` web app itself, release `20260922-47d20c0`: the
//!   feeling picker offers `values: [9, 7, 5, 3, 1]` labelled Very Weak …
//!   Very Strong, and the workout view names the ranks
//!   `{1: VeryStrong, 3: Strong, 5: Normal, 7: Weak, 9: VeryWeak}` (bundles
//!   `modules-79c78f06.js`, `modules-ec3328c5.js`, `modules-f765ba67.js`).
//! - The help centre's "`TrainingPeaks` Announces Subjective Feedback Feature"
//!   and "What are RPE and Subjective Feedback?": five faces ranging from
//!   strong to weak, labelled from "Very weak" through "Normal" to "Very
//!   strong".
//! - The 0–10 range is also what open-source clients validate against
//!   (`JamsusMaximus/trainingpeaks-mcp` @ `a412a84e`, `_validation.py`;
//!   `nagelflorian/trainingpeaks-mcp-server` @ `f1ec1f39`, `workouts.ts`),
//!   though neither states a direction.
//!
//! A rank that is not one of the five faces (0, an even value, anything past
//! 10) is not a rating the app lets an athlete give, and the app shows no
//! face for it, so it maps to `None` rather than to a neighbouring face. One
//! open-source client (`banananovej-chuan/tp-mcp-server`, `workout.py`)
//! renders the rank as 1–5 with 5 the best; that contradicts both the range
//! and the direction the vendor's API documentation and app state, and is
//! not followed.
//!
//! ## RPE: 1–10
//!
//! The API documents `Rpe` as "Rated perceived exertion scale of 1 to 10",
//! the same range as the platform's CR-10 rating. The scraper writes it as
//! its decimal string (`"6"`); a value that does not parse, or falls outside
//! 1–10, is not a rating and maps to `None`. The web app's level table also
//! names 0 ("No Exertion"), which the CR-10 field does not hold.

use crate::models::Feel;

/// Map `TrainingPeaks`' `feeling` rank onto the platform's named scale.
///
/// `TrainingPeaks` writes the five faces at 1, 3, 5, 7 and 9 with **1 as the
/// best** (Very Strong) and 9 the worst (Very Weak) — see the module
/// documentation for the sources. The rank never leaves this function: it
/// becomes a [`Feel`] that names the rating, so no consumer can read it
/// backwards. Any other rank is not a face and maps to `None`.
#[must_use]
pub const fn feel_from_trainingpeaks(rank: u8) -> Option<Feel> {
    match rank {
        1 => Some(Feel::Strong),
        3 => Some(Feel::Good),
        5 => Some(Feel::Normal),
        7 => Some(Feel::Poor),
        9 => Some(Feel::Weak),
        _ => None,
    }
}

/// `TrainingPeaks`' RPE, carried as its decimal string, as a CR-10 rating, or
/// `None` when the string is not a number inside 1–10.
#[must_use]
pub fn rpe_from_trainingpeaks(rpe: &str) -> Option<f32> {
    let rating: f32 = rpe.trim().parse().ok()?;
    (1.0..=10.0).contains(&rating).then_some(rating)
}
