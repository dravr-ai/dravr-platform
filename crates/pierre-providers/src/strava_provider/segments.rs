// ABOUTME: Strava segment efforts — the detail read's efforts mapped to cageux
// ABOUTME: A detail read stamps its efforts list unless every effort sent failed to map, so the row reads as segment-read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava's segment efforts, mapped to cageux's [`SegmentEffort`].
//!
//! A detail read sends `include_all_efforts=true`, so its answer holds every
//! effort the activity has: an absent or empty array is Strava's word that
//! the activity crossed no segment, and the mapper stamps it with an empty
//! list. That stamp is what marks a stored row's segments as read.
//!
//! Strava's published schema marks no effort field as required, so an
//! answer whose efforts all fail to map is possible, and it is no word that
//! the activity crossed no segment: it is left unstamped (`None`), and the
//! row reads as unread, an activity a walk tried and could not read.

use dravr_tronc::dates::Timestamp;

use super::{f32_to_u32, StravaProvider};
use crate::models::SegmentEffort;
use crate::strava_types::StravaSegmentEffort;

impl StravaProvider {
    /// A detail read's efforts, every one that maps: an empty list when the
    /// answer held none, and `None` when it held some and none of them
    /// mapped (see the module doc).
    pub(super) fn convert_strava_segment_efforts(
        raw: Option<&[StravaSegmentEffort]>,
    ) -> Option<Vec<SegmentEffort>> {
        let raw = raw.unwrap_or_default();
        let efforts: Vec<SegmentEffort> = raw.iter().filter_map(convert_segment_effort).collect();
        (raw.is_empty() || !efforts.is_empty()).then_some(efforts)
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
    let named = |name: &Option<String>| name.clone().filter(|name| !name.trim().is_empty());
    let name = named(&raw.name).or_else(|| segment.and_then(|s| named(&s.name)))?;
    let distance = raw
        .distance
        .map(f64::from)
        .or_else(|| segment.and_then(|s| s.distance))?;
    let start_date = Timestamp::parse_from_rfc3339(raw.start_date.as_deref()?).ok()?;
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
        assert_eq!(one.id, "9001", "bug: the effort keeps Strava's id");
        assert_eq!(
            one.segment_id.as_deref(),
            Some("611413"),
            "bug: the effort keeps the segment it was ridden on"
        );
        assert_eq!(
            one.name, "Camillien-Houde",
            "bug: the effort keeps its own name"
        );
        assert_eq!(one.elapsed_time, 412, "bug: elapsed time is mapped as sent");
        assert_eq!(
            one.moving_time,
            Some(410),
            "bug: moving time is mapped as sent"
        );
        assert_eq!(
            one.start_date.to_rfc3339(),
            "2026-11-07T13:10:00+00:00",
            "bug: the start is read as UTC"
        );
        assert!(
            (one.distance - 1_820.5).abs() < 0.01,
            "bug: the effort keeps its own distance: {}",
            one.distance
        );
        assert_eq!(
            one.average_heart_rate,
            Some(168),
            "bug: average heart rate is truncated to whole beats"
        );
        assert_eq!(
            one.max_heart_rate,
            Some(181),
            "bug: max heart rate is mapped as sent"
        );
        assert_eq!(
            one.average_cadence,
            Some(78),
            "bug: cadence is mapped as sent"
        );
        assert_eq!(
            one.average_watts,
            Some(301),
            "bug: average watts are truncated to whole watts"
        );
        assert_eq!(one.pr_rank, Some(2), "bug: the PR rank is mapped as sent");
        assert_eq!(one.kom_rank, None, "bug: a null KOM rank stays unset");
        assert_eq!(
            one.climb_category,
            Some(1),
            "bug: the segment's climb category reaches the effort"
        );
        assert!(
            one.average_grade.is_some_and(|g| (g - 7.1).abs() < 0.01),
            "bug: the segment's grade reaches the effort: {:?}",
            one.average_grade
        );
        assert_eq!(
            one.elevation_gain, None,
            "bug: the segment's elevation is the segment's, never this effort's"
        );
    }

    #[test]
    fn a_blank_name_or_missing_distance_on_the_effort_comes_from_its_segment() {
        let mut bare = effort(9_003, Some("2026-11-07T13:20:00Z"));
        bare["name"] = json!("  ");
        bare.as_object_mut().unwrap().remove("distance");
        bare["segment"]["name"] = json!("Remembrance");
        bare["segment"]["distance"] = json!(2_400.0);
        let efforts = mapped(&json!({ "segment_efforts": [bare] })).unwrap();
        assert_eq!(
            efforts[0].name, "Remembrance",
            "bug: a blank effort name falls back to its segment's"
        );
        assert!(
            (efforts[0].distance - 2_400.0).abs() < 0.01,
            "bug: a missing effort distance falls back to its segment's: {}",
            efforts[0].distance
        );
    }

    #[test]
    fn a_negative_climb_category_is_no_category() {
        let mut flat = effort(9_004, Some("2026-11-07T13:30:00Z"));
        flat["segment"]["climb_category"] = json!(-1);
        let efforts = mapped(&json!({ "segment_efforts": [flat] })).unwrap();
        assert_eq!(
            efforts[0].climb_category, None,
            "bug: Strava's -1 means uncategorized, never a category"
        );
    }

    /// The detail read asks for every effort, so an answer with none is
    /// Strava's word that the ride crossed no segment: the read is stamped
    /// with an empty list, absent array or not. An answer whose efforts all
    /// fail to map is no such word: it is left unstamped, so the ride reads
    /// as unread rather than as one that crossed no segment.
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
            mapped(&json!({ "segment_efforts": [effort(9_005, None), effort(9_006, None)] })),
            None,
            "bug: efforts Strava sent that all failed to map stamp the ride as \
             crossing no segment"
        );
    }
}
