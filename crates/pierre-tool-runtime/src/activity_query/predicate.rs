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
use pierre_core::models::{
    sport_matches_family, Activity, ActivityTag, SegmentEffort, SportFamily, SportType,
};
use pierre_core::text_fold::fold_for_match;
use pierre_services::workouts::Workout;

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
    pub workout_type: Vec<ActivityTag>,
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

    /// Matches on the activity's merged tag set (provider tags, the tag its
    /// workout code stands for, the commute flag): one vocabulary. A hit is a
    /// match; a miss is only a miss when the activity could have said so for
    /// every tag asked, otherwise the answer is unknown.
    fn workout_type(&self, session: &Activity) -> Verdict {
        if self.workout_type.is_empty() {
            return Verdict::Match;
        }
        let merged = session.merged_tags();
        if self.workout_type.iter().any(|asked| merged.contains(asked)) {
            return Verdict::Match;
        }
        if self
            .workout_type
            .iter()
            .all(|asked| tag_knowable(session, *asked))
        {
            Verdict::Miss
        } else {
            Verdict::Unknown(Unknowable::WorkoutType)
        }
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

/// Whether `session` could have carried `asked` had it been set: the sources
/// that can name a tag differ per tag. A workout code names only race, long
/// run and workout; the commute flag only commute; the rest come from a
/// tag read alone.
fn tag_knowable(session: &Activity, asked: ActivityTag) -> bool {
    if session.tags().is_some() {
        return true;
    }
    match asked {
        ActivityTag::Race | ActivityTag::LongRun | ActivityTag::Workout => {
            session.workout_type().is_some()
        }
        ActivityTag::Commute => session.commute().is_some(),
        _ => false,
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

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_core::models::{ActivityBuilder, TagSource, TaggedBy};

    use super::*;

    fn builder() -> ActivityBuilder {
        ActivityBuilder::new(
            "a1",
            "Run",
            SportType::Run,
            Timestamp::parse_from_rfc3339("2026-11-01T08:00:00Z").unwrap(),
            1800,
            "sciotte",
        )
    }

    fn asking(tag: ActivityTag) -> ActivityPredicate {
        ActivityPredicate {
            workout_type: vec![tag],
            ..ActivityPredicate::default()
        }
    }

    fn provider(tag: ActivityTag) -> TaggedBy {
        TaggedBy::new(tag, TagSource::ProviderTag)
    }

    #[test]
    fn a_workout_code_and_a_strava_tag_of_one_meaning_are_one_race() {
        // Code 1 (race) and tag 1 (race) are one vocabulary: the activity
        // matches once, whichever source said so.
        let both = builder()
            .workout_type(1)
            .tags(vec![provider(ActivityTag::Race)])
            .build();
        assert_eq!(
            asking(ActivityTag::Race).workout_type(&both),
            Verdict::Match,
            "bug: a race by code and by tag did not match a race ask"
        );
        assert_eq!(
            both.merged_tags(),
            vec![ActivityTag::Race],
            "bug: one race counted twice in the merged tags"
        );
    }

    #[test]
    fn tag_twelve_is_recovery_while_code_twelve_is_a_workout() {
        let recovery_tag = builder()
            .tags(vec![provider(ActivityTag::from_strava_tag(12).unwrap())])
            .build();
        assert_eq!(
            asking(ActivityTag::Recovery).workout_type(&recovery_tag),
            Verdict::Match,
            "bug: Strava tag id 12 is not recovery"
        );
        assert_eq!(
            asking(ActivityTag::Workout).workout_type(&recovery_tag),
            Verdict::Miss,
            "bug: Strava tag id 12 read as the workout code 12"
        );
        let workout_code = builder().workout_type(12).build();
        assert_eq!(
            asking(ActivityTag::Workout).workout_type(&workout_code),
            Verdict::Match,
            "bug: workout code 12 is not a workout"
        );
        assert_eq!(
            asking(ActivityTag::Recovery).workout_type(&workout_code),
            Verdict::Unknown(Unknowable::WorkoutType),
            "bug: a workout code cannot rule recovery out, it never names it"
        );
    }

    #[test]
    fn commute_and_recovery_filters_find_provider_tags_and_the_commute_flag() {
        let tagged = builder()
            .tags(vec![
                provider(ActivityTag::Commute),
                provider(ActivityTag::Recovery),
            ])
            .build();
        let flagged = builder().commute(true).build();
        for activity in [&tagged, &flagged] {
            assert_eq!(
                asking(ActivityTag::Commute).workout_type(activity),
                Verdict::Match,
                "bug: a commute by tag or flag did not match"
            );
        }
        assert_eq!(
            asking(ActivityTag::Recovery).workout_type(&tagged),
            Verdict::Match,
            "bug: a recovery provider tag did not match"
        );
        assert_eq!(
            asking(ActivityTag::Recovery).workout_type(&flagged),
            Verdict::Unknown(Unknowable::WorkoutType),
            "bug: a commute flag alone cannot rule recovery out"
        );
    }

    #[test]
    fn an_activity_with_no_tag_information_at_all_is_unknown_for_every_tag() {
        let nothing = builder().build();
        for tag in [
            ActivityTag::Race,
            ActivityTag::Commute,
            ActivityTag::Recovery,
        ] {
            assert_eq!(
                asking(tag).workout_type(&nothing),
                Verdict::Unknown(Unknowable::WorkoutType),
                "bug: {tag:?} judged on an activity that says nothing"
            );
        }
    }

    #[test]
    fn a_source_rules_out_only_the_tags_it_can_name() {
        // OAuth-Strava row: commute flag set, workout code absent, no tags.
        let oauth = builder().commute(false).build();
        assert_eq!(
            asking(ActivityTag::Commute).workout_type(&oauth),
            Verdict::Miss,
            "bug: a read commute flag of false is a commute miss"
        );
        assert_eq!(
            asking(ActivityTag::Race).workout_type(&oauth),
            Verdict::Unknown(Unknowable::WorkoutType),
            "bug: no workout code was read, race is unknowable"
        );
        // Intervals-style row: commute flag, no code.
        let intervals = builder().commute(true).build();
        assert_eq!(
            asking(ActivityTag::Recovery).workout_type(&intervals),
            Verdict::Unknown(Unknowable::WorkoutType),
            "bug: recovery only comes from a tag read"
        );
        // A code read rules out race, not commute.
        let coded = builder().workout_type(0).build();
        assert_eq!(
            asking(ActivityTag::Race).workout_type(&coded),
            Verdict::Miss,
            "bug: a default workout code is a race miss"
        );
        assert_eq!(
            asking(ActivityTag::Commute).workout_type(&coded),
            Verdict::Unknown(Unknowable::WorkoutType),
            "bug: a workout code cannot rule commute out"
        );
        // A tag read that found none rules every tag out.
        let read_none = builder().tags(Vec::new()).build();
        for tag in [
            ActivityTag::Recovery,
            ActivityTag::Race,
            ActivityTag::Commute,
        ] {
            assert_eq!(
                asking(tag).workout_type(&read_none),
                Verdict::Miss,
                "bug: a tag read with none set must miss {tag:?}"
            );
        }
    }

    #[test]
    fn a_miss_needs_every_asked_tag_knowable() {
        let oauth = builder().commute(false).build();
        let both = ActivityPredicate {
            workout_type: vec![ActivityTag::Commute, ActivityTag::Race],
            ..ActivityPredicate::default()
        };
        assert_eq!(
            both.workout_type(&oauth),
            Verdict::Unknown(Unknowable::WorkoutType),
            "bug: race is unknowable, so the pair cannot be a miss"
        );
    }
}
