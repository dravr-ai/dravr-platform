// ABOUTME: The one summing implementation: count, distance, time, climbing and heart rate over workouts, by group
// ABOUTME: Groups are civil years, months and Monday weeks, sports, places or gear; a mean is weighted once at the end

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The accumulator.
//!
//! Every count, distance or time over a period the aggregator answers is
//! summed here, once: [`Accumulator::add`] takes a workout that a read
//! merged, and [`Accumulator::finish`] turns the sums into [`MetricValues`] and
//! [`QueryGroup`]s. A read that comes in chunks adds them all to one
//! accumulator, so a group that spans chunks is summed across them and a
//! mean is computed once, at the end, never averaged across chunks.

use std::collections::{BTreeMap, HashMap};

use dravr_tronc::date_pattern;
use dravr_tronc::dates::{CivilDate, Pattern, Zone};
use pierre_core::civil_time::{local_date, monday_of};
use pierre_core::models::coverage::WindowTotals;
use pierre_core::models::{Activity, PlaceRef, SportFamily};
use pierre_core::untrusted::display_line;
use pierre_services::workouts::{sport_bucket, Workout};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::predicate::{export_only, segments_read, walkable_segments, Unknowable, Verdict};
use super::{ActivityQuery, SegmentCounts};

/// `2026-11`: the key of a month bucket.
const MONTH_KEY: Pattern<CivilDate> = date_pattern!(CivilDate, "%Y-%m");

/// Groups an answer carries at most, the largest by count.
///
/// 12 years by 12 months, or a year by 15 sports, fit. A payload other than an activity
/// list travels whole into the prompt and is replayed in history.
pub const MAX_GROUPS: usize = 200;

/// Longest gear name a gear with no id is keyed by.
const MAX_GEAR_KEY_CHARS: usize = 60;

/// The key value of a workout that lacks the field a group keys on.
pub const UNKNOWN_KEY: &str = "unknown";

/// What a count is broken down by.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GroupKey {
    /// The athlete's civil year.
    Year,
    /// The athlete's civil month, `YYYY-MM`.
    Month,
    /// The Monday-to-Sunday week, by its Monday, `YYYY-MM-DD`.
    Week,
    /// The sport, as the store spells it.
    SportType,
    /// The sport family: running, cycling, swimming or other.
    SportFamily,
    /// The place the workout starts at, by its id.
    Place,
    /// The gear the provider recorded, by its id (else its name).
    Gear,
}

/// What a count sums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMetric {
    /// Workouts.
    Count,
    /// Kilometres, over the workouts that record a distance.
    DistanceKm,
    /// Moving seconds, over the workouts that report moving time.
    MovingTimeS,
    /// Elapsed seconds.
    ElapsedTimeS,
    /// Metres climbed, over the workouts that record it.
    ElevationM,
    /// Average heart rate, weighted by moving time (else elapsed time).
    AvgHr,
}

/// The metrics a count sums when its caller names none.
pub const DEFAULT_METRICS: [QueryMetric; 4] = [
    QueryMetric::Count,
    QueryMetric::DistanceKm,
    QueryMetric::ElapsedTimeS,
    QueryMetric::ElevationM,
];

/// One group's value for one key: a civil year, or a text key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum KeyValue {
    /// A civil year.
    Year(i32),
    /// Any other key: a month, a week's Monday, a sport, a place id, a gear
    /// id, or `unknown`.
    Text(String),
}

/// The sums asked for; a metric not asked for is absent.
///
/// Held exact, so a caller summing or converting them loses nothing, and
/// serialized with one decimal on distance, climbing and heart rate, as a
/// window's totals are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, JsonSchema)]
#[serde(into = "MetricValuesWire")]
pub struct MetricValues {
    /// Workouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// Kilometres, over the workouts that record a distance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_km: Option<f64>,
    /// Moving seconds, over the workouts that report moving time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moving_time_s: Option<u64>,
    /// Workouts that reported moving time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moving_time_sessions: Option<u64>,
    /// Elapsed seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_time_s: Option<u64>,
    /// Metres climbed, over the workouts that record it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation_m: Option<f64>,
    /// Average heart rate, weighted by moving time, else elapsed time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_hr: Option<f64>,
    /// Workouts that recorded an average heart rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_hr_sessions: Option<u64>,
}

impl MetricValues {
    /// The distance in metres, to the millimetre. The sums are held in metres
    /// and served in kilometres, and the way back would otherwise carry the
    /// division's rounding (254.3 m read back as 254.30000000000004).
    #[must_use]
    pub fn distance_metres(&self) -> Option<f64> {
        self.distance_km
            .map(|km| (km * 1_000_000.0).round() / 1_000.0)
    }
}

