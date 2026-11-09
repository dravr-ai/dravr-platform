// ABOUTME: Garmin Training API rendering — a PlannedSession into a neutral structured workout, and its schedule on a day
// ABOUTME: Titles from structure, steps classed from a closed vocabulary, targets on the athlete's own Garmin zones and FTP only

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Garmin Training API rendering
//!
//! Garmin keeps a planned workout as two records: a *workout* in the
//! athlete's Garmin Connect library, holding the structured steps, and a
//! *schedule* putting that workout on a day of their calendar. Dravr writes
//! one of each per calendar entry, so an entry's provider id is the pair
//! ([`EventId`], `{workoutId}:{scheduleId}`), and both are read back from
//! the schedule listing without a further call.
//!
//! The developer agreement gives Garmin broad rights over anything sent to
//! it, so the workout is **neutral by construction**, under the rules the
//! Wahoo plan follows ([`crate::neutral_session`]):
//!
//! - the name is generated from the session's structure, never from its
//!   title, its notes, a step's label or a step's note; no description is
//!   sent;
//! - each step carries only its class (warm-up, interval, …), its extent and
//!   its target;
//! - a target names the athlete's own zones or FTP as Garmin holds them (a
//!   power or heart-rate zone number, a percent of FTP). A target that would
//!   need any threshold value in the payload (a percent of threshold heart
//!   rate or pace, a run zone Garmin has no zone family for) goes out open.

use std::fmt;

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::errors::{AppError, AppResult};
use crate::models::periodization::ThresholdBasis;
use crate::models::{PlannedSession, RelativeIntensity, SportType, WorkoutStep};
use crate::neutral_session::{is_power_sport, neutral_title, StepClass, SWEET_SPOT};

/// The workout provider Garmin Connect shows the athlete, and the source it
/// files Dravr's workouts under.
pub(super) const WORKOUT_PROVIDER: &str = "Dravr";

/// Most steps a single-sport workout takes, repeat blocks included.
const MAX_STEPS: usize = 100;

/// Highest heart-rate zone Garmin defines.
const MAX_HEART_RATE_ZONE: u8 = 5;

/// Highest power zone Garmin defines.
const MAX_POWER_ZONE: u8 = 7;

/// A Dravr calendar entry on Garmin: the workout and the schedule that puts
/// it on its day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EventId {
    pub(super) workout_id: i64,
    pub(super) schedule_id: i64,
}

impl EventId {
    /// Read a provider event id. Anything but two of Garmin's integer ids
    /// never came from Garmin and is never interpolated into a path.
    pub(super) fn parse(id: &str) -> AppResult<Self> {
        let refused =
            || AppError::invalid_input(format!("'{id}' is not a Garmin calendar entry id"));
        let (workout, schedule) = id.trim().split_once(':').ok_or_else(refused)?;
        Ok(Self {
            workout_id: workout.parse().map_err(|_| refused())?,
            schedule_id: schedule.parse().map_err(|_| refused())?,
        })
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.workout_id, self.schedule_id)
    }
}

/// The Garmin workout sport a session is filed under.
fn garmin_sport(sport: &SportType) -> &'static str {
    match sport {
        SportType::Ride
        | SportType::VirtualRide
        | SportType::MountainBike
        | SportType::GravelRide
        | SportType::EbikeRide => "CYCLING",
        SportType::Run | SportType::TrailRunning | SportType::VirtualRun => "RUNNING",
        SportType::Swim => "LAP_SWIMMING",
        SportType::StrengthTraining | SportType::Crossfit => "STRENGTH_TRAINING",
        SportType::Yoga => "YOGA",
        SportType::Pilates => "PILATES",
        _ => "GENERIC",
    }
}

