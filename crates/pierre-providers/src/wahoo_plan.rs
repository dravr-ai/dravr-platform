// ABOUTME: Wahoo plan rendering — a PlannedSession into plan.json and the plan/workout form fields the Cloud API takes
// ABOUTME: Neutral by construction: titles and interval names come from structure, ids are opaque hashes, no prose leaves Dravr
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Wahoo plan rendering
//!
//! Wahoo keeps a planned workout as two records: a *plan* in the athlete's
//! library, whose file (`plan.json`, schema 1.0.0) holds the intervals and
//! targets, and a *workout* scheduled on a day that points at it. The
//! ELEMNT app and the ELEMNT and RIVAL devices show a plan only through a
//! workout scheduled from today through six days out.
//!
//! Everything sent into the Wahoo Platform becomes data Wahoo "may use … for
//! any business purpose" (Wahoo API Agreement, uploading clause), and personal
//! data sent there carries consent and privacy-policy duties. So everything
//! rendered here is **neutral by construction** (carnet#34, Directive from
//! Phil 2026-10-07):
//!
//! - the title and every interval name are generated from the session's
//!   structure — sport, duration, the main set — never from the session's
//!   own title, its notes, a step's label or a step's note, which carry the
//!   coach's or the agent's words;
//! - the plan's `external_id` and the workout's `workout_token` are an
//!   opaque hash of Dravr's calendar key, which names the athlete's user id;
//! - the only athlete number in the file is the FTP that relative power
//!   targets are a fraction of, and it is the FTP Wahoo itself holds for the
//!   athlete (their Wahoo power zones). A target that would need any other
//!   threshold (heart rate, pace) goes out untargeted rather than hand Wahoo
//!   a physiological value it does not have.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use chrono::{DateTime, NaiveDate, Utc};
use dravr_cageux::physiological_constants::zone_percentages::{
    POWER_ZONE1_UPPER_LIMIT, POWER_ZONE2_UPPER_LIMIT, POWER_ZONE3_UPPER_LIMIT,
    POWER_ZONE4_UPPER_LIMIT,
};
use ring::digest::{digest, SHA256};
use serde_json::{json, Value};

use crate::models::periodization::ThresholdBasis;
use crate::models::{PlannedSession, RelativeIntensity, SportType, WorkoutStep};

/// Prefix of every id Dravr writes into Wahoo, so a listing tells Dravr's
/// scheduled workouts from the athlete's own.
pub const OPAQUE_ID_PREFIX: &str = "dravr-";

/// Hex characters of the SHA-256 digest an opaque id keeps (128 bits).
const OPAQUE_ID_HEX_CHARS: usize = 32;

/// The plan file's name. A constant, so the filename carries nothing either.
pub const PLAN_FILENAME: &str = "plan.json";

/// The `plan.json` schema version the file is written against.
const PLAN_JSON_VERSION: &str = "1.0.0";

/// Hour of the day (UTC) a session is scheduled at. Dravr plans a civil date,
/// and Wahoo takes a start time: noon UTC lands on the same civil date from
/// UTC−12 through UTC+11.
const SCHEDULED_HOUR_UTC: &str = "12:00:00.000Z";

/// Coggan's zone 5 (`VO2max`) upper bound, as a fraction of FTP.
///
/// Zones 1–4 come from the shipped `zone_percentages`; the step grammar names
/// seven zones (`anaerobic` is 6, `neuromuscular` 7), so the upper three
/// follow the same model. Coggan, A. & Allen, H. (2010). *Training and Racing
/// with a Power Meter* (2nd ed.). `VeloPress`.
const POWER_ZONE5_UPPER_LIMIT: f64 = 1.20;

/// Coggan's zone 6 (anaerobic capacity) upper bound, as a fraction of FTP.
const POWER_ZONE6_UPPER_LIMIT: f64 = 1.50;

/// The ceiling of zone 7 (neuromuscular), which Coggan leaves open: the step
/// grammar's own highest percentage (300 %).
const POWER_ZONE7_UPPER_LIMIT: f64 = 3.00;

/// Sweet spot, as the step grammar defines it: 88–94 % FTP.
const SWEET_SPOT: (f64, f64) = (0.88, 0.94);

