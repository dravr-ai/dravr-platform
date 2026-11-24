// ABOUTME: The consent-grant ceiling a call runs under, for the readers below dispatch: streams, tier, window, rows
// ABOUTME: under_grant scopes it; readers drop rows dated before its window and tally what they served for its budgets

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The grant ceiling a call runs under (plan slice 3, items 6 to 8).
//!
//! An executor serving a delegated token under a detailed consent grant runs
//! each tool body inside [`under_grant`], which scopes the grant's
//! [`GrantCeiling`] for every reader the body reaches:
//!
//! - [`tier_readable`](super::tier_readable) answers `false` above its
//!   `max_tier`;
//! - [`holds_stream`] answers whether the grant reads a stream, for a reader
//!   that serves a stream only incidentally: under a grant without routes,
//!   sessions are served without their start coordinates, city and segment
//!   efforts ([`without_route`]);
//! - [`window_since`] is the grant's window floor: a reader serves no row
//!   dated before it ([`retain_in_window`], [`in_window`]);
//! - each reader that passes rows on counts them ([`note_rows`],
//!   [`note_raw_rows`]) into the innermost [`counting_rows`] tally, which the
//!   executor levies against the grant's daily row budgets.
//!
//! A declaration only narrows: inside a call already under a ceiling, a
//! nested scope that names none keeps the outer one. Under
//! [`unfiltered`](super::unfiltered) (a cache write) nothing is floored or
//! counted: the cache must hold the athlete's full history, and the rows a
//! writer hands back are floored by the read that serves them.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use dravr_barrage::Stream;
use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_core::models::Activity;
use serde_json::Value;
use tracing::warn;

use super::{flag, UNFILTERED};

tokio::task_local! {
    /// The ceiling of the consent grant the current call is made under.
    static GRANT_CEILING: Option<Arc<GrantCeiling>>;
    /// The rows the current call served, for the innermost tally.
    static SERVED_ROWS: Arc<Mutex<ServedRows>>;
}

/// What one call served under a ceiling: the rows its readers passed on, and
/// among the time-series reads the samples ("raw rows").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServedRows {
    /// Rows served: sessions, nights, days, measurements
    pub rows: u64,
    /// Time-series samples served
    pub raw_rows: u64,
}

/// Run `fut` under `ceiling`, the ceiling of the consent grant its call is
/// made under; `None` keeps whatever ceiling encloses it.
pub async fn under_grant<F: Future>(ceiling: Option<GrantCeiling>, fut: F) -> F::Output {
    let effective = ceiling.map(Arc::new).or_else(current);
    GRANT_CEILING.scope(effective, fut).await
}

/// The ceiling in scope, shared.
fn current() -> Option<Arc<GrantCeiling>> {
    GRANT_CEILING.try_with(Clone::clone).ok().flatten()
}

/// The ceiling of the consent grant the current call is made under; `None`
/// outside one, under a plain grant, and for every other credential.
#[must_use]
pub fn grant_ceiling() -> Option<GrantCeiling> {
    current().map(|ceiling| (*ceiling).clone())
}

/// Whether the current call may read `stream`: always, outside a ceiling.
#[must_use]
pub fn holds_stream(stream: Stream) -> bool {
    current().is_none_or(|ceiling| ceiling.holds(stream))
}

/// When a row is dated: at a moment (a session, a night, a sample), or on
/// one of the athlete's days (a day's totals, recovery, body metrics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dated {
    /// At a moment
    At(Timestamp),
    /// On a day, in the athlete's zone
    Day(CivilDate),
}

/// The ceiling whose window floors a read made now: under a ceiling and
/// outside a cache write.
fn flooring() -> Option<Arc<GrantCeiling>> {
    if flag(&UNFILTERED) {
        return None;
    }
    current()
}

/// The earliest moment a row served now may be dated, under a ceiling and
/// outside a cache write; `None` when no window applies.
#[must_use]
pub fn window_since() -> Option<Timestamp> {
    flooring().map(|ceiling| ceiling.since)
}

/// Whether a row `dated` so may be served now: always, when no window
/// applies.
#[must_use]
pub fn in_window(dated: Dated) -> bool {
    flooring().is_none_or(|ceiling| match dated {
        Dated::At(at) => ceiling.admits_moment(at),
        Dated::Day(day) => ceiling.admits_day(day),
    })
}

/// Keep the items dated inside the window, `date_of` reading each one's
/// date; an item it names no date for is kept.
pub fn retain_in_window<T>(items: &mut Vec<T>, date_of: impl Fn(&T) -> Option<Dated>) {
    if flooring().is_some() {
        items.retain(|item| date_of(item).is_none_or(in_window));
    }
}

