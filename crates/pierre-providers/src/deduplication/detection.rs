// ABOUTME: Groups recordings of one physical workout and merges each group into one session
// ABOUTME: Fragment, cross-provider and cross-sport matching; canonical row enriched from peers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Session-merging algorithm.
//!
//! See [`merge_duplicates`] for the public entry point. Activities are sorted
//! by start time and every pair close enough in time to be the same workout is
//! tested against the matching rules; matching pairs are joined into groups
//! with a union-find, so a chain (watch ≈ bike computer ≈ Strava upload)
//! lands in one group whatever order the rows arrived in.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveTime, Utc};
use pierre_core::models::{Activity, SportType};
use serde::Serialize;

use super::config::DedupConfig;

/// Minimum fraction of the shorter activity's duration that must overlap
/// with the longer one for records of DIFFERENT sports to be one workout.
///
/// Wrist-based trackers (WHOOP, Apple Watch) frequently misclassify
/// activities — a long bike ride shows up as a `Run` because the wrist moves
/// enough to trigger the wrong heuristic, while the GPS provider (Strava)
/// reports the same physical session as `Ride`. Substantial wall-clock overlap
/// between two providers' records is strong evidence of one workout whatever
/// the declared sport label.
const CROSS_SPORT_OVERLAP_FRACTION: f64 = 0.6;

/// Minimum share of the canonical row's duration a peer must cover (in
/// percent) to be another recording of the WHOLE session and so fill the
/// canonical row's missing fields.
///
/// A Garmin auto-split fragment covering one hour of a three-hour ride shares
/// the session but not its summary: its average heart rate or calories describe
/// that hour alone, so copying them onto the session would misreport it. Peers
/// below this share still collapse into the group, they just donate nothing.
const FULL_RECORDING_MIN_PERCENT: u64 = 80;

/// Result of session merging over an activity collection.
///
/// `raw_count` is the original row count; `session_count` collapses each
/// group into one canonical session. `groups` lists only multi-member
/// groups — single-row activities are absent from the vector.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentReport {
    /// Total activities the merger saw, including every recording.
    pub raw_count: usize,
    /// Distinct training sessions after collapsing each group to one.
    pub session_count: usize,
    /// One entry per group of two or more recordings of the same workout.
    pub groups: Vec<FragmentGroup>,
}

/// A group of activities that all describe the same physical workout.
///
/// `canonical_id` is the row that stands for the session; the other members
/// of `fragment_ids` were folded into it and removed from the merged list.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentGroup {
    /// Activity id of the canonical member of the group.
    pub canonical_id: String,
    /// All member ids, canonical first.
    pub fragment_ids: Vec<String>,
    /// Providers that recorded the session, canonical's first, each once.
    pub providers: Vec<String>,
    /// Sport type of the canonical member.
    pub sport_type: SportType,
    /// Earliest start time across all members.
    pub window_start: DateTime<Utc>,
    /// Latest end time (start + duration) across all members.
    pub window_end: DateTime<Utc>,
    /// Fields the canonical row lacked and took from another recording.
    pub filled_fields: Vec<FilledField>,
}

/// One field a canonical row took from a peer recording of its session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FilledField {
    /// Activity field name, as the canonical model spells it.
    pub field: &'static str,
    /// Provider of the recording the value came from.
    pub provider: String,
}

impl FragmentReport {
    /// Returns `true` when at least one multi-member group was merged.
    #[must_use]
    pub fn has_fragments(&self) -> bool {
        !self.groups.is_empty()
    }

    /// The part of this report that describes `sessions`, a page of the
    /// merged list: the groups whose canonical row is on the page, with the
    /// counts recomputed for that page alone.
    ///
    /// A merge runs over the whole fetched window, which can hold two
    /// thousand rows; a caller showing twenty of them reports on those twenty.
    #[must_use]
    pub fn scoped_to(&self, sessions: &[Activity]) -> Self {
        let groups: Vec<FragmentGroup> = self
            .groups
            .iter()
            .filter(|group| sessions.iter().any(|a| a.id() == group.canonical_id))
            .cloned()
            .collect();
        let folded: usize = groups.iter().map(|g| g.fragment_ids.len() - 1).sum();
        Self {
            raw_count: sessions.len() + folded,
            session_count: sessions.len(),
            groups,
        }
    }
}