/// The opaque id Dravr writes into Wahoo for a calendar entry.
///
/// A SHA-256 of Dravr's own key, which names the athlete's user id and the
/// date. Stable, so a re-push finds the same plan, and one-way, so Wahoo
/// never learns the key.
#[must_use]
pub fn opaque_id(external_id: &str) -> String {
    let hash = hex::encode(digest(&SHA256, external_id.as_bytes()));
    format!(
        "{OPAQUE_ID_PREFIX}{}",
        hash.get(..OPAQUE_ID_HEX_CHARS).unwrap_or(&hash)
    )
}

/// Whether a workout token is one Dravr wrote.
#[must_use]
pub fn is_dravr_token(token: &str) -> bool {
    token.starts_with(OPAQUE_ID_PREFIX)
}

/// Wahoo's own `UNKNOWN` workout type (Wahoo Cloud API, "Workout Types":
/// id 255), what a workout that names no type is read as.
pub const WORKOUT_TYPE_UNKNOWN: u16 = 255;

/// Wahoo's `workout_type_id` for a sport (Wahoo Cloud API, "Workout Types").
#[must_use]
pub const fn workout_type_id(sport: &SportType) -> u16 {
    match sport {
        SportType::Ride | SportType::GravelRide => 0,
        SportType::Run => 1,
        SportType::TrailRunning => 4,
        SportType::VirtualRun => 5,
        SportType::Walk => 6,
        SportType::Hike | SportType::Snowshoe => 9,
        SportType::MountainBike => 13,
        SportType::Swim => 25,
        SportType::Snowboarding => 27,
        SportType::AlpineSkiing => 29,
        SportType::CrossCountrySkiing | SportType::BackcountrySkiing => 30,
        SportType::IceSkating => 32,
        SportType::InlineSkating => 33,
        SportType::Canoeing => 37,
        SportType::Kayaking => 38,
        SportType::Rowing => 39,
        SportType::Kitesurfing => 40,
        SportType::Paddleboarding => 41,
        SportType::Workout
        | SportType::StrengthTraining
        | SportType::Crossfit
        | SportType::Pilates => 42,
        SportType::Golf => 46,
        SportType::VirtualRide => 61,
        SportType::EbikeRide => 64,
        SportType::Yoga => 66,
        _ => 47,
    }
}

/// The sport a Wahoo `workout_type_id` names, for a completed workout.
#[must_use]
pub fn sport_for_workout_type(type_id: u16) -> SportType {
    match type_id {
        0 | 14 | 15 | 16 | 70 => SportType::Ride,
        11 => SportType::GravelRide,
        12 | 21 | 49 | 61 | 68 => SportType::VirtualRide,
        13 => SportType::MountainBike,
        64 => SportType::EbikeRide,
        1 | 3 | 67 => SportType::Run,
        4 => SportType::TrailRunning,
        5 | 19 | 71 => SportType::VirtualRun,
        6..=8 | 56 => SportType::Walk,
        9 | 10 => SportType::Hike,
        25 | 26 => SportType::Swim,
        27 => SportType::Snowboarding,
        28 | 29 => SportType::AlpineSkiing,
        30 => SportType::CrossCountrySkiing,
        31 | 32 => SportType::IceSkating,
        33 => SportType::InlineSkating,
        34 => SportType::Skateboarding,
        37 => SportType::Canoeing,
        38 => SportType::Kayaking,
        22 | 39 => SportType::Rowing,
        40 => SportType::Kitesurfing,
        41 => SportType::Paddleboarding,
        2 | 18 | 20 | 23 | 42..=44 => SportType::Workout,
        46 => SportType::Golf,
        66 => SportType::Yoga,
        other => SportType::Other(format!("wahoo_workout_type_{other}")),
    }
}

/// The plan file's `workout_type_family` and `workout_type_location` for a
/// sport, or `None` for a sport `plan.json` cannot carry: its family enum
/// names biking (0) and running (1) only. A session of any other sport is
/// scheduled as a workout with no plan.
const fn plan_family(sport: &SportType) -> Option<(u8, u8)> {
    const BIKING: u8 = 0;
    const RUNNING: u8 = 1;
    const INDOOR: u8 = 0;
    const OUTDOOR: u8 = 1;
    match sport {
        SportType::Ride
        | SportType::GravelRide
        | SportType::MountainBike
        | SportType::EbikeRide => Some((BIKING, OUTDOOR)),
        SportType::VirtualRide => Some((BIKING, INDOOR)),
        SportType::Run | SportType::TrailRunning => Some((RUNNING, OUTDOOR)),
        SportType::VirtualRun => Some((RUNNING, INDOOR)),
        _ => None,
    }
}

