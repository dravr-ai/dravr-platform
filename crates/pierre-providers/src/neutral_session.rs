// ABOUTME: Neutral rendering shared by the calendars whose terms take rights over what Dravr writes (Wahoo, Garmin)
// ABOUTME: A session's title from its structure and each step's class from a closed vocabulary — never the coach's words

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Neutral session rendering
//!
//! Wahoo and Garmin both take broad rights over anything Dravr writes into
//! them, so what reaches their calendars is generated from a session's
//! structure alone (carnet#34, carnet#1385):
//!
//! - the title is the sport, the planned length and the main set
//!   ([`neutral_title`]), never the session's own title, which is the first
//!   clause of the agent's prescription and can carry a reason;
//! - each step is named by its class ([`StepClass`]), read off its label
//!   against a closed vocabulary or off its target, so a step's label or note
//!   never leaves Dravr.

use dravr_cageux::periodization::{band_coordinates, AnchorRatios, SportFamily, ThresholdBasis};
use dravr_cageux::zones::COGGAN_POWER_ZONE_UPPER_FRACTIONS;

use crate::models::{PlannedSession, RelativeIntensity, SportType, WorkoutStep};

/// Sweet spot, as the step grammar defines it: 88–94 % FTP.
pub(crate) const SWEET_SPOT: (f64, f64) = (0.88, 0.94);

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
pub(crate) fn session_seconds(session: &PlannedSession) -> Option<u64> {
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

/// The title a neutral calendar shows for a session, generated from
/// structure alone: sport, duration and main set (`Ride 1 h 30 · 4×8 min`).
#[must_use]
pub(crate) fn neutral_title(session: &PlannedSession) -> String {
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

/// A step's class: what a neutral calendar calls the step, read from a
/// closed vocabulary so nothing of the step's own words leaves Dravr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepClass {
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

impl StepClass {
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
        let zone = COGGAN_POWER_ZONE_UPPER_FRACTIONS
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
    pub(crate) fn of_step(step: &WorkoutStep, intensity: Option<&RelativeIntensity>) -> Self {
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
            Some(RelativeIntensity::Percent {
                low,
                high,
                threshold:
                    None | Some(ThresholdBasis::Ftp | ThresholdBasis::HeartRate | ThresholdBasis::Pace),
            }) => Self::of_percent(*low, *high),
            Some(
                intensity @ (RelativeIntensity::Percent { .. } | RelativeIntensity::Lactate { .. }),
            ) => Self::of_coordinates(*intensity),
            Some(RelativeIntensity::Rpe(range)) => Self::of_rpe(range.max),
            None => Self::Active,
        }
    }

    /// A band the threshold classes do not read directly — an anchor they do
    /// not know (CP, CS, MAP, `vVO2max`, `HRmax`, HRR, MMP) or a blood-lactate
    /// band — placed on the CP or CS line with the kernel's default
    /// conversions and classed as the same percent of threshold would be. A
    /// speed basis (CS, `vVO2max`) is read on the CS line, everything else on
    /// the CP line.
    fn of_coordinates(intensity: RelativeIntensity) -> Self {
        let family = match intensity {
            RelativeIntensity::Percent {
                threshold: Some(ThresholdBasis::CriticalSpeed | ThresholdBasis::VVo2max),
                ..
            } => SportFamily::Run,
            _ => SportFamily::Ride,
        };
        band_coordinates(intensity, family, &AnchorRatios::default())
            .map_or(Self::Active, |(lo, hi)| {
                Self::of_percent(lo.round() as u16, hi.round() as u16)
            })
    }
}

/// Whether a sport's targets are power, so a zone is a power zone and a
/// bare percent is of FTP.
pub(crate) const fn is_power_sport(sport: &SportType) -> bool {
    matches!(
        sport,
        SportType::Ride
            | SportType::VirtualRide
            | SportType::EbikeRide
            | SportType::MountainBike
            | SportType::GravelRide
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PlannedSessionKind;
    use chrono::NaiveDate;

    fn step(label: &str, minutes: u32, zone: &str, repeat: u32) -> WorkoutStep {
        WorkoutStep {
            label: label.to_owned(),
            duration_seconds: minutes * 60,
            distance_meters: None,
            target_zone: zone.to_owned(),
            repeat,
            repeat_group: (repeat > 1).then_some(1),
            note: None,
        }
    }

    #[test]
    fn a_step_is_classed_by_a_closed_vocabulary_then_by_its_target() {
        let classify = |label: &str, zone: &str| {
            let step = step(label, 5, zone, 1);
            StepClass::of_step(&step, RelativeIntensity::parse(zone).as_ref())
        };
        assert_eq!(classify("Julie's warm-up", "Z2"), StepClass::Warmup);
        assert_eq!(classify("Retour au calme", "Z1"), StepClass::Cooldown);
        assert_eq!(classify("Threshold block", "Z4"), StepClass::Threshold);
        assert_eq!(classify("Over-unders", "88-94% FTP"), StepClass::Threshold);
        assert_eq!(classify("Steady state", "80% FTP"), StepClass::Tempo);
        assert_eq!(classify("Sweet spot", "sweet spot"), StepClass::Tempo);
        assert_eq!(classify("Strides", "RPE 9"), StepClass::Vo2Max);
        assert_eq!(classify("Easy", "chatty pace"), StepClass::Active);
    }

    #[test]
    fn an_anchor_or_lactate_band_is_classed_where_the_kernel_places_it() {
        let classify = |zone: &str| {
            let step = step("Work", 5, zone, 1);
            StepClass::of_step(&step, RelativeIntensity::parse(zone).as_ref())
        };
        // CP and CS are the threshold of their line.
        assert_eq!(classify("100% CP"), StepClass::Threshold);
        assert_eq!(classify("95-100% CS"), StepClass::Threshold);
        // 90-95 % of maximum heart rate sits between LT2 and the aerobic
        // maximum, 108-126 % of CP.
        assert_eq!(classify("90-95% HRmax"), StepClass::Vo2Max);
        // 2 and 4 mmol/L are LT1 and CP on the kernel's lactate line.
        assert_eq!(classify("2.0 mmol/L"), StepClass::Active);
        assert_eq!(classify("4.0 mmol/L"), StepClass::Threshold);
    }

    #[test]
    fn the_title_names_sport_length_and_main_set() {
        let session = PlannedSession {
            external_id: "k".to_owned(),
            kind: PlannedSessionKind::Workout,
            date: NaiveDate::from_ymd_opt(2026, 10, 8).unwrap_or_default(),
            sport: SportType::Run,
            name: "Coach's words".to_owned(),
            duration_seconds: None,
            notes: String::new(),
            steps: vec![step("w", 15, "Z2", 1), step("i", 3, "Z5", 5)],
        };
        assert_eq!(neutral_title(&session), "Run 30 min · 5×3 min");
    }
}
