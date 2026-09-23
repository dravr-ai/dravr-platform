// ABOUTME: When a group's weekly digest is due — Monday 08:00 in the group's own zone, caught up that week
// ABOUTME: Pure functions over an instant and a zone, so every boundary of the slot is testable without a clock
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The weekly digest's slot.
//!
//! The send time is a fact about the group's calendar, not about when a
//! worker last ticked:
//!
//! - It lands at [`SLOT_START_HOUR`]:00 on Monday, in the group's own zone.
//! - A Monday on which no instance ran (the service scales to zero) is caught
//!   up by the first tick later that same ISO week whose local hour falls in
//!   the daytime window `[SLOT_START_HOUR, SLOT_END_HOUR)`.
//! - A week is owed only when its Monday slot came after the group existed:
//!   a group bound on a Thursday gets its first digest the next Monday, not a
//!   "weekly" recap within minutes of its first message.
//! - It never goes out outside that window, and at most once per group per
//!   ISO week — the second half is the delivery ledger's job, keyed on the
//!   [`week_key`] this module computes.
//!
//! The zone is the owner's, else the human coach's, else the one most members
//! share ([`group_zone`]). With none on file the slot is read on the UTC clock
//! and opens at [`NO_ZONE_SLOT_START_HOUR_UTC`] instead — the no-zone rule.

use chrono::{DateTime, Datelike, Days, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use pierre_core::civil_time::{clock_date, parse_zone};

/// Local hour the digest lands at on Monday, and the first hour of the day
/// any catch-up may use.
pub const SLOT_START_HOUR: u32 = 8;

/// First local hour the digest may no longer go out: nothing is posted in the
/// evening or at night.
pub const SLOT_END_HOUR: u32 = 20;

/// The no-zone rule: the UTC hour the slot opens at when no zone is known.
///
/// When nobody in the group has a usable zone on file, the slot is read on
/// the UTC clock and opens at 12:00 UTC — 08:00 in Montreal, 14:00 in Paris —
/// so a group on either side of the Atlantic is never sent its digest in the
/// night. It still closes at [`SLOT_END_HOUR`] UTC.
pub const NO_ZONE_SLOT_START_HOUR_UTC: u32 = 12;

/// The ISO week whose digest is due at `now`, if any.
///
/// `None` when `now` is outside the daytime window in `zone`, or when the
/// week's Monday slot came before `owed_since` (the group did not exist yet).
///
/// Every daytime tick of a week answers that week, which is what makes the
/// Monday slot catch up on a later day: before 08:00 on Monday is outside the
/// window, so the first tick to answer a week is at or after its Monday slot.
/// A `None` zone applies the no-zone rule ([`NO_ZONE_SLOT_START_HOUR_UTC`]).
#[must_use]
pub fn due_week(now: DateTime<Utc>, zone: Option<Tz>, owed_since: DateTime<Utc>) -> Option<String> {
    let (zone, opens) = zone.map_or((Tz::UTC, NO_ZONE_SLOT_START_HOUR_UTC), |zone| {
        (zone, SLOT_START_HOUR)
    });
    let hour = now.with_timezone(&zone).hour();
    if !(opens..SLOT_END_HOUR).contains(&hour) {
        return None;
    }
    let today = clock_date(now, zone);
    let slot_owed = monday_slot(today, zone, opens).is_none_or(|slot| slot >= owed_since);
    slot_owed.then(|| week_key(today))
}

/// The instant the slot opens on the Monday of `date`'s ISO week, or `None`
/// when that local hour does not exist in `zone` (a transition at the slot
/// hour, which no zone in use has) — such a week is treated as owed.
fn monday_slot(date: NaiveDate, zone: Tz, opens: u32) -> Option<DateTime<Utc>> {
    let monday =
        date.checked_sub_days(Days::new(u64::from(date.weekday().num_days_from_monday())))?;
    let local = monday.and_hms_opt(opens, 0, 0)?;
    zone.from_local_datetime(&local)
        .earliest()
        .map(|slot| slot.with_timezone(&Utc))
}

/// The ISO week `date` falls in, as `2026-W39`: the key the delivery ledger
/// holds one digest per group under.
#[must_use]
pub fn week_key(date: NaiveDate) -> String {
    let week = date.iso_week();
    format!("{}-W{:02}", week.year(), week.week())
}

/// The zone a group's digest slot is read in.
///
/// The owner's, else the human coach's, else the zone most members have on
/// file (the earliest-joined one among equals). A zone that does not parse is
/// passed over. `None` when no one has a usable zone, which [`due_week`]
/// reads as the no-zone rule.
#[must_use]
pub fn group_zone<'a>(
    owner: Option<&str>,
    coach: Option<&str>,
    members: impl IntoIterator<Item = Option<&'a str>>,
) -> Option<Tz> {
    parse_zone(owner)
        .or_else(|| parse_zone(coach))
        .or_else(|| most_common(members.into_iter().filter_map(parse_zone)))
}

/// The value seen most often, the first-seen one among equals.
fn most_common<T: Copy + PartialEq>(values: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: Vec<(T, usize)> = Vec::new();
    for value in values {
        match counts.iter_mut().find(|(seen, _)| *seen == value) {
            Some((_, count)) => *count += 1,
            None => counts.push((value, 1)),
        }
    }
    // `max_by_key` keeps the last of equal maxima; walking backwards makes
    // that the first one seen.
    counts
        .into_iter()
        .rev()
        .max_by_key(|(_, count)| *count)
        .map(|(value, _)| value)
}