/// Whether a sport's targets are power, so a zone is a fraction of FTP.
const fn is_power_sport(sport: &SportType) -> bool {
    matches!(
        sport,
        SportType::Ride
            | SportType::VirtualRide
            | SportType::EbikeRide
            | SportType::MountainBike
            | SportType::GravelRide
    )
}

/// The sport as a title starts with it.
fn sport_label(sport: &SportType) -> &'static str {
    match sport {
        SportType::Ride => "Ride",
        SportType::VirtualRide => "Indoor ride",
        SportType::MountainBike => "MTB ride",
        SportType::GravelRide => "Gravel ride",
        SportType::EbikeRide => "E-bike ride",
        SportType::Run => "Run",
        SportType::VirtualRun => "Treadmill run",
        SportType::TrailRunning => "Trail run",
        SportType::Swim => "Swim",
        SportType::Walk => "Walk",
        SportType::Hike => "Hike",
        SportType::Rowing => "Row",
        SportType::StrengthTraining | SportType::Crossfit => "Strength",
        SportType::Yoga => "Yoga",
        SportType::CrossCountrySkiing | SportType::BackcountrySkiing => "Ski",
        _ => "Workout",
    }
}

/// A duration as a title shows it: `45 min`, `2 h`, `1 h 30`.
fn format_duration(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m:02}"),
    }
}

/// A step's extent as a title shows it: `8 min`, `30 s`, `400 m`, `2 km`.
fn format_extent(step: &WorkoutStep) -> String {
    if let Some(distance) = step.distance_meters {
        if distance >= 1000.0 {
            let km = format!("{:.1}", distance / 1000.0);
            return format!("{} km", km.trim_end_matches('0').trim_end_matches('.'));
        }
        return format!("{} m", distance.round());
    }
    if step.duration_seconds < 60 {
        return format!("{} s", step.duration_seconds);
    }
    let minutes = step.duration_seconds / 60;
    match step.duration_seconds % 60 {
        0 => format!("{minutes} min"),
        s => format!("{minutes} min {s:02}"),
    }
}

/// The session's planned length: its steps when it has them, else its
/// stated duration.
fn session_seconds(session: &PlannedSession) -> Option<u64> {
    let steps = WorkoutStep::total_seconds(&session.steps);
    if steps > 0 {
        return Some(steps);
    }
    session
        .duration_seconds
        .filter(|seconds| *seconds > 0)
        .map(u64::from)
}

/// The session's main set as a title shows it (`4×8 min`): the first
/// repeated step's count and extent.
fn main_set(session: &PlannedSession) -> Option<String> {
    session
        .steps
        .iter()
        .find(|step| step.repeat > 1)
        .map(|step| format!("{}×{}", step.repeat, format_extent(step)))
}

/// The title Wahoo shows for a session, generated from structure alone.
///
/// Sport, duration and main set (`Ride 1 h 30 · 4×8 min`). The session's own
/// title is never used: it is the first clause of the agent's prescription
/// and can carry a reason.
#[must_use]
pub fn neutral_title(session: &PlannedSession) -> String {
    let mut title = sport_label(&session.sport).to_owned();
    if let Some(seconds) = session_seconds(session) {
        title.push(' ');
        title.push_str(&format_duration(seconds));
    }
    if let Some(set) = main_set(session) {
        title.push_str(" · ");
        title.push_str(&set);
    }
    title
}

/// `plan.json`'s intensity label for a step: the closed `INTENSITY_TYPE`
/// enum, so nothing of the step's own words reaches the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntensityType {
    Warmup,
    Cooldown,
    Recover,
    Rest,
    Active,
    Tempo,
    Threshold,
    Vo2Max,
    Anaerobic,
    Neuromuscular,
}

