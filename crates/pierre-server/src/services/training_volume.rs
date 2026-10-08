// ABOUTME: The athlete's weekly training volume for Home — distance, time, climbing and sessions per sport, week by week
// ABOUTME: Summed from the merged cached workouts on the athlete's own calendar; a week before the stored history begins is absent, never zero

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Weekly training volume, as Home shows it.
//!
//! Each week runs Monday to Sunday on the athlete's own calendar, the week
//! Home's plan strip already uses. A workout counts once, whichever providers
//! hold a copy of it: the caller hands in the sessions the Home list merges,
//! so the volume and the list cannot disagree on what was done.
//!
//! A week the stored history does not reach carries no row. An athlete whose
//! oldest stored activity is three weeks old gets three weeks, not twelve with
//! nine zeros: an empty week is only ever a week the cache vouches for and the
//! athlete did not train in.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use pierre_core::civil_time::local_date;
use pierre_core::models::Activity;
use pierre_database::repositories::sport_type_string;
use serde::Serialize;

/// Weeks the volume spans, the current one included.
pub const VOLUME_WEEKS: i64 = 12;

/// The sport an activity is summed under when its sport has no spelling.
const UNNAMED_SPORT: &str = "other";

/// Body of `GET /api/me/training-volume`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrainingVolume {
    /// The athlete's civil date the volume is read on, `YYYY-MM-DD`.
    pub today: NaiveDate,
    /// One entry per week the stored history reaches, oldest first, ending
    /// with the week `today` falls in. At most [`VOLUME_WEEKS`]; empty when
    /// nothing is stored.
    pub weeks: Vec<VolumeWeek>,
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

/// How far back the stored history reaches, as far as the volume needs to
/// know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeCoverage {
    /// The cache holds activities from before the window: every week of it is
    /// vouched for.
    Whole,
    /// The stored history begins on this day: weeks before the one it falls
    /// in are absent.
    From(NaiveDate),
    /// Nothing is stored.
    Nothing,
}

/// The Monday of the week `date` falls in.
#[must_use]
pub fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// The Monday the volume window opens on: [`VOLUME_WEEKS`] weeks, the current
/// one included.
#[must_use]
pub fn window_start(today: NaiveDate) -> NaiveDate {
    monday_of(today) - Duration::weeks(VOLUME_WEEKS - 1)
}

/// The instant the athlete's `date` begins in `zone`. A midnight the zone
/// skips (a DST gap) falls back to midnight UTC, which is never later than an
/// hour off and only widens the read.
#[must_use]
pub fn day_start_utc(date: NaiveDate, zone: Tz) -> DateTime<Utc> {
    let midnight = date.and_time(chrono::NaiveTime::MIN);
    zone.from_local_datetime(&midnight)
        .earliest()
        .map_or_else(|| midnight.and_utc(), |start| start.with_timezone(&Utc))
}

