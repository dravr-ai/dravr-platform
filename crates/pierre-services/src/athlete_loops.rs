// ABOUTME: The athlete's own loops near a point: past workouts of one sport that start and end at a place they train from
// ABOUTME: Ranked for an asked effort against their own workouts at that place, read only from stored rows and tracks

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete loops (carnet#1665).
//!
//! When the athlete asks for an easy loop near where they ride, the best
//! answer is often one they already rode. [`own_loops`] finds them in what
//! the store keeps, with no provider asked and no external call:
//!
//! - **Where.** Only workouts whose start belongs to one of the athlete's
//!   places (slice 6, [`crate::athlete_places`]), and only places near the
//!   asked point. A start inside the home exclusion carries no place, so a
//!   loop from home is never offered.
//! - **What counts as a loop.** A workout of the asked sport with a stored
//!   track ([`local_track`]) whose two trimmed ends lie within
//!   [`ONE_SPOT_REACH_METERS`]: it started and ended at one spot. The track
//!   is the privacy-trimmed one, so the line handed back never shows the
//!   spot itself.
//! - **Effort.** A workout's effort is where its distance and climb fall
//!   among the athlete's own workouts of that sport at that place: the
//!   shortest, flattest ones there are their easy ones ([`Effort`]).
//!
//! Every copy of a workout is read through the provider-terms filter and
//! collapsed by the workout merge the places pass uses, so one ride stored
//! under two providers is offered once.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use dravr_tronc::dates::Timestamp;
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::AppResult;
use pierre_core::models::{PlaceRef, SportType, TenantId};
use pierre_core::untrusted::display_line;
use pierre_database::repositories::CachedActivityRow;
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::places::ONE_SPOT_REACH_METERS;
use pierre_fitness_compute::route_track::{RouteTrack, MAP_MAX_POINTS};
use pierre_fitness_compute::routes::haversine_meters_between;
use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

use crate::activity_track::local_track;
use crate::athlete_places::{all_rows, workout_rows, Workout};
use crate::workouts::filter_activity_rows;

/// The most loops one search answers.
const MAX_OWN_LOOPS: usize = 5;

/// The most workouts whose track one search reads. A track may be a stored
/// map or stored samples, one database read each, so a search over a
/// history of hundreds of rides at one place reads the best-ranked few.
const MAX_TRACK_READS: usize = 25;

/// The fewest workouts of the sport at a place for an effort there to mean
/// anything: below it, no workout is called easy or hard.
const MIN_WORKOUTS_FOR_EFFORT: usize = 3;

/// Two loops at one place whose distances differ by no more than this share
/// of the longer are one loop ridden again, unless their climbs differ.
const SAME_LOOP_DISTANCE_SHARE: f64 = 0.05;

/// Two loops whose climbs differ by no more than this, in metres, climb
/// alike.
const SAME_LOOP_GAIN_METERS: f64 = 20.0;

/// The longest loop name, in characters, after defanging.
const MAX_LOOP_NAME_CHARS: usize = 80;

/// The share of the effort range each band covers: the lowest third of the
/// athlete's workouts at a place are their easy ones.
const BAND_WIDTH: f64 = 1.0 / 3.0;

/// The middle of the effort range, where a moderate session sits.
const MIDDLE: f64 = 0.5;

/// How hard a workout is, relative to the athlete's own workouts of that
/// sport at that place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    /// Among the shortest and flattest third there.
    Easy,
    /// In the middle third.
    Moderate,
    /// Among the longest and hilliest third.
    Hard,
}

impl Effort {
    /// The effort a caller names, or `None` for a word that is none.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "easy" => Some(Self::Easy),
            "moderate" => Some(Self::Moderate),
            "hard" => Some(Self::Hard),
            _ => None,
        }
    }

    /// The band a score in `0.0..=1.0` falls in.
    fn of_score(score: f64) -> Self {
        if score < BAND_WIDTH {
            Self::Easy
        } else if score < 2.0 * BAND_WIDTH {
            Self::Moderate
        } else {
            Self::Hard
        }
    }
}

