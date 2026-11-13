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
//!   title, its notes, a step's label or a step's note;
//! - each step carries only its class (warm-up, interval, …), its extent, its
//!   target, and as its description the zone the step grammar parsed (`Z3`);
//! - the workout's description lists those same steps, one line each
//!   (`Tempo 4 min · Z3`), under their repeat counts: Garmin Connect may show
//!   a partner's workout without its steps (carnet#1532), so the notes are
//!   where the athlete reads the structure before the device runs it;
//! - a target names the athlete's own zones or FTP as Garmin holds them (a
//!   power or heart-rate zone number, a percent of FTP). A run's training
//!   zone names the heart-rate zone it sits nearest on Dravr's run line
//!   (Z1–Z5); faster than Z5 it goes out open, as does any target that would
//!   need a threshold value in the payload (a percent of threshold heart rate
//!   or pace).

use std::fmt;

use chrono::{NaiveDate, Utc};
use dravr_cageux::periodization::{band_coordinates, AnchorRatios, SportFamily};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tracing::{info, warn};

use crate::errors::{AppError, AppResult};
use crate::models::periodization::ThresholdBasis;
use crate::models::sport_family;
use crate::models::{PlannedSession, RelativeIntensity, SportType, WorkoutStep};
use crate::neutral_session::{format_extent, is_power_sport, neutral_title, StepClass, SWEET_SPOT};

/// The workout provider Garmin Connect shows the athlete, and the source it
/// files Dravr's workouts under.
pub(super) const WORKOUT_PROVIDER: &str = "Dravr";

/// Most steps a single-sport workout takes, repeat blocks included.
const MAX_STEPS: usize = 100;

/// Highest heart-rate zone Garmin defines.
const MAX_HEART_RATE_ZONE: u8 = 5;

/// Highest power zone Garmin defines.
const MAX_POWER_ZONE: u8 = 7;

/// Fastest training zone a run step targets by heart rate. Above it (Z6
/// anaerobic, Z7 sprint) the effort is over before heart rate catches up.
const FASTEST_HEART_RATE_RUN_ZONE: u8 = 5;

/// Longest workout description Garmin keeps whole; it truncates the rest.
const MAX_DESCRIPTION_CHARS: usize = 1024;

/// What ends a description cut to fit, on a line of its own.
const DESCRIPTION_CUT: &str = "\n…";

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
        sport if is_run_sport(sport) => "RUNNING",
        SportType::Swim => "LAP_SWIMMING",
        SportType::StrengthTraining | SportType::Crossfit => "STRENGTH_TRAINING",
        SportType::Yoga => "YOGA",
        SportType::Pilates => "PILATES",
        _ => "GENERIC",
    }
}

/// Whether Garmin files a session as a run: a sport of the running family,
/// as the one sport-family mapping reads it.
fn is_run_sport(sport: &SportType) -> bool {
    sport_family::SportFamily::of(sport) == sport_family::SportFamily::Running
}

