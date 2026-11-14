// ABOUTME: What the activity cache records beside a cached activity: a fetch that did not count as a sync, a detail read
// ABOUTME: Re-exported from activity_cache, whose repository stores and returns them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::models::{Activity, GearRef, Lap, SegmentEffort, Split};
use serde::{Deserialize, Serialize};

#[cfg(doc)]
use super::ActivityCacheRepository;

/// Why an activity fetch did not count as a sync.
///
/// Only a provider answer that vouches for its list moves freshness. Each
/// of these leaves freshness where it was and is recorded instead, so the
/// head stays stale — the next load refreshes it again — and Home can say
/// the attempt failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityFetchFailure {
    /// The provider could not be reached or refused the read: a transport
    /// error, a scrape error, a timeout.
    FetchError,
    /// The provider answered, but said its capture never read the list head
    /// (`head_complete: false`), so the newest activities may be missing.
    HeadIncomplete,
    /// The provider answered with no activities for a window the cache holds
    /// activities in. Activities do not vanish from a provider between two
    /// reads, so an empty answer over them is a failed read presented as
    /// success, not an honest zero.
    EmptyOverCached,
}

impl ActivityFetchFailure {
    /// The slug the stored row carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FetchError => "fetch_error",
            Self::HeadIncomplete => "head_incomplete",
            Self::EmptyOverCached => "empty_over_cached",
        }
    }

    /// The failure a stored slug names, or `None` for a slug this build does
    /// not write.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "fetch_error" => Some(Self::FetchError),
            "head_incomplete" => Some(Self::HeadIncomplete),
            "empty_over_cached" => Some(Self::EmptyOverCached),
            _ => None,
        }
    }
}

/// The last recorded fetch of a provider that did not count as a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityFetchFailureRecord {
    /// When it happened.
    pub failed_at: DateTime<Utc>,
    /// Why it did not count as a sync.
    pub failure: ActivityFetchFailure,
    /// How many fetches in a row had failed when it was recorded: counted
    /// from the provider's last good fetch, so the first failure after a
    /// sync is `1`.
    pub consecutive: u32,
    /// How many of those, ending with this one, failed for this same
    /// reason: `1` when the one before it failed otherwise or was a sync.
    pub streak: u32,
    /// When the first failure of that streak happened; `failed_at` itself
    /// for a streak of one.
    pub streak_started_at: DateTime<Utc>,
}

/// What a detail read of an activity found that its list read never carries.
///
/// A provider serves splits, laps, the athlete's description, the recording
/// device and the segment efforts only on a detail read, while every list
/// write-through replaces the cached activity whole. Stored beside the list
/// copy ([`ActivityCacheRepository::store_activity_detail`]), never inside
/// it, and read back into every cached activity whose list copy lacks them:
/// a detail read once is served from here from then on. The keys are the
/// [`Activity`] fields they fill, under the same names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActivityDetail {
    /// The provider's uniform distance buckets, in order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub splits: Option<Vec<Split>>,
    /// The laps the athlete or the workout marked, in order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub laps: Option<Vec<Lap>>,
    /// The athlete's own description of the activity. Untrusted text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The device that recorded the activity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// The segment efforts the activity holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_efforts: Option<Vec<SegmentEffort>>,
    /// The gear the activity was recorded with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<GearRef>,
    /// The athlete's private note. The athlete's alone: readers other than
    /// the athlete strip it before serializing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_note: Option<String>,
}

impl ActivityDetail {
    /// The detail-only fields a detailed read of `activity` carried. A read
    /// that carried none of them yields an empty detail ([`Self::is_empty`]):
    /// stored, it fills nothing, and it answers the row only until its
    /// recheck instant ([`ActivityCacheRepository::store_activity_detail`]).
    #[must_use]
    pub fn from_activity(activity: &Activity) -> Self {
        Self {
            splits: activity.splits().cloned(),
            laps: activity.laps().cloned(),
            description: activity
                .description()
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned),
            device_name: activity.device_name().map(str::to_owned),
            segment_efforts: activity.segment_efforts().cloned(),
            gear: activity.gear().cloned(),
            private_note: activity.private_note().map(str::to_owned),
        }
    }

    /// Whether the read carried none of the detail-only fields: an activity
    /// that has none reads this way, and so does one whose provider served it
    /// without them because the request carrying them failed.
    ///
    /// An empty efforts list counts as none. A Strava API detail read always
    /// stamps its efforts, `[]` for a ride over no segment, and that stamp
    /// alone proves no more than an empty read does, so the read is checked
    /// again at its recheck instant. The stamp still serializes, and that key
    /// is what marks the row's segments as read.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        let no_efforts = match &self.segment_efforts {
            None => true,
            Some(efforts) => efforts.is_empty(),
        };
        self.splits.is_none()
            && self.laps.is_none()
            && self.description.is_none()
            && self.device_name.is_none()
            && no_efforts
            && self.gear.is_none()
            && self.private_note.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::ActivityDetail;

    /// A Strava API detail read always stamps its segment efforts, an empty
    /// list for a ride over no segment. That stamp alone fills nothing, so
    /// the read stays empty and is checked again; the key still serializes,
    /// which is what marks the row's segments as read.
    #[test]
    fn an_empty_efforts_list_alone_leaves_the_detail_empty() {
        let stamped = ActivityDetail {
            segment_efforts: Some(Vec::new()),
            ..ActivityDetail::default()
        };
        assert!(
            stamped.is_empty(),
            "bug: an empty efforts list fills nothing, so the read must be rechecked"
        );
        let json = serde_json::to_string(&stamped).unwrap();
        assert_eq!(
            json, r#"{"segment_efforts":[]}"#,
            "bug: the empty efforts list must still serialize as the segment-read stamp"
        );
        let with_laps = ActivityDetail {
            laps: Some(Vec::new()),
            segment_efforts: Some(Vec::new()),
            ..ActivityDetail::default()
        };
        assert!(
            !with_laps.is_empty(),
            "bug: only the efforts list is read through to its contents"
        );
    }
}