/// Garmin's step intensity for a step's class.
const fn step_intensity(class: StepClass) -> &'static str {
    match class {
        StepClass::Warmup => "WARMUP",
        StepClass::Cooldown => "COOLDOWN",
        StepClass::Rest => "REST",
        StepClass::Recover => "RECOVERY",
        StepClass::Active | StepClass::Tempo => "ACTIVE",
        StepClass::Threshold
        | StepClass::Vo2Max
        | StepClass::Anaerobic
        | StepClass::Neuromuscular => "INTERVAL",
    }
}

/// A percent of FTP as Garmin's target fields take it.
fn ftp_percent(low: f64, high: f64) -> Map<String, Value> {
    let mut target = Map::new();
    target.insert("targetType".to_owned(), json!("POWER"));
    target.insert("targetValueLow".to_owned(), json!(low));
    target.insert("targetValueHigh".to_owned(), json!(high));
    target.insert("targetValueType".to_owned(), json!("PERCENT"));
    target
}

/// A zone of the athlete's own as Garmin's target fields take it.
fn zone(target_type: &str, zone: u8) -> Map<String, Value> {
    let mut target = Map::new();
    target.insert("targetType".to_owned(), json!(target_type));
    target.insert("targetValue".to_owned(), json!(zone));
    target
}

/// A step's target fields, or `None` when the step goes out open: Garmin
/// resolves a zone against the zones the athlete keeps in Garmin Connect and
/// a percent against the FTP it holds for them, so no threshold of Dravr's
/// reaches the payload.
fn step_target(
    intensity: Option<&RelativeIntensity>,
    sport: &SportType,
) -> Option<Map<String, Value>> {
    let power = is_power_sport(sport);
    match intensity? {
        RelativeIntensity::Zone(z) if power && (1..=MAX_POWER_ZONE).contains(z) => {
            Some(zone("POWER", *z))
        }
        RelativeIntensity::HeartRateZone(z) if (1..=MAX_HEART_RATE_ZONE).contains(z) => {
            Some(zone("HEART_RATE", *z))
        }
        RelativeIntensity::SweetSpot if power => Some(ftp_percent(
            (SWEET_SPOT.0 * 100.0).round(),
            (SWEET_SPOT.1 * 100.0).round(),
        )),
        RelativeIntensity::Percent {
            low,
            high,
            threshold,
        } if match threshold {
            Some(ThresholdBasis::Ftp) => true,
            None => power,
            Some(_) => false,
        } =>
        {
            Some(ftp_percent(f64::from(*low), f64::from(*high)))
        }
        _ => None,
    }
}

/// One step, numbered `order`, or `None` for a step with no extent.
fn workout_step(step: &WorkoutStep, sport: &SportType, order: usize) -> Option<Value> {
    let swim = matches!(sport, SportType::Swim);
    let intensity = RelativeIntensity::parse(&step.target_zone);
    let class = StepClass::of_step(step, intensity.as_ref());
    let mut out = Map::new();
    out.insert("type".to_owned(), json!("WorkoutStep"));
    out.insert("stepOrder".to_owned(), json!(order));
    out.insert("intensity".to_owned(), json!(step_intensity(class)));
    match step.distance_meters {
        Some(distance) if distance > 0.0 => {
            out.insert("durationType".to_owned(), json!("DISTANCE"));
            out.insert("durationValue".to_owned(), json!(distance.round()));
            out.insert("durationValueType".to_owned(), json!("METER"));
        }
        _ if step.duration_seconds > 0 => {
            // A pool rest is its own duration type; everywhere else rest is
            // timed like any step.
            let kind = if swim && class == StepClass::Rest {
                "FIXED_REST"
            } else {
                "TIME"
            };
            out.insert("durationType".to_owned(), json!(kind));
            out.insert("durationValue".to_owned(), json!(step.duration_seconds));
        }
        _ => return None,
    }
    // Garmin takes no primary target on a swim step.
    if !swim {
        match step_target(intensity.as_ref(), sport) {
            Some(target) => out.extend(target),
            None => {
                out.insert("targetType".to_owned(), json!("OPEN"));
            }
        }
    }
    Some(Value::Object(out))
}