/// The Garmin heart-rate zone a run's training zone `zone` sits nearest:
/// both placed on Dravr's run line by the kernel's coordinates, so the
/// mapping is the kernel's, never a table of the platform's. A zone above
/// [`FASTEST_HEART_RATE_RUN_ZONE`] is `None` and goes out open. Garmin
/// resolves the number against the zones the athlete keeps in Garmin Connect.
fn run_heart_rate_zone(zone: u8) -> Option<u8> {
    if !(1..=FASTEST_HEART_RATE_RUN_ZONE).contains(&zone) {
        return None;
    }
    let ratios = AnchorRatios::default();
    let on_run_line =
        |intensity| band_coordinates(intensity, SportFamily::Run, &ratios).map(|(at, _)| at);
    let pace = on_run_line(RelativeIntensity::Zone(zone))?;
    (1..=MAX_HEART_RATE_ZONE)
        .filter_map(|hr| {
            on_run_line(RelativeIntensity::HeartRateZone(hr)).map(|at| (hr, (at - pace).abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(hr, _)| hr)
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
        RelativeIntensity::Zone(z) if is_run_sport(sport) => {
            run_heart_rate_zone(*z).map(|hr| zone("HEART_RATE", hr))
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

/// Whether a step has an extent a device can run: a distance or a time. A
/// step without one is left out of the workout and of its description alike.
fn has_extent(step: &WorkoutStep) -> bool {
    step.distance_meters.is_some_and(|meters| meters > 0.0) || step.duration_seconds > 0
}

/// One step, numbered `order`, or `None` for a step with no extent.
fn workout_step(step: &WorkoutStep, sport: &SportType, order: usize) -> Option<Value> {
    if !has_extent(step) {
        return None;
    }
    let swim = matches!(sport, SportType::Swim);
    let intensity = RelativeIntensity::parse(&step.target_zone);
    let class = StepClass::of_step(step, intensity.as_ref());
    let mut out = Map::new();
    out.insert("type".to_owned(), json!("WorkoutStep"));
    out.insert("stepOrder".to_owned(), json!(order));
    out.insert("intensity".to_owned(), json!(step_intensity(class)));
    if let Some(parsed) = intensity.as_ref() {
        out.insert("description".to_owned(), json!(parsed.to_string()));
    }
    match step.distance_meters {
        Some(distance) if distance > 0.0 => {
            out.insert("durationType".to_owned(), json!("DISTANCE"));
            out.insert("durationValue".to_owned(), json!(distance.round()));
            out.insert("durationValueType".to_owned(), json!("METER"));
        }
        _ => {
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

/// A stretch of a session's steps as Garmin runs it: one step, or the steps
/// of one pass of a set run `repeat` times.
enum Block<'a> {
    Step(&'a WorkoutStep),
    Repeat(u32, Vec<&'a WorkoutStep>),
}

/// A repeated set being gathered: its `(repeat, repeat_group)` key and the
/// steps of one pass.
type OpenSet<'a> = Option<((u32, Option<u32>), Vec<&'a WorkoutStep>)>;

/// Close the set being gathered, if any, onto `out`.
fn close<'a>(open: &mut OpenSet<'a>, out: &mut Vec<Block<'a>>) {
    if let Some(((repeat, _), members)) = open.take() {
        out.push(Block::Repeat(repeat, members));
    }
}

/// The session's steps in document order, consecutive steps sharing a
/// `repeat` above one and the same `repeat_group` gathered into one repeated
/// set. The workout's steps and its description both walk these, so the
/// notes list exactly the sets the device runs.
fn blocks(steps: &[WorkoutStep]) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    let mut open: OpenSet<'_> = None;
    for step in steps {
        if step.repeat > 1 {
            let set = (step.repeat, step.repeat_group);
            if open.as_ref().map(|(key, _)| *key) != Some(set) {
                close(&mut open, &mut out);
                open = Some((set, Vec::new()));
            }
            if let Some((_, members)) = open.as_mut() {
                members.push(step);
            }
        } else {
            close(&mut open, &mut out);
            out.push(Block::Step(step));
        }
    }
    close(&mut open, &mut out);
    out
}

/// The workout's steps, numbered in document order as Garmin numbers them
/// (a repeat block, then the steps inside it). A repeated set becomes one
/// repeat block run `repeat` times; a step with no extent is left out, and a
/// set left with no steps with it.
fn workout_steps(steps: &[WorkoutStep], sport: &SportType) -> Vec<Value> {
    let mut out = Vec::new();
    let mut order = 0_usize;
    for block in blocks(steps) {
        match block {
            Block::Step(step) => {
                if let Some(rendered) = workout_step(step, sport, order + 1) {
                    order += 1;
                    out.push(rendered);
                }
            }
            Block::Repeat(repeat, members) => {
                let block_order = order + 1;
                let mut last = block_order;
                let children: Vec<Value> = members
                    .into_iter()
                    .filter_map(|step| {
                        let rendered = workout_step(step, sport, last + 1)?;
                        last += 1;
                        Some(rendered)
                    })
                    .collect();
                if children.is_empty() {
                    continue;
                }
                order = last;
                out.push(json!({
                    "type": "WorkoutRepeatStep",
                    "stepOrder": block_order,
                    "repeatType": "REPEAT_UNTIL_STEPS_CMPLT",
                    "repeatValue": repeat,
                    "steps": children,
                }));
            }
        }
    }
    out
}

/// One step as the workout's description lists it — its class, extent and
/// parsed zone (`Tempo 4 min · Z3`) — or `None` for a step with no extent,
/// which the workout leaves out too.
fn step_line(step: &WorkoutStep) -> Option<String> {
    if !has_extent(step) {
        return None;
    }
    let intensity = RelativeIntensity::parse(&step.target_zone);
    let class = StepClass::of_step(step, intensity.as_ref());
    let mut line = format!("{} {}", class.label(), format_extent(step));
    if let Some(parsed) = intensity {
        line.push_str(" · ");
        line.push_str(&parsed.to_string());
    }
    Some(line)
}

/// The workout's description: one line per step, a repeated set as its count
/// (`6×`) over its steps indented. Built from the closed vocabulary alone,
/// like the name. `None` for a session with no step to list. A description
/// longer than Garmin keeps ends at the last whole line that fits, followed
/// by `…`.
fn workout_description(steps: &[WorkoutStep]) -> Option<String> {
    let mut lines = Vec::new();
    for block in blocks(steps) {
        match block {
            Block::Step(step) => lines.extend(step_line(step)),
            Block::Repeat(repeat, members) => {
                let inner: Vec<String> = members
                    .into_iter()
                    .filter_map(step_line)
                    .map(|line| format!("  {line}"))
                    .collect();
                if !inner.is_empty() {
                    lines.push(format!("{repeat}×"));
                    lines.extend(inner);
                }
            }
        }
    }
    if lines.is_empty() {
        return None;
    }
    let whole = lines.join("\n");
    if whole.chars().count() <= MAX_DESCRIPTION_CHARS {
        return Some(whole);
    }
    let room = MAX_DESCRIPTION_CHARS - DESCRIPTION_CUT.chars().count();
    let mut kept = String::new();
    for line in &lines {
        let separator = usize::from(!kept.is_empty());
        if kept.chars().count() + separator + line.chars().count() > room {
            break;
        }
        if separator == 1 {
            kept.push('\n');
        }
        kept.push_str(line);
    }
    kept.push_str(DESCRIPTION_CUT);
    Some(kept)
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
    let mut body = json!({
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
    });
    if let Some(description) = workout_description(&session.steps) {
        body["description"] = json!(description);
    }
    Ok(body)
}

/// How many steps a workout's segments hold, every repeat block and the
/// steps inside it counted, or `None` when `segments` is not a list.
fn segment_step_count(segments: &Value) -> Option<usize> {
    segments.as_array().map(|segments| {
        segments
            .iter()
            .map(|segment| {
                segment["steps"]
                    .as_array()
                    .map_or(0, |steps| step_count(steps))
            })
            .sum()
    })
}

/// How many steps the workout Dravr sends holds.
pub(super) fn sent_step_count(body: &Value) -> Option<usize> {
    segment_step_count(&body["segments"])
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

/// A workout as Garmin returns it: its ids, the provider and source it is
/// filed under, Garmin's own stamps on it, and the segments holding its
/// steps. Everything past the ids is kept as the JSON Garmin sent, so a
/// field of an unexpected type never makes the workout unreadable to the
/// update that needs its ids.
#[derive(Debug, Deserialize)]
pub(super) struct StoredWorkout {
    #[serde(rename = "workoutId")]
    pub(super) workout_id: i64,
    #[serde(rename = "ownerId", default)]
    pub(super) owner_id: Option<i64>,
    #[serde(rename = "workoutProvider", default)]
    provider: Option<Value>,
    #[serde(rename = "workoutSourceId", default)]
    source_id: Option<Value>,
    #[serde(rename = "createdDate", default)]
    created_date: Option<Value>,
    #[serde(rename = "updatedDate", default)]
    updated_date: Option<Value>,
    #[serde(default)]
    segments: Option<Value>,
}

impl StoredWorkout {
    /// Whether Dravr wrote this workout: Garmin files it under
    /// [`WORKOUT_PROVIDER`] as both its provider and its source, as it does
    /// every workout [`workout_body`] sends. A workout the athlete or another
    /// app made carries neither.
    pub(super) fn is_dravrs(&self) -> bool {
        let dravr = |field: Option<&Value>| field.and_then(Value::as_str) == Some(WORKOUT_PROVIDER);
        dravr(self.provider.as_ref()) && dravr(self.source_id.as_ref())
    }

    /// How many steps Garmin holds for this workout, repeat blocks included,
    /// or `None` when its answer carried no segments.
    pub(super) fn held_steps(&self) -> Option<usize> {
        self.segments.as_ref().and_then(segment_step_count)
    }

    /// Log Garmin's stamps on this workout beside the instant Dravr saw them,
    /// `seen` naming the call that returned it (carnet#1389), with the steps
    /// it holds (carnet#1532).
    ///
    /// Garmin documents `createdDate` and `updatedDate` without a zone. A
    /// stamp logged on a workout Dravr has just created or updated, against
    /// `observed_at`, shows which zone Garmin writes them in; the same stamps
    /// on a later read show whether an edit in Garmin Connect moved them.
    /// The step count read back is what tells a workout Garmin kept whole
    /// from one it stored without its steps. Nothing here is the athlete's:
    /// ids, Garmin's stamps, a count, and the provider and source names the
    /// workout is filed under.
    pub(super) fn log_stamps(&self, seen: &'static str) {
        let raw =
            |field: Option<&Value>| field.map_or_else(|| "absent".to_owned(), Value::to_string);
        info!(
            provider = "garmin",
            seen,
            workout_id = self.workout_id,
            created_date = %raw(self.created_date.as_ref()),
            updated_date = %raw(self.updated_date.as_ref()),
            workout_provider = %raw(self.provider.as_ref()),
            workout_source_id = %raw(self.source_id.as_ref()),
            held_steps = ?self.held_steps(),
            observed_at = %Utc::now().to_rfc3339(),
            "Garmin workout stamps"
        );
    }

    /// Warn when Garmin answered a write of `sent` steps with a workout
    /// holding fewer, `seen` naming the call: the entry is on the athlete's
    /// calendar, but the device has less of the session to run than Dravr
    /// prescribed (carnet#1532). An answer carrying no segments says nothing
    /// either way and is not warned about.
    pub(super) fn warn_if_steps_dropped(&self, seen: &'static str, sent: Option<usize>) {
        if let Some((sent, held)) = self.dropped_steps(sent) {
            warn!(
                provider = "garmin",
                seen,
                workout_id = self.workout_id,
                sent_steps = sent,
                held_steps = held,
                "Garmin holds fewer workout steps than Dravr sent"
            );
        }
    }

    /// The `(sent, held)` step counts when this workout, Garmin's answer to
    /// a write of `sent` steps, holds fewer than were sent; `None` when it
    /// holds them all or the answer carried no segments.
    fn dropped_steps(&self, sent: Option<usize>) -> Option<(usize, usize)> {
        let (sent, held) = (sent?, self.held_steps()?);
        (held < sent).then_some((sent, held))
    }

    /// Log the stamps on the workout Garmin answered an update of `sent`
    /// steps with. Garmin may answer with the workout it now holds, or with
    /// nothing; an answer that is not a workout logs nothing.
    pub(super) fn log_update_answer(answer: Option<&str>, sent: Option<usize>) {
        let updated = answer.and_then(|text| serde_json::from_str::<Self>(text).ok());
        if let Some(updated) = updated {
            updated.log_stamps("updated");
            updated.warn_if_steps_dropped("updated", sent);
        }
    }
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
        ] {
            assert!(
                !sent.contains(forbidden),
                "'{forbidden}' reached the Garmin workout: {sent}"
            );
        }
    }

    #[test]
    fn the_description_lists_each_step_in_the_closed_vocabulary_under_its_repeats() {
        let body = body(&loaded_session());
        assert_eq!(
            body["description"],
            "Warm-up 20 min · Z2\n4×\n  Threshold 8 min · Z4\n  Recovery 4 min · Z1\nCool-down 22 min · Z1",
            "class, extent and parsed zone only, never a label or note"
        );
        let steps = steps(&body);
        assert_eq!(steps[0]["description"], "Z2");
        assert_eq!(steps[1]["steps"][0]["description"], "Z4");
        assert_eq!(steps[2]["description"], "Z1");
    }

    /// The run behind carnet#1532, as the plan stored it.
    fn hill_repeats() -> PlannedSession {
        let mut session = loaded_session();
        session.sport = SportType::Run;
        session.duration_seconds = Some(4020);
        session.steps = vec![
            step("Warm-up", 15, "Z2", 1, None),
            step("Climb", 4, "tempo", 6, Some("steep hill, controlled")),
            step("Walk down", 3, "recovery", 6, None),
            step("Cool-down", 10, "Z1", 1, None),
        ];
        session
    }

    #[test]
    fn a_run_reaches_garmin_with_its_structure_in_the_notes_and_heart_rate_targets() {
        let body = body(&hill_repeats());
        assert_eq!(body["workoutName"], "Run 1 h 07 · 6×4 min");
        assert_eq!(
            body["description"],
            "Warm-up 15 min · Z2\n6×\n  Tempo 4 min · Z3\n  Recovery 3 min · Z1\nCool-down 10 min · Z1"
        );
        let steps = steps(&body);
        let targets: Vec<(Value, Value)> = [
            &steps[0],
            &steps[1]["steps"][0],
            &steps[1]["steps"][1],
            &steps[2],
        ]
        .iter()
        .map(|step| (step["targetType"].clone(), step["targetValue"].clone()))
        .collect();
        assert_eq!(
            targets,
            vec![
                (json!("HEART_RATE"), json!(2)),
                (json!("HEART_RATE"), json!(3)),
                (json!("HEART_RATE"), json!(1)),
                (json!("HEART_RATE"), json!(1)),
            ]
        );
        assert!(
            !body.to_string().contains("steep hill"),
            "a step's note never leaves Dravr"
        );
    }

    #[test]
    fn a_run_zone_names_the_heart_rate_zone_it_sits_nearest_and_faster_than_z5_goes_open() {
        for sport in [
            SportType::Run,
            SportType::TrailRunning,
            SportType::VirtualRun,
        ] {
            let mut session = loaded_session();
            session.sport = sport;
            session.steps = ["Z1", "Z2", "Z3", "Z4", "Z5", "Z6", "Z7"]
                .iter()
                .map(|zone| step("s", 5, zone, 1, None))
                .collect();
            let steps = steps(&body(&session));
            for (index, hr) in (1..=5).enumerate() {
                assert_eq!(steps[index]["targetType"], "HEART_RATE", "Z{hr}");
                assert_eq!(steps[index]["targetValue"], hr, "Z{hr}");
            }
            assert_eq!(
                steps[5]["targetType"], "OPEN",
                "Z6 is over before HR follows"
            );
            assert_eq!(
                steps[6]["targetType"], "OPEN",
                "Z7 is over before HR follows"
            );
        }
        let mut walk = loaded_session();
        walk.sport = SportType::Walk;
        walk.steps = vec![step("s", 30, "Z2", 1, None)];
        assert_eq!(
            steps(&body(&walk))[0]["targetType"],
            "OPEN",
            "only a run's zones map onto heart rate"
        );
    }

    #[test]
    fn a_description_longer_than_garmin_keeps_ends_at_a_whole_line() {
        let line = "Threshold 1 min · 95-100% threshold HR";
        let mut session = loaded_session();
        session.steps = (0..MAX_STEPS)
            .map(|_| step("s", 1, "95-100% threshold HR", 1, None))
            .collect();
        let description = body(&session)["description"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_default();
        let length = description.chars().count();
        assert!(length <= MAX_DESCRIPTION_CHARS, "{length} chars");
        assert!(
            length + 1 + line.chars().count() > MAX_DESCRIPTION_CHARS,
            "bug: the cut left room for another whole line ({length} chars)"
        );
        assert!(description.ends_with("\n…"), "{description}");
        let kept: Vec<&str> = description.lines().collect();
        assert!(kept.len() > 1, "bug: no whole line kept: {description}");
        assert!(
            kept[..kept.len() - 1].iter().all(|whole| *whole == line),
            "no line is cut through: {description}"
        );
    }

    #[test]
    fn a_session_without_steps_carries_no_description() {
        let mut session = loaded_session();
        session.steps.clear();
        let body = body(&session);
        assert!(
            body.get("description").is_none(),
            "bug: a stepless session sent a description: {body}"
        );
    }

    #[test]
    fn the_steps_garmin_holds_are_counted_from_its_answer() {
        let sent = body(&loaded_session());
        assert_eq!(
            sent_step_count(&sent),
            Some(5),
            "3 steps, 1 block, 2 inside"
        );
        let held: StoredWorkout = serde_json::from_value(json!({
            "workoutId": 41,
            "segments": sent["segments"].clone(),
        }))
        .unwrap_or_else(|e| panic!("stored workout: {e}"));
        assert_eq!(
            held.held_steps(),
            Some(5),
            "every step Garmin answered with"
        );
        assert_eq!(
            held.dropped_steps(Some(5)),
            None,
            "a workout kept whole is not warned about"
        );
        let emptied: StoredWorkout =
            serde_json::from_value(json!({"workoutId": 41, "segments": [{"steps": []}]}))
                .unwrap_or_else(|e| panic!("stored workout: {e}"));
        assert_eq!(emptied.held_steps(), Some(0), "segments with no steps");
        assert_eq!(
            emptied.dropped_steps(Some(5)),
            Some((5, 0)),
            "a workout stored without its steps is caught"
        );
        let bare: StoredWorkout = serde_json::from_value(json!({"workoutId": 41}))
            .unwrap_or_else(|e| panic!("stored workout: {e}"));
        assert_eq!(bare.held_steps(), None, "no segments says nothing");
        assert_eq!(
            bare.dropped_steps(Some(5)),
            None,
            "an answer without segments is not warned about"
        );
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
            step("Tempo", 20, "95-100% threshold HR", 1, None),
            step("Cruise", 10, "90-95% threshold pace", 1, None),
            step("Strides", 1, "RPE 9", 6, None),
            step("Aerobic", 20, "Z2 HR", 1, None),
        ];
        let steps = steps(&body(&session));
        assert_eq!(steps[0]["targetType"], "OPEN");
        assert_eq!(steps[1]["targetType"], "OPEN");
        assert_eq!(steps[2]["steps"][0]["targetType"], "OPEN");
        assert_eq!(steps[3]["targetType"], "HEART_RATE");
        assert_eq!(steps[3]["targetValue"], 2);
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
        let stored: StoredWorkout =
            serde_json::from_value(json!({"workoutId": 41, "ownerId": 9_000}))
                .unwrap_or_else(|e| panic!("stored workout: {e}"));
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