/// [`MetricValues`] as it travels: distance, climbing and heart rate to one
/// decimal.
#[derive(Serialize, JsonSchema)]
#[schemars(rename = "MetricValues")]
struct MetricValuesWire {
    /// Workouts.
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<u64>,
    /// Kilometres, over the workouts that record a distance.
    #[serde(skip_serializing_if = "Option::is_none")]
    distance_km: Option<f64>,
    /// Moving seconds, over the workouts that report moving time.
    #[serde(skip_serializing_if = "Option::is_none")]
    moving_time_s: Option<u64>,
    /// Workouts that reported moving time.
    #[serde(skip_serializing_if = "Option::is_none")]
    moving_time_sessions: Option<u64>,
    /// Elapsed seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    elapsed_time_s: Option<u64>,
    /// Metres climbed, over the workouts that record it.
    #[serde(skip_serializing_if = "Option::is_none")]
    elevation_m: Option<f64>,
    /// Average heart rate, weighted by moving time, else elapsed time.
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_hr: Option<f64>,
    /// Workouts that recorded an average heart rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_hr_sessions: Option<u64>,
}

impl From<MetricValues> for MetricValuesWire {
    fn from(values: MetricValues) -> Self {
        let one_decimal = |value: Option<f64>| value.map(|value| (value * 10.0).round() / 10.0);
        Self {
            count: values.count,
            distance_km: one_decimal(values.distance_km),
            moving_time_s: values.moving_time_s,
            moving_time_sessions: values.moving_time_sessions,
            elapsed_time_s: values.elapsed_time_s,
            elevation_m: one_decimal(values.elevation_m),
            avg_hr: one_decimal(values.avg_hr),
            avg_hr_sessions: values.avg_hr_sessions,
        }
    }
}

/// One group of a broken-down count.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct QueryGroup {
    /// The group's value for each key it is broken down by.
    pub key: BTreeMap<GroupKey, KeyValue>,
    /// A readable name for a place or gear key, when there is one this
    /// reader may see.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub label: BTreeMap<GroupKey, String>,
    /// The group's sums.
    #[serde(flatten)]
    pub values: MetricValues,
}

/// Workouts a filter could not judge for lack of the field it needs, per
/// field: never counted as a miss.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UnknownCounts {
    /// Workouts whose segments were never read, or that no detail walk reads.
    pub has_segment: u64,
    /// Workouts that belong to no place.
    pub place: u64,
    /// Workouts that name no gear.
    pub gear: u64,
    /// Workouts with no trainer flag.
    pub trainer: u64,
    /// Workouts with no workout code.
    pub workout_type: u64,
}

impl UnknownCounts {
    fn count(&mut self, field: Unknowable) {
        let slot = match field {
            Unknowable::HasSegment => &mut self.has_segment,
            Unknowable::Place => &mut self.place,
            Unknowable::Gear => &mut self.gear,
            Unknowable::Trainer => &mut self.trainer,
            Unknowable::WorkoutType => &mut self.workout_type,
        };
        *slot = slot.saturating_add(1);
    }
}

/// The running sums of one group, or of the totals.
#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    count: u64,
    distance_m: f64,
    moving_s: u64,
    moving_sessions: u64,
    elapsed_s: u64,
    elevation_m: f64,
    hr_weighted: f64,
    hr_weight: f64,
    hr_sessions: u64,
}

impl Cell {
    fn add(&mut self, session: &Activity) {
        self.count = self.count.saturating_add(1);
        if let Some(metres) = session.distance_meters().filter(|m| m.is_finite()) {
            self.distance_m += metres.max(0.0);
        }
        self.elapsed_s = self.elapsed_s.saturating_add(session.duration_seconds());
        let moving = session.moving_time_seconds();
        if let Some(moving) = moving {
            self.moving_s = self.moving_s.saturating_add(moving);
            self.moving_sessions = self.moving_sessions.saturating_add(1);
        }
        if let Some(climb) = session.elevation_gain().filter(|m| m.is_finite()) {
            self.elevation_m += climb.max(0.0);
        }
        if let Some(hr) = session.average_heart_rate() {
            let seconds = moving
                .filter(|s| *s > 0)
                .unwrap_or_else(|| session.duration_seconds());
            // A session with no time at all still counts once.
            let weight = if seconds == 0 {
                1.0
            } else {
                f64::from(u32::try_from(seconds).unwrap_or(u32::MAX))
            };
            self.hr_weighted = f64::from(hr).mul_add(weight, self.hr_weighted);
            self.hr_weight += weight;
            self.hr_sessions = self.hr_sessions.saturating_add(1);
        }
    }

