// ABOUTME: Pins that the sciotte mapper fills every field each mirror descriptor declares, list row and detail
// ABOUTME: Fixtures carry the keys the pinned client's extracts write for Strava, Garmin, TrainingPeaks and COROS
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The fixtures name only keys the pinned scraper's page extracts write
//! (`providers/<target>.toml` at the pinned tag), a list row's apart from a
//! detail page's, so a declaration cannot claim a field the scrape never
//! reads.

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::super::{convert_activity, SciotteActivity, SciotteTarget};
    use crate::field_support::{tests::assert_mapper_fills, ActivityFieldSupport};
    use crate::spi::{
        ProviderDescriptor, SciotteCorosDescriptor, SciotteDescriptor, SciotteGarminDescriptor,
        SciotteTrainingPeaksDescriptor,
    };

    /// A scraped activity: the identity every row carries, then `fields`.
    fn scraped(provider: &str, fields: &Value) -> SciotteActivity {
        let mut row = json!({
            "id": "16001",
            "name": "Session",
            "sport_type": "ride",
            "start_date": "2026-11-07T13:00:00Z",
            "duration_seconds": 3_600,
            "distance_meters": 30_000.0,
            "provider": provider,
        });
        if let (Some(row), Some(fields)) = (row.as_object_mut(), fields.as_object()) {
            row.extend(fields.clone());
        }
        serde_json::from_value(row).unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    /// Merge `more` over `base`.
    fn with(base: &Value, more: &Value) -> Value {
        let mut merged = base.clone();
        if let (Some(merged), Some(more)) = (merged.as_object_mut(), more.as_object()) {
            merged.extend(more.clone());
        }
        merged
    }

    /// The detail page's route: a track with its elevations.
    fn route() -> Value {
        json!({
            "coordinates": [[45.5, -73.6], [45.5001, -73.6001]],
            "altitudes_meters": [44.0, 44.2]
        })
    }

    /// One lap and one split as the detail extracts write them.
    fn laps_and_splits() -> Value {
        json!({
            "laps": [{ "index": 1, "distance_meters": 30_000.0, "elapsed_time_seconds": 3_900 }],
            "splits": [{ "index": 1, "distance_meters": 1_000.0, "elapsed_time_seconds": 120 }]
        })
    }

    fn check(
        target: SciotteTarget,
        backend: &str,
        declared: ActivityFieldSupport,
        list: &Value,
        detail: &Value,
    ) {
        let provider = target.scraper_provider_name();
        assert_mapper_fills(
            backend,
            declared,
            &convert_activity(&scraped(provider, list), target),
            &convert_activity(&scraped(provider, &with(list, detail)), target),
        );
    }

    /// The recorded streams a Strava detail read answers, every channel on
    /// one time axis.
    fn streams() -> Value {
        json!({
            "timestamps": [0, 1],
            "heart_rate": [128, 131],
            "power": [190, null],
            "cadence": [84, 85],
            "speed": [8.1, 8.3],
            "altitude": [44.0, 44.2],
            "temperature": [12.0, 12.0],
            "gps_coordinates": [[45.5, -73.6], [45.5001, -73.6001]],
            "distance": [0.0, 8.2]
        })
    }

    #[test]
    fn the_strava_scrape_fills_what_its_descriptor_declares() {
        // The list feed carries the athlete's words and tags and the heart
        // rate; the detail page's stat block, its embedded activity values
        // and its streams the rest.
        let list = json!({
            "suffer_score": 42,
            "elevation_gain": 300.0,
            "average_heart_rate": 141,
            "moving_time_seconds": 3_300,
            "description": "Sortie test, vent de face",
            "workout_type": 11,
            "commute": false,
            "trainer": false
        });
        let detail = with(
            &json!({
                "elapsed_time_seconds": 3_900,
                "max_heart_rate": 176,
                "average_power": 196,
                "average_cadence": 84,
                "calories": 980,
                "device_name": "Garmin Edge 840",
                "gear_name": "Gravel (2 140 km)",
                "perceived_exertion": "Moderate",
                "private_note": "Note test: selle à remonter",
                "route": route(),
                "streams": streams()
            }),
            &laps_and_splits(),
        );
        check(
            SciotteTarget::Strava,
            "sciotte",
            SciotteDescriptor.activity_fields(),
            &list,
            &detail,
        );
    }

    /// The athlete's own words cross from Strava alone: the same keys from
    /// another target, whose description is no athlete's account (on
    /// TrainingPeaks, the coach's prescription), are left behind.
    #[test]
    fn only_strava_carries_the_athletes_description_and_private_note() {
        let words = json!({
            "description": "Texte test",
            "private_note": "Note test"
        });
        let strava = convert_activity(
            &scraped(SciotteTarget::Strava.scraper_provider_name(), &words),
            SciotteTarget::Strava,
        );
        assert_eq!(
            strava.description(),
            Some("Texte test"),
            "bug: a Strava capture lost the athlete's description"
        );
        assert_eq!(
            strava.private_note(),
            Some("Note test"),
            "bug: a Strava capture lost the athlete's private note"
        );
        for target in [
            SciotteTarget::Garmin,
            SciotteTarget::TrainingPeaks,
            SciotteTarget::Coros,
        ] {
            let other = convert_activity(&scraped(target.scraper_provider_name(), &words), target);
            assert!(
                other.description().is_none() && other.private_note().is_none(),
                "bug: {target:?}'s text crossed as the athlete's own words"
            );
        }
    }

    #[test]
    fn the_garmin_scrape_fills_what_its_descriptor_declares() {
        let list = json!({
            "average_heart_rate": 141,
            "max_heart_rate": 176,
            "calories": 980,
            "average_cadence": 84,
            "average_power": 196,
            "normalized_power": 214
        });
        let detail = with(
            &json!({
                "elapsed_time_seconds": 3_900,
                "device_name": "Edge 840",
                "perceived_exertion": "60",
                "route": route()
            }),
            &laps_and_splits(),
        );
        check(
            SciotteTarget::Garmin,
            "sciotte_garmin",
            SciotteGarminDescriptor.activity_fields(),
            &list,
            &detail,
        );
    }

    #[test]
    fn the_trainingpeaks_scrape_fills_what_its_descriptor_declares() {
        let list = json!({
            "average_heart_rate": 141,
            "max_heart_rate": 176,
            "calories": 980,
            "average_cadence": 84,
            "average_power": 196,
            "normalized_power": 214,
            "perceived_exertion": "7",
            "comments": [
                { "author": "coach", "text": "Bien géré" },
                { "author": "athlete", "text": "Jambes lourdes" }
            ]
        });
        check(
            SciotteTarget::TrainingPeaks,
            "sciotte_trainingpeaks",
            SciotteTrainingPeaksDescriptor.activity_fields(),
            &list,
            &json!({}),
        );
    }

    #[test]
    fn the_coros_scrape_fills_what_its_descriptor_declares() {
        let list = json!({
            "average_heart_rate": 141,
            "calories": 980,
            "average_cadence": 84,
            "average_power": 196,
            "normalized_power": 214,
            "device_name": "PACE 3"
        });
        let detail = json!({
            "elapsed_time_seconds": 3_900,
            "max_heart_rate": 176,
            "comments": [{ "author": "athlete", "text": "Vent de face" }],
            "laps": [{ "index": 1, "distance_meters": 30_000.0, "elapsed_time_seconds": 3_900 }],
            "route": route()
        });
        check(
            SciotteTarget::Coros,
            "sciotte_coros",
            SciotteCorosDescriptor.activity_fields(),
            &list,
            &detail,
        );
    }
}
