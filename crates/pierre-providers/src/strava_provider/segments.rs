// ABOUTME: Strava segment efforts — the detail read's efforts mapped to cageux, and one segment's efforts listed
// ABOUTME: A detail read always stamps its efforts list, so a row read through the API reads as segment-read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava's segment efforts, mapped to cageux's [`SegmentEffort`].
//!
//! A detail read sends `include_all_efforts=true`, so its answer holds every
//! effort the activity has: an absent or empty array is Strava's word that
//! the activity crossed no segment, and the mapper stamps it with an empty
//! list. That stamp is what marks a stored row's segments as read.

use chrono::{DateTime, Utc};

use super::{f32_to_u32, StravaProvider};
use crate::models::SegmentEffort;
use crate::strava_types::StravaSegmentEffort;

impl StravaProvider {
    /// A detail read's efforts, every one that maps; an empty list, never
    /// `None`, when the answer held none or none mapped (see the module doc).
    pub(super) fn convert_strava_segment_efforts(
        raw: Option<&[StravaSegmentEffort]>,
    ) -> Vec<SegmentEffort> {
        raw.unwrap_or_default()
            .iter()
            .filter_map(convert_segment_effort)
            .collect()
    }
}

/// One Strava effort as cageux's [`SegmentEffort`], or `None` when it lacks a
/// field cageux requires: its id, a name, its elapsed time, its start and a
/// distance. The name and the distance fall back to the segment's own.
///
/// Such an effort is skipped rather than given an invented value: a made-up
/// start or distance would rank it among the athlete's real efforts.
///
/// The segment's elevation is the segment's, not what this effort climbed, so
/// `elevation_gain` stays unset; a negative climb category is none.
pub(super) fn convert_segment_effort(raw: &StravaSegmentEffort) -> Option<SegmentEffort> {
    let segment = raw.segment.as_ref();
    let name = raw
        .name
        .clone()
        .or_else(|| segment.and_then(|s| s.name.clone()))
        .filter(|name| !name.trim().is_empty())?;
    let distance = raw
        .distance
        .map(f64::from)
        .or_else(|| segment.and_then(|s| s.distance))?;
    let start_date = DateTime::parse_from_rfc3339(raw.start_date.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    Some(SegmentEffort {
        id: raw.id?.to_string(),
        segment_id: segment.and_then(|s| s.id).map(|id| id.to_string()),
        name,
        elapsed_time: u64::from(raw.elapsed_time?),
        moving_time: raw.moving_time.map(u64::from),
        start_date,
        distance,
        average_heart_rate: raw.average_heartrate.map(f32_to_u32),
        max_heart_rate: raw.max_heartrate.map(f32_to_u32),
        average_cadence: raw.average_cadence.map(f32_to_u32),
        average_watts: raw.average_watts.map(f32_to_u32),
        kom_rank: raw.kom_rank,
        pr_rank: raw.pr_rank,
        climb_category: segment
            .and_then(|s| s.climb_category)
            .and_then(|category| u32::try_from(category).ok()),
        average_grade: segment.and_then(|s| s.average_grade),
        elevation_gain: None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::super::StravaProvider;
    use crate::models::SegmentEffort;
    use crate::strava_types::DetailedActivityResponse;

    /// A detailed ride as `GET /activities/{id}?include_all_efforts=true`
    /// answers it, with `extra` laid over it.
    fn detail(extra: &Value) -> DetailedActivityResponse {
        let mut body = json!({
            "id": 16_101,
            "name": "Côte du Mont-Royal",
            "type": "Ride",
            "sport_type": "Ride",
            "start_date": "2026-11-07T13:00:00Z",
            "elapsed_time": 3_600,
            "distance": 30_000.0
        });
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        serde_json::from_value(body).unwrap()
    }

    /// One effort as Strava sends it: the effort's own fields and the summary
    /// segment it was ridden on.
    fn effort(id: u64, start_date: Option<&str>) -> Value {
        let mut effort = json!({
            "id": id,
            "name": "Camillien-Houde",
            "elapsed_time": 412,
            "moving_time": 410,
            "distance": 1_820.5,
            "average_heartrate": 168.4,
            "max_heartrate": 181.0,
            "average_cadence": 78.0,
            "average_watts": 301.6,
            "pr_rank": 2,
            "kom_rank": null,
            "activity": { "id": 16_101 },
            "segment": {
                "id": 611_413,
                "name": "Camillien-Houde",
                "distance": 1_820.5,
                "average_grade": 7.1,
                "climb_category": 1,
                "activity_type": "Ride"
            }
        });
        if let Some(start) = start_date {
            effort["start_date"] = json!(start);
        }
        effort
    }

    fn mapped(extra: &Value) -> Option<Vec<SegmentEffort>> {
        StravaProvider::convert_detailed_strava_activity(detail(extra), None)
            .unwrap()
            .segment_efforts()
            .cloned()
    }

    #[test]
    fn an_effort_with_every_required_field_maps_and_one_without_a_start_is_skipped() {
        let efforts = mapped(&json!({
            "segment_efforts": [effort(9_001, Some("2026-11-07T13:10:00Z")), effort(9_002, None)]
        }))
        .expect("bug: a detail read carries its efforts");
        assert_eq!(
            efforts.len(),
            1,
            "bug: an effort with no start is skipped: {efforts:?}"
        );
        let one = &efforts[0];
        assert_eq!(one.id, "9001");
        assert_eq!(one.segment_id.as_deref(), Some("611413"));
        assert_eq!(one.name, "Camillien-Houde");
        assert_eq!(one.elapsed_time, 412);
        assert_eq!(one.moving_time, Some(410));
        assert_eq!(one.start_date.to_rfc3339(), "2026-11-07T13:10:00+00:00");
        assert!((one.distance - 1_820.5).abs() < 0.01);
        assert_eq!(one.average_heart_rate, Some(168));
        assert_eq!(one.max_heart_rate, Some(181));
        assert_eq!(one.average_cadence, Some(78));
        assert_eq!(one.average_watts, Some(301));
        assert_eq!(one.pr_rank, Some(2));
        assert_eq!(one.kom_rank, None);
        assert_eq!(one.climb_category, Some(1));
        assert!(one.average_grade.is_some_and(|g| (g - 7.1).abs() < 0.01));
        assert_eq!(
            one.elevation_gain, None,
            "bug: the segment's elevation is the segment's, never this effort's"
        );
    }

    #[test]
    fn a_name_or_distance_missing_on_the_effort_comes_from_its_segment() {
        let mut bare = effort(9_003, Some("2026-11-07T13:20:00Z"));
        bare["name"] = Value::Null;
        bare.as_object_mut().unwrap().remove("distance");
        bare["segment"]["name"] = json!("Remembrance");
        bare["segment"]["distance"] = json!(2_400.0);
        let efforts = mapped(&json!({ "segment_efforts": [bare] })).unwrap();
        assert_eq!(efforts[0].name, "Remembrance");
        assert!((efforts[0].distance - 2_400.0).abs() < 0.01);
    }

    #[test]
    fn a_negative_climb_category_is_no_category() {
        let mut flat = effort(9_004, Some("2026-11-07T13:30:00Z"));
        flat["segment"]["climb_category"] = json!(-1);
        let efforts = mapped(&json!({ "segment_efforts": [flat] })).unwrap();
        assert_eq!(efforts[0].climb_category, None);
    }

    /// The detail read asks for every effort, so an answer with none is
    /// Strava's word that the ride crossed no segment: the read is stamped
    /// with an empty list, absent array or not.
    #[test]
    fn an_absent_or_empty_efforts_array_stamps_an_empty_list() {
        assert_eq!(
            mapped(&json!({})),
            Some(Vec::new()),
            "bug: an absent array on the detailed path is read as no efforts"
        );
        assert_eq!(
            mapped(&json!({ "segment_efforts": [] })),
            Some(Vec::new()),
            "bug: an empty array is read as no efforts"
        );
        assert_eq!(
            mapped(&json!({ "segment_efforts": [effort(9_005, None)] })),
            Some(Vec::new()),
            "bug: every effort skipped still stamps the read"
        );
    }
}
