// ABOUTME: get_segment_history's answer — the efforts listed, the segments matched and their summary
// ABOUTME: Summed over every effort, then the list trimmed to the bests, the ranked and the newest

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What [`GetSegmentHistoryTool`](super::GetSegmentHistoryTool) answers
//! with, and how the efforts either place holds become it.

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use pierre_core::civil_time::local_date;
use pierre_core::models::coverage::Coverage;
use pierre_core::models::SegmentEffort;
use pierre_core::untrusted::display_line;
use serde::Serialize;

use crate::implementations::activity_summary::SEGMENT_NAME_MAX_CHARS;

/// Most efforts one answer lists.
///
/// A commute segment ridden most days holds thousands of efforts over the
/// default window, and the list is re-paid on every later turn. The answer
/// keeps each matched segment's best, then the efforts the athlete ranked on
/// (a personal record or a leaderboard place), then the newest; `summary`
/// counts and times every effort, and `efforts_total` says how many there
/// are when the list holds fewer.
const MAX_LISTED_EFFORTS: usize = 50;

/// Which of the two places answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredBy {
    /// The provider's live listing of the segment's efforts: complete.
    LiveSegmentHistory,
    /// The segment efforts stored with the athlete's activities.
    StoredDetails,
}

/// One effort on a matched segment.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SegmentHistoryEffort {
    /// The service the effort came from, as the athlete knows it (`strava`).
    pub provider: String,
    /// The service that recorded it, when the provider relays another's.
    pub source: String,
    /// The provider's id of the activity the effort was made in.
    pub activity_id: String,
    /// That activity's title, as one line; absent from a live listing, which
    /// names only the activity's id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_name: Option<String>,
    /// When the effort started (RFC 3339, UTC).
    pub start_date: String,
    /// Seconds from the segment's start to its end.
    pub elapsed_time: u64,
    /// Seconds moving on the segment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moving_time: Option<u64>,
    /// The segment's length, in meters.
    pub distance: f64,
    /// Average heart rate over the effort (bpm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_heart_rate: Option<u32>,
    /// Average power over the effort (watts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_watts: Option<u32>,
    /// The effort's rank among the athlete's own on the segment when it was
    /// made (1 = their best then).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_rank: Option<u32>,
    /// The effort's rank on the segment's overall leaderboard when it was
    /// made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kom_rank: Option<u32>,
    /// The segment's id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    /// The segment's name, as one line: whoever created the segment wrote it.
    pub segment_name: String,
}

/// One segment the call matched.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct MatchedSegment {
    /// The service the segment is on (`strava`).
    pub provider: String,
    /// The segment's id, when its efforts carry one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The segment's name, as one line.
    pub name: String,
}

/// One segment's efforts on one service, summed.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SegmentSummary {
    /// The service the efforts came from (`strava`).
    pub provider: String,
    /// The segment's id, when its efforts carry one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    /// The segment's name, as one line.
    pub segment_name: String,
    /// Efforts listed.
    pub count: usize,
    /// Efforts listed per calendar year, in the athlete's timezone.
    pub by_year: BTreeMap<String, usize>,
    /// The fastest effort's elapsed time, in seconds.
    pub best_elapsed_time: u64,
    /// When the fastest effort started (RFC 3339, UTC).
    pub best_start_date: String,
}

/// What `get_segment_history` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SegmentHistoryResult {
    /// The efforts on the matched segments, newest first: every one, or, past
    /// 50, each segment's best, the ranked ones and the newest.
    pub efforts: Vec<SegmentHistoryEffort>,
    /// How many efforts the matched segments hold, when `efforts` lists only
    /// some of them; `summary` counts and times them all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efforts_total: Option<usize>,
    /// The distinct segments matched.
    pub segments: Vec<MatchedSegment>,
    /// The efforts summed per segment and service.
    pub summary: Vec<SegmentSummary>,
    /// Where the efforts were read: the provider's live listing, or the
    /// stored details of the athlete's activities.
    pub answered_by: AnsweredBy,
    /// Whether the efforts are all there are in the window: `window.total` is
    /// the activities in it, `window.returned` the activities whose segments
    /// were read; with `incomplete_reason` and the walk still `pending` when
    /// some were not, and the `note` saying what may be claimed.
    pub coverage: Coverage,
}