impl IntensityType {
    /// The enum value `plan.json` takes.
    const fn wire(self) -> &'static str {
        match self {
            Self::Warmup => "wu",
            Self::Cooldown => "cd",
            Self::Recover => "recover",
            Self::Rest => "rest",
            Self::Active => "active",
            Self::Tempo => "tempo",
            Self::Threshold => "lt",
            Self::Vo2Max => "map",
            Self::Anaerobic => "ac",
            Self::Neuromuscular => "nm",
        }
    }

    /// The interval name the device shows: the label of the class itself.
    const fn name(self) -> &'static str {
        match self {
            Self::Warmup => "Warm up",
            Self::Cooldown => "Cool down",
            Self::Recover => "Recovery",
            Self::Rest => "Rest",
            Self::Active => "Steady",
            Self::Tempo => "Tempo",
            Self::Threshold => "Threshold",
            Self::Vo2Max => "VO2max",
            Self::Anaerobic => "Anaerobic",
            Self::Neuromuscular => "Sprint",
        }
    }

    /// The class of a training zone 1–7.
    const fn of_zone(zone: u8) -> Self {
        match zone {
            0 | 1 => Self::Recover,
            2 => Self::Active,
            3 => Self::Tempo,
            4 => Self::Threshold,
            5 => Self::Vo2Max,
            6 => Self::Anaerobic,
            _ => Self::Neuromuscular,
        }
    }

    /// The class of a percent-of-threshold band, by its midpoint against the
    /// power zones (the zone bounds are the same fractions on any threshold
    /// as a label, never as a target).
    fn of_percent(low: u16, high: u16) -> Self {
        let mid = f64::from(low + high) / 200.0;
        let zone = [
            POWER_ZONE1_UPPER_LIMIT,
            POWER_ZONE2_UPPER_LIMIT,
            POWER_ZONE3_UPPER_LIMIT,
            POWER_ZONE4_UPPER_LIMIT,
            POWER_ZONE5_UPPER_LIMIT,
            POWER_ZONE6_UPPER_LIMIT,
        ]
        .iter()
        .position(|upper| mid < *upper)
        .map_or(7, |index| index + 1);
        Self::of_zone(u8::try_from(zone).unwrap_or(7))
    }

    /// The class of a perceived-exertion band, by its upper bound.
    const fn of_rpe(max: u8) -> Self {
        match max {
            0..=3 => Self::Recover,
            4 | 5 => Self::Active,
            6 => Self::Tempo,
            7 | 8 => Self::Threshold,
            9 => Self::Vo2Max,
            _ => Self::Anaerobic,
        }
    }

    /// A step's class. A warm-up, cool-down, recovery or rest step is read
    /// off its label against a closed vocabulary — only the class leaves,
    /// never the label — and any other step off its target.
    fn of_step(step: &WorkoutStep, intensity: Option<&RelativeIntensity>) -> Self {
        let label = step.label.to_lowercase();
        if label.contains("warm") || label.contains("échauff") {
            return Self::Warmup;
        }
        if label.contains("cool") || label.contains("retour au calme") {
            return Self::Cooldown;
        }
        if label.contains("rest") || label.contains("repos") {
            return Self::Rest;
        }
        if label.contains("recover") || label.contains("récup") {
            return Self::Recover;
        }
        match intensity {
            Some(RelativeIntensity::Zone(zone) | RelativeIntensity::HeartRateZone(zone)) => {
                Self::of_zone(*zone)
            }
            Some(RelativeIntensity::SweetSpot) => Self::Tempo,
            Some(RelativeIntensity::Percent { low, high, .. }) => Self::of_percent(*low, *high),
            Some(RelativeIntensity::Rpe(range)) => Self::of_rpe(range.max),
            None => Self::Active,
        }
    }
}

/// The FTP band of a power zone 1–7, as fractions of FTP.
fn zone_ftp_band(zone: u8) -> (f64, f64) {
    match zone {
        0 | 1 => (0.0, POWER_ZONE1_UPPER_LIMIT),
        2 => (POWER_ZONE1_UPPER_LIMIT, POWER_ZONE2_UPPER_LIMIT),
        3 => (POWER_ZONE2_UPPER_LIMIT, POWER_ZONE3_UPPER_LIMIT),
        4 => (POWER_ZONE3_UPPER_LIMIT, POWER_ZONE4_UPPER_LIMIT),
        5 => (POWER_ZONE4_UPPER_LIMIT, POWER_ZONE5_UPPER_LIMIT),
        6 => (POWER_ZONE5_UPPER_LIMIT, POWER_ZONE6_UPPER_LIMIT),
        _ => (POWER_ZONE6_UPPER_LIMIT, POWER_ZONE7_UPPER_LIMIT),
    }
}

