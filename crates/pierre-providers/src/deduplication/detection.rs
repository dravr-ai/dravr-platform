// ABOUTME: Temporal-overlap fragment grouping for cageux Activities
// ABOUTME: Non-destructive: emits FragmentReport sidecar, never mutates Activity stream
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Fragment-detection algorithm.
//!
//! See [`detect_fragments`] for the public entry point. The algorithm runs in
//! O(n log n) time dominated by the per-`sport_type` sort: activities are
//! grouped by sport type, each bucket sorted by start time, and a single
//! linear sweep collapses temporally-overlapping rows into fragment groups.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Activity, SportType};
use serde::Serialize;

use super::config::DedupConfig;

/// Result of fragment detection over an activity collection.
///
/// `raw_count` is the original row count; `session_count` collapses each
/// fragment group into one canonical session. `groups` lists only multi-member
/// groups — single-row activities are absent from the vector.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentReport {
    /// Total activities the detector saw, including all fragments.
    pub raw_count: usize,
    /// Distinct training sessions after collapsing each fragment group to one.
    pub session_count: usize,
    /// One entry per group of two or more overlapping activities. Single-row
    /// activities (no overlap) are not represented.
    pub groups: Vec<FragmentGroup>,
}

/// A group of activities that all describe the same physical workout.
///
/// `canonical_id` is the row a consumer should treat as the "real" session.
/// The remaining members of `fragment_ids` (excluding the canonical) are
/// recordings to ignore for counts, volume, and TSS purposes. `fragment_ids`
/// always includes `canonical_id` to keep the group self-contained.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentGroup {
    /// Activity id of the canonical member of the group.
    pub canonical_id: String,
    /// All member ids, canonical included.
    pub fragment_ids: Vec<String>,
    /// Sport type shared by every member of the group.
    pub sport_type: SportType,
    /// Earliest start time across all members.
    pub window_start: DateTime<Utc>,
    /// Latest end time (start + duration) across all members.
    pub window_end: DateTime<Utc>,
}

impl FragmentReport {
    /// Returns `true` when at least one multi-member group was detected.
    #[must_use]
    pub fn has_fragments(&self) -> bool {
        !self.groups.is_empty()
    }

    /// Returns `true` when the activity id is either un-grouped (a session of
    /// its own) or the canonical member of its group. Use this when filtering
    /// an activity slice down to one row per distinct session.
    #[must_use]
    pub fn is_canonical(&self, activity_id: &str) -> bool {
        self.groups
            .iter()
            .find(|g| g.fragment_ids.iter().any(|id| id == activity_id))
            .is_none_or(|g| g.canonical_id == activity_id)
    }

    /// Look up the fragment group that contains `activity_id`, if any.
    #[must_use]
    pub fn group_of(&self, activity_id: &str) -> Option<&FragmentGroup> {
        self.groups
            .iter()
            .find(|g| g.fragment_ids.iter().any(|id| id == activity_id))
    }

    /// Filter `activities` down to canonical rows only — one row per session.
    /// Activities not part of any group are retained unchanged.
    #[must_use]
    pub fn canonical_activities(&self, activities: &[Activity]) -> Vec<Activity> {
        if self.groups.is_empty() {
            return activities.to_vec();
        }
        activities
            .iter()
            .filter(|a| self.is_canonical(a.id()))
            .cloned()
            .collect()
    }
}

/// Detect fragment groups in the provided activity slice.
///
/// Activities are bucketed by [`SportType`]; each bucket is sorted ascending
/// by `start_date` and swept linearly to group rows whose `[start, end]`
/// windows intersect within `config.overlap_tolerance_secs`. Canonical row is
/// picked deterministically: longest duration first, then largest distance,
/// then lowest id (string compare) for tie-breaking.
///
/// Empty input returns an empty report (no groups, `raw_count = 0`,
/// `session_count = 0`). Single-activity input is reported with
/// `session_count = 1` and no groups.
#[must_use]
pub fn detect_fragments(activities: &[Activity], config: &DedupConfig) -> FragmentReport {
    let raw_count = activities.len();
    if raw_count < 2 {
        return FragmentReport {
            raw_count,
            session_count: raw_count,
            groups: Vec::new(),
        };
    }

    let buckets = bucket_by_sport(activities);
    let tolerance = Duration::seconds(i64::try_from(config.overlap_tolerance_secs).unwrap_or(0));
    let mut groups = Vec::new();
    for (_sport, mut bucket) in buckets {
        bucket.sort_by_key(|a| a.start_date());
        groups.extend(sweep_bucket(&bucket, tolerance));
    }
    // Deterministic ordering of the report itself — sort groups by their
    // earliest start time so consumers (and the LLM rendering this) see
    // chronological grouping rather than sport_type bucket order.
    groups.sort_by_key(|g| g.window_start);

    let collapsed: usize = groups.iter().map(|g| g.fragment_ids.len() - 1).sum();
    FragmentReport {
        raw_count,
        session_count: raw_count - collapsed,
        groups,
    }
}