/// What a search for the athlete's own loops asks.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopSearch {
    /// The sport the loop is for; only workouts of exactly this sport count.
    pub sport: SportType,
    /// Latitude of the point searched around, in degrees.
    pub latitude: f64,
    /// Longitude of the point searched around, in degrees.
    pub longitude: f64,
    /// How far from the point a place may lie, in metres.
    pub radius_meters: f64,
    /// The effort asked for, which orders the answer.
    pub effort: Option<Effort>,
    /// The session distance asked for, in metres, which orders the answer
    /// when no effort is asked.
    pub target_distance_meters: Option<f64>,
}

/// One of the athlete's own loops.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnLoop {
    /// The workout's activity id, its canonical copy's.
    pub activity_id: String,
    /// The workout's name, defanged; `None` when it has none.
    pub name: Option<String>,
    /// The place it started at, as a row carries it.
    pub place: PlaceRef,
    /// When it started.
    pub started_at: Timestamp,
    /// Its recorded distance, in metres.
    pub distance_meters: f64,
    /// Its recorded climb, in metres; `None` when none was recorded.
    pub elevation_gain_meters: Option<f64>,
    /// How hard it is among the athlete's workouts of the sport there;
    /// `None` when they have too few there to say.
    pub relative_effort: Option<Effort>,
    /// The privacy-trimmed track, simplified to a map's points.
    pub line: Vec<(f64, f64)>,
    /// How far the trimmed track starts from the searched point, in metres.
    pub distance_from_center_meters: f64,
}

/// A workout that may be a loop, before its track is read.
struct Candidate<'rows> {
    workout: &'rows Workout,
    canonical: &'rows CachedActivityRow,
    place: &'rows PlaceRef,
    distance_meters: f64,
    elevation_gain_meters: Option<f64>,
    score: Option<f64>,
}

/// The athlete's own loops near the searched point, best first, at most
/// [`MAX_OWN_LOOPS`].
///
/// Reads only the tenant's and athlete's own rows, places and stored tracks.
/// An athlete with no place near the point, no workout of the sport there,
/// or no stored track that closes on itself has no loop: the answer is then
/// empty, which is what the store holds.
///
/// # Errors
///
/// Returns the repository error when a read fails.
pub async fn own_loops(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    tenant_id: &TenantId,
    user_id: Uuid,
    search: &LoopSearch,
    now: Timestamp,
) -> AppResult<Vec<OwnLoop>> {
    let near: HashSet<String> = repos
        .athlete_places
        .list(tenant_id, user_id)
        .await?
        .into_iter()
        .filter(|place| {
            haversine_meters_between(
                search.latitude,
                search.longitude,
                place.centroid_lat,
                place.centroid_lon,
            ) <= search.radius_meters + place.radius_m
        })
        .map(|place| place.id)
        .collect();
    if near.is_empty() {
        return Ok(Vec::new());
    }
    let rows = filter_activity_rows(terms, all_rows(repos, tenant_id, user_id, now).await?);
    let workouts = workout_rows(&rows);
    let mut candidates = candidates(&rows, &workouts, &near, &search.sport);
    score(&mut candidates);
    rank(&mut candidates, search);

    let mut loops: Vec<OwnLoop> = Vec::new();
    let mut reads = 0;
    for candidate in &candidates {
        if loops.len() >= MAX_OWN_LOOPS || reads >= MAX_TRACK_READS {
            break;
        }
        // A ride of a loop already found reads nothing, so it spends none of
        // the reads: one loop ridden often never hides the next.
        if loops.iter().any(|known| same_loop(known, candidate)) {
            continue;
        }
        reads += 1;
        let Some(track) = workout_track(repos, tenant_id, user_id, &rows, candidate).await? else {
            continue;
        };
        if let Some(found) = as_loop(candidate, track, search) {
            loops.push(found);
        }
    }
    Ok(loops)
}

/// Every workout of `sport` whose start belongs to a place in `near` and
/// whose distance is recorded.
fn candidates<'rows>(
    rows: &'rows [CachedActivityRow],
    workouts: &'rows [Workout],
    near: &HashSet<String>,
    sport: &SportType,
) -> Vec<Candidate<'rows>> {
    workouts
        .iter()
        .filter_map(|workout| {
            let canonical = &rows[*workout.rows.first()?];
            if canonical.activity.sport_type() != sport {
                return None;
            }
            let place = workout
                .rows
                .iter()
                .find_map(|&index| rows[index].place.as_ref())
                .filter(|place| near.contains(&place.id))?;
            let distance_meters = canonical
                .activity
                .distance_meters()
                .filter(|meters| meters.is_finite() && *meters > 0.0)?;
            Some(Candidate {
                workout,
                canonical,
                place,
                distance_meters,
                elevation_gain_meters: canonical
                    .activity
                    .elevation_gain()
                    .filter(|meters| meters.is_finite() && *meters >= 0.0),
                score: None,
            })
        })
        .collect()
}

