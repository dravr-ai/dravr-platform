// ABOUTME: How the coach's editor addresses one session of a date — its position and the session the client last saw — and splices it
// ABOUTME: A write whose session moved under it writes nothing; every write keeps a date to its entry bounds and stamps the coach's marks

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # One session of a date (carnet#1716)
//!
//! A date may hold several entries: the sessions in the order they are done,
//! and at most one rest entry beside them. A session is known by its date and
//! its 0-based position among the date's entries; it has no stable id. So
//! every write names the session as the client last saw it
//! ([`SessionExpect`]), and a write whose position no longer holds that
//! session writes nothing ([`DraftDayOutcome::SessionMoved`]): an agent or a
//! second sheet may have moved it.

use pierre_core::errors::AppResult;
use pierre_memory::training_plans::PlannedDay;
use serde::Deserialize;

use super::{refused, DraftDayOutcome};
use crate::implementations::training_plans::validate_date_entries;

/// The `details.reason` of a day past the bounds a plan day keeps.
pub(super) const INVALID_SESSION: &str = "invalid_session";

/// The session as the client last saw it: the card's exact `sport` and
/// `workout` strings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionExpect {
    /// The session's sport, as the card wrote it.
    pub sport: String,
    /// The session's workout, as the card wrote it.
    pub workout: String,
}

impl SessionExpect {
    /// Whether `day` is still the session the client saw.
    fn matches(&self, day: &PlannedDay) -> bool {
        day.sport == self.sport && day.workout == self.workout
    }
}

/// One session of a date as a write names it: its position among the date's
/// entries, and the session the client saw there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAt {
    /// The 0-based position among the date's entries, a rest entry included.
    pub position: u32,
    /// The session the client last saw at that position.
    pub expect: SessionExpect,
}

/// How many entries `days` holds on `date`.
pub(super) fn entries_on(days: &[PlannedDay], date: &str) -> usize {
    days.iter().filter(|day| day.date == date).count()
}

/// The index in `days` of entry `position` of `date`, when the date holds
/// that many.
fn entry_index(days: &[PlannedDay], date: &str, position: u32) -> Option<usize> {
    let position = usize::try_from(position).ok()?;
    days.iter()
        .enumerate()
        .filter(|(_, day)| day.date == date)
        .nth(position)
        .map(|(index, _)| index)
}

/// The index in `days` of the session `at` names on `date`, or
/// `SessionMoved` when that position is gone or holds another session.
pub(super) fn expected_index(
    days: &[PlannedDay],
    date: &str,
    at: &SessionAt,
) -> Result<usize, DraftDayOutcome> {
    entry_index(days, date, at.position)
        .filter(|index| days.get(*index).is_some_and(|day| at.expect.matches(day)))
        .ok_or(DraftDayOutcome::SessionMoved)
}

/// The snapshot a coach's day save stamps on the day it writes: the one the
/// replaced day already carries, so a second edit keeps what the first one
/// replaced; else the replaced day itself. `None` where no day stood on the
/// date. A snapshot never carries one of its own.
pub(super) fn first_edit_snapshot(replaced: Option<&PlannedDay>) -> Option<Box<PlannedDay>> {
    let replaced = replaced?;
    Some(replaced.coach_original.clone().unwrap_or_else(|| {
        Box::new(PlannedDay {
            coach_original: None,
            ..replaced.clone()
        })
    }))
}

/// Stamp the coach's marks on `day`, written over `replaced`: a session the
/// coach added stays an added session, with no snapshot, so restoring it
/// still removes it; any other edit keeps the day as it stood before the
/// coach's first edit ([`first_edit_snapshot`]).
pub(super) fn stamp_edit(day: &mut PlannedDay, replaced: Option<&PlannedDay>) {
    if replaced.is_some_and(|held| held.coach_added) {
        day.coach_added = true;
        day.coach_original = None;
    } else {
        day.coach_added = false;
        day.coach_original = first_edit_snapshot(replaced);
    }
}

/// `days` with `day` in place of the entry at `index`, or, with no index,
/// after the last entry on its date — in date order where the date holds
/// none — its date held to the entries one date may hold.
///
/// # Errors
///
/// Invalid input with `details.reason` `invalid_session` when the date would
/// hold more entries than a date holds, or a second rest entry.
pub(super) fn placed(
    days: &[PlannedDay],
    index: Option<usize>,
    day: PlannedDay,
) -> AppResult<Vec<PlannedDay>> {
    let mut days = days.to_vec();
    let date = day.date.clone();
    // A replaced day keeps its place; a new one goes after its date's
    // entries, else where its date falls in the week.
    let replaced = index.filter(|&index| index < days.len());
    let at = replaced.unwrap_or_else(|| {
        days.iter()
            .rposition(|held| held.date == day.date)
            .map_or_else(
                || days.partition_point(|held| held.date < day.date),
                |last| last + 1,
            )
    });
    if replaced.is_some() {
        days.remove(at);
    }
    days.insert(at, day);
    // Only the date written is held to the bounds: a week stored before
    // them may hold another date this edit leaves as it stands.
    let on_date: Vec<PlannedDay> = days
        .iter()
        .filter(|held| held.date == date)
        .cloned()
        .collect();
    validate_date_entries(&on_date).map_err(|error| refused(INVALID_SESSION, error.message))?;
    Ok(days)
}