/// A repeated set being gathered: its `(repeat, repeat_group)` key and the
/// steps of one pass.
type OpenSet<'a> = Option<((u32, Option<u32>), Vec<&'a WorkoutStep>)>;

/// The workout's steps, numbered in document order as Garmin numbers them
/// (a repeat block, then the steps inside it). Consecutive steps sharing a
/// `repeat` above one and the same `repeat_group` become one repeat block run
/// `repeat` times.
fn workout_steps(steps: &[WorkoutStep], sport: &SportType) -> Vec<Value> {
    let mut out = Vec::new();
    let mut order = 0_usize;
    let mut open: OpenSet<'_> = None;
    let close = |open: &mut OpenSet<'_>, out: &mut Vec<Value>, order: &mut usize| {
        let Some(((repeat, _), members)) = open.take() else {
            return;
        };
        *order += 1;
        let block_order = *order;
        let children: Vec<Value> = members
            .into_iter()
            .filter_map(|step| {
                let next = *order + 1;
                let rendered = workout_step(step, sport, next)?;
                *order = next;
                Some(rendered)
            })
            .collect();
        if children.is_empty() {
            *order = block_order - 1;
            return;
        }
        out.push(json!({
            "type": "WorkoutRepeatStep",
            "stepOrder": block_order,
            "repeatType": "REPEAT_UNTIL_STEPS_CMPLT",
            "repeatValue": repeat,
            "steps": children,
        }));
    };
    for step in steps {
        if step.repeat > 1 {
            let set = (step.repeat, step.repeat_group);
            if open.as_ref().map(|(key, _)| *key) != Some(set) {
                close(&mut open, &mut out, &mut order);
                open = Some((set, Vec::new()));
            }
            if let Some((_, members)) = open.as_mut() {
                members.push(step);
            }
        } else {
            close(&mut open, &mut out, &mut order);
            if let Some(rendered) = workout_step(step, sport, order + 1) {
                order += 1;
                out.push(rendered);
            }
        }
    }
    close(&mut open, &mut out, &mut order);
    out
}

/// How many steps Garmin counts in `steps`: every step and every repeat
/// block, at any depth.
fn step_count(steps: &[Value]) -> usize {
    steps
        .iter()
        .map(|step| {
            1 + step["steps"]
                .as_array()
                .map_or(0, |inner| step_count(inner))
        })
        .sum()
}

/// The workout Garmin stores for a session.
///
/// A session without steps runs one steady step for its stated duration, or
/// one open step (ended by the lap button) when it has none, so the device
/// still has a workout to run.
///
/// # Errors
///
/// Returns an invalid-input error for a session with more steps than a
/// Garmin workout takes.
pub(super) fn workout_body(session: &PlannedSession) -> AppResult<Value> {
    let sport = garmin_sport(&session.sport);
    let mut steps = workout_steps(&session.steps, &session.sport);
    if steps.is_empty() {
        let mut step = Map::new();
        step.insert("type".to_owned(), json!("WorkoutStep"));
        step.insert("stepOrder".to_owned(), json!(1));
        step.insert(
            "intensity".to_owned(),
            json!(step_intensity(StepClass::Active)),
        );
        match session.duration_seconds.filter(|seconds| *seconds > 0) {
            Some(seconds) => {
                step.insert("durationType".to_owned(), json!("TIME"));
                step.insert("durationValue".to_owned(), json!(seconds));
            }
            None => {
                step.insert("durationType".to_owned(), json!("OPEN"));
            }
        }
        if !matches!(session.sport, SportType::Swim) {
            step.insert("targetType".to_owned(), json!("OPEN"));
        }
        steps.push(Value::Object(step));
    }
    let count = step_count(&steps);
    if count > MAX_STEPS {
        return Err(AppError::invalid_input(format!(
            "a Garmin workout takes at most {MAX_STEPS} steps, repeat blocks included; this \
             session has {count}"
        )));
    }
    Ok(json!({
        "workoutName": neutral_title(session),
        "sport": sport,
        "workoutProvider": WORKOUT_PROVIDER,
        "workoutSourceId": WORKOUT_PROVIDER,
        "isSessionTransitionEnabled": false,
        "segments": [{
            "segmentOrder": 1,
            "sport": sport,
            "steps": steps,
        }],
    }))
}

