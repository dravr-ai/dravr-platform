// ABOUTME: The athlete's weekly training volume for Home — distance, time, climbing and sessions per sport, week by week
// ABOUTME: Mapped from the one aggregator's week-by-sport count on the athlete's own calendar; a week before the stored history is absent, never zero

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Weekly training volume, as Home shows it.
//!
//! Each week runs Monday to Sunday on the athlete's own calendar, the week
//! Home's plan strip already uses. The weeks are the one aggregator's count
//! ([`query_activities`](pierre_tool_runtime::activity_query::query_activities))
//! broken down by week and sport: a workout counts once, whichever providers
//! hold a copy of it, merged as the Home list merges it, so the volume and the
//! list cannot disagree on what was done.
//!
//! A week the stored history does not reach carries no row. An athlete whose
//! oldest stored activity is three weeks old gets three weeks, not twelve with
//! nine zeros: an empty week is only ever a week the cache vouches for and the
//! athlete did not train in.

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate};
use pierre_core::civil_time::monday_of;
use pierre_core::constants::oauth_providers::GarminContribution;
use pierre_core::models::coverage::IncompleteReason;
use pierre_tool_runtime::activity_query::{GroupKey, KeyValue, QueryAnswer, QueryMetric};
use serde::Serialize;

/// Weeks the volume spans, the current one included.
pub const VOLUME_WEEKS: i64 = 12;

/// What the volume's count is broken down by: the week, then the sport.
pub(crate) const VOLUME_GROUPS: [GroupKey; 2] = [GroupKey::Week, GroupKey::SportType];

/// What the volume's count sums: a week's sport entry carries exactly these.
pub(crate) const VOLUME_METRICS: [QueryMetric; 4] = [
    QueryMetric::Count,
    QueryMetric::DistanceKm,
    QueryMetric::ElapsedTimeS,
    QueryMetric::ElevationM,
];

/// Body of `GET /api/me/training-volume`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrainingVolume {
    /// The athlete's civil date the volume is read on, `YYYY-MM-DD`.
    pub today: NaiveDate,
    /// One entry per week the stored history reaches, oldest first, ending
    /// with the week `today` falls in. At most [`VOLUME_WEEKS`]; empty when
    /// nothing is stored.
    pub weeks: Vec<VolumeWeek>,
    /// What Garmin-sourced data contributed to the weeks, which decides the
    /// line Garmin's brand guidelines owe them (carnet#1154); `null` when no
    /// workout they count carries such data.
    pub garmin_contribution: Option<GarminContribution>,
}

/// One Monday-to-Sunday week.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VolumeWeek {
    /// The Monday the week starts on, `YYYY-MM-DD`.
    pub week_start: NaiveDate,
    /// One entry per sport trained that week, by sport; empty for a week
    /// without training.
    pub sports: Vec<SportVolume>,
}

/// What one sport added up to in one week.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SportVolume {
    /// The sport, as the activity cache's own `sport_type` column spells it.
    pub sport_type: String,
    /// Workouts counted.
    pub activities: u32,
    /// Distance in metres, over the workouts that recorded one.
    pub distance_meters: f64,
    /// Elapsed time in seconds.
    pub duration_seconds: u64,
    /// Elevation gained in metres, over the workouts that recorded it.
    pub elevation_gain_meters: f64,
}

/// The Monday the volume window opens on: [`VOLUME_WEEKS`] weeks, the current
/// one included.
#[must_use]
pub fn window_start(today: NaiveDate) -> NaiveDate {
    monday_of(today) - Duration::weeks(VOLUME_WEEKS - 1)
}

