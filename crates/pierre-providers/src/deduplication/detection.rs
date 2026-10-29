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
use std::collections::{BTreeMap, HashMap};
use std::{iter, mem};

use chrono::{DateTime, Duration, NaiveTime, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{Activity, SportType};
use serde::Serialize;

use super::config::DedupConfig;
use crate::backend_resolver;

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

/// Minimum share of the session's duration a recording must cover (in
/// percent) to be another recording of the WHOLE session and so fill the
/// canonical row's missing fields.
///
/// A second device that recorded one hour of a three-hour ride shares the
/// session but not its summary: its average heart rate or calories describe
/// that hour alone, so copying them onto the session would misreport it.
/// Recordings below this share still collapse into the group, they just donate
/// nothing.
const FULL_RECORDING_MIN_PERCENT: u64 = 80;

/// Largest share of the shorter row's duration (in percent) two timed rows of
/// a group may share and still be two pieces of the session rather than two
/// recordings of the same time.
///
/// An auto-split leaves pieces that touch end to start, give or take a few
/// seconds of clock skew; two devices recording the same ride share nearly all
/// of it. Anything past this share is the same time recorded twice.
const PIECE_MAX_OVERLAP_PERCENT: i64 = 10;

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
/// "overlaps" every other, which would fold distinct sessions together. A
/// date-only row pairs with one recording of its day, never with all of them:
/// a pairing is refused when it would put two distinct activities of one
/// provider in one group, so two same-day runs captured date-only stay two
/// workouts beside the two timed rows of another connection.
///
/// Whatever their times and sports, rows that carry one service's own
/// activity id are one workout ([`native_activity_id`]): the Garmin API's row
/// and the usr/pwd capture of the same Garmin activity (carnet#1155); the
/// Strava API's row, the usr/pwd capture and the athlete's imported export of
/// the same Strava activity (carnet#1242); likewise for TrainingPeaks and
/// COROS, the other services read both ways. The API row stands for it, then the
/// export, then the capture, whatever the ranking below says, and it takes
/// the fields it lacks from the others.
///
/// The canonical row is the one carrying a GPS trace ([`carries_gps_trace`]),
/// then a distance, then the longest, then the farthest, then the lowest id.
///
/// Every other member is then either a piece of the session or a recording of
/// it. A **piece** recorded time no row already counted recorded: it has the
/// canonical row's sport, it is not one service's activity id with any
/// better-ranked member (a piece or a recording), both have real start times,
/// and they share at most [`PIECE_MAX_OVERLAP_PERCENT`] of the shorter one's
/// duration. A row of another sport tied in by the cross-sport rule is never a
/// piece: a brick's run is not kilometres of its ride. Members are tried
/// best-ranked first against the canonical row and the pieces already
/// counted, and the session sums the volume of every piece — distance, moving
/// time, elevation gain, and the calories, steps, provider training stress and
/// suffer score when every piece carries them — its elapsed time running from
/// the earliest piece's start to the latest one's end, stops between them
/// included (`Activity::extend_with_piece`): a ride the recorder split into
/// 70 km and 30 km is a 100 km session. Every other member is a **recording**
/// of time already counted — a second device, a second provider, an export or
/// a capture of the same activity — and adds no volume, so the same kilometre
/// is never counted twice. A recording that covers the canonical row's time
/// and the pieces' stands over their sum: when it ranks canonical, the pieces
/// overlap it and are recordings of it; when a piece ranks canonical, the
/// other pieces are summed and the whole-session recording, overlapping them,
/// adds nothing.
///
/// Each recording covering at least [`FULL_RECORDING_MIN_PERCENT`] of the
/// session's summed duration then fills the fields the canonical row lacks,
/// best-ranked member first; a field the canonical row carries is never
/// overwritten. A piece fills nothing: its averages describe its own part.
#[must_use]
pub fn merge_duplicates(
    activities: Vec<Activity>,
    config: &DedupConfig,
) -> (Vec<Activity>, FragmentReport) {
    let raw_count = activities.len();
    let mut member_sets = group_members(&activities, config);
    for members in &mut member_sets {
        members.sort_by(|&a, &b| canonical_order(&activities[a], &activities[b]));
        lift_native_twins(&activities, members);
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

/// The provider a merged recording is named by in the group's description,
/// which reaches the model as the activity list's merge line and as
/// `fragment_dedup`: the service the athlete connected, never a backend.
fn origin_name(activity: &Activity) -> String {
    backend_resolver::activity_origin(activity.provider(), activity.source()).to_owned()
}

/// Fold one group (members ranked canonical-first) into its canonical slot,
/// emptying every other member's slot, and describe what was merged. Groups
/// are disjoint, so every slot a group names is still occupied.
///
/// The canonical row first takes the volume of every piece of the session,
/// then the missing fields of every recording of the whole of it, as
/// [`merge_duplicates`] documents.
fn fold_group(slots: &mut [Option<Activity>], members: &[usize]) -> Option<FragmentGroup> {
    let (&canonical_index, peer_indices) = members.split_first()?;
    let mut canonical = slots[canonical_index].take()?;
    let peers: Vec<Activity> = peer_indices
        .iter()
        .filter_map(|&index| slots[index].take())
        .collect();

    let mut fragment_ids = vec![canonical.id().to_owned()];
    let mut providers = vec![origin_name(&canonical)];
    let mut window_start = canonical.start_date();
    let mut window_end = activity_end(&canonical);
    for peer in &peers {
        fragment_ids.push(peer.id().to_owned());
        let peer_origin = origin_name(peer);
        if !providers.contains(&peer_origin) {
            providers.push(peer_origin);
        }
        window_start = window_start.min(peer.start_date());
        window_end = window_end.max(activity_end(peer));
    }

    let pieces = classify_pieces(&canonical, &peers);
    for (peer, _) in peers.iter().zip(&pieces).filter(|(_, &piece)| piece) {
        canonical.extend_with_piece(peer);
    }
    let mut filled_fields = Vec::new();
    for (peer, _) in peers.iter().zip(&pieces).filter(|(_, &piece)| !piece) {
        if covers_session(peer, &canonical) {
            filled_fields.extend(canonical.fill_missing_from(peer).into_iter().map(|field| {
                FilledField {
                    field,
                    provider: origin_name(peer),
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

/// Whether `recording` is the row a merged `session` stands on.
///
/// A caller holding the rows it merged finds that row's stored key by it: the
/// merger never changes a canonical row's provider or id, and adding the pieces of
/// its session only moves its start earlier and its duration up.
#[must_use]
pub fn is_canonical_copy(recording: &Activity, session: &Activity) -> bool {
    recording.provider() == session.provider()
        && recording.id() == session.id()
        && session.start_date() <= recording.start_date()
        && recording.duration_seconds() <= session.duration_seconds()
}

/// Which of `peers` (ranked best first) are pieces of the session the
/// canonical row stands for, as [`merge_duplicates`] defines them: each is
/// tried against the canonical row and every piece already counted, so of two
/// copies of one stretch only the better-ranked is a piece.
///
/// A peer must also share the canonical row's sport, and must not be a native
/// twin of any better-ranked member, piece or recording: the twin of a
/// recording is that recording again, whatever clock it carries.
///
/// Read off the rows as recorded, before any piece's volume reaches the
/// canonical row, whose window then spans the whole session.
fn classify_pieces(canonical: &Activity, peers: &[Activity]) -> Vec<bool> {
    let canonical_sport = sport_key(canonical.sport_type());
    let mut counted: Vec<&Activity> = vec![canonical];
    peers
        .iter()
        .enumerate()
        .map(|(position, peer)| {
            let piece = sport_key(peer.sport_type()) == canonical_sport
                && !iter::once(canonical)
                    .chain(&peers[..position])
                    .any(|better| are_native_twins(peer, better))
                && counted.iter().all(|row| is_other_piece(peer, row));
            if piece {
                counted.push(peer);
            }
            piece
        })
        .collect()
}

/// True when `a` and `b` carry one service's own activity id
/// ([`native_activity_id`]): two rows of one workout.
fn are_native_twins(a: &Activity, b: &Activity) -> bool {
    matches!(
        (native_activity_id(a), native_activity_id(b)),
        (Some((service_a, id_a, _)), Some((service_b, id_b, _)))
            if service_a == service_b && id_a == id_b
    )
}

/// True when `a` and `b` recorded two different stretches of time: both have
/// a real start and a duration, and they share at most
/// [`PIECE_MAX_OVERLAP_PERCENT`] of the shorter's duration.
fn is_other_piece(a: &Activity, b: &Activity) -> bool {
    let shorter = a.duration_seconds().min(b.duration_seconds());
    if shorter == 0 || !has_known_time(a) || !has_known_time(b) {
        return false;
    }
    let overlap_start = a.start_date().max(b.start_date());
    let overlap_end = activity_end(a).min(activity_end(b));
    let overlap = (overlap_end - overlap_start).num_seconds().max(0);
    overlap.saturating_mul(100)
        <= i64::try_from(shorter)
            .unwrap_or(i64::MAX)
            .saturating_mul(PIECE_MAX_OVERLAP_PERCENT)
}

/// True when `peer` recorded at least [`FULL_RECORDING_MIN_PERCENT`] of the
/// session's duration (the canonical row's, its pieces' volume included) —
/// another recording of the whole session rather than of a part of it.
fn covers_session(peer: &Activity, canonical: &Activity) -> bool {
    peer.duration_seconds().saturating_mul(100)
        >= canonical
            .duration_seconds()
            .saturating_mul(FULL_RECORDING_MIN_PERCENT)
}

/// How a row of a service's own activity came in, best first: the order in
/// which twins of one activity stand for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TwinPath {
    /// The service's API (`garmin`, `strava`, ...).
    Api,
    /// The athlete's export of the service (`strava_archive`): the original
    /// recording and its real start, but not the edits made since.
    Archive,
    /// The connection using usr/pwd from Dravr (`sciotte`), whose list read
    /// can carry only the day.
    Credentials,
}

/// The service whose own activity id a row is keyed by, the id, and the path
/// the row came in by.
///
/// Every service Dravr reads both through its API and through the connection
/// using usr/pwd (the mirror pairs [`backend_resolver`] holds: Strava, Garmin,
/// TrainingPeaks, COROS) keys both rows by its own activity id: the API's row
/// under the service's name, the capture under the mirror backend with the
/// service as its source. Garmin's API row and capture share Garmin's
/// `activityId` (carnet#1155); Strava's API row, capture and the athlete's
/// imported export (`strava_archive`, keyed by its `Activity ID`) share
/// Strava's activity id (carnet#1242). Within one service the same id is the
/// same workout. `None` for every other row, whose ids name nothing in common.
fn native_activity_id(activity: &Activity) -> Option<(&str, &str, TwinPath)> {
    let provider = activity.provider();
    let (service, path) = if provider == oauth_providers::STRAVA_ARCHIVE {
        (oauth_providers::STRAVA, TwinPath::Archive)
    } else if backend_resolver::is_mirror_backend(provider) {
        (
            backend_resolver::activity_origin(provider, activity.source()),
            TwinPath::Credentials,
        )
    } else if backend_resolver::mirror_backend_for(provider).is_some() {
        (provider, TwinPath::Api)
    } else {
        return None;
    };
    Some((service, activity.id(), path))
}

/// Move each row of a ranked group ahead of every twin (same service, same
/// activity id) that came in by a worse [`TwinPath`], keeping every other
/// position: the API row wins its own activity, then the export, then the
/// usr/pwd capture, while the group's ranking still decides between
/// different activities' rows.
///
/// Done after the sort rather than inside [`canonical_order`]: a rule that
/// compares only twins would not be a total order across the group. Each row
/// lands before the first earlier twin that ranks below it, so the twins of
/// one activity end up in path order whatever order they arrived in.
fn lift_native_twins(activities: &[Activity], members: &mut Vec<usize>) {
    for position in 0..members.len() {
        let Some((service, id, path)) = native_activity_id(&activities[members[position]]) else {
            continue;
        };
        let worse_twin = members[..position].iter().position(|&earlier| {
            native_activity_id(&activities[earlier]).is_some_and(
                |(other_service, other_id, other_path)| {
                    other_service == service && other_id == id && other_path > path
                },
            )
        });
        if let Some(twin_position) = worse_twin {
            let row = members.remove(position);
            members.insert(twin_position, row);
        }
    }
}

/// Canonical ranking: carries a GPS trace, then a distance, then longest, then
/// farthest, then lowest id. A total order, so the pick is deterministic for
/// any input.
///
/// GPS comes first because a wrist tracker's copy can carry a distance of its
/// own (WHOOP reports one) while it names the sport by guess; the recording
/// with a route is the one whose sport, name and figures describe the session.
fn canonical_order(a: &Activity, b: &Activity) -> Ordering {
    carries_gps_trace(b)
        .cmp(&carries_gps_trace(a))
        .then_with(|| has_distance(b).cmp(&has_distance(a)))
        .then_with(|| b.duration_seconds().cmp(&a.duration_seconds()))
        .then_with(|| {
            distance_of(b)
                .partial_cmp(&distance_of(a))
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| a.id().cmp(b.id()))
}

/// Whether the activity's own payload carries a GPS trace: a route overview
/// that is not blank, or a start position.
#[must_use]
pub fn carries_gps_trace(activity: &Activity) -> bool {
    activity
        .summary_polyline()
        .is_some_and(|encoded| !encoded.trim().is_empty())
        || (activity.start_latitude().is_some() && activity.start_longitude().is_some())
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
    // One service's activity id is one workout, whatever the times say: a
    // usr/pwd row scraped date-only sits at midnight, beyond the time-sorted
    // reach of the API row's or the export's exact start, and may carry the
    // sport the athlete relabelled it to.
    let mut by_native_id: HashMap<(&str, &str), usize> = HashMap::new();
    for (index, activity) in activities.iter().enumerate() {
        if let Some((service, id, _)) = native_activity_id(activity) {
            match by_native_id.get(&(service, id)) {
                Some(&first) => sets.union(first, index),
                None => {
                    by_native_id.insert((service, id), index);
                }
            }
        }
    }
    // Date-only pairings wait until every timed pairing has formed its
    // groups, then join closest distance first, so each lands on the
    // recording it describes rather than on whichever came first.
    let mut date_only: Vec<(f64, usize, usize)> = Vec::new();
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
            match workout_match(a, b, config) {
                Match::Timed => sets.union(i, j),
                Match::DateOnly => date_only.push((distance_gap(a, b), i, j)),
                Match::None => {}
            }
        }
    }
    date_only.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(Ordering::Equal));
    for (_, i, j) in date_only {
        let (root_i, root_j) = (sets.find(i), sets.find(j));
        if root_i != root_j
            && !holds_two_activities_of_one_provider(
                activities,
                &sets.members[root_i],
                &sets.members[root_j],
                config,
            )
        {
            sets.union(i, j);
        }
    }

    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..activities.len() {
        by_root.entry(sets.find(index)).or_default().push(index);
    }
    by_root.into_values().filter(|m| m.len() > 1).collect()
}

/// How two rows matched under the rules documented on [`merge_duplicates`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Match {
    /// Not one workout.
    None,
    /// One workout by their real start times.
    Timed,
    /// One workout by day and distance, one side carrying only the day.
    DateOnly,
}

/// The matching rules documented on [`merge_duplicates`].
fn workout_match(a: &Activity, b: &Activity, config: &DedupConfig) -> Match {
    let two_providers = a.provider() != b.provider();
    let both_timed = has_known_time(a) && has_known_time(b);
    let matched = if sport_key(a.sport_type()) == sport_key(b.sport_type()) {
        (both_timed && windows_touch(a, b, config.overlap_tolerance_secs))
            || (two_providers && starts_and_distances_agree(a, b, config))
    } else {
        two_providers && both_timed && overlaps_substantially(a, b)
    };
    match (matched, both_timed) {
        (false, _) => Match::None,
        (true, true) => Match::Timed,
        (true, false) => Match::DateOnly,
    }
}

/// How far apart two rows' distances sit, `0` when either lacks one: the
/// order date-only pairings are tried in.
fn distance_gap(a: &Activity, b: &Activity) -> f64 {
    match (a.distance_meters(), b.distance_meters()) {
        (Some(da), Some(db)) => (da - db).abs(),
        _ => 0.0,
    }
}

/// Whether joining groups `left` and `right` would put two distinct
/// activities of one provider in one group: two of its rows with different
/// ids that are not pieces of one recording (both timed, windows touching).
/// A provider lists each of its workouts once, so two such rows are two
/// workouts, and a date-only row that matches both cannot make them one.
fn holds_two_activities_of_one_provider(
    activities: &[Activity],
    left: &[usize],
    right: &[usize],
    config: &DedupConfig,
) -> bool {
    left.iter().any(|&x| {
        right.iter().any(|&y| {
            let (a, b) = (&activities[x], &activities[y]);
            a.provider() == b.provider()
                && a.id() != b.id()
                && !(has_known_time(a)
                    && has_known_time(b)
                    && windows_touch(a, b, config.overlap_tolerance_secs))
        })
    })
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

/// Minimal union-find over activity indices, path-halving on `find`, that
/// also keeps each root's members (what a date-only pairing is checked
/// against).
struct DisjointSets {
    parent: Vec<usize>,
    members: Vec<Vec<usize>>,
}

impl DisjointSets {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
            members: (0..len).map(|index| vec![index]).collect(),
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
            let (root, child) = (root_a.min(root_b), root_a.max(root_b));
            self.parent[child] = root;
            let moved = mem::take(&mut self.members[child]);
            self.members[root].extend(moved);
        }
    }
}
