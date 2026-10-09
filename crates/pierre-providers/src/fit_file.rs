// ABOUTME: Activity FIT files decoded once for every reader: each session's summary and laps, and its per-sample series
// ABOUTME: Only what the file holds — a field the file leaves out stays absent, and position is read only when asked for
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Activity FIT files
//!
//! The FIT protocol (Garmin's Flexible and Interoperable Data Transfer) is
//! what nearly every watch and bike computer records a workout in. Two paths
//! read one here, through the same decoder:
//!
//! - Wahoo serves a completed workout's samples only as an activity FIT file
//!   on its CDN (`wahoo_streams`), and its API agreement bars storing the
//!   track, so that read asks for the series [`Position::Withheld`];
//! - an athlete uploads the `.fit` of a completed workout themselves
//!   (`upload_provider`), which becomes one activity per session in it, with
//!   the laps and the route the file recorded.
//!
//! The file says what it holds and nothing is made up around it: a session
//! without a distance stays without one, a sample without a heart rate keeps
//! a gap in that channel. The one conversion applied is cadence on foot: the
//! FIT profile records a run's cadence per leg (strides per minute) where the
//! activity model carries steps per minute, so a running, walking or hiking
//! cadence is doubled, its fractional half included.

use std::fmt;

use chrono::{DateTime, Utc};
use fitparser::profile::MesgNum;
use fitparser::{FitDataRecord, Value};

use crate::core::no_recorded_samples;
use crate::models::{Activity, ActivityBuilder, Lap, SportType, TimeSeriesData};
use crate::utils::conversions::{f64_to_f32, f64_to_u32, f64_to_u64};

/// Degrees in one FIT semicircle: positions are stored as `2^31` semicircles
/// per 180 degrees.
const DEGREES_PER_SEMICIRCLE: f64 = 180.0 / 2_147_483_648.0;

/// Why a file is not an activity this decoder can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FitFileError {
    /// The bytes are not a FIT file, or fail its header or data checksum.
    NotFit(String),
    /// A FIT file of another kind — a course, a workout, the device's
    /// settings — named by the type its `file_id` gives.
    NotAnActivity(String),
    /// An activity file holding no session: nothing was completed in it.
    NoSession,
}

impl fmt::Display for FitFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFit(reason) => write!(f, "not a FIT file: {reason}"),
            Self::NotAnActivity(kind) => write!(f, "a FIT {kind} file, not an activity"),
            Self::NoSession => f.write_str("the activity file holds no completed session"),
        }
    }
}

/// Whether a series carries the samples' positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// The track is read into `gps_coordinates`.
    Read,
    /// The track is never read out of the file.
    Withheld,
}

/// A numeric field value, or `None` for a missing, invalid or non-numeric one.
fn number(value: &Value) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    let number = match value {
        Value::Byte(v) | Value::Enum(v) | Value::UInt8(v) | Value::UInt8z(v) => f64::from(*v),
        Value::SInt8(v) => f64::from(*v),
        Value::SInt16(v) => f64::from(*v),
        Value::UInt16(v) | Value::UInt16z(v) => f64::from(*v),
        Value::SInt32(v) => f64::from(*v),
        Value::UInt32(v) | Value::UInt32z(v) => f64::from(*v),
        Value::SInt64(v) => *v as f64,
        Value::UInt64(v) | Value::UInt64z(v) => *v as f64,
        Value::Float32(v) => f64::from(*v),
        Value::Float64(v) => *v,
        _ => return None,
    };
    number.is_finite().then_some(number)
}

/// The named fields of one message, read by name.
struct Fields<'a>(&'a FitDataRecord);