/// The fields of an activity that place it: its start coordinates, route
/// overview, city, region, country, trail and segment efforts (a segment's
/// name says where it lies).
const PLACING_FIELDS: [&str; 8] = [
    "start_latitude",
    "start_longitude",
    "summary_polyline",
    "city",
    "region",
    "country",
    "trail_name",
    "segment_efforts",
];

/// `activities` as a grant without [`Stream::WorkoutRoute`] reads them.
///
/// The sessions are served without what places them ([`PLACING_FIELDS`]) and
/// without the position samples of their streams; unchanged outside a
/// ceiling and under one that holds routes.
#[must_use]
pub fn without_route(activities: Vec<Activity>) -> Vec<Activity> {
    if holds_stream(Stream::WorkoutRoute) {
        return activities;
    }
    activities.iter().filter_map(placeless).collect()
}

/// One activity without what places it; `None` when it does not round-trip,
/// which drops it rather than serving its route.
fn placeless(activity: &Activity) -> Option<Activity> {
    let mut value = serde_json::to_value(activity)
        .inspect_err(|e| warn!("An activity's route could not be stripped; dropping it: {e}"))
        .ok()?;
    if let Some(fields) = value.as_object_mut() {
        for field in PLACING_FIELDS {
            fields.remove(field);
        }
        if let Some(series) = fields
            .get_mut("time_series_data")
            .and_then(Value::as_object_mut)
        {
            series.remove("gps_coordinates");
        }
    }
    serde_json::from_value(value)
        .inspect_err(|e| warn!("An activity's route could not be stripped; dropping it: {e}"))
        .ok()
}

/// Run `fut` counting the rows its readers serve under a ceiling: its output
/// and the count, its own, apart from any enclosing tally.
pub async fn counting_rows<F: Future>(fut: F) -> (F::Output, ServedRows) {
    let tally = Arc::new(Mutex::new(ServedRows::default()));
    let output = SERVED_ROWS.scope(tally.clone(), fut).await;
    let served = *tally.lock().unwrap_or_else(PoisonError::into_inner);
    (output, served)
}

/// Add `served` to the innermost tally, under a ceiling and outside a cache
/// write.
fn note(served: ServedRows) {
    if current().is_none() || flag(&UNFILTERED) {
        return;
    }
    let _ = SERVED_ROWS.try_with(|tally| {
        let mut tally = tally.lock().unwrap_or_else(PoisonError::into_inner);
        tally.rows = tally.rows.saturating_add(served.rows);
        tally.raw_rows = tally.raw_rows.saturating_add(served.raw_rows);
    });
}

/// Count `rows` rows a reader served.
pub fn note_rows(rows: usize) {
    note(ServedRows {
        rows: u64::try_from(rows).unwrap_or(u64::MAX),
        raw_rows: 0,
    });
}

/// Count `samples` time-series samples a reader served.
pub fn note_raw_rows(samples: usize) {
    note(ServedRows {
        rows: 0,
        raw_rows: u64::try_from(samples).unwrap_or(u64::MAX),
    });
}

#[cfg(test)]
mod tests {
    use dravr_barrage::{Preset, Tier};
    use dravr_tronc::dates::{Elapsed, Zone};
    use pierre_auth::oauth2_server::authorization_details::preset_defaults;

    use super::*;
    use crate::ai_scope::{tier_readable, unfiltered};

    fn ceiling(since: Timestamp) -> GrantCeiling {
        let details = preset_defaults(Preset::SleepCoach, "https://mcp.example.test/mcp");
        let mut ceiling = GrantCeiling::of_details(
            "g",
            &details.freeze(Timestamp::now(), &Zone::utc()).unwrap().0,
            &Zone::utc(),
        )
        .unwrap();
        ceiling.since = since;
        ceiling.first_day = since.utc_date();
        ceiling
    }

    #[tokio::test]
    async fn outside_a_ceiling_nothing_is_floored_or_counted() {
        let (kept, served) = counting_rows(async {
            let mut days = vec![Timestamp::now() - Elapsed::days(400)];
            retain_in_window(&mut days, |at| Some(Dated::At(*at)));
            note_rows(days.len());
            days
        })
        .await;
        assert_eq!(kept.len(), 1, "bug: a read outside a ceiling was floored");
        assert_eq!(
            served,
            ServedRows::default(),
            "bug: a read outside a ceiling was counted"
        );
        assert!(
            holds_stream(Stream::WorkoutRoute),
            "bug: a call outside a ceiling lost the route stream"
        );
        assert!(
            tier_readable(Tier::T1),
            "bug: a call outside a ceiling lost T1"
        );
    }

