// ABOUTME: Session-read segment efforts mapped to cageux's, keeping None (not read) apart from Some([]) (read, none)
// ABOUTME: An effort without a start date is skipped, and an answer whose efforts all fail to map stays unread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The segment efforts of a Strava activity read through the athlete's
//! signed-in session, mapped to cageux's
//! [`SegmentEffort`] the way the Strava grant's detail read maps them
//! (`strava_provider::segments`), so a row read by either backend answers
//! the segment history alike.
//!
//! The service's `segment_efforts` is `None` when the page's efforts were not
//! read and `Some([])` when they were and the activity crossed no segment.
//! That difference is what stamps a stored row's segments as read, so the
//! mapping keeps it: an absent list stays absent, an empty one stays empty,
//! and a list whose every effort fails to map stays absent too, since it is
//! no word that the activity crossed no segment.
//!
//! An effort without a start date is skipped, never given an invented one:
//! the service leaves it out when the activity's streams were not read, and a
//! made-up start would rank the effort among the athlete's real ones.

use dravr_sciotte::models::{Activity as SciotteActivity, SegmentEffort as SciotteEffort};

use crate::models::SegmentEffort;

/// The entry type on which the service's `effort_rank` is the athlete's own
/// personal-record rank (Strava's `pr_rank`).
const PERSONAL_RECORD: &str = "pr";

/// The session-read activity's efforts as cageux's.
///
/// An activity whose detail read settled that it recorded no GPS (a `route`
/// with no coordinates: a trainer ride, a treadmill run, a manual entry)
/// crosses no segment, because Strava matches segments on the GPS track. Its
/// page carries no `segmentEfforts()` block at all (measured on a no-GPS run
/// page, carnet#1603), so the service answers `None`, which would leave the row
/// unread and a candidate of every detail walk forever. A read that settled
/// there is no track reads as `Some` and empty, but only when the page had no
/// efforts block at all: efforts that were listed and none of which mapped
/// stay `None`, since that is no word that none was crossed. Every other
/// `None` stays `None`: a list row, or a page whose streams were not read,
/// settles nothing.
pub(super) fn read_efforts(sciotte: &SciotteActivity) -> Option<Vec<SegmentEffort>> {
    let mapped = segment_efforts(sciotte.segment_efforts.as_deref());
    let no_block = sciotte.segment_efforts.is_none();
    let recorded_no_gps = sciotte
        .route
        .as_ref()
        .is_some_and(|route| route.coordinates.is_empty());
    mapped.or_else(|| (no_block && recorded_no_gps).then(Vec::new))
}

/// The activity's efforts as cageux's: `None` when the service read none
/// (`None`) or every effort it listed failed to map, `Some` of the ones that
/// mapped otherwise, an empty list when it read the page and found none.
fn segment_efforts(raw: Option<&[SciotteEffort]>) -> Option<Vec<SegmentEffort>> {
    let raw = raw?;
    let efforts: Vec<SegmentEffort> = raw.iter().filter_map(convert_effort).collect();
    (raw.is_empty() || !efforts.is_empty()).then_some(efforts)
}

/// One session-read effort as cageux's [`SegmentEffort`], or `None` when it lacks
/// a field cageux requires: a start date or a name.
///
/// The rank crosses as `pr_rank` only on the athlete's own ladder
/// (`entry_type` `"pr"`); the effort's elevation is the segment's, not what
/// this effort climbed, so `elevation_gain` stays unset.
fn convert_effort(raw: &SciotteEffort) -> Option<SegmentEffort> {
    let name = raw.name.trim();
    if name.is_empty() {
        return None;
    }
    let ranked_pr = raw
        .entry_type
        .as_deref()
        .is_some_and(|entry| entry.eq_ignore_ascii_case(PERSONAL_RECORD));
    Some(SegmentEffort {
        id: raw.id.clone(),
        segment_id: raw.segment_id.clone().filter(|id| !id.trim().is_empty()),
        name: name.to_owned(),
        elapsed_time: raw.elapsed_time,
        moving_time: raw.moving_time,
        start_date: raw.start_date?,
        distance: raw.distance,
        average_heart_rate: raw.average_heart_rate,
        max_heart_rate: raw.max_heart_rate,
        average_cadence: raw.average_cadence,
        average_watts: raw.average_watts,
        kom_rank: None,
        pr_rank: raw.effort_rank.filter(|_| ranked_pr),
        climb_category: raw.climb_category.as_deref().and_then(api_climb_category),
        average_grade: raw.average_grade,
        elevation_gain: None,
    })
}

