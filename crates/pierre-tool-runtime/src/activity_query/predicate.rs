// ABOUTME: What a count filters on — sports, places, a name, distance, a segment, gear, a workout tag, the trainer flag
// ABOUTME: A filter on a field a workout lacks answers unknown, never no; one sport matcher and one segment matcher serve every reader

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The filters of an [`ActivityQuery`](super::ActivityQuery).
//!
//! Each filter answers a workout with a [`Verdict`]: it matches, it does
//! not, or the workout lacks the field the filter needs. A workout whose
//! segments were never read is not a workout without the segment; one with
//! no trainer flag is not an outdoor one. Those are unknown, counted apart,
//! and never in the negative bucket.

use std::iter;

use pierre_core::constants::oauth::providers::STRAVA_ARCHIVE;
use pierre_core::models::{sport_matches_family, Activity, SegmentEffort, SportFamily, SportType};
use pierre_core::text_fold::fold_for_match;
use pierre_services::workouts::Workout;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::segment_keys;

/// Which segment a call asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentMatch {
    /// Its id, exactly.
    Id(String),
    /// Every segment whose folded name holds this folded text
    /// ([`fold_for_match`]): accents, case and spacing ignored.
    Name(String),
}

impl SegmentMatch {
    /// Whether `effort` is on the segment asked.
    #[must_use]
    pub fn matches(&self, effort: &SegmentEffort) -> bool {
        match self {
            Self::Id(id) => effort.segment_id.as_deref() == Some(id.as_str()),
            Self::Name(folded) => fold_for_match(&effort.name).contains(folded.as_str()),
        }
    }
}

/// A workout's kind, from the provider's workout code. Strava's codes: 1
/// and 11 a race, 2 a long run, 3 and 12 a workout; 0 and 10 none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkoutTag {
    /// A race.
    Race,
    /// A long run.
    LongRun,
    /// A structured workout.
    Workout,
}

impl WorkoutTag {
    /// The tag a provider's workout code names, `None` for a code naming
    /// none of them (a default run or ride).
    #[must_use]
    pub const fn of(code: u32) -> Option<Self> {
        match code {
            1 | 11 => Some(Self::Race),
            2 => Some(Self::LongRun),
            3 | 12 => Some(Self::Workout),
            _ => None,
        }
    }
}

/// Whether `activity` is one of `types` or of `families`; with neither
/// named, any activity is.
///
/// The one alias-aware sport matcher: a type matches the disciplines it
/// heads ([`sport_matches_family`]: `Run` matches a trail run, `Ride` a
/// virtual ride), and a family matches every type of it ([`SportFamily::of`]).
/// A type and a family both named widen each other: an activity of either
/// matches.
#[must_use]
pub fn sport_matches(activity: &Activity, types: &[SportType], families: &[SportFamily]) -> bool {
    if types.is_empty() && families.is_empty() {
        return true;
    }
    let sport = activity.sport_type();
    types.iter().any(|asked| sport_matches_family(sport, asked))
        || families.contains(&SportFamily::of(sport))
}

/// The fields a filter can find a workout lacks, each counted apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unknowable {
    /// Its segments were never read, or no copy a detail walk reads exists.
    HasSegment,
    /// It belongs to no place.
    Place,
    /// It names no gear.
    Gear,
    /// It carries no trainer flag.
    Trainer,
    /// It carries no workout code.
    WorkoutType,
}

/// What one filter, or all of them, answers a workout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// It matches.
    Match,
    /// It does not.
    Miss,
    /// It lacks the field the filter needs.
    Unknown(Unknowable),
}

/// Every filter a count runs. Empty lists and `None` filter nothing.
#[derive(Debug, Clone, Default)]
pub struct ActivityPredicate {
    /// Sport types, resolved; empty = any.
    pub sport_types: Vec<SportType>,
    /// Sport families; empty = any.
    pub sport_families: Vec<SportFamily>,
    /// Place ids, a row's `place.id`; empty = any.
    pub place_ids: Vec<String>,
    /// Text the activity name holds, both folded ([`fold_for_match`]).
    pub name_contains: Option<String>,
    /// Least distance, metres, included.
    pub min_distance_m: Option<f64>,
    /// Most distance, metres, included.
    pub max_distance_m: Option<f64>,
    /// A segment a workout has an effort on.
    pub has_segment: Option<SegmentMatch>,
    /// A gear id, equal, or folded text the folded gear name holds.
    pub gear: Option<String>,
    /// Workout tags; empty = any.
    pub workout_type: Vec<WorkoutTag>,
    /// Whether the workout was on a trainer.
    pub trainer: Option<bool>,
}

