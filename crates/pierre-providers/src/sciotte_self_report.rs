// ABOUTME: The athlete's self-report on a scraped activity, routed to the scale of the platform it was scraped from
// ABOUTME: Picks the per-platform mapping (TrainingPeaks, COROS, Garmin, Strava) so a raw rank never reaches cageux
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Scraped self-report
//!
//! dravr-sciotte carries the athlete's perceived exertion and feel as each
//! platform encodes them; which encoding a row holds depends only on the
//! platform it was scraped from. This module picks the mapping, and each
//! platform's own module holds its scale and the sources for it.

use dravr_sciotte::models::Activity as SciotteActivity;

use crate::coros_self_report::feel_from_coros;
use crate::garmin_self_report::{feel_from_garmin, rpe_from_garmin};
use crate::models::Feel;
use crate::sciotte_provider::SciotteTarget;
use crate::strava_self_report::rpe_from_strava_label;
use crate::trainingpeaks_self_report::{feel_from_trainingpeaks, rpe_from_trainingpeaks};

/// The athlete's self-report on a scraped row as `(perceived exertion, feel)`
/// in the platform's scales: a 1–10 CR-10 rating and a named [`Feel`].
///
/// - `TrainingPeaks` states both: [`crate::trainingpeaks_self_report`].
/// - COROS states only the feeling: [`crate::coros_self_report`].
/// - Garmin Connect states both, the effort ×10 and the feel as a 0–100
///   score where 100 is the best: `garmin_self_report`.
/// - Strava states only the exertion, as the slider's band label ("Hard") or
///   its number; a band becomes its midpoint: `strava_self_report`. A feel
///   rank on a Strava row is no Strava rating and is ignored.
pub(crate) fn self_report(
    sciotte: &SciotteActivity,
    target: SciotteTarget,
) -> (Option<f32>, Option<Feel>) {
    let exertion = sciotte.perceived_exertion.as_deref();
    match target {
        SciotteTarget::TrainingPeaks => (
            exertion.and_then(rpe_from_trainingpeaks),
            sciotte.feel.and_then(feel_from_trainingpeaks),
        ),
        SciotteTarget::Coros => (None, sciotte.feel.and_then(feel_from_coros)),
        SciotteTarget::Garmin => (
            exertion.and_then(rpe_from_garmin),
            sciotte.feel.and_then(feel_from_garmin),
        ),
        SciotteTarget::Strava => (exertion.and_then(rpe_from_strava_label), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn scraped(provider: &str, report: &Value) -> SciotteActivity {
        let mut row = json!({
            "id": "a1",
            "name": "Session",
            "sport_type": "run",
            "start_date": "2026-10-15T07:30:00Z",
            "duration_seconds": 3_600,
            "provider": provider,
        });
        if let (Some(row), Some(report)) = (row.as_object_mut(), report.as_object()) {
            row.extend(report.clone());
        }
        serde_json::from_value(row).expect("a scraped activity")
    }

    #[test]
    fn a_garmin_row_carries_its_effort_over_ten_and_its_feel() {
        let row = scraped("garmin", &json!({ "perceived_exertion": "60", "feel": 75 }));
        assert_eq!(
            self_report(&row, SciotteTarget::Garmin),
            (Some(6.0), Some(Feel::Good))
        );
    }

    #[test]
    fn a_garmin_row_rated_very_weak_keeps_its_feel() {
        // 0 is Garmin's "Very Weak", a rating — not an absent one.
        let row = scraped("garmin", &json!({ "feel": 0 }));
        assert_eq!(
            self_report(&row, SciotteTarget::Garmin),
            (None, Some(Feel::Weak))
        );
    }

    #[test]
    fn a_garmin_row_off_its_scales_carries_nothing() {
        let row = scraped("garmin", &json!({ "perceived_exertion": "6", "feel": 200 }));
        assert_eq!(self_report(&row, SciotteTarget::Garmin), (None, None));
    }

    #[test]
    fn a_row_without_a_self_report_carries_nothing() {
        let row = scraped("garmin", &json!({}));
        for target in [
            SciotteTarget::Garmin,
            SciotteTarget::Strava,
            SciotteTarget::TrainingPeaks,
            SciotteTarget::Coros,
        ] {
            assert_eq!(self_report(&row, target), (None, None), "{target:?}");
        }
    }

    #[test]
    fn a_strava_band_label_becomes_its_midpoint_and_no_feel() {
        let row = scraped(
            "strava",
            &json!({ "perceived_exertion": "Moderate", "feel": 1 }),
        );
        assert_eq!(self_report(&row, SciotteTarget::Strava), (Some(5.0), None));
    }

    #[test]
    fn a_strava_number_is_read_as_the_rating() {
        let row = scraped("strava", &json!({ "perceived_exertion": "7" }));
        assert_eq!(self_report(&row, SciotteTarget::Strava), (Some(7.0), None));
    }

    #[test]
    fn a_strava_label_off_the_slider_is_no_rating() {
        for label in ["Somewhat hard", "", "0", "12"] {
            let row = scraped("strava", &json!({ "perceived_exertion": label }));
            assert_eq!(
                self_report(&row, SciotteTarget::Strava),
                (None, None),
                "{label:?}"
            );
        }
    }
}