/// Strava's API climb category for the label the page shows: `NC` 0, the
/// categories `4` to `1` as 1 to 4 and `HC` 5, the numbers the grant's
/// detail read stores. A label that is none of these is no category.
fn api_climb_category(label: &str) -> Option<u32> {
    match label.trim().to_ascii_uppercase().as_str() {
        "NC" => Some(0),
        "4" => Some(1),
        "3" => Some(2),
        "2" => Some(3),
        "1" => Some(4),
        "HC" => Some(5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::super::{convert_activity, SciotteTarget};
    use super::{SciotteActivity, SciotteEffort};
    use crate::models::SegmentEffort;

    /// A session-read effort, `extra` laid over a complete one.
    fn effort(extra: &Value) -> SciotteEffort {
        let mut body = json!({
            "id": "9001",
            "name": "Côte Camillien-Houde",
            "elapsed_time": 372,
            "distance": 1_800.5,
            "segment_id": "1234567",
            "start_date": "2026-11-07T13:10:00Z"
        });
        if let Some(extra) = extra.as_object() {
            for (key, value) in extra {
                body[key] = value.clone();
            }
        }
        serde_json::from_value(body).unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    fn mapped(efforts: Option<&[SciotteEffort]>) -> Option<Vec<SegmentEffort>> {
        super::segment_efforts(efforts)
    }

    #[test]
    fn an_effort_crosses_with_its_segment_start_and_climb() {
        let raw = effort(&json!({
            "moving_time": 370,
            "average_heart_rate": 171,
            "average_watts": 280,
            "average_grade": 6.4,
            "climb_category": "3",
            "effort_rank": 2,
            "entry_type": "pr"
        }));
        let out = mapped(Some(&[raw]))
            .and_then(|mut v| v.pop())
            .unwrap_or_else(|| panic!("bug: a complete effort did not map"));
        assert_eq!(out.id, "9001", "bug: the effort id changed");
        assert_eq!(
            out.segment_id.as_deref(),
            Some("1234567"),
            "bug: the segment id was dropped"
        );
        assert_eq!(out.elapsed_time, 372, "bug: the elapsed time changed");
        assert_eq!(
            out.moving_time,
            Some(370),
            "bug: the moving time was dropped"
        );
        assert_eq!(out.pr_rank, Some(2), "bug: a pr rank was dropped");
        assert_eq!(out.kom_rank, None, "bug: a kom rank was invented");
        assert_eq!(out.average_grade, Some(6.4), "bug: the grade was dropped");
        assert_eq!(
            out.average_heart_rate,
            Some(171),
            "bug: the heart rate was dropped"
        );
        assert_eq!(
            out.climb_category,
            Some(2),
            "bug: the climb category was dropped"
        );
    }

    #[test]
    fn absent_empty_and_unmappable_lists_stay_distinct() {
        assert_eq!(
            mapped(None),
            None,
            "bug: an unread list was stamped as read"
        );
        assert_eq!(
            mapped(Some(&[])),
            Some(Vec::new()),
            "bug: an empty read list lost its stamp"
        );
        let undated = effort(&json!({ "start_date": null }));
        assert_eq!(
            mapped(Some(&[undated.clone(), undated])),
            None,
            "bug: efforts that all failed to map were stamped as none crossed"
        );
    }

    #[test]
    fn an_effort_without_a_start_is_skipped_and_the_rest_cross() {
        let kept = effort(&json!({ "id": "9002" }));
        let undated = effort(&json!({ "id": "9003", "start_date": null }));
        let blank = effort(&json!({ "id": "9004", "name": "  " }));
        let out = mapped(Some(&[undated, kept, blank]))
            .unwrap_or_else(|| panic!("bug: a partly mapped list was dropped"));
        let ids: Vec<&str> = out.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["9002"], "bug: an effort without a start crossed");
    }

    #[test]
    fn a_rank_on_another_ladder_is_no_pr_rank() {
        for entry in [Some("overall"), None] {
            let mut body = json!({ "effort_rank": 1 });
            if let Some(entry) = entry {
                body["entry_type"] = json!(entry);
            }
            let out = mapped(Some(&[effort(&body)]))
                .and_then(|mut v| v.pop())
                .unwrap_or_else(|| panic!("bug: effort dropped"));
            assert_eq!(out.pr_rank, None, "{entry:?}");
        }
    }

    /// The label the page shows and the integer the grant's API read stores
    /// (`strava_provider::segments`, which keeps `segment.climb_category`
    /// as it is, 0 to 5) are one climb category.
    #[test]
    fn a_climb_label_crosses_as_the_number_the_grant_stores() {
        let cases = [("NC", 0), ("4", 1), ("3", 2), ("2", 3), ("1", 4), ("HC", 5)];
        for (label, api) in cases {
            let session = mapped(Some(&[effort(&json!({ "climb_category": label }))]))
                .and_then(|mut v| v.pop())
                .unwrap_or_else(|| panic!("bug: effort dropped"));
            assert_eq!(session.climb_category, Some(api), "{label}");
        }
        let odd = mapped(Some(&[effort(&json!({ "climb_category": "5" }))]))
            .and_then(|mut v| v.pop())
            .unwrap_or_else(|| panic!("bug: effort dropped"));
        assert_eq!(odd.climb_category, None, "an unknown label is no category");
    }

    #[test]
    fn convert_activity_keeps_the_stamp_apart_from_no_read() {
        let page = |efforts: Option<Value>| {
            let mut row = json!({
                "id": "16001",
                "name": "Session",
                "sport_type": "ride",
                "start_date": "2026-11-07T13:00:00Z",
                "duration_seconds": 3_600,
                "provider": "strava"
            });
            if let Some(efforts) = efforts {
                row["segment_efforts"] = efforts;
            }
            serde_json::from_value(row).unwrap_or_else(|e| panic!("fixture: {e}"))
        };
        let unread = convert_activity(&page(None), SciotteTarget::Strava);
        assert!(unread.segment_efforts().is_none(), "bug: unread stamped");
        let none = convert_activity(&page(Some(json!([]))), SciotteTarget::Strava);
        assert_eq!(
            none.segment_efforts().map(Vec::len),
            Some(0),
            "bug: a read empty list lost its stamp"
        );
        let one = convert_activity(
            &page(Some(json!([{
                "id": "9001", "name": "Climb", "elapsed_time": 60, "distance": 500.0,
                "start_date": "2026-11-07T13:10:00Z"
            }]))),
            SciotteTarget::Strava,
        );
        assert_eq!(
            one.segment_efforts().map(Vec::len),
            Some(1),
            "bug: a mapped effort was dropped"
        );
    }

    /// A page with no segment block and no GPS (measured on a no-GPS run
    /// page: it carries no `segmentEfforts()` block at all) reads as no
    /// efforts, so its row converges; a page that never settled whether it
    /// had a track stays unread.
    #[test]
    fn an_activity_that_recorded_no_gps_reads_as_crossing_no_segment() {
        let activity = |route: Option<Value>| {
            let mut row = json!({
                "id": "16002",
                "name": "Treadmill",
                "sport_type": "run",
                "start_date": "2026-11-07T13:00:00Z",
                "duration_seconds": 1_800,
                "provider": "strava"
            });
            if let Some(route) = route {
                row["route"] = route;
            }
            serde_json::from_value::<SciotteActivity>(row)
                .unwrap_or_else(|e| panic!("fixture: {e}"))
        };
        assert_eq!(
            super::read_efforts(&activity(Some(json!({ "coordinates": [] })))),
            Some(Vec::new()),
            "bug: an activity with no GPS stayed a walk candidate forever"
        );
        assert_eq!(
            super::read_efforts(&activity(None)),
            None,
            "bug: a read that settled nothing about the track was stamped as read"
        );
        assert_eq!(
            super::read_efforts(&activity(Some(
                json!({ "coordinates": [[45.5, -73.5], [45.6, -73.6]] })
            ))),
            None,
            "bug: a GPS activity whose block was absent was stamped as read"
        );
    }

    /// Efforts that were listed but none of which mapped are no word that the
    /// activity crossed no segment, whatever its route holds.
    #[test]
    fn listed_efforts_that_none_map_stay_unread_even_on_an_empty_route() {
        let row = json!({
            "id": "16003",
            "name": "Ride",
            "sport_type": "ride",
            "start_date": "2026-11-07T13:00:00Z",
            "duration_seconds": 1_800,
            "provider": "strava",
            "route": { "coordinates": [] },
            "segment_efforts": [{
                "id": "9005", "name": "Climb", "elapsed_time": 60, "distance": 500.0,
                "start_date": null
            }]
        });
        let activity: SciotteActivity =
            serde_json::from_value(row).unwrap_or_else(|e| panic!("fixture: {e}"));
        assert_eq!(
            super::read_efforts(&activity),
            None,
            "bug: listed efforts that none mapped were stamped as none crossed"
        );
    }
}