impl Fields<'_> {
    fn value(&self, name: &str) -> Option<&Value> {
        self.0
            .fields()
            .iter()
            .find(|field| field.name() == name)
            .map(fitparser::FitDataField::value)
    }

    fn number(&self, name: &str) -> Option<f64> {
        self.value(name).and_then(number)
    }

    /// The enhanced field when present, else its 16-bit original.
    fn enhanced(&self, name: &str) -> Option<f64> {
        self.number(&format!("enhanced_{name}"))
            .or_else(|| self.number(name))
    }

    fn text(&self, name: &str) -> Option<&str> {
        match self.value(name) {
            Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim()),
            _ => None,
        }
    }

    fn instant(&self, name: &str) -> Option<DateTime<Utc>> {
        match self.value(name) {
            Some(Value::Timestamp(at)) => Some(at.with_timezone(&Utc)),
            _ => None,
        }
    }

    /// A position pair in degrees, from its semicircle fields.
    fn position(&self, lat: &str, long: &str) -> Option<(f64, f64)> {
        let latitude = self.number(lat)? * DEGREES_PER_SEMICIRCLE;
        let longitude = self.number(long)? * DEGREES_PER_SEMICIRCLE;
        ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude))
            .then_some((latitude, longitude))
    }

    /// A cadence, in steps per minute on foot: the FIT profile counts one
    /// leg, so the whole and fractional halves are added and doubled.
    fn cadence(&self, name: &str, fractional: &str, on_foot: bool) -> Option<f64> {
        let whole = self.number(name)?;
        if !on_foot {
            return Some(whole);
        }
        Some((whole + self.number(fractional).unwrap_or(0.0)) * 2.0)
    }
}

/// One record message's samples, by channel.
#[derive(Debug, Default, Clone)]
struct Sample {
    timestamp: Option<i64>,
    heart_rate: Option<f64>,
    power: Option<f64>,
    cadence: Option<f64>,
    speed: Option<f64>,
    altitude: Option<f64>,
    temperature: Option<f64>,
    distance: Option<f64>,
    position: Option<(f64, f64)>,
}

impl Sample {
    fn read(record: &FitDataRecord, on_foot: bool) -> Self {
        let fields = Fields(record);
        Self {
            timestamp: fields.instant("timestamp").map(|at| at.timestamp()),
            heart_rate: fields.number("heart_rate"),
            power: fields.number("power"),
            cadence: fields.cadence("cadence", "fractional_cadence", on_foot),
            speed: fields.enhanced("speed"),
            altitude: fields.enhanced("altitude"),
            temperature: fields.number("temperature"),
            distance: fields.number("distance"),
            position: fields.position("position_lat", "position_long"),
        }
    }
}

/// A channel kept only when at least one sample carries it.
fn channel<T>(values: Vec<Option<T>>) -> Option<Vec<Option<T>>> {
    values.iter().any(Option::is_some).then_some(values)
}

/// A non-negative whole-number reading (bpm, watts, rpm).
fn whole(value: Option<f64>) -> Option<u32> {
    value
        .filter(|v| *v >= 0.0 && *v <= f64::from(u32::MAX))
        .map(|v| f64_to_u32(v.round()))
}

/// A reading kept at f32 precision (m/s, metres, °C).
fn fractional(value: Option<f64>) -> Option<f32> {
    value.map(f64_to_f32)
}

/// The series of `samples`, timed from the first one.
///
/// Distance is cumulative, one value per sample: a sample without it has not
/// moved since the last one that had it. Positions are the samples' that
/// recorded one, in order; a sample recorded before the device had a fix
/// carries none.
fn series(samples: &[Sample], position: Position) -> TimeSeriesData {
    let Some(start) = samples.first().and_then(|sample| sample.timestamp) else {
        return no_recorded_samples();
    };
    let mut timestamps = Vec::with_capacity(samples.len());
    let mut distance = Vec::with_capacity(samples.len());
    let mut last_distance = 0.0;
    for sample in samples {
        let offset = sample.timestamp.unwrap_or(start).saturating_sub(start);
        timestamps.push(u32::try_from(offset).unwrap_or(u32::MAX));
        if let Some(meters) = sample.distance {
            last_distance = meters;
        }
        distance.push(last_distance);
    }
    let has_distance = samples.iter().any(|sample| sample.distance.is_some());
    let track: Vec<(f64, f64)> = match position {
        Position::Read => samples.iter().filter_map(|s| s.position).collect(),
        Position::Withheld => Vec::new(),
    };
    TimeSeriesData {
        timestamps,
        heart_rate: channel(samples.iter().map(|s| whole(s.heart_rate)).collect()),
        power: channel(samples.iter().map(|s| whole(s.power)).collect()),
        cadence: channel(samples.iter().map(|s| whole(s.cadence)).collect()),
        speed: channel(samples.iter().map(|s| fractional(s.speed)).collect()),
        altitude: channel(samples.iter().map(|s| fractional(s.altitude)).collect()),
        temperature: channel(samples.iter().map(|s| fractional(s.temperature)).collect()),
        gps_coordinates: (!track.is_empty()).then_some(track),
        distance: has_distance.then_some(distance),
    }
}