    #[tokio::test]
    async fn under_a_ceiling_rows_before_its_window_go_and_the_rest_are_counted() {
        let since = Timestamp::now() - Elapsed::days(1);
        let (kept, served) = under_grant(
            Some(ceiling(since)),
            counting_rows(async {
                let mut days = vec![since - Elapsed::hours(1), since, since + Elapsed::hours(1)];
                retain_in_window(&mut days, |at| Some(Dated::At(*at)));
                assert!(
                    in_window(Dated::Day(since.utc_date())),
                    "bug: the window's first day is outside it"
                );
                assert!(
                    !in_window(Dated::Day(since.utc_date().checked_sub_days(1).unwrap())),
                    "bug: the day before the window is inside it"
                );
                note_rows(days.len());
                note_raw_rows(40);
                assert!(
                    !holds_stream(Stream::Workout),
                    "a sleep grant reads no workout"
                );
                assert!(
                    holds_stream(Stream::Sleep),
                    "bug: a sleep grant lost its nights"
                );
                days
            }),
        )
        .await;
        assert_eq!(
            kept,
            [since, since + Elapsed::hours(1)],
            "bug: a row before the window was served, or one inside it dropped"
        );
        assert_eq!(
            served,
            ServedRows {
                rows: 2,
                raw_rows: 40
            },
            "bug: the served rows were not tallied"
        );
    }

    #[tokio::test]
    async fn a_grant_without_routes_reads_sessions_without_their_place() {
        use pierre_core::models::{ActivityBuilder, SegmentEffort, SportType};
        let start = Timestamp::now();
        let effort = SegmentEffort {
            id: "e".to_owned(),
            segment_id: Some("s".to_owned()),
            name: "Camillien-Houde climb".to_owned(),
            elapsed_time: 300,
            moving_time: None,
            start_date: start,
            distance: 1_800.0,
            average_heart_rate: None,
            max_heart_rate: None,
            average_cadence: None,
            average_watts: None,
            kom_rank: None,
            pr_rank: None,
            climb_category: None,
            average_grade: None,
            elevation_gain: None,
        };
        let ride = ActivityBuilder::new("r", "Ride", SportType::Ride, start, 3_600, "strava")
            .start_latitude(45.5)
            .start_longitude(-73.6)
            .summary_polyline("_p~iF~ps|U".to_owned())
            .city("Montreal".to_owned())
            .region("Quebec".to_owned())
            .country("Canada".to_owned())
            .trail_name("Mount Royal".to_owned())
            .segment_efforts(vec![effort])
            .build();
        let outside = &without_route(vec![ride.clone()])[0];
        assert_eq!(
            outside.start_latitude(),
            Some(45.5),
            "bug: a session outside a ceiling lost its place"
        );
        assert_eq!(
            outside.segment_efforts().map(Vec::len),
            Some(1),
            "bug: a session outside a ceiling lost its segment efforts"
        );
        let served = under_grant(Some(ceiling(start)), async { without_route(vec![ride]) }).await;
        assert_eq!(served.len(), 1, "the session is served");
        let placeless = &served[0];
        assert_eq!(
            (placeless.start_latitude(), placeless.start_longitude()),
            (None, None),
            "bug: a preset grant read a route's start"
        );
        assert_eq!(
            placeless.summary_polyline(),
            None,
            "bug: a preset grant read a route overview"
        );
        assert_eq!(
            (
                placeless.city(),
                placeless.region(),
                placeless.country(),
                placeless.trail_name()
            ),
            (None, None, None, None),
            "bug: a preset grant read where a session took place"
        );
        assert!(
            placeless.segment_efforts().is_none(),
            "bug: a preset grant read segment efforts, whose names place the session"
        );
    }

    #[tokio::test]
    async fn a_nested_scope_naming_no_ceiling_keeps_the_outer_one() {
        let since = Timestamp::now();
        under_grant(Some(ceiling(since)), async {
            under_grant(None, async {
                assert_eq!(
                    window_since(),
                    Some(since),
                    "bug: a nested call widened its grant"
                );
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn a_cache_write_under_a_ceiling_is_neither_floored_nor_counted() {
        let since = Timestamp::now();
        let ((), served) = under_grant(
            Some(ceiling(since)),
            counting_rows(unfiltered(async {
                assert_eq!(
                    window_since(),
                    None,
                    "bug: a cache write stored a truncated history"
                );
                note_rows(5);
            })),
        )
        .await;
        assert_eq!(
            served,
            ServedRows::default(),
            "bug: a cache write was counted against the grant"
        );
    }
}