/// The weeks of a count over `[window_start(today), today]` broken down by
/// [`VOLUME_GROUPS`] and summing [`VOLUME_METRICS`].
///
/// The weeks run from the first one the answer's coverage vouches for to the
/// week of `today`: the window's start when it is complete, the week of its
/// `trustworthy_from` when the stored history stops short of it, none when
/// nothing is stored or an incomplete answer names no such day. A group
/// outside those weeks is left out.
#[must_use]
pub fn volume_from_answer(today: NaiveDate, answer: &QueryAnswer) -> TrainingVolume {
    let opens = window_start(today);
    let coverage = &answer.coverage;
    // An incomplete answer vouches only from the day it names, and for no
    // week when it names none.
    let first_week = match coverage.completeness.reason() {
        None => Some(opens),
        Some(IncompleteReason::NoStoredActivities) => None,
        Some(_) => coverage
            .trustworthy_from
            .map(|day| monday_of(day).max(opens)),
    };
    let last_week = monday_of(today);
    let Some(first_week) = first_week.filter(|first| *first <= last_week) else {
        return TrainingVolume {
            today,
            weeks: Vec::new(),
            garmin_contribution: None,
        };
    };

    let mut totals: BTreeMap<(NaiveDate, String), SportVolume> = BTreeMap::new();
    for group in &answer.groups {
        let (Some(KeyValue::Text(week)), Some(KeyValue::Text(sport))) = (
            group.key.get(&GroupKey::Week),
            group.key.get(&GroupKey::SportType),
        ) else {
            continue;
        };
        let Ok(week) = week.parse::<NaiveDate>() else {
            continue;
        };
        if week < first_week || week > last_week {
            continue;
        }
        let values = &group.values;
        totals.insert(
            (week, sport.clone()),
            SportVolume {
                sport_type: sport.clone(),
                activities: values
                    .count
                    .map_or(0, |count| u32::try_from(count).unwrap_or(u32::MAX)),
                distance_meters: values.distance_metres().unwrap_or(0.0),
                duration_seconds: values.elapsed_time_s.unwrap_or(0),
                elevation_gain_meters: values.elevation_m.unwrap_or(0.0),
            },
        );
    }

    let mut weeks = Vec::new();
    let mut week_start = first_week;
    while week_start <= last_week {
        let sports = totals
            .range((week_start, String::new())..(week_start + Duration::days(1), String::new()))
            .map(|(_, volume)| volume.clone())
            .collect();
        weeks.push(VolumeWeek { week_start, sports });
        week_start += Duration::weeks(1);
    }
    TrainingVolume {
        today,
        weeks,
        // The answer counts no workout before the first vouched week: its
        // vouching rule stands from the oldest row it read.
        garmin_contribution: answer.garmin_contribution,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use chrono_tz::Tz;
    use pierre_core::models::coverage::{Completeness, Coverage, CoverageWindow};
    use pierre_core::models::{Activity, ActivityBuilder, SportType};
    use pierre_database::repositories::CachedActivityRow;
    use pierre_services::workouts::workouts;
    use pierre_tool_runtime::activity_query::{
        aggregate, ActivityPredicate, ActivityQuery, AppliedQuery, QueryGroup,
    };

    fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    fn session(
        id: &str,
        sport: SportType,
        start: DateTime<Utc>,
        seconds: u64,
        metres: Option<f64>,
        climb: Option<f64>,
    ) -> Activity {
        let mut builder = ActivityBuilder::new(
            id.to_owned(),
            id.to_owned(),
            sport,
            start,
            seconds,
            "strava".to_owned(),
        );
        if let Some(metres) = metres {
            builder = builder.distance_meters(metres);
        }
        if let Some(climb) = climb {
            builder = builder.elevation_gain(climb);
        }
        builder.build()
    }

    fn at(date: NaiveDate, hour: u32) -> DateTime<Utc> {
        date.and_hms_opt(hour, 0, 0).unwrap_or_default().and_utc()
    }

    // Wednesday 7 October 2026.
    const TODAY: (i32, u32, u32) = (2026, 10, 7);

    fn today() -> NaiveDate {
        ymd(TODAY.0, TODAY.1, TODAY.2)
    }

    /// The count Home asks for, over the volume window.
    fn query() -> ActivityQuery {
        ActivityQuery {
            from: Some(window_start(today())),
            to: today(),
            filter: ActivityPredicate::default(),
            group_by: VOLUME_GROUPS.to_vec(),
            metrics: VOLUME_METRICS.to_vec(),
            provider: None,
        }
    }

    /// The aggregator's answer over `sessions`, stored one row each, on the
    /// athlete's calendar in `zone`, standing as `coverage` says.
    fn answer(zone: Tz, sessions: &[Activity], coverage: Coverage) -> QueryAnswer {
        let rows: Vec<CachedActivityRow> = sessions
            .iter()
            .map(|activity| CachedActivityRow {
                provider: activity.provider().to_owned(),
                activity: activity.clone(),
                route: None,
                detail_read: false,
                segment_read: false,
                stored_at: None,
                place: None,
            })
            .collect();
        let query = query();
        let (totals, groups, unknown) = aggregate(&workouts(&rows), &query, zone);
        QueryAnswer {
            applied: AppliedQuery::of(&query),
            totals,
            groups_total: groups.len(),
            groups_truncated: false,
            groups,
            unknown,
            coverage,
            coverage_by_source: Vec::new(),
            garmin_contribution: None,
            segments: None,
        }
    }

    /// The cache holds activities from before the window.
    fn whole() -> Coverage {
        Coverage::complete(CoverageWindow::default())
    }

    /// The stored history begins on `day`.
    fn from(day: NaiveDate) -> Coverage {
        Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::ShallowHistory),
            window: CoverageWindow {
                oldest: Some(day),
                ..CoverageWindow::default()
            },
            trustworthy_from: Some(day),
            pending: None,
        }
    }

    /// Nothing is stored.
    fn nothing() -> Coverage {
        Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::NoStoredActivities),
            window: CoverageWindow::default(),
            trustworthy_from: None,
            pending: None,
        }
    }

    #[test]
    fn a_whole_history_spans_twelve_monday_weeks_ending_this_week() {
        let volume = volume_from_answer(today(), &answer(Tz::UTC, &[], whole()));
        assert_eq!(volume.weeks.len(), 12, "bug: not twelve weeks");
        assert_eq!(
            volume.weeks[11].week_start,
            ymd(2026, 10, 5),
            "bug: the last week is not this week"
        );
        assert_eq!(
            volume.weeks[0].week_start,
            ymd(2026, 7, 20),
            "bug: the first week is not eleven weeks back"
        );
        assert!(
            volume.weeks.iter().all(|week| week.sports.is_empty()),
            "bug: an empty history filled a week"
        );
    }

    #[test]
    fn sessions_are_summed_per_week_and_per_sport() {
        let sessions = [
            session(
                "a",
                SportType::Run,
                at(ymd(2026, 10, 5), 7),
                3_600,
                Some(10_000.0),
                Some(80.0),
            ),
            session(
                "b",
                SportType::Run,
                at(ymd(2026, 10, 6), 7),
                1_800,
                Some(5_000.0),
                None,
            ),
            session(
                "c",
                SportType::Ride,
                at(ymd(2026, 10, 6), 17),
                5_400,
                Some(40_000.0),
                Some(450.0),
            ),
            session(
                "d",
                SportType::Run,
                at(ymd(2026, 9, 30), 7),
                2_700,
                Some(8_000.0),
                Some(20.0),
            ),
            // A session without a distance still counts its time.
            session(
                "e",
                SportType::Run,
                at(ymd(2026, 10, 4), 7),
                600,
                None,
                None,
            ),
        ];
        let volume = volume_from_answer(today(), &answer(Tz::UTC, &sessions, whole()));
        let this_week = &volume.weeks[11];
        assert_eq!(
            this_week.sports,
            vec![
                SportVolume {
                    sport_type: "ride".to_owned(),
                    activities: 1,
                    distance_meters: 40_000.0,
                    duration_seconds: 5_400,
                    elevation_gain_meters: 450.0,
                },
                SportVolume {
                    sport_type: "run".to_owned(),
                    activities: 2,
                    distance_meters: 15_000.0,
                    duration_seconds: 5_400,
                    elevation_gain_meters: 80.0,
                },
            ],
            "bug: this week's sports are not summed per sport"
        );
        let last_week = &volume.weeks[10];
        assert_eq!(
            last_week.week_start,
            ymd(2026, 9, 28),
            "bug: the week before this one is not the Monday before"
        );
        assert_eq!(
            last_week.sports,
            vec![SportVolume {
                sport_type: "run".to_owned(),
                activities: 2,
                distance_meters: 8_000.0,
                duration_seconds: 3_300,
                elevation_gain_meters: 20.0,
            }],
            "bug: last week's runs are not summed, or a distance-less run dropped its time"
        );
    }

    #[test]
    fn a_session_lands_on_the_athletes_own_calendar_day() {
        // 02:00 UTC on Monday 5 October is 22:00 on Sunday the 4th in Toronto:
        // last week, not this one.
        let zone: Tz = "America/Toronto".parse().unwrap_or(Tz::UTC);
        let sessions = [session(
            "late",
            SportType::Run,
            at(ymd(2026, 10, 5), 2),
            3_600,
            Some(10_000.0),
            None,
        )];
        let volume = volume_from_answer(today(), &answer(zone, &sessions, whole()));
        assert_eq!(
            volume.weeks[11].sports,
            [],
            "bug: a Sunday-night run counted in the next week"
        );
        assert_eq!(
            volume.weeks[10].sports.len(),
            1,
            "bug: a Sunday-night run is not in its own week"
        );
    }

    #[test]
    fn weeks_before_the_stored_history_are_absent_not_zero() {
        let volume = volume_from_answer(today(), &answer(Tz::UTC, &[], from(ymd(2026, 9, 24))));
        let starts: Vec<_> = volume.weeks.iter().map(|week| week.week_start).collect();
        assert_eq!(
            starts,
            vec![ymd(2026, 9, 21), ymd(2026, 9, 28), ymd(2026, 10, 5)],
            "bug: a week before the stored history is listed"
        );
    }

    #[test]
    fn a_history_older_than_the_window_still_opens_on_the_window() {
        let volume = volume_from_answer(today(), &answer(Tz::UTC, &[], from(ymd(2025, 1, 1))));
        assert_eq!(
            volume.weeks.len(),
            12,
            "bug: a history older than the window is cut"
        );
    }

    #[test]
    fn nothing_stored_answers_no_weeks() {
        let volume = volume_from_answer(today(), &answer(Tz::UTC, &[], nothing()));
        assert_eq!(volume.weeks, [], "bug: nothing stored answered weeks");
    }

    /// An answer that stopped short of the window without naming the first
    /// day it stands behind vouches for no week: none is summed as if whole.
    #[test]
    fn an_incomplete_answer_naming_no_first_day_answers_no_weeks() {
        let short = Coverage {
            completeness: Completeness::Incomplete(IncompleteReason::ReadCapped),
            window: CoverageWindow::default(),
            trustworthy_from: None,
            pending: None,
        };
        let sessions = [session(
            "a",
            SportType::Run,
            at(ymd(2026, 10, 5), 7),
            3_600,
            Some(10_000.0),
            None,
        )];
        let volume = volume_from_answer(today(), &answer(Tz::UTC, &sessions, short));
        assert_eq!(
            volume.weeks,
            [],
            "bug: an incomplete answer with no vouched day was summed over the whole window"
        );
    }

    /// A group the answer carries for a week before the window, or before
    /// the first week it vouches for, is left out: the read stops at the
    /// window, and the weeks at the history.
    #[test]
    fn groups_outside_the_vouched_weeks_are_left_out() {
        let sessions = [
            session(
                "old",
                SportType::Run,
                at(ymd(2026, 7, 19), 7),
                3_600,
                Some(10_000.0),
                None,
            ),
            session(
                "early",
                SportType::Run,
                at(ymd(2026, 9, 15), 7),
                3_600,
                Some(10_000.0),
                None,
            ),
        ];
        let window = volume_from_answer(today(), &answer(Tz::UTC, &sessions[..1], whole()));
        assert!(
            window.weeks.iter().all(|week| week.sports.is_empty()),
            "bug: a week before the window was summed"
        );
        let shallow = volume_from_answer(
            today(),
            &answer(Tz::UTC, &sessions[1..], from(ymd(2026, 9, 24))),
        );
        assert!(
            shallow.weeks.iter().all(|week| week.sports.is_empty()),
            "bug: a week before the first vouched one was summed"
        );
    }

    /// A distance comes back in metres as it was recorded, not as the
    /// kilometres the count serves it in.
    #[test]
    fn a_distance_comes_back_in_the_metres_recorded() {
        let sessions = [session(
            "odd",
            SportType::Ride,
            at(ymd(2026, 10, 5), 7),
            3_600,
            Some(254.3),
            None,
        )];
        let volume = volume_from_answer(today(), &answer(Tz::UTC, &sessions, whole()));
        assert_eq!(
            volume.weeks[11].sports[0].distance_meters, 254.3,
            "bug: the distance drifted on its way through kilometres"
        );
    }

    /// A group that is not keyed by a week and a sport is not a volume
    /// entry.
    #[test]
    fn a_group_without_a_week_and_a_sport_is_ignored() {
        let mut answer = answer(Tz::UTC, &[], whole());
        answer.groups.push(QueryGroup {
            key: BTreeMap::from([(GroupKey::Week, KeyValue::Text("2026-10-05".to_owned()))]),
            label: BTreeMap::new(),
            values: answer.totals,
        });
        let volume = volume_from_answer(today(), &answer);
        assert!(
            volume.weeks.iter().all(|week| week.sports.is_empty()),
            "bug: a group with no sport made a volume entry"
        );
    }
}