/// The sport a FIT `sport` / `sub_sport` pair names, and the detail string
/// (`"cycling/gravel_cycling"`) kept beside it.
fn sport_of(sport: Option<&str>, sub_sport: Option<&str>) -> SportType {
    match (sport.unwrap_or("generic"), sub_sport.unwrap_or("generic")) {
        ("running", "treadmill" | "indoor_running" | "virtual_activity") => SportType::VirtualRun,
        ("running", "trail") => SportType::TrailRunning,
        ("running", _) => SportType::Run,
        ("cycling", "indoor_cycling" | "virtual_activity" | "spin") => SportType::VirtualRide,
        ("cycling", "mountain" | "downhill") => SportType::MountainBike,
        ("cycling", "gravel_cycling" | "cyclocross") => SportType::GravelRide,
        ("cycling", "e_bike_fitness" | "e_bike_mountain") | ("e_biking", _) => SportType::EbikeRide,
        ("cycling", _) => SportType::Ride,
        ("swimming", _) => SportType::Swim,
        ("walking", _) => SportType::Walk,
        ("hiking", _) => SportType::Hike,
        ("training", "strength_training") => SportType::StrengthTraining,
        ("training", "yoga") => SportType::Yoga,
        ("training", "pilates") => SportType::Pilates,
        ("training" | "fitness_equipment", _) => SportType::Workout,
        ("cross_country_skiing", _) => SportType::CrossCountrySkiing,
        ("alpine_skiing", "backcountry") => SportType::BackcountrySkiing,
        ("alpine_skiing", _) => SportType::AlpineSkiing,
        ("snowboarding", _) => SportType::Snowboarding,
        ("snowshoeing", _) => SportType::Snowshoe,
        ("ice_skating", _) => SportType::IceSkating,
        ("rowing", _) => SportType::Rowing,
        ("kayaking", _) => SportType::Kayaking,
        ("paddling" | "stand_up_paddleboarding", _) => SportType::Paddleboarding,
        ("surfing", _) => SportType::Surfing,
        ("kitesurfing", _) => SportType::Kitesurfing,
        ("rock_climbing", _) => SportType::RockClimbing,
        ("soccer", _) => SportType::Soccer,
        ("basketball", _) => SportType::Basketball,
        ("tennis", _) => SportType::Tennis,
        ("golf", _) => SportType::Golf,
        ("inline_skating", _) => SportType::InlineSkating,
        (other, _) => SportType::Other(other.to_owned()),
    }
}

/// Whether a FIT sport is done on foot, where cadence counts one leg.
fn on_foot(sport: Option<&str>) -> bool {
    matches!(sport, Some("running" | "walking" | "hiking"))
}

/// One session's window and identity, read off its `session` message.
struct Session<'a> {
    fields: Fields<'a>,
    start: DateTime<Utc>,
    /// The session's last instant: its start plus its elapsed time, or its
    /// own `timestamp` when that is later.
    end: DateTime<Utc>,
    on_foot: bool,
}