/// The full workout an update sends: the session's workout under the id and
/// owner Garmin gave it.
///
/// # Errors
///
/// Returns the error [`workout_body`] gives.
pub(super) fn updated_workout_body(
    session: &PlannedSession,
    stored: &StoredWorkout,
) -> AppResult<Value> {
    let mut body = workout_body(session)?;
    body["workoutId"] = json!(stored.workout_id);
    if let Some(owner) = stored.owner_id {
        body["ownerId"] = json!(owner);
    }
    Ok(body)
}

/// A schedule putting `workout_id` on `date`, under `schedule_id` when it
/// already exists.
pub(super) fn schedule_body(workout_id: i64, date: NaiveDate, schedule_id: Option<i64>) -> Value {
    let mut body = json!({
        "workoutId": workout_id,
        "date": date.format("%Y-%m-%d").to_string(),
    });
    if let Some(id) = schedule_id {
        body["scheduleId"] = json!(id);
    }
    body
}

/// A workout as Garmin returns it: only its ids are read.
#[derive(Debug, Deserialize)]
pub(super) struct StoredWorkout {
    #[serde(rename = "workoutId")]
    pub(super) workout_id: i64,
    #[serde(rename = "ownerId", default)]
    pub(super) owner_id: Option<i64>,
}

/// One schedule of the athlete's calendar.
#[derive(Debug, Deserialize)]
pub(super) struct Schedule {
    #[serde(rename = "scheduleId")]
    pub(super) id: i64,
    #[serde(rename = "workoutId")]
    pub(super) workout_id: i64,
    date: String,
}

impl Schedule {
    /// The schedule's civil date, or `None` when it is unreadable.
    pub(super) fn date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(self.date.get(..10)?, "%Y-%m-%d").ok()
    }

    /// The calendar entry this schedule is.
    pub(super) const fn event_id(&self) -> EventId {
        EventId {
            workout_id: self.workout_id,
            schedule_id: self.id,
        }
    }
}

/// The schedule id a schedule create answered with: a bare id, a quoted id,
/// or the created schedule. `None` for an answer that names none.
pub(super) fn created_schedule_id(body: &str) -> Option<i64> {
    match serde_json::from_str::<Value>(body.trim()).ok()? {
        Value::Number(id) => id.as_i64(),
        Value::String(id) => id.trim().parse().ok(),
        Value::Object(created) => created.get("scheduleId").and_then(Value::as_i64),
        _ => None,
    }
}

/// The permissions `GET user/permissions` answers with: a bare list, or an
/// object carrying it.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum Permissions {
    List(Vec<String>),
    Wrapped { permissions: Vec<String> },
}

impl Permissions {
    /// Whether `grant` is among them.
    pub(super) fn grants(&self, grant: &str) -> bool {
        let (Self::List(all) | Self::Wrapped { permissions: all }) = self;
        all.iter().any(|g| g.trim().eq_ignore_ascii_case(grant))
    }
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

    /// A session loaded with everything that must never reach Garmin.
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

    fn body(session: &PlannedSession) -> Value {
        workout_body(session).unwrap_or_else(|e| panic!("workout body: {e}"))
    }

    fn steps(body: &Value) -> Vec<Value> {
        body["segments"][0]["steps"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn nothing_personal_or_from_the_prose_reaches_the_workout() {
        let sent = body(&loaded_session()).to_string();
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
                "'{forbidden}' reached the Garmin workout: {sent}"
            );
        }
    }