/// One effort as the answer lists it.
pub(super) fn effort_row(
    provider: &str,
    source: &str,
    activity_id: &str,
    activity_name: Option<&str>,
    effort: &SegmentEffort,
) -> SegmentHistoryEffort {
    SegmentHistoryEffort {
        provider: provider.to_owned(),
        source: source.to_owned(),
        activity_id: activity_id.to_owned(),
        activity_name: activity_name.map(|name| display_line(name, SEGMENT_NAME_MAX_CHARS)),
        start_date: effort.start_date.to_rfc3339(),
        elapsed_time: effort.elapsed_time,
        moving_time: effort.moving_time,
        distance: effort.distance,
        average_heart_rate: effort.average_heart_rate,
        average_watts: effort.average_watts,
        pr_rank: effort.pr_rank,
        kom_rank: effort.kom_rank,
        segment_id: effort.segment_id.clone(),
        segment_name: display_line(&effort.name, SEGMENT_NAME_MAX_CHARS),
    }
}

/// The answer over `efforts`, newest first, with the segments they are on
/// and their summary per segment and service, summed over every effort
/// before the list is trimmed to [`MAX_LISTED_EFFORTS`].
pub(super) fn answer(
    mut efforts: Vec<SegmentHistoryEffort>,
    answered_by: AnsweredBy,
    coverage: Coverage,
    zone: Tz,
) -> SegmentHistoryResult {
    efforts.sort_by(|a, b| b.start_date.cmp(&a.start_date));
    let mut groups: BTreeMap<(String, String), Vec<&SegmentHistoryEffort>> = BTreeMap::new();
    for effort in &efforts {
        let segment = effort
            .segment_id
            .clone()
            .unwrap_or_else(|| effort.segment_name.clone());
        groups
            .entry((effort.provider.clone(), segment))
            .or_default()
            .push(effort);
    }
    let mut segments = Vec::with_capacity(groups.len());
    let mut summary = Vec::with_capacity(groups.len());
    for ((provider, _), group) in &groups {
        let Some(first) = group.first() else {
            continue;
        };
        segments.push(MatchedSegment {
            provider: provider.clone(),
            id: first.segment_id.clone(),
            name: first.segment_name.clone(),
        });
        summary.push(summarize(provider, group, zone));
    }
    let bests: HashSet<(String, String)> = summary
        .iter()
        .map(|best| (best.provider.clone(), best.best_start_date.clone()))
        .collect();
    let (efforts, efforts_total) = listed_efforts(efforts, &bests);
    SegmentHistoryResult {
        efforts,
        efforts_total,
        segments,
        summary,
        answered_by,
        coverage,
    }
}

/// `efforts` (newest first) trimmed to [`MAX_LISTED_EFFORTS`], newest
/// first, with how many there were when any was dropped: each segment's best
/// (`bests`, by provider and start) first, then the ranked efforts, then the
/// newest.
fn listed_efforts(
    efforts: Vec<SegmentHistoryEffort>,
    bests: &HashSet<(String, String)>,
) -> (Vec<SegmentHistoryEffort>, Option<usize>) {
    let total = efforts.len();
    if total <= MAX_LISTED_EFFORTS {
        return (efforts, None);
    }
    let mut ranked: Vec<(usize, usize, SegmentHistoryEffort)> = efforts
        .into_iter()
        .enumerate()
        .map(|(newest, effort)| {
            let best = bests.contains(&(effort.provider.clone(), effort.start_date.clone()));
            let tier = if best {
                0
            } else if effort.pr_rank.is_some() || effort.kom_rank.is_some() {
                1
            } else {
                2
            };
            (tier, newest, effort)
        })
        .collect();
    ranked.sort_by_key(|(tier, newest, _)| (*tier, *newest));
    ranked.truncate(MAX_LISTED_EFFORTS);
    ranked.sort_by_key(|(_, newest, _)| *newest);
    (
        ranked.into_iter().map(|(_, _, effort)| effort).collect(),
        Some(total),
    )
}