/// Collapse every group of recordings of one workout into a single session.
///
/// Returns the merged list — input order preserved, each group represented by
/// its canonical row at that row's position — and a report naming every group
/// and every field the canonical row took from a peer.
///
/// Two activities are recordings of one workout when:
/// - **same sport, touching in time**: their `[start, end]` windows intersect
///   within `overlap_tolerance_secs` (auto-splits, re-uploads, a watch and a
///   bike computer on one provider or two), or
/// - **same sport, two providers**: their starts sit within
///   `time_window_minutes` — or on the same calendar day when either side is a
///   date-only scrape (`T00:00:00`) — and their distances agree within
///   `distance_tolerance_pct` (or either lacks a distance), or
/// - **different sports, two providers**: their windows overlap by at least
///   [`CROSS_SPORT_OVERLAP_FRACTION`] of the shorter one.
///
/// Rows with no real start time never match on overlap: every midnight row
/// "overlaps" every other, which would fold distinct sessions together.
///
/// The canonical row is the one carrying a distance (GPS), then the longest,
/// then the farthest, then the lowest id. Each other member covering at least
/// [`FULL_RECORDING_MIN_PERCENT`] of its duration then fills the fields it
/// lacks, best-ranked member first; a field the canonical row carries is never
/// overwritten.
#[must_use]
pub fn merge_duplicates(
    activities: Vec<Activity>,
    config: &DedupConfig,
) -> (Vec<Activity>, FragmentReport) {
    let raw_count = activities.len();
    let mut member_sets = group_members(&activities, config);
    for members in &mut member_sets {
        members.sort_by(|&a, &b| canonical_order(&activities[a], &activities[b]));
    }

    let mut slots: Vec<Option<Activity>> = activities.into_iter().map(Some).collect();
    let mut groups: Vec<FragmentGroup> = member_sets
        .iter()
        .filter_map(|members| fold_group(&mut slots, members))
        .collect();
    groups.sort_by_key(|g| g.window_start);

    let merged: Vec<Activity> = slots.into_iter().flatten().collect();
    let report = FragmentReport {
        raw_count,
        session_count: merged.len(),
        groups,
    };
    (merged, report)
}

/// Fold one group (members ranked canonical-first) into its canonical slot,
/// emptying every other member's slot, and describe what was merged. Groups
/// are disjoint, so every slot a group names is still occupied.
fn fold_group(slots: &mut [Option<Activity>], members: &[usize]) -> Option<FragmentGroup> {
    let (&canonical_index, peers) = members.split_first()?;
    let mut canonical = slots[canonical_index].take()?;

    let mut fragment_ids = vec![canonical.id().to_owned()];
    let mut providers = vec![canonical.provider().to_owned()];
    let mut window_start = canonical.start_date();
    let mut window_end = activity_end(&canonical);
    let mut filled_fields = Vec::new();

    for peer in peers.iter().filter_map(|&index| slots[index].take()) {
        fragment_ids.push(peer.id().to_owned());
        if !providers.iter().any(|p| p == peer.provider()) {
            providers.push(peer.provider().to_owned());
        }
        window_start = window_start.min(peer.start_date());
        window_end = window_end.max(activity_end(&peer));
        if covers_session(&peer, &canonical) {
            filled_fields.extend(canonical.fill_missing_from(&peer).into_iter().map(|field| {
                FilledField {
                    field,
                    provider: peer.provider().to_owned(),
                }
            }));
        }
    }

    let group = FragmentGroup {
        canonical_id: canonical.id().to_owned(),
        fragment_ids,
        providers,
        sport_type: canonical.sport_type().clone(),
        window_start,
        window_end,
        filled_fields,
    };
    slots[canonical_index] = Some(canonical);
    Some(group)
}

/// True when `peer` recorded at least [`FULL_RECORDING_MIN_PERCENT`] of the
/// canonical row's duration — another recording of the whole session rather
/// than a piece of it.
fn covers_session(peer: &Activity, canonical: &Activity) -> bool {
    peer.duration_seconds().saturating_mul(100)
        >= canonical
            .duration_seconds()
            .saturating_mul(FULL_RECORDING_MIN_PERCENT)
}

/// Canonical ranking: carries a distance, then longest, then farthest, then
/// lowest id. A total order, so the pick is deterministic for any input.
fn canonical_order(a: &Activity, b: &Activity) -> Ordering {
    has_distance(b)
        .cmp(&has_distance(a))
        .then_with(|| b.duration_seconds().cmp(&a.duration_seconds()))
        .then_with(|| {
            distance_of(b)
                .partial_cmp(&distance_of(a))
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| a.id().cmp(b.id()))
}

fn has_distance(activity: &Activity) -> bool {
    activity.distance_meters().is_some_and(|d| d > 0.0)
}

fn distance_of(activity: &Activity) -> f64 {
    activity.distance_meters().unwrap_or(0.0)
}

/// Partition activity indices into groups of two or more recordings of one
/// workout. Groups come back in no particular order; singletons are dropped.
fn group_members(activities: &[Activity], config: &DedupConfig) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..activities.len()).collect();
    order.sort_by_key(|&i| activities[i].start_date());

    let slack = Duration::seconds(i64::try_from(config.overlap_tolerance_secs).unwrap_or(0))
        .max(Duration::minutes(config.time_window_minutes));
    let mut sets = DisjointSets::new(activities.len());
    for (pos, &i) in order.iter().enumerate() {
        let a = &activities[i];
        let horizon = activity_end(a) + slack;
        for &j in &order[pos + 1..] {
            let b = &activities[j];
            // Sorted by start: once `b` starts past `a`'s horizon on a later
            // calendar day, no later row can match `a` either. Same-day rows
            // stay in reach for the date-only scrape match.
            if b.start_date() > horizon && b.start_date().date_naive() > a.start_date().date_naive()
            {
                break;
            }
            if same_workout(a, b, config) {
                sets.union(i, j);
            }
        }
    }

    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..activities.len() {
        by_root.entry(sets.find(index)).or_default().push(index);
    }
    by_root.into_values().filter(|m| m.len() > 1).collect()
}