impl<'a> Session<'a> {
    fn read(record: &'a FitDataRecord) -> Option<Self> {
        let fields = Fields(record);
        let start = fields.instant("start_time")?;
        let elapsed = fields.number("total_elapsed_time").unwrap_or(0.0);
        let by_elapsed = start
            + chrono::Duration::milliseconds(
                i64::try_from(f64_to_u64(elapsed * 1000.0)).unwrap_or(0),
            );
        let end = fields
            .instant("timestamp")
            .map_or(by_elapsed, |stamp| stamp.max(by_elapsed));
        let foot = on_foot(fields.text("sport"));
        Some(Self {
            fields,
            start,
            end,
            on_foot: foot,
        })
    }

    fn holds(&self, at: DateTime<Utc>) -> bool {
        self.start <= at && at <= self.end
    }
}

/// A decoded activity FIT file.
pub struct FitActivityFile {
    records: Vec<FitDataRecord>,
}

impl FitActivityFile {
    /// Decode `bytes`, checking it is an activity file with at least one
    /// session.
    ///
    /// # Errors
    ///
    /// Returns [`FitFileError`] when the bytes are not a FIT file, the file is
    /// not an activity, or it holds no session.
    pub fn parse(bytes: &[u8]) -> Result<Self, FitFileError> {
        let file = Self::decode(bytes)?;
        if let Some(kind) = file
            .of_kind(MesgNum::FileId)
            .next()
            .and_then(|record| Fields(record).text("type").map(str::to_owned))
        {
            if kind != "activity" {
                return Err(FitFileError::NotAnActivity(kind));
            }
        }
        if file.sessions().next().is_none() {
            return Err(FitFileError::NoSession);
        }
        Ok(file)
    }

    /// Decode `bytes` as they are, whatever kind of FIT file they hold.
    ///
    /// # Errors
    ///
    /// Returns [`FitFileError::NotFit`] when the bytes are not a FIT file or
    /// fail its checksum.
    pub fn decode(bytes: &[u8]) -> Result<Self, FitFileError> {
        fitparser::from_bytes(bytes)
            .map(|records| Self { records })
            .map_err(|e| FitFileError::NotFit(e.to_string()))
    }

    fn of_kind(&self, kind: MesgNum) -> impl Iterator<Item = &FitDataRecord> {
        self.records
            .iter()
            .filter(move |record| record.kind() == kind)
    }