    fn values(&self, metrics: &[QueryMetric]) -> MetricValues {
        let mut values = MetricValues::default();
        for metric in metrics {
            match metric {
                QueryMetric::Count => values.count = Some(self.count),
                QueryMetric::DistanceKm => {
                    values.distance_km = Some(self.distance_m / 1000.0);
                }
                QueryMetric::MovingTimeS => {
                    values.moving_time_s = Some(self.moving_s);
                    values.moving_time_sessions = Some(self.moving_sessions);
                }
                QueryMetric::ElapsedTimeS => values.elapsed_time_s = Some(self.elapsed_s),
                QueryMetric::ElevationM => values.elevation_m = Some(self.elevation_m),
                QueryMetric::AvgHr => {
                    values.avg_hr =
                        (self.hr_weight > 0.0).then(|| self.hr_weighted / self.hr_weight);
                    values.avg_hr_sessions = Some(self.hr_sessions);
                }
            }
        }
        values
    }

    const fn window_totals(&self) -> WindowTotals {
        WindowTotals {
            count: self.count,
            distance_km: self.distance_m / 1000.0,
            duration_seconds: self.elapsed_s,
            elevation_gain_m: self.elevation_m,
        }
    }
}

/// What one fold came to.
#[derive(Debug, Clone)]
pub struct Folded {
    /// The sums over every counted workout.
    pub totals: MetricValues,
    /// The same sums as a window's totals.
    pub window_totals: WindowTotals,
    /// The groups kept, by key, at most [`MAX_GROUPS`].
    pub groups: Vec<QueryGroup>,
    /// Every group there was, before the cut.
    pub groups_total: usize,
    /// The workouts a filter could not judge.
    pub unknown: UnknownCounts,
    /// The oldest and newest civil day a counted workout started on.
    pub span: Option<(CivilDate, CivilDate)>,
    /// Counted workouts that carry Garmin-sourced data, and those that do
    /// not.
    pub garmin: (usize, usize),
    /// The segment population's counts, when a segment was asked for.
    pub segments: Option<SegmentCounts>,
    /// One place each place group stands for, by id, for its label.
    pub places: HashMap<String, PlaceRef>,
    /// The name each gear group's gear carries, by its key.
    pub gear_names: HashMap<String, String>,
}

/// The running sums of one count.
#[derive(Debug)]
pub struct Accumulator {
    query: ActivityQuery,
    zone: Zone,
    totals: Cell,
    groups: HashMap<Vec<KeyValue>, Cell>,
    unknown: UnknownCounts,
    span: Option<(CivilDate, CivilDate)>,
    garmin: (usize, usize),
    segments: Option<SegmentCounts>,
    places: HashMap<String, PlaceRef>,
    gear_names: HashMap<String, String>,
}

impl Accumulator {
    /// An empty count of `query` on the athlete's calendar in `zone`.
    #[must_use]
    pub fn new(query: &ActivityQuery, zone: &Zone) -> Self {
        Self {
            query: query.clone(),
            zone: zone.clone(),
            totals: Cell::default(),
            groups: HashMap::new(),
            unknown: UnknownCounts::default(),
            span: None,
            garmin: (0, 0),
            segments: query
                .filter
                .has_segment
                .as_ref()
                .map(|_| SegmentCounts::default()),
            places: HashMap::new(),
            gear_names: HashMap::new(),
        }
    }

    /// Count `workout` when every filter matches it; a filter that cannot
    /// judge it counts it unknown instead. With a segment asked, a workout
    /// every other filter matches and a detail walk could read joins the
    /// segment population ([`SegmentCounts`]) before the segment filter runs.
    pub fn add(&mut self, workout: &Workout<'_>) {
        match self.query.filter.test_without_segment(workout) {
            Verdict::Match => {}
            Verdict::Miss => return,
            Verdict::Unknown(field) => {
                self.unknown.count(field);
                return;
            }
        }
        if let Some(segments) = self.segments.as_mut() {
            if walkable_segments(workout) {
                segments.total = segments.total.saturating_add(1);
                if segments_read(workout) {
                    segments.read = segments.read.saturating_add(1);
                } else if export_only(workout) {
                    segments.export_only = segments.export_only.saturating_add(1);
                }
            }
        }
        match self.query.filter.segment(workout) {
            Verdict::Match => {}
            Verdict::Miss => return,
            Verdict::Unknown(field) => {
                self.unknown.count(field);
                return;
            }
        }
        let key = self.key_of(workout);
        self.count(&workout.session, key, workout.carries_garmin_data());
    }

