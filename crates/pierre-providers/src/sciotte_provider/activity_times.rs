// ABOUTME: Splits a scraped activity's duration fields into elapsed time and moving time
// ABOUTME: A detail page reads moving time as its duration; the platform's duration is elapsed time
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_sciotte::models::Activity as SciotteActivity;

/// The activity's elapsed and moving time, in the platform's meaning:
/// `duration_seconds` is elapsed time, as every API provider reports it,
/// and moving time is `Some` only when the page showed both.
///
/// A detail page carries the moving time in `duration_seconds` and the
/// elapsed time beside it in `elapsed_time_seconds`. A list row carries
/// elapsed time alone, sometimes copied into both fields. Only when the
/// elapsed time is the longer of the two is the pair split; otherwise
/// `duration_seconds` stands and no moving time is claimed, since equal
/// figures cannot tell a non-stop activity from a copied one.
pub(super) fn elapsed_and_moving(sciotte: &SciotteActivity) -> (u64, Option<u64>) {
    match sciotte.elapsed_time_seconds {
        Some(elapsed) if elapsed > sciotte.duration_seconds && sciotte.duration_seconds > 0 => {
            (elapsed, Some(sciotte.duration_seconds))
        }
        _ => (sciotte.duration_seconds, None),
    }
}

/// The recording device the page named, trimmed; `None` when it named none.
pub(super) fn device_name(sciotte: &SciotteActivity) -> Option<String> {
    sciotte
        .device_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::super::{convert_activity, SciotteTarget};
    use super::SciotteActivity;

    /// A scraped Strava activity with the given duration fields.
    fn scraped(duration: u64, elapsed: Option<u64>) -> SciotteActivity {
        let mut payload = serde_json::json!({
            "id": "16001",
            "name": "Bûcheronner !",
            "sport_type": "workout",
            "start_date": "2026-11-07T13:00:00Z",
            "duration_seconds": duration,
            "provider": "strava",
            "workout_type": 3,
            "device_name": "  Garmin fēnix 6S Pro  "
        });
        if let Some(elapsed) = elapsed {
            payload["elapsed_time_seconds"] = serde_json::json!(elapsed);
        }
        serde_json::from_value(payload).unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    #[test]
    fn a_detail_page_splits_into_elapsed_duration_and_moving_time() {
        let activity = convert_activity(&scraped(4_080, Some(13_320)), SciotteTarget::Strava);
        assert_eq!(
            activity.duration_seconds(),
            13_320,
            "duration is elapsed time"
        );
        assert_eq!(
            activity.moving_time_seconds(),
            Some(4_080),
            "bug: the page's duration is the moving time"
        );
        assert_eq!(activity.workout_type(), Some(3), "bug: workout tag dropped");
        assert_eq!(
            activity.device_name(),
            Some("Garmin fēnix 6S Pro"),
            "bug: device name not trimmed or dropped"
        );
    }

    #[test]
    fn a_row_without_a_longer_elapsed_time_claims_no_moving_time() {
        for elapsed in [None, Some(4_080), Some(1_000)] {
            let activity = convert_activity(&scraped(4_080, elapsed), SciotteTarget::Strava);
            assert_eq!(activity.duration_seconds(), 4_080, "{elapsed:?}");
            assert_eq!(activity.moving_time_seconds(), None, "{elapsed:?}");
        }
    }
}