/// The matching rules documented on [`merge_duplicates`].
fn same_workout(a: &Activity, b: &Activity, config: &DedupConfig) -> bool {
    let two_providers = a.provider() != b.provider();
    let both_timed = has_known_time(a) && has_known_time(b);
    if sport_key(a.sport_type()) == sport_key(b.sport_type()) {
        (both_timed && windows_touch(a, b, config.overlap_tolerance_secs))
            || (two_providers && starts_and_distances_agree(a, b, config))
    } else {
        two_providers && both_timed && overlaps_substantially(a, b)
    }
}

/// Same-sport windows intersect within `tolerance_secs` of each other.
fn windows_touch(a: &Activity, b: &Activity, tolerance_secs: u64) -> bool {
    let tolerance = Duration::seconds(i64::try_from(tolerance_secs).unwrap_or(0));
    a.start_date() <= activity_end(b) + tolerance && b.start_date() <= activity_end(a) + tolerance
}

/// Two providers' records of one sport: starts close (or the same day when
/// either is a date-only scrape) and distances within tolerance when both
/// carry one.
fn starts_and_distances_agree(a: &Activity, b: &Activity, config: &DedupConfig) -> bool {
    let starts_agree = if has_known_time(a) && has_known_time(b) {
        (a.start_date() - b.start_date()).num_minutes().abs() <= config.time_window_minutes
    } else {
        a.start_date().date_naive() == b.start_date().date_naive()
    };
    if !starts_agree {
        return false;
    }
    match (a.distance_meters(), b.distance_meters()) {
        (Some(da), Some(db)) => {
            let longer = da.max(db);
            longer > 0.0 && (da - db).abs() / longer * 100.0 < config.distance_tolerance_pct
        }
        _ => true,
    }
}

/// Wall-clock overlap covers at least [`CROSS_SPORT_OVERLAP_FRACTION`] of the
/// shorter activity. Zero-length rows never qualify.
fn overlaps_substantially(a: &Activity, b: &Activity) -> bool {
    let shorter = a.duration_seconds().min(b.duration_seconds());
    if shorter == 0 {
        return false;
    }
    let overlap_start = a.start_date().max(b.start_date());
    let overlap_end = activity_end(a).min(activity_end(b));
    let overlap = (overlap_end - overlap_start).num_seconds();
    if overlap <= 0 {
        return false;
    }
    #[allow(clippy::cast_precision_loss)]
    // bounded by single-activity durations; far below f64 limits
    let fraction = overlap as f64 / shorter as f64;
    fraction >= CROSS_SPORT_OVERLAP_FRACTION
}

/// True when the activity has a real start time-of-day. Sciotte's date-only
/// list scraping yields `T00:00:00` (UTC midnight) when the start time is
/// unknown; such rows never match on temporal overlap.
fn has_known_time(activity: &Activity) -> bool {
    activity.start_date().time() != NaiveTime::MIN
}

/// Stable string key for a [`SportType`]. `SportType::Other("Trail Run")` is
/// distinguished from `SportType::Run`; coalescing near-cousin sports is the
/// cross-sport overlap rule's job, which demands two providers.
fn sport_key(sport: &SportType) -> String {
    match sport {
        SportType::Other(label) => format!("other:{label}"),
        named => format!("{named:?}"),
    }
}

/// Activity end timestamp = `start_date + duration_seconds`. Saturates on
/// overflow (impossible for plausible durations but keeps the arithmetic
/// total).
fn activity_end(activity: &Activity) -> DateTime<Utc> {
    let duration =
        Duration::seconds(i64::try_from(activity.duration_seconds()).unwrap_or(i64::MAX));
    activity.start_date() + duration
}

/// Minimal union-find over activity indices, path-halving on `find`.
struct DisjointSets {
    parent: Vec<usize>,
}

impl DisjointSets {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    fn find(&mut self, mut index: usize) -> usize {
        while self.parent[index] != index {
            self.parent[index] = self.parent[self.parent[index]];
            index = self.parent[index];
        }
        index
    }

    fn union(&mut self, a: usize, b: usize) {
        let (root_a, root_b) = (self.find(a), self.find(b));
        if root_a != root_b {
            self.parent[root_b.max(root_a)] = root_a.min(root_b);
        }
    }
}