    fn sessions(&self) -> impl Iterator<Item = Session<'_>> {
        self.of_kind(MesgNum::Session).filter_map(Session::read)
    }

    /// How many sessions the file holds: one per sport of a multisport file.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions().count()
    }

    /// The device that recorded the file, as its `file_id` names it: the
    /// product name when the file carries one, else the manufacturer and the
    /// model the profile knows it by.
    #[must_use]
    pub fn device_name(&self) -> Option<String> {
        let file_id = Fields(self.of_kind(MesgNum::FileId).next()?);
        if let Some(name) = file_id.text("product_name") {
            return Some(name.to_owned());
        }
        let manufacturer = file_id.text("manufacturer")?;
        let product = file_id
            .text("garmin_product")
            .or_else(|| file_id.text("product"));
        Some(product.map_or_else(
            || manufacturer.to_owned(),
            |product| format!("{manufacturer} {product}"),
        ))
    }

    /// Every record message, timed and in file order.
    fn samples(&self, on_foot: bool) -> Vec<Sample> {
        self.of_kind(MesgNum::Record)
            .map(|record| Sample::read(record, on_foot))
            .filter(|sample| sample.timestamp.is_some())
            .collect()
    }

    /// The whole file's series, every record message in it, timed from the
    /// first one.
    #[must_use]
    pub fn time_series(&self, position: Position) -> TimeSeriesData {
        let on_foot = self.sessions().next().is_some_and(|s| s.on_foot);
        series(&self.samples(on_foot), position)
    }

    /// The series of the session at `index`: the record messages timed
    /// inside its window. `None` when the file has no such session.
    #[must_use]
    pub fn session_series(&self, index: usize, position: Position) -> Option<TimeSeriesData> {
        let session = self.sessions().nth(index)?;
        let samples: Vec<Sample> = self
            .samples(session.on_foot)
            .into_iter()
            .filter(|sample| {
                sample
                    .timestamp
                    .and_then(|at| DateTime::<Utc>::from_timestamp(at, 0))
                    .is_some_and(|at| session.holds(at))
            })
            .collect();
        Some(series(&samples, position))
    }

    /// One activity per session, in file order, each carrying its laps and
    /// the position it started at. `id_of` names the activity of each session
    /// index. The series is not attached: it is read on demand
    /// ([`Self::session_series`]).
    #[must_use]
    pub fn activities(&self, id_of: impl Fn(usize) -> String, provider: &str) -> Vec<Activity> {
        self.sessions()
            .enumerate()
            .map(|(index, session)| {
                self.session_activity(&session, id_of(index), provider)
                    .build()
            })
            .collect()
    }

    /// The activity of the session at `index`, named `id`, with its series
    /// attached — what a detail read with streams answers. `None` when the
    /// file has no such session.
    #[must_use]
    pub fn activity_with_series(
        &self,
        index: usize,
        id: String,
        provider: &str,
        position: Position,
    ) -> Option<Activity> {
        let session = self.sessions().nth(index)?;
        let series = self.session_series(index, position);
        Some(
            self.session_activity(&session, id, provider)
                .time_series_data_opt(series)
                .build(),
        )
    }

    /// A session's summary, laps and recording device.
    fn session_activity(
        &self,
        session: &Session<'_>,
        id: String,
        provider: &str,
    ) -> ActivityBuilder {
        let laps = self.laps_of(session);
        summary(session, id, provider)
            .laps_opt((!laps.is_empty()).then_some(laps))
            .device_name_opt(self.device_name())
    }

    /// The laps whose start falls inside `session`, numbered from one.
    fn laps_of(&self, session: &Session<'_>) -> Vec<Lap> {
        self.of_kind(MesgNum::Lap)
            .map(Fields)
            .filter(|lap| {
                lap.instant("start_time")
                    .is_some_and(|at| session.holds(at))
            })
            .enumerate()
            .map(|(index, lap)| Lap {
                id: None,
                index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                distance_meters: lap.number("total_distance").unwrap_or(0.0),
                elapsed_time_seconds: f64_to_u64(
                    lap.number("total_elapsed_time").unwrap_or(0.0).round(),
                ),
                moving_time_seconds: lap
                    .number("total_timer_time")
                    .map(|t| f64_to_u64(t.round())),
                elevation_gain_meters: lap.number("total_ascent"),
                average_speed_mps: lap.enhanced("avg_speed"),
                max_speed_mps: lap.enhanced("max_speed"),
                average_heart_rate: whole(lap.number("avg_heart_rate")),
                max_heart_rate: whole(lap.number("max_heart_rate")),
                average_cadence: whole(lap.cadence(
                    "avg_cadence",
                    "avg_fractional_cadence",
                    session.on_foot,
                )),
                average_power: whole(lap.number("avg_power")),
            })
            .collect()
    }
}