    #[test]
    fn the_workout_is_named_from_structure_and_filed_under_dravr() {
        let body = body(&loaded_session());
        assert_eq!(body["workoutName"], "Ride 1 h 30 · 4×8 min");
        assert_eq!(body["sport"], "CYCLING");
        assert_eq!(body["segments"][0]["sport"], "CYCLING");
        assert_eq!(body["workoutProvider"], WORKOUT_PROVIDER);
        assert!(body.get("workoutId").is_none(), "a create sends no id");
    }

    #[test]
    fn repeated_steps_become_one_block_run_that_many_times_numbered_in_order() {
        let steps = steps(&body(&loaded_session()));
        assert_eq!(steps.len(), 3, "warm-up, the block, cool-down");
        assert_eq!(steps[0]["intensity"], "WARMUP");
        assert_eq!(steps[0]["stepOrder"], 1);
        let block = &steps[1];
        assert_eq!(block["type"], "WorkoutRepeatStep");
        assert_eq!(block["stepOrder"], 2);
        assert_eq!(block["repeatType"], "REPEAT_UNTIL_STEPS_CMPLT");
        assert_eq!(block["repeatValue"], 4, "Garmin counts every pass");
        let inner = block["steps"].as_array().cloned().unwrap_or_default();
        assert_eq!(inner.len(), 2);
        assert_eq!(inner[0]["stepOrder"], 3);
        assert_eq!(inner[0]["intensity"], "INTERVAL");
        assert_eq!(inner[0]["durationType"], "TIME");
        assert_eq!(inner[0]["durationValue"], 480);
        assert_eq!(inner[1]["stepOrder"], 4);
        assert_eq!(inner[1]["intensity"], "RECOVERY");
        assert_eq!(steps[2]["stepOrder"], 5);
        assert_eq!(steps[2]["intensity"], "COOLDOWN");
    }

    #[test]
    fn power_targets_name_the_athletes_garmin_zones_and_ftp_never_a_value() {
        let mut session = loaded_session();
        session.steps = vec![
            step("a", 10, "Z4", 1, None),
            step("b", 10, "sweet spot", 1, None),
            step("c", 10, "105-110% FTP", 1, None),
        ];
        let steps = steps(&body(&session));
        assert_eq!(steps[0]["targetType"], "POWER");
        assert_eq!(steps[0]["targetValue"], 4);
        assert_eq!(steps[1]["targetValueType"], "PERCENT");
        assert_eq!(steps[1]["targetValueLow"], 88.0);
        assert_eq!(steps[1]["targetValueHigh"], 94.0);
        assert_eq!(steps[2]["targetValueLow"], 105.0);
        assert_eq!(steps[2]["targetValueHigh"], 110.0);
        let text = body(&session).to_string();
        assert!(!text.contains("ftp\""), "no FTP value is sent: {text}");
    }

    #[test]
    fn targets_that_would_need_a_threshold_go_out_open() {
        let mut session = loaded_session();
        session.sport = SportType::Run;
        session.steps = vec![
            step("Warm up", 15, "Z2", 1, None),
            step("Tempo", 20, "95-100% threshold HR", 1, None),
            step("Cruise", 10, "90-95% threshold pace", 1, None),
            step("Strides", 1, "RPE 9", 6, None),
            step("Aerobic", 20, "Z2 HR", 1, None),
        ];
        let steps = steps(&body(&session));
        assert_eq!(
            steps[0]["targetType"], "OPEN",
            "a run zone has no Garmin family"
        );
        assert_eq!(steps[1]["targetType"], "OPEN");
        assert_eq!(steps[2]["targetType"], "OPEN");
        assert_eq!(steps[3]["steps"][0]["targetType"], "OPEN");
        assert_eq!(steps[4]["targetType"], "HEART_RATE");
        assert_eq!(steps[4]["targetValue"], 2);
        assert_eq!(body(&session)["sport"], "RUNNING");
    }

