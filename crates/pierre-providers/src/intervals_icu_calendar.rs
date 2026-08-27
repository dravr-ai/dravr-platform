// ABOUTME: Intervals.icu calendar rendering — a PlannedSession into the EventEx body, steps into the workout text DSL
// ABOUTME: Pure functions and wire shapes; the provider's four calendar-write methods call event_body and read the responses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu calendar rendering
//!
//! Everything between a provider-agnostic [`PlannedSession`] and the JSON
//! Intervals.icu accepts on `POST /events` and `PUT /events/{id}`: the event
//! category and `type`, the `external_id`, `moving_time`, and the description
//! — coach prose first (escaped so it cannot be read as steps), then the
//! steps in Intervals.icu's workout text DSL (`- Warm-up 10m Z2`, `4x`
//! repeat headers), with targets resolved through [`RelativeIntensity`] so a
//! zone stays a zone and the athlete's own thresholds apply on the calendar.
//! The DSL is parsed server-side on every write and cannot be disabled, which
//! is why prose is escaped rather than trusted.

use serde::Deserialize;
use serde_json::json;

use crate::errors::{AppError, AppResult};
use crate::models::{
    PlannedSession, PlannedSessionKind, RelativeIntensity, SportType, WorkoutStep,
};

/// Map a [`SportType`] to the Intervals.icu calendar event `type` string
/// (Intervals.icu uses Strava's activity-type vocabulary). Anything without a
/// named type goes out as the generic `Workout`.
fn intervals_event_type(sport: &SportType) -> &'static str {
    match sport {
        SportType::Ride => "Ride",
        SportType::VirtualRide => "VirtualRide",
        SportType::EbikeRide => "EBikeRide",
        SportType::MountainBike => "MountainBikeRide",
        SportType::GravelRide => "GravelRide",
        SportType::Run => "Run",
        SportType::VirtualRun => "VirtualRun",
        SportType::TrailRunning => "TrailRun",
        SportType::Swim => "Swim",
        SportType::Walk => "Walk",
        SportType::Hike => "Hike",
        SportType::Yoga => "Yoga",
        SportType::StrengthTraining => "WeightTraining",
        SportType::Rowing => "Rowing",
        SportType::CrossCountrySkiing => "NordicSki",
        SportType::AlpineSkiing => "AlpineSki",
        _ => "Workout",
    }
}

/// Event category for a training session.
const EVENT_CATEGORY_WORKOUT: &str = "WORKOUT";

/// Event category for a note pinned to the calendar.
const EVENT_CATEGORY_NOTE: &str = "NOTE";

/// Which target family the step DSL resolves a zone against for a sport.
///
/// Intervals.icu resolves `Z2` against the athlete's own power zones, `Z2 HR`
/// against their heart-rate zones and `Z2 Pace` against their pace zones, so a
/// relative zone stays relative — the athlete's thresholds apply underneath,
/// and an FTP retest never invalidates a pushed session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetFamily {
    /// Power, for every cycling discipline.
    Power,
    /// Pace, for running and swimming.
    Pace,
    /// Heart rate, for everything else.
    HeartRate,
}

fn target_family(sport: &SportType) -> TargetFamily {
    match sport {
        SportType::Ride
        | SportType::VirtualRide
        | SportType::EbikeRide
        | SportType::MountainBike
        | SportType::GravelRide => TargetFamily::Power,
        SportType::Run | SportType::VirtualRun | SportType::TrailRunning | SportType::Swim => {
            TargetFamily::Pace
        }
        _ => TargetFamily::HeartRate,
    }
}

fn zone_target(n: u8, family: TargetFamily) -> String {
    match family {
        TargetFamily::Power => format!("Z{n}"),
        TargetFamily::Pace => format!("Z{n} Pace"),
        TargetFamily::HeartRate => format!("Z{n} HR"),
    }
}

/// A percentage band in the DSL's own spelling (`75%`, `88-93%`), suffixed
/// with the family so pace and heart-rate targets are not read as power.
fn percent_target(low: u16, high: u16, family: TargetFamily) -> String {
    let band = if low == high {
        format!("{low}%")
    } else {
        format!("{low}-{high}%")
    };
    match family {
        TargetFamily::Power => band,
        TargetFamily::Pace => format!("{band} Pace"),
        TargetFamily::HeartRate => format!("{band} HR"),
    }
}

/// Render a step's `target_zone` as an Intervals.icu target, or `None` when
/// the label is outside [`RelativeIntensity`]'s grammar — the step then goes
/// out as a timed step with no target rather than a wrong one.
fn dsl_target(target_zone: &str, family: TargetFamily) -> Option<String> {
    RelativeIntensity::parse(target_zone).map(|intensity| match intensity {
        RelativeIntensity::Zone(n) => zone_target(n, family),
        RelativeIntensity::HeartRateZone(n) => zone_target(n, TargetFamily::HeartRate),
        RelativeIntensity::SweetSpot => match family {
            TargetFamily::Power => percent_target(88, 94, family),
            other => zone_target(3, other),
        },
        RelativeIntensity::Percent { low, high } => percent_target(low, high, family),
    })
}