/// `days` without the entry at `index`.
pub(super) fn without(days: &[PlannedDay], index: usize) -> Vec<PlannedDay> {
    let mut days = days.to_vec();
    if index < days.len() {
        days.remove(index);
    }
    days
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DATE: &str = "2026-11-18";

    fn day(date: &str, sport: &str, workout: &str) -> PlannedDay {
        serde_json::from_value(json!({ "date": date, "sport": sport, "workout": workout })).unwrap()
    }

    fn at(position: u32, sport: &str, workout: &str) -> SessionAt {
        SessionAt {
            position,
            expect: SessionExpect {
                sport: sport.to_owned(),
                workout: workout.to_owned(),
            },
        }
    }

    fn week() -> Vec<PlannedDay> {
        vec![
            day("2026-11-17", "run", "Easy"),
            day(DATE, "ride", "Endurance"),
            day(DATE, "strength", "Core"),
            day("2026-11-19", "run", "Tempo"),
        ]
    }

    fn workouts(days: &[PlannedDay]) -> Vec<&str> {
        days.iter().map(|day| day.workout.as_str()).collect()
    }

    #[test]
    fn a_session_is_found_by_its_position_while_it_holds_the_session_the_client_saw() {
        let days = week();
        assert_eq!(
            expected_index(&days, DATE, &at(1, "strength", "Core")),
            Ok(2),
            "bug: the second entry on the date is found by its position"
        );
        for (stale, why) in [
            (at(1, "strength", "Core 30"), "another workout"),
            (at(1, "ride", "Core"), "another sport"),
            (
                at(2, "strength", "Core"),
                "a position the date does not hold",
            ),
            (at(0, "strength", "Core"), "another session's position"),
        ] {
            assert_eq!(
                expected_index(&days, DATE, &stale),
                Err(DraftDayOutcome::SessionMoved),
                "bug: {why} is a session that moved"
            );
        }
        assert_eq!(
            expected_index(&days, "2026-11-20", &at(0, "ride", "Endurance")),
            Err(DraftDayOutcome::SessionMoved),
            "bug: a date holding no entry holds no session to write over"
        );
    }

    #[test]
    fn an_added_session_goes_after_its_dates_entries_and_a_date_keeps_its_bounds() {
        let added = placed(&week(), None, day(DATE, "swim", "Drills")).unwrap();
        assert_eq!(
            workouts(&added),
            ["Easy", "Endurance", "Core", "Drills", "Tempo"],
            "bug: an added session is the date's last entry"
        );
        let first = placed(&week(), None, day("2026-11-16", "run", "Shakeout")).unwrap();
        assert_eq!(
            workouts(&first)[0],
            "Shakeout",
            "bug: a session on an empty date joins the week in date order"
        );
        let replaced = placed(&week(), Some(2), day(DATE, "yoga", "Stretch")).unwrap();
        assert_eq!(
            workouts(&replaced),
            ["Easy", "Endurance", "Stretch", "Tempo"],
            "bug: a save replaces only the entry at its position"
        );

        let full: Vec<PlannedDay> = (0..5).map(|n| day(DATE, "ride", &n.to_string())).collect();
        let refused = placed(&full, None, day(DATE, "run", "sixth")).unwrap_err();
        assert_eq!(
            refused
                .details
                .as_ref()
                .map(|details| details["reason"].clone()),
            Some(json!(INVALID_SESSION)),
            "bug: a sixth entry is refused as an invalid session"
        );
        let rested = placed(&week(), Some(1), day(DATE, "rest", "Rest")).unwrap();
        assert!(
            placed(&rested, None, day(DATE, "rest", "Rest")).is_err(),
            "bug: a second rest entry on a date is refused"
        );
        assert_eq!(
            workouts(&without(&week(), 2)),
            ["Easy", "Endurance", "Tempo"],
            "bug: removing takes only the entry at its position"
        );
    }

    #[test]
    fn an_edit_snapshots_the_day_and_an_added_session_stays_added() {
        let original = day(DATE, "ride", "Endurance");
        let mut edited = day(DATE, "ride", "Tempo");
        stamp_edit(&mut edited, Some(&original));
        assert_eq!(
            edited.coach_original.as_deref(),
            Some(&original),
            "bug: an edit keeps the day it replaced"
        );
        assert!(!edited.coach_added, "bug: an edit is not an added session");

        let added = PlannedDay {
            coach_added: true,
            ..day(DATE, "strength", "Core")
        };
        let mut over_added = day(DATE, "strength", "Core 30");
        stamp_edit(&mut over_added, Some(&added));
        assert!(
            over_added.coach_added && over_added.coach_original.is_none(),
            "bug: an edit of an added session keeps it added, with no snapshot: {over_added:?}"
        );
    }
}
