// ABOUTME: Which dates of an athlete's plan a write changed: weeks compared entry by entry on the training they hold
// ABOUTME: The coach's call and the editor marks are no part of a session, so a date that only gained or lost one is no change

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeSet;

use super::{PlanWeek, PlannedDay};

/// The dates whose entries `after` changes over `before`, in date order.
///
/// Each week of `after` is compared with the week of `before` that has the
/// same `week_start`, or with no entries when `before` holds none: weeks that
/// are not in `after` are not compared. A date counts as changed when its
/// entries differ, compared in order and in every field — a session
/// rewritten, added, taken away, or moved within the date (carnet#1716).
/// What is compared is the session the athlete trains on: the coach's call
/// (carnet#1814) and the coach's editor marks (carnet#1714, carnet#1716) are
/// not part of it, so a session that only gained or lost one of them is no
/// change.
#[must_use]
pub fn changed_dates<'a>(before: &'a [PlanWeek], after: &'a [PlanWeek]) -> BTreeSet<&'a str> {
    let mut changed = BTreeSet::new();
    for week in after {
        let prior = before
            .iter()
            .find(|held| held.week_start == week.week_start)
            .map_or(&[][..], |held| held.days.as_slice());
        let dates: BTreeSet<&str> = prior
            .iter()
            .chain(&week.days)
            .map(|day| day.date.as_str())
            .collect();
        for date in dates {
            let was = prior
                .iter()
                .filter(|day| day.date == date)
                .map(training_content);
            let is = week
                .days
                .iter()
                .filter(|day| day.date == date)
                .map(training_content);
            if !was.eq(is) {
                changed.insert(date);
            }
        }
    }
    changed
}

/// `day` as [`changed_dates`] compares it: without the coach's call and the
/// coach's editor marks. Two entries whose training content is equal are the
/// same session, whoever marked either.
#[must_use]
pub fn training_content(day: &PlannedDay) -> PlannedDay {
    PlannedDay {
        coach_original: None,
        coach_added: false,
        coach_call: None,
        ..day.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{called, session, week_of};
    use super::*;

    #[test]
    fn the_changed_dates_are_those_whose_sessions_differ_and_never_a_mark_alone() {
        let before = vec![
            week_of(
                "2026-11-16",
                vec![
                    session("2026-11-16", "Monday ride"),
                    session("2026-11-17", "Morning ride"),
                    session("2026-11-17", "Evening ride"),
                    session("2026-11-18", "Wednesday ride"),
                    session("2026-11-19", "Thursday ride"),
                    session("2026-11-20", "Friday ride"),
                ],
            ),
            week_of("2026-11-23", vec![session("2026-11-24", "Untouched week")]),
        ];
        let marked = PlannedDay {
            coach_original: Some(Box::new(session("2026-11-19", "Older"))),
            coach_added: true,
            ..called("2026-11-19", "Thursday ride", 7)
        };
        let after = vec![
            week_of(
                "2026-11-16",
                vec![
                    // Monday unchanged; Tuesday's two sessions swap order.
                    session("2026-11-16", "Monday ride"),
                    session("2026-11-17", "Evening ride"),
                    session("2026-11-17", "Morning ride"),
                    // Wednesday rewritten; Thursday the same session with a
                    // call and both editor marks; Friday taken away, and
                    // Saturday added.
                    session("2026-11-18", "Hill repeats"),
                    marked,
                    session("2026-11-21", "Long ride"),
                ],
            ),
            // A week with no counterpart before: every date it holds.
            week_of("2026-11-30", vec![session("2026-12-01", "New week")]),
        ];
        let changed: Vec<&str> = changed_dates(&before, &after).into_iter().collect();
        assert_eq!(
            changed,
            [
                "2026-11-17",
                "2026-11-18",
                "2026-11-20",
                "2026-11-21",
                "2026-12-01"
            ],
            "bug: a date counts when its sessions were rewritten, reordered, taken away or added, \
             never for a call or an editor mark alone, nor in a week the change does not hold"
        );
        assert!(
            changed_dates(&before, &before).is_empty(),
            "bug: weeks written over themselves change no date"
        );
    }
}