impl ActivityPredicate {
    /// What every filter but `has_segment` answers `workout`: the first
    /// miss, else the first field it lacks, else a match. Only the merged
    /// session is read, but for its place, which is its rows'.
    pub(crate) fn test_without_segment(&self, workout: &Workout<'_>) -> Verdict {
        let session = &workout.session;
        let known = [
            self.sport(session),
            self.place(workout),
            self.name(session),
            self.distance(session),
            self.gear(session),
            self.workout_type(session),
            self.trainer(session),
        ];
        known
            .iter()
            .copied()
            .find(|verdict| *verdict == Verdict::Miss)
            .or_else(|| {
                known
                    .iter()
                    .copied()
                    .find(|verdict| matches!(verdict, Verdict::Unknown(_)))
            })
            .unwrap_or(Verdict::Match)
    }

    /// What `has_segment` answers `workout`: unknown when no copy a detail
    /// walk reads exists, or its segments were never read; else whether any
    /// recording holds an effort on the segment.
    pub(crate) fn segment(&self, workout: &Workout<'_>) -> Verdict {
        let Some(segment) = &self.has_segment else {
            return Verdict::Match;
        };
        if !walkable_segments(workout) || !segments_read(workout) {
            return Verdict::Unknown(Unknowable::HasSegment);
        }
        let on_segment = iter::once(&workout.session)
            .chain(workout.rows().map(|row| &row.activity))
            .filter_map(Activity::segment_efforts)
            .flatten()
            .any(|effort| segment.matches(effort));
        if on_segment {
            Verdict::Match
        } else {
            Verdict::Miss
        }
    }

    fn sport(&self, session: &Activity) -> Verdict {
        known(sport_matches(
            session,
            &self.sport_types,
            &self.sport_families,
        ))
    }

    fn place(&self, workout: &Workout<'_>) -> Verdict {
        if self.place_ids.is_empty() {
            return Verdict::Match;
        }
        workout
            .place()
            .map_or(Verdict::Unknown(Unknowable::Place), |place| {
                known(self.place_ids.contains(&place.id))
            })
    }

    fn name(&self, session: &Activity) -> Verdict {
        self.name_contains
            .as_deref()
            .map_or(Verdict::Match, |asked| {
                known(fold_for_match(session.name()).contains(&fold_for_match(asked)))
            })
    }

    fn distance(&self, session: &Activity) -> Verdict {
        if self.min_distance_m.is_none() && self.max_distance_m.is_none() {
            return Verdict::Match;
        }
        // A workout that recorded no distance is under no bound: it is left
        // out, as a distance filter says nothing of it.
        session.distance_meters().map_or(Verdict::Miss, |metres| {
            known(
                self.min_distance_m.is_none_or(|least| metres >= least)
                    && self.max_distance_m.is_none_or(|most| metres <= most),
            )
        })
    }

    fn gear(&self, session: &Activity) -> Verdict {
        let Some(asked) = self.gear.as_deref() else {
            return Verdict::Match;
        };
        session
            .gear()
            .map_or(Verdict::Unknown(Unknowable::Gear), |gear| {
                known(
                    gear.id.as_deref() == Some(asked)
                        || fold_for_match(&gear.name).contains(&fold_for_match(asked)),
                )
            })
    }

    fn workout_type(&self, session: &Activity) -> Verdict {
        if self.workout_type.is_empty() {
            return Verdict::Match;
        }
        session
            .workout_type()
            .map_or(Verdict::Unknown(Unknowable::WorkoutType), |code| {
                known(WorkoutTag::of(code).is_some_and(|tag| self.workout_type.contains(&tag)))
            })
    }

    fn trainer(&self, session: &Activity) -> Verdict {
        self.trainer.map_or(Verdict::Match, |asked| {
            session
                .trainer()
                .map_or(Verdict::Unknown(Unknowable::Trainer), |on| {
                    known(on == asked)
                })
        })
    }
}

const fn known(matched: bool) -> Verdict {
    if matched {
        Verdict::Match
    } else {
        Verdict::Miss
    }
}

/// Whether a copy of `workout` sits under a key a detail walk reads
/// segments from ([`segment_keys`]): only those can ever be read.
pub(crate) fn walkable_segments(workout: &Workout<'_>) -> bool {
    let keys = segment_keys();
    workout
        .rows()
        .any(|row| keys.contains(&row.provider.as_str()))
}

/// Whether `workout`'s segment efforts were read: a detail read of a copy
/// read them, or a recording carries them.
pub(crate) fn segments_read(workout: &Workout<'_>) -> bool {
    workout.segments_read() || workout.session.segment_efforts().is_some()
}

/// Whether `workout`'s segments are unread and only the athlete's own
/// export holds it among the copies a walk could read, which no walk reads.
pub(crate) fn export_only(workout: &Workout<'_>) -> bool {
    let keys = segment_keys();
    !segments_read(workout)
        && workout
            .rows()
            .filter(|row| keys.contains(&row.provider.as_str()))
            .all(|row| row.activity.provider() == STRAVA_ARCHIVE)
}