/// Round a fraction to the two decimals `plan.json`'s examples use.
fn two_decimals(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// A step's `plan.json` target, or `None` when Wahoo cannot state it without
/// a threshold it does not hold.
///
/// Power targets on a cycling session become fractions of FTP when the
/// athlete's Wahoo FTP is known (`ftp_known`); a perceived-exertion band
/// becomes Wahoo's absolute `rpe` target on any sport. Heart-rate and pace
/// targets would need the athlete's threshold heart rate or pace in the
/// file — a physiological value Wahoo does not hold — so those steps go out
/// untargeted, carrying only their intensity class.
fn step_target(
    intensity: Option<&RelativeIntensity>,
    sport: &SportType,
    ftp_known: bool,
) -> Option<Value> {
    let ftp_band = |low: f64, high: f64| json!({ "type": "ftp", "low": two_decimals(low), "high": two_decimals(high) });
    let power = is_power_sport(sport) && ftp_known;
    match intensity? {
        RelativeIntensity::Zone(zone) if power => {
            let (low, high) = zone_ftp_band(*zone);
            Some(ftp_band(low, high))
        }
        RelativeIntensity::SweetSpot if power => Some(ftp_band(SWEET_SPOT.0, SWEET_SPOT.1)),
        RelativeIntensity::Percent {
            low,
            high,
            threshold,
        } if ftp_known
            && match threshold {
                Some(ThresholdBasis::Ftp) => true,
                None => is_power_sport(sport),
                Some(_) => false,
            } =>
        {
            Some(ftp_band(f64::from(*low) / 100.0, f64::from(*high) / 100.0))
        }
        RelativeIntensity::Rpe(range) => {
            Some(json!({ "type": "rpe", "low": range.min, "high": range.max }))
        }
        _ => None,
    }
}

/// One `plan.json` interval for a step, or `None` for a step with no extent.
fn step_interval(step: &WorkoutStep, sport: &SportType, ftp_known: bool) -> Option<Value> {
    let (trigger, value) = match step.distance_meters {
        Some(distance) if distance > 0.0 => ("distance", distance.round()),
        _ if step.duration_seconds > 0 => ("time", f64::from(step.duration_seconds)),
        _ => return None,
    };
    let intensity = RelativeIntensity::parse(&step.target_zone);
    let class = IntensityType::of_step(step, intensity.as_ref());
    let mut interval = json!({
        "name": class.name(),
        "exit_trigger_type": trigger,
        "exit_trigger_value": value,
        "intensity_type": class.wire(),
    });
    if let Some(target) = step_target(intensity.as_ref(), sport, ftp_known) {
        interval["targets"] = json!([target]);
    }
    Some(interval)
}

/// A repeated set being gathered: its `(repeat, repeat_group)` key and the
/// intervals of its steps so far.
type OpenSet = Option<((u32, Option<u32>), Vec<Value>)>;

/// The `plan.json` intervals of a session's steps. Consecutive steps sharing
/// a `repeat` above one and the same `repeat_group` become one `repeat`
/// interval — `plan.json` counts repeats *after* the first, so four times
/// over is an `exit_trigger_value` of 3.
fn step_intervals(steps: &[WorkoutStep], sport: &SportType, ftp_known: bool) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut open: OpenSet = None;
    let close = |open: &mut OpenSet, out: &mut Vec<Value>| {
        if let Some(((repeat, _), children)) = open.take() {
            if !children.is_empty() {
                out.push(json!({
                    "name": format!("{repeat}× set"),
                    "exit_trigger_type": "repeat",
                    "exit_trigger_value": repeat - 1,
                    "intensity_type": IntensityType::Active.wire(),
                    "intervals": children,
                }));
            }
        }
    };
    for step in steps {
        let Some(interval) = step_interval(step, sport, ftp_known) else {
            continue;
        };
        if step.repeat > 1 {
            let set = (step.repeat, step.repeat_group);
            if open.as_ref().map(|(key, _)| *key) != Some(set) {
                close(&mut open, &mut out);
                open = Some((set, Vec::new()));
            }
            if let Some((_, children)) = open.as_mut() {
                children.push(interval);
            }
        } else {
            close(&mut open, &mut out);
            out.push(interval);
        }
    }
    close(&mut open, &mut out);
    out
}