/// The activity a session's own summary fields describe.
fn summary(session: &Session<'_>, id: String, provider: &str) -> ActivityBuilder {
    let fields = &session.fields;
    let sport_name = fields.text("sport");
    let sub_sport = fields.text("sub_sport");
    let detail = sport_name.map(|sport| match sub_sport {
        Some(sub) if sub != "generic" => format!("{sport}/{sub}"),
        _ => sport.to_owned(),
    });
    let elapsed = f64_to_u64(fields.number("total_elapsed_time").unwrap_or(0.0).round());
    let start = fields.position("start_position_lat", "start_position_long");
    ActivityBuilder::new(
        id,
        String::new(),
        sport_of(sport_name, sub_sport),
        session.start,
        elapsed,
        provider,
    )
    .sport_type_detail_opt(detail)
    .distance_meters_opt(fields.number("total_distance"))
    .elevation_gain_opt(fields.number("total_ascent"))
    .average_heart_rate_opt(whole(fields.number("avg_heart_rate")))
    .max_heart_rate_opt(whole(fields.number("max_heart_rate")))
    .average_speed_opt(fields.enhanced("avg_speed"))
    .max_speed_opt(fields.enhanced("max_speed"))
    .calories_opt(whole(fields.number("total_calories")))
    .average_power_opt(whole(fields.number("avg_power")))
    .max_power_opt(whole(fields.number("max_power")))
    .normalized_power_opt(whole(fields.number("normalized_power")))
    .ftp_opt(whole(fields.number("threshold_power")))
    .average_cadence_opt(whole(fields.cadence(
        "avg_cadence",
        "avg_fractional_cadence",
        session.on_foot,
    )))
    .max_cadence_opt(whole(fields.cadence(
        "max_cadence",
        "max_fractional_cadence",
        session.on_foot,
    )))
    .temperature_opt(fractional(fields.number("avg_temperature")))
    .training_stress_score_opt(fractional(fields.number("training_stress_score")))
    .intensity_factor_opt(fractional(fields.number("intensity_factor")))
    .start_latitude_opt(start.map(|(lat, _)| lat))
    .start_longitude_opt(start.map(|(_, long)| long))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-minute ride a Garmin fenix 5 recorded on 2017-06-12: GPS, wrist
    /// heart rate, one lap. Provenance: the `fitparser` 0.11.0 crate's test
    /// fixture `tests/fixtures/garmin-fenix-5-bike.fit` (MIT), copied
    /// byte for byte, so the decoder is checked against a device's own file
    /// rather than one written to match it.
    const FENIX_RIDE: &[u8] = include_bytes!("../tests/fixtures/garmin-fenix-5-bike.fit");

    fn ride() -> FitActivityFile {
        FitActivityFile::parse(FENIX_RIDE).unwrap_or_else(|e| panic!("fixture decodes: {e}"))
    }

    #[test]
    fn a_device_file_becomes_one_activity_with_its_session_summary() {
        let activities = ride().activities(|i| format!("ride-{i}"), "upload");
        assert_eq!(activities.len(), 1);
        let activity = &activities[0];
        assert_eq!(activity.id(), "ride-0");
        assert_eq!(activity.provider(), "upload");
        assert_eq!(*activity.sport_type(), SportType::Ride);
        assert_eq!(activity.sport_type_detail(), Some("cycling"));
        assert_eq!(
            activity.start_date().to_rfc3339(),
            "2017-06-12T16:09:22+00:00"
        );
        assert_eq!(activity.duration_seconds(), 60);
        assert_eq!(activity.distance_meters(), Some(459.52));
        assert_eq!(activity.elevation_gain(), Some(0.0));
        assert_eq!(activity.average_heart_rate(), Some(101));
        assert_eq!(activity.max_heart_rate(), Some(114));
        assert_eq!(activity.calories(), Some(7));
        assert_eq!(activity.average_speed(), Some(7.613));
        assert_eq!(activity.max_speed(), Some(8.65));
        assert_eq!(activity.device_name(), Some("garmin fenix5"));
        let (lat, long) = (
            activity.start_latitude().unwrap_or_default(),
            activity.start_longitude().unwrap_or_default(),
        );
        assert!((lat - 37.410_7).abs() < 0.001, "latitude {lat}");
        assert!((long + 122.065).abs() < 0.01, "longitude {long}");
    }

    #[test]
    fn fields_the_file_does_not_hold_stay_absent() {
        let activity = &ride().activities(|i| i.to_string(), "upload")[0];
        assert_eq!(activity.name(), "", "a FIT session carries no title");
        assert_eq!(activity.average_power(), None, "no power meter");
        assert_eq!(activity.normalized_power(), None);
        assert_eq!(activity.training_stress_score(), None);
        assert_eq!(activity.average_cadence(), None, "no cadence sensor");
        assert!(activity.time_series_data().is_none(), "read on demand");
    }

    #[test]
    fn laps_come_from_the_lap_messages_inside_the_session() {
        let activity = &ride().activities(|i| i.to_string(), "upload")[0];
        let laps = activity
            .laps()
            .unwrap_or_else(|| panic!("the ride has a lap"));
        assert_eq!(laps.len(), 1);
        assert_eq!(laps[0].index, 1);
        assert!((laps[0].distance_meters - 459.52).abs() < 1e-9);
        assert_eq!(laps[0].elapsed_time_seconds, 60);
        assert_eq!(laps[0].average_heart_rate, Some(101));
        assert_eq!(laps[0].max_speed_mps, Some(8.65));
    }

    #[test]
    fn the_session_series_reads_every_channel_the_records_carry() {
        let series = ride()
            .session_series(0, Position::Read)
            .unwrap_or_else(|| panic!("session 0"));
        // Smart recording: the watch wrote 19 samples over the minute.
        assert_eq!(series.timestamps.len(), 19);
        assert_eq!(series.timestamps[0], 0);
        assert_eq!(series.timestamps[1], 1);
        assert!(series.timestamps.iter().all(|offset| *offset <= 61));
        let heart_rate = series.heart_rate.unwrap_or_default();
        assert_eq!(heart_rate[0], Some(77));
        assert_eq!(heart_rate[1], Some(83));
        let distance = series.distance.unwrap_or_default();
        assert!((distance[1] - 16.15).abs() < 1e-9);
        assert!(series.temperature.is_some());
        assert_eq!(series.power, None);
        let track = series.gps_coordinates.unwrap_or_default();
        assert_eq!(track.len(), series.timestamps.len());
        assert!((track[0].0 - 37.410_7).abs() < 0.001);
    }

    #[test]
    fn a_detail_read_carries_the_series_beside_the_summary() {
        let activity = ride()
            .activity_with_series(0, "ride-0".to_owned(), "upload", Position::Read)
            .unwrap_or_else(|| panic!("session 0"));
        assert_eq!(activity.distance_meters(), Some(459.52));
        assert_eq!(activity.laps().map(Vec::len), Some(1));
        let series = activity
            .time_series_data()
            .unwrap_or_else(|| panic!("the detail read carries the series"));
        assert!(series.gps_coordinates.is_some());
        assert!(ride()
            .activity_with_series(1, "ride-1".to_owned(), "upload", Position::Read)
            .is_none());
    }

    #[test]
    fn a_withheld_position_never_reaches_the_series() {
        let file = ride();
        assert_eq!(file.time_series(Position::Withheld).gps_coordinates, None);
        assert_eq!(
            file.session_series(0, Position::Withheld)
                .and_then(|series| series.gps_coordinates),
            None
        );
    }

    #[test]
    fn a_session_index_past_the_last_reads_nothing() {
        assert!(ride().session_series(1, Position::Read).is_none());
        assert_eq!(ride().session_count(), 1);
    }

    #[test]
    fn bytes_that_are_not_a_fit_file_are_refused() {
        assert!(matches!(
            FitActivityFile::parse(b"not a fit file"),
            Err(FitFileError::NotFit(_))
        ));
        let mut corrupted = FENIX_RIDE.to_vec();
        let middle = corrupted.len() / 2;
        corrupted[middle] ^= 0xFF;
        assert!(matches!(
            FitActivityFile::parse(&corrupted),
            Err(FitFileError::NotFit(_))
        ));
    }

    #[test]
    fn fit_sports_map_to_the_activity_model() {
        assert_eq!(
            sport_of(Some("running"), Some("trail")),
            SportType::TrailRunning
        );
        assert_eq!(
            sport_of(Some("running"), Some("treadmill")),
            SportType::VirtualRun
        );
        assert_eq!(
            sport_of(Some("cycling"), Some("indoor_cycling")),
            SportType::VirtualRide
        );
        assert_eq!(
            sport_of(Some("cycling"), Some("gravel_cycling")),
            SportType::GravelRide
        );
        assert_eq!(sport_of(Some("swimming"), None), SportType::Swim);
        assert_eq!(
            sport_of(Some("training"), Some("strength_training")),
            SportType::StrengthTraining
        );
        assert_eq!(
            sport_of(Some("floor_climbing"), None),
            SportType::Other("floor_climbing".to_owned())
        );
    }
}