    #[test]
    fn a_swim_carries_no_primary_target_and_rests_in_place() {
        let mut session = loaded_session();
        session.sport = SportType::Swim;
        let mut lengths = step("Main", 0, "Z3", 1, None);
        lengths.distance_meters = Some(400.0);
        session.steps = vec![lengths, step("Rest", 1, "", 1, None)];
        let steps = steps(&body(&session));
        assert_eq!(steps[0]["durationType"], "DISTANCE");
        assert_eq!(steps[0]["durationValue"], 400.0);
        assert_eq!(steps[0]["durationValueType"], "METER");
        assert!(steps[0].get("targetType").is_none());
        assert_eq!(steps[1]["durationType"], "FIXED_REST");
        assert_eq!(body(&session)["sport"], "LAP_SWIMMING");
    }

    #[test]
    fn a_session_without_steps_runs_its_duration_or_an_open_step() {
        let mut session = loaded_session();
        session.steps.clear();
        let timed = steps(&body(&session));
        assert_eq!(timed.len(), 1);
        assert_eq!(timed[0]["durationType"], "TIME");
        assert_eq!(timed[0]["durationValue"], 5400);
        session.duration_seconds = None;
        let open = steps(&body(&session));
        assert_eq!(open[0]["durationType"], "OPEN");
    }

    #[test]
    fn a_session_with_more_steps_than_garmin_takes_is_refused() {
        let mut session = loaded_session();
        session.steps = (0..MAX_STEPS)
            .map(|_| step("s", 1, "Z2", 1, None))
            .collect();
        assert_eq!(
            steps(&body(&session)).len(),
            MAX_STEPS,
            "bug: a workout of exactly Garmin's step limit was not sent whole"
        );
        session.steps.push(step("s", 1, "Z2", 1, None));
        let err = workout_body(&session).expect_err("too many steps");
        assert!(err.to_string().contains("at most 100"), "{err}");
    }

    #[test]
    fn an_update_carries_the_id_and_owner_garmin_gave_the_workout() {
        let stored = StoredWorkout {
            workout_id: 41,
            owner_id: Some(9_000),
        };
        let body = updated_workout_body(&loaded_session(), &stored)
            .unwrap_or_else(|e| panic!("update body: {e}"));
        assert_eq!(body["workoutId"], 41);
        assert_eq!(body["ownerId"], 9_000);
    }

    #[test]
    fn event_ids_round_trip_and_anything_else_is_refused() {
        let id = EventId {
            workout_id: 41,
            schedule_id: 77,
        };
        assert_eq!(id.to_string(), "41:77");
        assert_eq!(EventId::parse("41:77").ok(), Some(id));
        for bad in ["77", "41:", "a:b", "41:77/../x", ""] {
            assert!(EventId::parse(bad).is_err(), "'{bad}' accepted");
        }
    }

    #[test]
    fn a_schedule_body_names_the_workout_and_the_civil_date() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap_or_default();
        assert_eq!(
            schedule_body(41, date, None),
            json!({"workoutId": 41, "date": "2026-10-08"})
        );
        assert_eq!(schedule_body(41, date, Some(77))["scheduleId"], 77);
    }

    #[test]
    fn a_created_schedule_id_is_read_from_any_shape_garmin_answers_with() {
        assert_eq!(created_schedule_id("77"), Some(77));
        assert_eq!(created_schedule_id("\"77\""), Some(77));
        assert_eq!(
            created_schedule_id(r#"{"scheduleId":77,"workoutId":41,"date":"2026-10-08"}"#),
            Some(77)
        );
        assert_eq!(created_schedule_id(""), None);
        assert_eq!(created_schedule_id("{}"), None);
    }

    #[test]
    fn permissions_are_read_bare_or_wrapped() {
        let bare: Permissions = serde_json::from_str(r#"["ACTIVITY_EXPORT","WORKOUT_IMPORT"]"#)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(bare.grants("WORKOUT_IMPORT"));
        let wrapped: Permissions = serde_json::from_str(r#"{"permissions":["HEALTH_EXPORT"]}"#)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!wrapped.grants("WORKOUT_IMPORT"));
    }
}