/// Give each candidate its effort score: the mean of where its distance and
/// its climb fall among the candidates at its place, from 0 (shortest,
/// flattest) to 1. A place with fewer than [`MIN_WORKOUTS_FOR_EFFORT`]
/// workouts scores none; a climb is scored only where enough workouts there
/// recorded one.
fn score(candidates: &mut [Candidate<'_>]) {
    let mut distances: HashMap<&str, Vec<f64>> = HashMap::new();
    let mut gains: HashMap<&str, Vec<f64>> = HashMap::new();
    for candidate in candidates.iter() {
        let place = candidate.place.id.as_str();
        distances
            .entry(place)
            .or_default()
            .push(candidate.distance_meters);
        if let Some(gain) = candidate.elevation_gain_meters {
            gains.entry(place).or_default().push(gain);
        }
    }
    let scores: Vec<Option<f64>> = candidates
        .iter()
        .map(|candidate| {
            let place = candidate.place.id.as_str();
            let by_distance = distances
                .get(place)
                .and_then(|all| rank_share(candidate.distance_meters, all))?;
            let by_gain = candidate
                .elevation_gain_meters
                .zip(gains.get(place))
                .and_then(|(gain, all)| rank_share(gain, all));
            Some(by_gain.map_or(by_distance, |by_gain| f64::midpoint(by_distance, by_gain)))
        })
        .collect();
    for (candidate, score) in candidates.iter_mut().zip(scores) {
        candidate.score = score;
    }
}

/// Where `value` falls among `all` (which holds it), from 0 (the lowest) to
/// 1 (the highest), ties at their middle; `None` when `all` holds fewer than
/// [`MIN_WORKOUTS_FOR_EFFORT`] values.
fn rank_share(value: f64, all: &[f64]) -> Option<f64> {
    if all.len() < MIN_WORKOUTS_FOR_EFFORT {
        return None;
    }
    let below = all.iter().filter(|other| **other < value).count();
    let above = all.iter().filter(|other| **other > value).count();
    // `all` holds `value`, so `equal` is at least 1 and the others number
    // `len - 1`, at least two here.
    let equal = all.len() - below - above;
    let others = count(all.len() - 1);
    let position = count(below) + count(equal.saturating_sub(1)) / 2.0;
    Some(position / others)
}

/// A count of workouts as a float; a history holds far fewer than
/// `u32::MAX` of them.
fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Order the candidates best first for `search`: by effort when one is
/// asked (an unscored workout sits in the middle), else by closeness to the
/// target distance, else newest first; ties newest first.
fn rank(candidates: &mut [Candidate<'_>], search: &LoopSearch) {
    let key = |candidate: &Candidate<'_>| -> f64 {
        let score = candidate.score.unwrap_or(MIDDLE);
        match (search.effort, search.target_distance_meters) {
            (Some(Effort::Easy), _) => score,
            (Some(Effort::Hard), _) => -score,
            (Some(Effort::Moderate), _) => (score - MIDDLE).abs(),
            (None, Some(target)) => (candidate.distance_meters - target).abs(),
            (None, None) => 0.0,
        }
    };
    candidates.sort_by(|a, b| {
        key(a)
            .partial_cmp(&key(b))
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                b.canonical
                    .activity
                    .start_date()
                    .cmp(&a.canonical.activity.start_date())
            })
            .then_with(|| a.canonical.activity.id().cmp(b.canonical.activity.id()))
    });
}