/// One segment's efforts on one service, summed.
fn summarize(provider: &str, group: &[&SegmentHistoryEffort], zone: Tz) -> SegmentSummary {
    let mut by_year: BTreeMap<String, usize> = BTreeMap::new();
    for effort in group {
        let year = DateTime::parse_from_rfc3339(&effort.start_date)
            .map(|at| {
                local_date(at.with_timezone(&Utc), zone)
                    .format("%Y")
                    .to_string()
            })
            .unwrap_or_default();
        *by_year.entry(year).or_default() += 1;
    }
    let best = group
        .iter()
        .min_by_key(|effort| effort.elapsed_time)
        .copied();
    SegmentSummary {
        provider: provider.to_owned(),
        segment_id: best.and_then(|effort| effort.segment_id.clone()),
        segment_name: best
            .map(|effort| effort.segment_name.clone())
            .unwrap_or_default(),
        count: group.len(),
        by_year,
        best_elapsed_time: best.map_or(0, |effort| effort.elapsed_time),
        best_start_date: best
            .map(|effort| effort.start_date.clone())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};
    use pierre_core::constants::oauth::providers::STRAVA;
    use pierre_core::models::coverage::Completeness;

    use super::*;
    use crate::implementations::data_helpers::segments_read_coverage;

    /// The effort ridden `days` after 2016-01-01 on La Bête, in `elapsed`
    /// seconds.
    fn ridden(days: i64, elapsed: u64, pr_rank: Option<u32>) -> SegmentHistoryEffort {
        let at = Utc.timestamp_opt(1_451_606_400 + days * 86_400, 0).single();
        SegmentHistoryEffort {
            provider: STRAVA.to_owned(),
            source: STRAVA.to_owned(),
            activity_id: days.to_string(),
            activity_name: None,
            start_date: at.map(|at| at.to_rfc3339()).unwrap_or_default(),
            elapsed_time: elapsed,
            moving_time: None,
            distance: 1_820.5,
            average_heart_rate: None,
            average_watts: None,
            pr_rank,
            kom_rank: None,
            segment_id: Some("611413".to_owned()),
            segment_name: "La Bête".to_owned(),
        }
    }

    #[test]
    fn a_long_history_lists_its_best_ranked_and_newest_efforts_and_counts_all() {
        let complete = segments_read_coverage(
            None,
            0,
            (NaiveDate::MIN, NaiveDate::MIN),
            Completeness::Complete,
            None,
        );
        // The oldest effort is the fastest; one in the middle set a record.
        let mut efforts: Vec<_> = (0..120).map(|day| ridden(day, 400, None)).collect();
        efforts[0].elapsed_time = 300;
        efforts[60].pr_rank = Some(1);
        let answered = answer(efforts, AnsweredBy::StoredDetails, complete, Tz::UTC);

        assert_eq!(
            answered.efforts.len(),
            MAX_LISTED_EFFORTS,
            "bug: efforts are not capped"
        );
        assert_eq!(
            answered.efforts_total,
            Some(120),
            "bug: a trimmed list hides its total"
        );
        let listed: Vec<&str> = answered
            .efforts
            .iter()
            .map(|e| e.activity_id.as_str())
            .collect();
        assert!(
            listed.contains(&"0"),
            "bug: the segment's best was trimmed: {listed:?}"
        );
        assert!(
            listed.contains(&"60"),
            "bug: the ranked effort was trimmed: {listed:?}"
        );
        assert!(
            listed.contains(&"119"),
            "bug: the newest effort was trimmed: {listed:?}"
        );
        assert!(
            answered
                .efforts
                .windows(2)
                .all(|pair| pair[0].start_date >= pair[1].start_date),
            "bug: the listed efforts are not newest first"
        );
        let summary = answered
            .summary
            .first()
            .map(|summary| (summary.count, summary.best_elapsed_time));
        assert_eq!(
            summary,
            Some((120, 300)),
            "bug: the summary counts only the listed efforts"
        );

        let short = answer(
            (0..3).map(|day| ridden(day, 400, None)).collect(),
            AnsweredBy::StoredDetails,
            segments_read_coverage(
                None,
                0,
                (NaiveDate::MIN, NaiveDate::MIN),
                Completeness::Complete,
                None,
            ),
            Tz::UTC,
        );
        assert_eq!(short.efforts.len(), 3);
        assert_eq!(
            short.efforts_total, None,
            "bug: an untrimmed list names a total"
        );
    }
}
