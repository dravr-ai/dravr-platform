// ABOUTME: Strava segment efforts — the detail read's efforts mapped to cageux, and one segment's efforts listed
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

use chrono::{DateTime, Utc};
use tracing::{info, warn};

use super::{f32_to_u32, StravaProvider};
use crate::constants::oauth_providers;
use crate::core::{segment_listing_refused, SegmentEffortHit, SegmentEffortWindow};
use crate::errors::{AppError, AppResult};
use crate::models::{resolve_sport_type, SegmentEffort, SportType};
use crate::strava_types::StravaSegmentEffort;

/// Efforts one `GET /segment_efforts` page holds at most: Strava's ceiling.
const EFFORTS_PER_PAGE: usize = 200;

/// Pages one listing reads before giving up: 4,000 efforts, more than a
/// decade of a daily commute over one segment.
///
/// A segment with more efforts than that in the window is refused rather
/// than listed short, so the caller answers from the stored details instead
/// of from a list that silently stops.
const MAX_PAGES: u32 = 20;

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

    /// Every effort the athlete made on segment `segment_id` over `window`,
    /// page by page until a short page (`GET /segment_efforts`).
    ///
    /// # Errors
    /// `InvalidInput` for a segment id that is not a number or a window that
    /// ends before it starts, before any request; Strava refusing the athlete
    /// the listing ([`listing_refusal`]); the request's own error otherwise;
    /// an external-service error for a segment with more efforts in the
    /// window than [`MAX_PAGES`] pages hold; and one for a page holding a row
    /// this mapping cannot read, so the listing is never answered short of
    /// what Strava listed.
    pub(super) async fn list_segment_efforts(
        &self,
        segment_id: &str,
        window: SegmentEffortWindow,
    ) -> AppResult<Vec<SegmentEffortHit>> {
        let segment: u64 = segment_id
            .trim()
            .parse()
            .map_err(|_| AppError::invalid_input("a Strava segment id is a number"))?;
        if window.to < window.from {
            return Err(AppError::invalid_input(
                "a segment's efforts are listed over a window that ends after it starts",
            ));
        }
        let mut hits = Vec::new();
        for page in 1..=MAX_PAGES {
            let rows: Vec<StravaSegmentEffort> = self
                .api_request_reading(
                    &segment_efforts_endpoint(segment, window, page),
                    listing_refusal,
                )
                .await?;
            let mapped: Vec<SegmentEffortHit> = rows.iter().filter_map(convert_hit).collect();
            info!(
                segment,
                page,
                listed = rows.len(),
                read = mapped.len(),
                "Strava segment effort listing page"
            );
            if mapped.len() < rows.len() {
                warn!(
                    segment,
                    page,
                    listed = rows.len(),
                    read = mapped.len(),
                    "Strava listed segment efforts this mapping cannot read; the listing is refused rather than answered short"
                );
                return Err(AppError::external_service(
                    oauth_providers::STRAVA,
                    format!(
                        "Strava listed {} efforts on segment {segment} and {} could not be read",
                        rows.len(),
                        rows.len() - mapped.len()
                    ),
                ));
            }
            hits.extend(mapped);
            if rows.len() < EFFORTS_PER_PAGE {
                return Ok(hits);
            }
        }
        Err(AppError::external_service(
            oauth_providers::STRAVA,
            format!(
                "segment {segment} holds more than {} efforts in the window; the listing stopped",
                EFFORTS_PER_PAGE * MAX_PAGES as usize
            ),
        ))
    }
}

/// Strava's reading of a non-success answer to the listing: a `402` or a
/// `403` is Strava refusing this athlete the listing, which needs a
/// subscription ([`segment_listing_refused`]); anything else reads as any
/// other request's answer does ([`StravaProvider::missing_resource`]).
fn listing_refusal(status: reqwest::StatusCode, text: &str, url: &str) -> Option<AppError> {
    match status.as_u16() {
        code @ (402 | 403) => Some(segment_listing_refused(oauth_providers::STRAVA, code)),
        _ => StravaProvider::missing_resource(status, text, url),
    }
}

/// Page `page` of the athlete's efforts on `segment` over `window`. Strava
/// reads `start_date_local` and `end_date_local` on the athlete's own clock.
fn segment_efforts_endpoint(segment: u64, window: SegmentEffortWindow, page: u32) -> String {
    format!(
        "segment_efforts?segment_id={segment}&start_date_local={}T00:00:00Z&end_date_local={}T23:59:59Z&per_page={EFFORTS_PER_PAGE}&page={page}",
        window.from.format("%Y-%m-%d"),
        window.to.format("%Y-%m-%d"),
    )
}

/// One row of `GET /segment_efforts` as a hit, or `None` when the effort does
/// not map or names no activity: a hit is always addressable by its activity.
fn convert_hit(raw: &StravaSegmentEffort) -> Option<SegmentEffortHit> {
    let activity_id = raw
        .activity
        .as_ref()
        .and_then(|activity| activity.id)
        .or(raw.activity_id)?
        .to_string();
    let activity_type = raw
        .segment
        .as_ref()
        .and_then(|segment| segment.activity_type.as_deref())
        .filter(|kind| !kind.is_empty())
        .map(|kind| resolve_sport_type(kind).unwrap_or_else(|| SportType::Other(kind.to_owned())));
    Some(SegmentEffortHit {
        effort: convert_segment_effort(raw)?,
        activity_id,
        activity_type,
    })
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

    /// The Strava API lists one segment's efforts; Strava's session (sciotte)
    /// reads the athlete's pages and does not.
    #[test]
    fn strava_declares_the_segment_effort_listing() {
        use crate::spi::{ProviderCapabilities, ProviderDescriptor, StravaDescriptor};
        assert!(
            StravaDescriptor
                .capabilities()
                .supports_segment_effort_history(),
            "bug: the Strava API lists a segment's efforts and must say so"
        );
        assert!(
            !ProviderCapabilities::activity_only().supports_segment_effort_history(),
            "bug: the flag is Strava's own, not every activity provider's"
        );
    }
}