/// Sum `sessions`, one per workout, into weeks on the athlete's calendar.
///
/// The sessions are already merged across providers. The weeks run from the
/// first one `coverage` vouches for to the week of `today`, in `zone`; a
/// session outside those weeks, or after `today`, is left out.
#[must_use]
pub fn volume_from_sessions(
    today: NaiveDate,
    zone: Tz,
    sessions: &[Activity],
    coverage: VolumeCoverage,
) -> TrainingVolume {
    let opens = window_start(today);
    let first_week = match coverage {
        VolumeCoverage::Nothing => {
            return TrainingVolume {
                today,
                weeks: Vec::new(),
            }
        }
        VolumeCoverage::Whole => opens,
        VolumeCoverage::From(day) => monday_of(day).max(opens),
    };
    let last_week = monday_of(today);
    if first_week > last_week {
        return TrainingVolume {
            today,
            weeks: Vec::new(),
        };
    }

    let mut totals: BTreeMap<(NaiveDate, String), SportVolume> = BTreeMap::new();
    for session in sessions {
        let day = local_date(session.start_date(), zone);
        if day < first_week || day > today {
            continue;
        }
        let sport = sport_type_string(session).unwrap_or_else(|| UNNAMED_SPORT.to_owned());
        let entry = totals
            .entry((monday_of(day), sport.clone()))
            .or_insert_with(|| SportVolume {
                sport_type: sport,
                activities: 0,
                distance_meters: 0.0,
                duration_seconds: 0,
                elevation_gain_meters: 0.0,
            });
        entry.activities = entry.activities.saturating_add(1);
        entry.distance_meters += session.distance_meters().unwrap_or(0.0).max(0.0);
        entry.duration_seconds = entry
            .duration_seconds
            .saturating_add(session.duration_seconds());
        entry.elevation_gain_meters += session.elevation_gain().unwrap_or(0.0).max(0.0);
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
    TrainingVolume { today, weeks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::{ActivityBuilder, SportType};

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

    #[test]
    fn a_whole_history_spans_twelve_monday_weeks_ending_this_week() {
        let volume = volume_from_sessions(today(), Tz::UTC, &[], VolumeCoverage::Whole);
        assert_eq!(volume.weeks.len(), 12);
        assert_eq!(volume.weeks[11].week_start, ymd(2026, 10, 5));
        assert_eq!(volume.weeks[0].week_start, ymd(2026, 7, 20));
        assert!(volume.weeks.iter().all(|week| week.sports.is_empty()));
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
        let volume = volume_from_sessions(today(), Tz::UTC, &sessions, VolumeCoverage::Whole);
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
            ]
        );
        let last_week = &volume.weeks[10];
        assert_eq!(last_week.week_start, ymd(2026, 9, 28));
        assert_eq!(
            last_week.sports,
            vec![SportVolume {
                sport_type: "run".to_owned(),
                activities: 2,
                distance_meters: 8_000.0,
                duration_seconds: 3_300,
                elevation_gain_meters: 20.0,
            }]
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
        let volume = volume_from_sessions(today(), zone, &sessions, VolumeCoverage::Whole);
        assert!(volume.weeks[11].sports.is_empty());
        assert_eq!(volume.weeks[10].sports.len(), 1);
    }

    #[test]
    fn weeks_before_the_stored_history_are_absent_not_zero() {
        let volume = volume_from_sessions(
            today(),
            Tz::UTC,
            &[],
            VolumeCoverage::From(ymd(2026, 9, 24)),
        );
        let starts: Vec<_> = volume.weeks.iter().map(|week| week.week_start).collect();
        assert_eq!(
            starts,
            vec![ymd(2026, 9, 21), ymd(2026, 9, 28), ymd(2026, 10, 5)]
        );
    }

    #[test]
    fn a_history_older_than_the_window_still_opens_on_the_window() {
        let volume =
            volume_from_sessions(today(), Tz::UTC, &[], VolumeCoverage::From(ymd(2025, 1, 1)));
        assert_eq!(volume.weeks.len(), 12);
    }

    #[test]
    fn nothing_stored_answers_no_weeks() {
        let volume = volume_from_sessions(today(), Tz::UTC, &[], VolumeCoverage::Nothing);
        assert!(volume.weeks.is_empty());
    }

    #[test]
    fn sessions_outside_the_window_or_after_today_are_left_out() {
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
                "future",
                SportType::Run,
                at(ymd(2026, 10, 9), 7),
                3_600,
                Some(10_000.0),
                None,
            ),
        ];
        let volume = volume_from_sessions(today(), Tz::UTC, &sessions, VolumeCoverage::Whole);
        assert!(volume.weeks.iter().all(|week| week.sports.is_empty()));
    }

    #[test]
    fn the_window_opens_at_the_athletes_local_midnight() {
        let zone: Tz = "America/Toronto".parse().unwrap_or(Tz::UTC);
        assert_eq!(
            day_start_utc(ymd(2026, 7, 20), zone),
            at(ymd(2026, 7, 20), 4)
        );
    }
}