    /// Mark the segment population withheld in part from this reader.
    pub(crate) fn segments_withheld(&mut self) {
        if let Some(segments) = self.segments.as_mut() {
            segments.withheld = true;
        }
    }

    fn count(&mut self, session: &Activity, key: Vec<KeyValue>, garmin: bool) {
        self.totals.add(session);
        if !self.query.group_by.is_empty() {
            self.groups.entry(key).or_default().add(session);
        }
        let day = local_date(session.start_date(), &self.zone);
        self.span = Some(self.span.map_or((day, day), |(oldest, newest)| {
            (oldest.min(day), newest.max(day))
        }));
        if garmin {
            self.garmin.0 = self.garmin.0.saturating_add(1);
        } else {
            self.garmin.1 = self.garmin.1.saturating_add(1);
        }
    }

    fn key_of(&mut self, workout: &Workout<'_>) -> Vec<KeyValue> {
        let session = &workout.session;
        let day = local_date(session.start_date(), &self.zone);
        let group_by = self.query.group_by.clone();
        group_by
            .iter()
            .map(|key| match key {
                GroupKey::Year => KeyValue::Year(day.year()),
                GroupKey::Month => KeyValue::Text(day.format_with(&MONTH_KEY)),
                GroupKey::Week => KeyValue::Text(monday_of(day).to_string()),
                GroupKey::SportType => KeyValue::Text(sport_bucket(session)),
                GroupKey::SportFamily => {
                    KeyValue::Text(family_name(SportFamily::of(session.sport_type())))
                }
                GroupKey::Place => KeyValue::Text(workout.place().map_or_else(
                    || UNKNOWN_KEY.to_owned(),
                    |place| {
                        self.places
                            .entry(place.id.clone())
                            .or_insert_with(|| place.clone());
                        place.id.clone()
                    },
                )),
                GroupKey::Gear => KeyValue::Text(session.gear().map_or_else(
                    || UNKNOWN_KEY.to_owned(),
                    |gear| {
                        // A gear with no id is keyed by its name, as one
                        // display line: athlete text never travels raw.
                        let id = gear
                            .id
                            .clone()
                            .unwrap_or_else(|| display_line(&gear.name, MAX_GEAR_KEY_CHARS));
                        self.gear_names
                            .entry(id.clone())
                            .or_insert_with(|| gear.name.clone());
                        id
                    },
                )),
            })
            .collect()
    }

    /// The sums, the groups (the [`MAX_GROUPS`] largest by count, then in
    /// key order) and what could not be judged.
    #[must_use]
    pub fn finish(self) -> Folded {
        let metrics = self.query.metrics.clone();
        let groups_total = self.groups.len();
        let mut cells: Vec<(Vec<KeyValue>, Cell)> = self.groups.into_iter().collect();
        cells
            .sort_by(|(a_key, a), (b_key, b)| b.count.cmp(&a.count).then_with(|| a_key.cmp(b_key)));
        cells.truncate(MAX_GROUPS);
        cells.sort_by(|(a, _), (b, _)| a.cmp(b));
        let groups = cells
            .into_iter()
            .map(|(values, cell)| QueryGroup {
                key: self.query.group_by.iter().copied().zip(values).collect(),
                label: BTreeMap::new(),
                values: cell.values(&metrics),
            })
            .collect();
        Folded {
            totals: self.totals.values(&metrics),
            window_totals: self.totals.window_totals(),
            groups,
            groups_total,
            unknown: self.unknown,
            span: self.span,
            garmin: self.garmin,
            segments: self.segments,
            places: self.places,
            gear_names: self.gear_names,
        }
    }
}

/// A sport family's wire name.
fn family_name(family: SportFamily) -> String {
    match family {
        SportFamily::Running => "running",
        SportFamily::Cycling => "cycling",
        SportFamily::Swimming => "swimming",
        SportFamily::Other => "other",
    }
    .to_owned()
}

/// Sum `workouts` as `query` asks, on the athlete's calendar in `zone`: one
/// [`Accumulator`] over the slice.
#[must_use]
pub fn aggregate(
    workouts: &[Workout<'_>],
    query: &ActivityQuery,
    zone: &Zone,
) -> (MetricValues, Vec<QueryGroup>, UnknownCounts) {
    let mut accumulator = Accumulator::new(query, zone);
    for workout in workouts {
        accumulator.add(workout);
    }
    let folded = accumulator.finish();
    (folded.totals, folded.groups, folded.unknown)
}