/// A step's extent in DSL units: a distance when the step has one (`mtr`, not
/// `m` — in this DSL `m` is minutes), otherwise its duration.
fn dsl_extent(step: &WorkoutStep) -> String {
    if let Some(distance) = step.distance_meters {
        if distance >= 1000.0 {
            let km = format!("{:.2}", distance / 1000.0);
            return format!("{}km", km.trim_end_matches('0').trim_end_matches('.'));
        }
        return format!("{}mtr", distance.round() as u32);
    }
    let minutes = step.duration_seconds / 60;
    let seconds = step.duration_seconds % 60;
    match (minutes, seconds) {
        (0, s) => format!("{s}s"),
        (m, 0) => format!("{m}m"),
        (m, s) => format!("{m}m{s}s"),
    }
}

/// One DSL step line: the label is the cue Intervals.icu shows on the device,
/// then the extent, then the target when the zone label resolves to one.
fn dsl_step_line(step: &WorkoutStep, family: TargetFamily) -> String {
    let mut line = format!("- {} {}", step.label.trim(), dsl_extent(step));
    if let Some(target) = dsl_target(&step.target_zone, family) {
        line.push(' ');
        line.push_str(&target);
    }
    line
}

/// Render the steps as DSL text. Consecutive steps sharing a `repeat` above
/// one form one `{n}x` block (the DSL's repeat header groups the lines that
/// follow it until a blank line), so a 4×(8 min on / 4 min off) set renders as
/// a single four-repeat block rather than eight lines.
fn render_steps(steps: &[WorkoutStep], family: TargetFamily) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut open_repeat: Option<u32> = None;
    for step in steps {
        let line = dsl_step_line(step, family);
        if step.repeat > 1 {
            if open_repeat != Some(step.repeat) {
                if !out.is_empty() {
                    out.push(String::new());
                }
                out.push(format!("{}x", step.repeat));
                open_repeat = Some(step.repeat);
            }
        } else if open_repeat.take().is_some() {
            out.push(String::new());
        }
        out.push(line);
    }
    out.join("\n")
}

/// Keep a prose line from being read as DSL: a leading `-` starts a step and
/// a trailing `{n}x` opens a repeat block, and Intervals.icu parses the
/// description on every write with no way to opt out. The prose keeps its
/// words; only the two markers change shape.
fn escape_prose_line(line: &str) -> String {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('-') {
        return format!("– {}", rest.trim_start());
    }
    let ends_with_repeat = trimmed
        .strip_suffix('x')
        .map(|head| head.chars().rev().take_while(char::is_ascii_digit).count())
        .is_some_and(|digits| digits > 0);
    if ends_with_repeat {
        return format!("{trimmed}.");
    }
    line.to_owned()
}

/// The event description: the coach's prose first (session notes, then each
/// step's cue as `label: note`), a blank line, then the step DSL. Either half
/// may be absent; a prose-only session is a timed entry the athlete reads, a
/// steps-only one is pure structure.
fn render_description(session: &PlannedSession) -> String {
    let mut prose: Vec<String> = session
        .notes
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(escape_prose_line)
        .collect();
    for step in &session.steps {
        if let Some(note) = step.note.as_deref().map(str::trim) {
            if !note.is_empty() {
                prose.push(escape_prose_line(&format!("{}: {note}", step.label.trim())));
            }
        }
    }
    let steps = render_steps(&session.steps, target_family(&session.sport));
    match (prose.is_empty(), steps.is_empty()) {
        (true, true) => String::new(),
        (false, true) => prose.join("\n"),
        (true, false) => steps,
        (false, false) => format!("{}\n\n{steps}", prose.join("\n")),
    }
}

/// The `EventEx` body Intervals.icu expects for a session.
pub fn event_body(session: &PlannedSession) -> serde_json::Value {
    let start_date_local = format!("{}T00:00:00", session.date.format("%Y-%m-%d"));
    match session.kind {
        PlannedSessionKind::Workout => {
            let mut body = json!({
                "category": EVENT_CATEGORY_WORKOUT,
                "start_date_local": start_date_local,
                "type": intervals_event_type(&session.sport),
                "name": session.name,
                "description": render_description(session),
                "external_id": session.external_id,
            });
            if let Some(seconds) = session.duration_seconds {
                body["moving_time"] = json!(seconds);
            }
            body
        }
        PlannedSessionKind::WeekNote => json!({
            "category": EVENT_CATEGORY_NOTE,
            "start_date_local": start_date_local,
            "name": session.name,
            "description": render_description(session),
            "external_id": session.external_id,
            "for_week": true,
        }),
    }
}

/// An Intervals.icu event id as it appears in a URL path. Provider ids are the
/// integers the create call returned; anything else never came from this
/// provider and must not be interpolated into a path.
pub fn event_id_segment(provider_event_id: &str) -> AppResult<i64> {
    provider_event_id.trim().parse::<i64>().map_err(|_| {
        AppError::invalid_input(format!(
            "'{provider_event_id}' is not an intervals.icu event id"
        ))
    })
}

/// Minimal shape of the Intervals.icu calendar event create response — we
/// only need the generated event id to record against the ledger row.
#[derive(Debug, Deserialize)]
pub struct CreatedEvent {
    pub id: i64,
}

/// Shape of the `PUT /events/bulk-delete` response.
#[derive(Debug, Deserialize)]
pub struct DeleteEventsResponse {
    #[serde(rename = "eventsDeleted", default)]
    pub events_deleted: u64,
}