/// Whether any interval, at any depth, carries an FTP target.
fn uses_ftp(intervals: &[Value]) -> bool {
    intervals.iter().any(|interval| {
        let targets = interval["targets"].as_array().is_some_and(|targets| {
            targets
                .iter()
                .any(|target| target["type"].as_str() == Some("ftp"))
        });
        let nested = interval["intervals"]
            .as_array()
            .is_some_and(|children| uses_ftp(children));
        targets || nested
    })
}

/// The session's `plan.json`, or `None` when Wahoo cannot carry it as a plan.
///
/// `None` for a sport outside biking and running, or a session with neither
/// steps nor a duration. `ftp` is the athlete's FTP as Wahoo holds it (their
/// power zones); it reaches the file only when a target is a fraction of it.
#[must_use]
pub fn plan_file(session: &PlannedSession, ftp: Option<u32>) -> Option<Value> {
    let (family, location) = plan_family(&session.sport)?;
    let mut intervals = step_intervals(&session.steps, &session.sport, ftp.is_some());
    if intervals.is_empty() {
        // A timed session with no structure: one steady interval, so the
        // device still runs the clock.
        let seconds = session.duration_seconds.filter(|seconds| *seconds > 0)?;
        intervals.push(json!({
            "name": IntensityType::Active.name(),
            "exit_trigger_type": "time",
            "exit_trigger_value": seconds,
            "intensity_type": IntensityType::Active.wire(),
        }));
    }
    let mut header = json!({
        "name": neutral_title(session),
        "version": PLAN_JSON_VERSION,
        "workout_type_family": family,
        "workout_type_location": location,
    });
    if let Some(seconds) = session_seconds(session) {
        header["duration_s"] = json!(seconds);
    }
    if let Some(ftp) = ftp.filter(|_| uses_ftp(&intervals)) {
        header["ftp"] = json!(ftp);
    }
    Some(json!({ "header": header, "intervals": intervals }))
}

/// Whether the session's plan would carry an FTP target once the athlete's
/// FTP is known — so the FTP is read from Wahoo only for a plan that uses it.
#[must_use]
pub fn targets_ftp(session: &PlannedSession) -> bool {
    plan_file(session, Some(1)).is_some_and(|file| file["header"].get("ftp").is_some())
}

/// A UTC instant as Wahoo's date-time fields take it (`2026-10-08T12:00:00.000Z`).
#[must_use]
pub fn wahoo_timestamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The start time a session on `date` is scheduled at.
#[must_use]
pub fn scheduled_start(date: NaiveDate) -> String {
    format!("{}T{SCHEDULED_HOUR_UTC}", date.format("%Y-%m-%d"))
}

/// The form fields of a plan upload (`POST /v1/plans`, or `PUT` with
/// `with_external_id` false, since an update cannot change it). The file goes
/// as a base64 data URI, as the API takes it.
#[must_use]
pub fn plan_form(
    session: &PlannedSession,
    file: &Value,
    updated_at: DateTime<Utc>,
    with_external_id: bool,
) -> Vec<(&'static str, String)> {
    let encoded = BASE64.encode(file.to_string());
    let mut form = vec![
        (
            "plan[file]",
            format!("data:application/json;base64,{encoded}"),
        ),
        ("plan[filename]", PLAN_FILENAME.to_owned()),
        ("plan[provider_updated_at]", wahoo_timestamp(updated_at)),
    ];
    if with_external_id {
        form.push(("plan[external_id]", opaque_id(&session.external_id)));
    }
    form
}