/// Whether `candidate` is a ride of a loop already found: same place, about
/// the same distance and the same climb.
fn same_loop(known: &OwnLoop, candidate: &Candidate<'_>) -> bool {
    let longer = known.distance_meters.max(candidate.distance_meters);
    let distance_alike = (known.distance_meters - candidate.distance_meters).abs()
        <= longer * SAME_LOOP_DISTANCE_SHARE;
    let gain_alike = match (known.elevation_gain_meters, candidate.elevation_gain_meters) {
        (Some(a), Some(b)) => (a - b).abs() <= SAME_LOOP_GAIN_METERS,
        (None, None) => true,
        _ => false,
    };
    known.place.id == candidate.place.id && distance_alike && gain_alike
}

/// The workout's stored track: the first of its copies with one, read
/// without asking any provider.
async fn workout_track(
    repos: &RepositoryRegistry,
    tenant_id: &TenantId,
    user_id: Uuid,
    rows: &[CachedActivityRow],
    candidate: &Candidate<'_>,
) -> AppResult<Option<RouteTrack>> {
    for &index in &candidate.workout.rows {
        if let Some(track) = local_track(repos, tenant_id, user_id, &rows[index]).await? {
            return Ok(Some(track));
        }
    }
    Ok(None)
}

/// The candidate as a loop, when its trimmed track ends where it started.
fn as_loop(candidate: &Candidate<'_>, track: RouteTrack, search: &LoopSearch) -> Option<OwnLoop> {
    let (first, last) = (track.first_point()?, track.last_point()?);
    if haversine_meters_between(first.0, first.1, last.0, last.1) > ONE_SPOT_REACH_METERS {
        return None;
    }
    let line = track.simplified(MAP_MAX_POINTS).coordinates;
    let activity = &candidate.canonical.activity;
    let name = display_line(activity.name(), MAX_LOOP_NAME_CHARS);
    let name = name.trim();
    Some(OwnLoop {
        activity_id: activity.id().to_owned(),
        name: (!name.is_empty()).then(|| name.to_owned()),
        place: candidate.place.clone(),
        started_at: activity.start_date(),
        distance_meters: candidate.distance_meters,
        elevation_gain_meters: candidate.elevation_gain_meters,
        relative_effort: candidate.score.map(Effort::of_score),
        line,
        distance_from_center_meters: haversine_meters_between(
            search.latitude,
            search.longitude,
            first.0,
            first.1,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_share_runs_from_the_lowest_to_the_highest_with_ties_at_their_middle() {
        let all = [10.0, 20.0, 20.0, 40.0];
        assert_eq!(
            rank_share(10.0, &all),
            Some(0.0),
            "bug: the lowest value is at 0"
        );
        assert_eq!(
            rank_share(40.0, &all),
            Some(1.0),
            "bug: the highest value is at 1"
        );
        assert_eq!(
            rank_share(20.0, &all),
            Some(0.5),
            "bug: two equal values share the middle of their two positions"
        );
    }

    #[test]
    fn too_few_workouts_at_a_place_give_no_effort() {
        assert_eq!(
            rank_share(10.0, &[10.0, 20.0]),
            None,
            "bug: two workouts are too few to call one easy"
        );
        assert_eq!(
            rank_share(10.0, &[10.0, 20.0, 30.0]),
            Some(0.0),
            "bug: three workouts are enough"
        );
    }

    #[test]
    fn a_score_falls_in_the_third_of_the_range_it_names() {
        assert_eq!(Effort::of_score(0.0), Effort::Easy, "bug: 0 is easy");
        assert_eq!(
            Effort::of_score(0.33),
            Effort::Easy,
            "bug: the lowest third is easy"
        );
        assert_eq!(
            Effort::of_score(0.34),
            Effort::Moderate,
            "bug: past the first third is moderate"
        );
        assert_eq!(
            Effort::of_score(0.67),
            Effort::Hard,
            "bug: past the second third is hard"
        );
        assert_eq!(Effort::of_score(1.0), Effort::Hard, "bug: 1 is hard");
    }

    #[test]
    fn an_effort_is_named_by_its_word_alone() {
        assert_eq!(
            Effort::parse(" Easy "),
            Some(Effort::Easy),
            "bug: case and spaces do not change the word"
        );
        assert_eq!(
            Effort::parse("hard"),
            Some(Effort::Hard),
            "bug: hard is an effort"
        );
        assert_eq!(
            Effort::parse("facile"),
            None,
            "bug: a word that is no effort is refused, not guessed"
        );
    }
}