/// Partition activities into per-sport buckets. Uses [`BTreeMap`] keyed by the
/// serialized sport-type token so the iteration order is stable across runs
/// for any given input — important for reproducible test assertions and for
/// the chronological sort that follows.
fn bucket_by_sport(activities: &[Activity]) -> BTreeMap<String, Vec<&Activity>> {
    let mut buckets: BTreeMap<String, Vec<&Activity>> = BTreeMap::new();
    for activity in activities {
        let key = sport_key(activity.sport_type());
        buckets.entry(key).or_default().push(activity);
    }
    buckets
}

/// Stable string key for a [`SportType`]. `SportType::Other("Trail Run")` is
/// distinguished from `SportType::Run`; this is intentional for v1 — coalescing
/// near-cousin sport categories is left to future work once a `SportCategory`
/// abstraction exists upstream in dravr-cageux.
fn sport_key(sport: &SportType) -> String {
    match sport {
        SportType::Other(label) => format!("other:{label}"),
        named => format!("{named:?}"),
    }
}

/// Single-pass sweep over a sport bucket sorted by `start_date`. Each activity
/// either extends the current open group (when it overlaps the group's running
/// `window_end + tolerance`) or starts a fresh one.
fn sweep_bucket(bucket: &[&Activity], tolerance: Duration) -> Vec<FragmentGroup> {
    let mut groups = Vec::new();
    let mut current: Option<GroupBuilder> = None;

    for activity in bucket {
        let start = activity.start_date();
        let end = activity_end(activity);

        match current.as_mut() {
            Some(building) if start <= building.window_end + tolerance => {
                building.push(activity, end);
            }
            _ => {
                if let Some(finished) = current.take() {
                    if let Some(group) = finished.finalize() {
                        groups.push(group);
                    }
                }
                current = Some(GroupBuilder::new(activity, start, end));
            }
        }
    }
    if let Some(finished) = current.take() {
        if let Some(group) = finished.finalize() {
            groups.push(group);
        }
    }
    groups
}

/// Activity end timestamp = `start_date + duration_seconds`. Saturates on
/// overflow (impossible for plausible durations but keeps the arithmetic
/// total). `duration_seconds = 0` yields `end == start`, which still allows a
/// zero-length row to be grouped if it falls inside another row's window.
fn activity_end(activity: &Activity) -> DateTime<Utc> {
    let duration =
        Duration::seconds(i64::try_from(activity.duration_seconds()).unwrap_or(i64::MAX));
    activity.start_date() + duration
}

/// Accumulator used during the single-pass sweep. `members` collects
/// (id, duration, distance) tuples that drive canonical selection in
/// [`GroupBuilder::finalize`].
struct GroupBuilder {
    sport_type: SportType,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    members: Vec<MemberCandidate>,
}

struct MemberCandidate {
    id: String,
    duration_seconds: u64,
    distance_meters: f64,
}

impl GroupBuilder {
    fn new(first: &Activity, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self {
            sport_type: first.sport_type().clone(),
            window_start: start,
            window_end: end,
            members: vec![MemberCandidate::from(first)],
        }
    }

    fn push(&mut self, activity: &Activity, end: DateTime<Utc>) {
        if end > self.window_end {
            self.window_end = end;
        }
        self.members.push(MemberCandidate::from(activity));
    }

    /// Returns `Some(group)` when the accumulated bucket holds two or more
    /// members; single-member buckets are not fragments and return `None`.
    fn finalize(mut self) -> Option<FragmentGroup> {
        if self.members.len() < 2 {
            return None;
        }
        // Canonical = longest duration, then largest distance, then lowest id.
        // The negation on duration/distance keeps the comparator monotonic
        // ascending; combined with `id` (already ascending) it yields a
        // total order without f64 NaN traps because distance is finite.
        self.members.sort_by(|a, b| {
            b.duration_seconds
                .cmp(&a.duration_seconds)
                .then_with(|| {
                    b.distance_meters
                        .partial_cmp(&a.distance_meters)
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| a.id.cmp(&b.id))
        });
        let canonical_id = self.members[0].id.clone();
        let fragment_ids = self.members.into_iter().map(|m| m.id).collect();
        Some(FragmentGroup {
            canonical_id,
            fragment_ids,
            sport_type: self.sport_type,
            window_start: self.window_start,
            window_end: self.window_end,
        })
    }
}

impl From<&&Activity> for MemberCandidate {
    fn from(activity: &&Activity) -> Self {
        Self {
            id: (*activity).id().to_owned(),
            duration_seconds: (*activity).duration_seconds(),
            distance_meters: (*activity).distance_meters().unwrap_or(0.0),
        }
    }
}

impl From<&Activity> for MemberCandidate {
    fn from(activity: &Activity) -> Self {
        Self {
            id: activity.id().to_owned(),
            duration_seconds: activity.duration_seconds(),
            distance_meters: activity.distance_meters().unwrap_or(0.0),
        }
    }
}