/// The form fields of a scheduled workout (`POST /v1/workouts`, or `PUT` on
/// an update), attached to `plan_id` when the session has a plan.
#[must_use]
pub fn workout_form(session: &PlannedSession, plan_id: Option<i64>) -> Vec<(&'static str, String)> {
    let minutes = session_seconds(session).map_or(0, |seconds| seconds.div_ceil(60));
    let mut form = vec![
        ("workout[name]", neutral_title(session)),
        ("workout[workout_token]", opaque_id(&session.external_id)),
        (
            "workout[workout_type_id]",
            workout_type_id(&session.sport).to_string(),
        ),
        ("workout[starts]", scheduled_start(session.date)),
        ("workout[minutes]", minutes.to_string()),
    ];
    if let Some(plan_id) = plan_id {
        form.push(("workout[plan_id]", plan_id.to_string()));
    }
    form
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PlannedSessionKind;

    fn step(label: &str, minutes: u32, zone: &str, repeat: u32, note: Option<&str>) -> WorkoutStep {
        WorkoutStep {
            label: label.to_owned(),
            duration_seconds: minutes * 60,
            distance_meters: None,
            target_zone: zone.to_owned(),
            repeat,
            repeat_group: (repeat > 1).then_some(1),
            note: note.map(str::to_owned),
        }
    }

    /// A session loaded with everything that must never reach Wahoo: the
    /// athlete's name, health values, the coach's reasoning, a fuelling line
    /// and a calendar key carrying the user id.
    fn loaded_session() -> PlannedSession {
        PlannedSession {
            external_id: "dravr:plan:4b0f3c1e-8a2d-4f6b-9c3e-2d1a7e5f9b10:2026-10-08:0".to_owned(),
            kind: PlannedSessionKind::Workout,
            date: NaiveDate::from_ymd_opt(2026, 10, 8).unwrap_or_default(),
            sport: SportType::Ride,
            name: "Julie threshold — HRV low so keep it honest".to_owned(),
            duration_seconds: Some(5400),
            notes: "Coach note: Julie's knee pain is back; fuel 60 g/h; weight 58 kg".to_owned(),
            steps: vec![
                step("Julie's warm-up", 20, "Z2", 1, Some("illness last week")),
                step("Threshold block", 8, "Z4", 4, Some("HRV 42 ms")),
                step("Easy spin", 4, "Z1", 4, None),
                step("Cool-down", 22, "Z1", 1, Some("sleep 5 h")),
            ],
        }
    }

    fn everything_sent(session: &PlannedSession, ftp: Option<u32>) -> String {
        let file = plan_file(session, ftp).unwrap_or_default();
        let mut sent = file.to_string();
        let now = Utc::now();
        for (key, value) in plan_form(session, &file, now, true)
            .into_iter()
            .chain(workout_form(session, Some(7)))
        {
            sent.push_str(key);
            sent.push_str(&value);
        }
        sent
    }

    #[test]
    fn the_title_is_generated_from_structure_never_from_the_session_title() {
        let session = loaded_session();
        assert_eq!(neutral_title(&session), "Ride 1 h 30 · 4×8 min");
        let file = plan_file(&session, Some(250)).unwrap_or_default();
        assert_eq!(file["header"]["name"], "Ride 1 h 30 · 4×8 min");
        let form = workout_form(&session, Some(7));
        assert!(form.contains(&("workout[name]", "Ride 1 h 30 · 4×8 min".to_owned())));
    }

    #[test]
    fn nothing_personal_or_from_the_prose_reaches_any_field_sent_to_wahoo() {
        let session = loaded_session();
        let sent = everything_sent(&session, Some(250));
        for forbidden in [
            "Julie",
            "HRV",
            "honest",
            "knee",
            "illness",
            "sleep",
            "fuel",
            "weight",
            "58 kg",
            "Coach",
            "Threshold block",
            "Easy spin",
            "4b0f3c1e",
            "dravr:plan",
            "description",
        ] {
            assert!(
                !sent.contains(forbidden),
                "'{forbidden}' reached a field sent to Wahoo: {sent}"
            );
        }
    }

    #[test]
    fn ids_are_opaque_stable_and_shared_by_the_plan_and_its_workout() {
        let session = loaded_session();
        let id = opaque_id(&session.external_id);
        assert!(id.starts_with(OPAQUE_ID_PREFIX));
        assert_eq!(id.len(), OPAQUE_ID_PREFIX.len() + OPAQUE_ID_HEX_CHARS);
        assert!(id[OPAQUE_ID_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id, opaque_id(&session.external_id), "stable across pushes");
        let file = plan_file(&session, None).unwrap_or_default();
        let plan = plan_form(&session, &file, Utc::now(), true);
        let workout = workout_form(&session, Some(7));
        assert!(plan.contains(&("plan[external_id]", id.clone())));
        assert!(workout.contains(&("workout[workout_token]", id.clone())));
        assert!(is_dravr_token(&id));
        assert!(!is_dravr_token("123"));
    }

    #[test]
    fn an_update_never_resends_the_external_id() {
        let session = loaded_session();
        let file = plan_file(&session, None).unwrap_or_default();
        assert!(plan_form(&session, &file, Utc::now(), false)
            .iter()
            .all(|(key, _)| *key != "plan[external_id]"));
    }

    #[test]
    fn repeated_steps_become_one_repeat_interval_counting_after_the_first() {
        let file = plan_file(&loaded_session(), Some(250)).unwrap_or_default();
        let intervals = file["intervals"].as_array().cloned().unwrap_or_default();
        assert_eq!(intervals.len(), 3, "warm-up, the set, cool-down");
        let set = &intervals[1];
        assert_eq!(set["exit_trigger_type"], "repeat");
        assert_eq!(set["exit_trigger_value"], 3);
        let children = set["intervals"].as_array().cloned().unwrap_or_default();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0]["exit_trigger_value"], 480.0);
        assert_eq!(children[0]["intensity_type"], "lt");
        assert_eq!(children[0]["name"], "Threshold");
        assert_eq!(intervals[0]["intensity_type"], "wu");
        assert_eq!(intervals[2]["intensity_type"], "cd");
    }

    #[test]
    fn power_zones_become_ftp_fractions_with_wahoos_own_ftp_in_the_header() {
        let file = plan_file(&loaded_session(), Some(250)).unwrap_or_default();
        assert_eq!(file["header"]["ftp"], 250);
        let threshold = &file["intervals"][1]["intervals"][0]["targets"][0];
        assert_eq!(threshold["type"], "ftp");
        assert_eq!(threshold["low"], 0.9);
        assert_eq!(threshold["high"], 1.05);
    }

    #[test]
    fn without_a_wahoo_ftp_power_steps_go_out_untargeted_and_no_threshold_is_sent() {
        let file = plan_file(&loaded_session(), None).unwrap_or_default();
        assert!(file["header"].get("ftp").is_none());
        let intervals = file["intervals"].as_array().cloned().unwrap_or_default();
        assert!(!uses_ftp(&intervals));
        assert_eq!(file["intervals"][0]["intensity_type"], "wu");
    }

    #[test]
    fn heart_rate_and_pace_targets_never_put_a_threshold_in_the_file() {
        let mut session = loaded_session();
        session.sport = SportType::Run;
        session.steps = vec![
            step("Warm up", 15, "Z2", 1, None),
            step("Tempo", 20, "95-100% threshold HR", 1, None),
            step("Strides", 1, "RPE 9", 6, None),
        ];
        let file = plan_file(&session, Some(250)).unwrap_or_default();
        let text = file.to_string();
        for threshold in ["threshold_hr", "max_hr", "threshold_speed", "\"ftp\""] {
            assert!(!text.contains(threshold), "{threshold} in {text}");
        }
        let strides = &file["intervals"][2]["intervals"][0]["targets"][0];
        assert_eq!(strides["type"], "rpe");
        assert_eq!(strides["low"], 9);
    }

    #[test]
    fn a_sport_plan_json_cannot_carry_has_no_plan() {
        let mut session = loaded_session();
        session.sport = SportType::Swim;
        assert!(plan_file(&session, Some(250)).is_none());
        let form = workout_form(&session, None);
        assert!(form.contains(&("workout[workout_type_id]", "25".to_owned())));
        assert!(form.iter().all(|(key, _)| *key != "workout[plan_id]"));
    }

    #[test]
    fn a_timed_session_without_steps_runs_one_steady_interval() {
        let mut session = loaded_session();
        session.steps.clear();
        session.duration_seconds = Some(3600);
        let file = plan_file(&session, Some(250)).unwrap_or_default();
        assert_eq!(neutral_title(&session), "Ride 1 h");
        assert_eq!(file["intervals"][0]["exit_trigger_value"], 3600);
        assert!(file["header"].get("ftp").is_none());
    }

    #[test]
    fn a_session_is_scheduled_at_noon_utc_on_its_civil_date() {
        let session = loaded_session();
        let form = workout_form(&session, Some(7));
        assert!(form.contains(&("workout[starts]", "2026-10-08T12:00:00.000Z".to_owned())));
        assert!(form.contains(&("workout[minutes]", "90".to_owned())));
    }

    #[test]
    fn workout_types_round_trip_for_the_sports_dravr_plans() {
        for sport in [
            SportType::Ride,
            SportType::VirtualRide,
            SportType::MountainBike,
            SportType::EbikeRide,
            SportType::Run,
            SportType::VirtualRun,
            SportType::TrailRunning,
            SportType::Swim,
            SportType::Walk,
            SportType::Hike,
            SportType::Yoga,
            SportType::Rowing,
        ] {
            assert_eq!(sport_for_workout_type(workout_type_id(&sport)), sport);
        }
    }
}
