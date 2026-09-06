// ABOUTME: Pure tests for the plan → calendar mapping: sport aliases, day rendering, desired-entry keys, the ledger diff
// ABOUTME: Also pins the RelativeIntensity grammar and the CalendarKey formats every calendar write depends on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The reconciler's decisions, judged without a provider or a database:
//! which plan days become which calendar entries under which keys, and what
//! a push would create, update, or remove against a given ledger.

use chrono::{NaiveDate, Utc};
use pierre_core::models::{
    CalendarEventSource, CalendarKey, PlannedSession, PlannedSessionKind, PrescribedWorkout,
    RelativeIntensity, SportType, WorkoutStep,
};
use pierre_memory::training_plans::{PlanWeek, PlannedDay, WeekStatus};
use pierre_services::plan_calendar_push::{
    desired_entries, diff_against_ledger, plan_day_session, plan_sport, DesiredEntry,
};
use uuid::Uuid;

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn day(
    date: &str,
    sport: &str,
    workout: &str,
    minutes: Option<u32>,
    intensity: &str,
) -> PlannedDay {
    PlannedDay {
        date: date.to_owned(),
        sport: sport.to_owned(),
        workout: workout.to_owned(),
        duration_min: minutes,
        intensity: intensity.to_owned(),
        steps: Vec::new(),
        fueling: None,
        template_slug: None,
        template_params: None,
    }
}

fn rest(date: &str) -> PlannedDay {
    day(date, "rest", "", None, "")
}

/// The steps of a classic threshold session as a coach states them: 15 min
/// warm-up, 3 × (8 min on / 4 min off), 10 min cool-down — 61 minutes.
fn threshold_steps() -> Vec<WorkoutStep> {
    let step = |label: &str, seconds: u32, zone: &str, repeat: u32| WorkoutStep {
        label: label.to_owned(),
        duration_seconds: seconds,
        distance_meters: None,
        target_zone: zone.to_owned(),
        repeat,
        note: None,
    };
    vec![
        step("Warm-up", 900, "Z1", 1),
        step("Work", 480, "88-93% FTP", 3),
        step("Recovery", 240, "Z1", 3),
        step("Cool-down", 600, "Z1", 1),
    ]
}

fn week(id: &str, week_start: &str, focus: &str, days: Vec<PlannedDay>) -> PlanWeek {
    PlanWeek {
        id: id.to_owned(),
        tenant_id: "t".to_owned(),
        user_id: "u".to_owned(),
        plan_id: "p".to_owned(),
        week_start: week_start.to_owned(),
        focus: focus.to_owned(),
        days,
        status: WeekStatus::Active,
        supersedes_id: None,
        adjustment_reason: String::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        phase_index: None,
    }
}

/// A live ledger row as a push leaves one, carrying the entry's own hash.
fn live_row(user_id: Uuid, entry: &DesiredEntry) -> PrescribedWorkout {
    let now = Utc::now();
    PrescribedWorkout {
        id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        user_id,
        coach_id: None,
        template_slug: None,
        sport: entry.session.sport.clone(),
        prescribed_for_date: entry.session.date,
        provider: "intervals_icu".to_owned(),
        provider_event_id: Some("1".to_owned()),
        external_id: Some(entry.session.external_id.clone()),
        source: entry.source,
        plan_week_id: Some(entry.plan_week_id.clone()),
        replaces_id: None,
        payload_hash: Some(entry.session.payload_hash().unwrap()),
        payload_json: serde_json::to_string(&entry.session).unwrap(),
        status: PrescribedWorkout::STATUS_PUSHED.to_owned(),
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn sports_are_read_the_way_coaches_write_them() {
    assert_eq!(plan_sport("vélo"), SportType::Ride);
    assert_eq!(plan_sport("Bike"), SportType::Ride);
    assert_eq!(plan_sport("mtb"), SportType::MountainBike);
    assert_eq!(plan_sport("gravel"), SportType::GravelRide);
    assert_eq!(plan_sport("course à pied"), SportType::Run);
    assert_eq!(plan_sport("trail"), SportType::TrailRunning);
    assert_eq!(plan_sport("natation"), SportType::Swim);
    assert_eq!(plan_sport("muscu"), SportType::StrengthTraining);
    assert_eq!(plan_sport("yoga"), SportType::Yoga);
    // Canonical snake-case names still resolve, and an unknown label is kept
    // rather than guessed at.
    assert_eq!(
        plan_sport("cross_country_skiing"),
        SportType::CrossCountrySkiing
    );
    assert_eq!(
        plan_sport("curling"),
        SportType::Other("curling".to_owned())
    );
}

#[test]
fn a_zoned_day_becomes_one_timed_step_and_a_prose_day_stays_prose() {
    let user = Uuid::new_v4();
    let zoned = plan_day_session(
        user,
        &day(
            "2026-09-07",
            "vélo",
            "2h endurance, low HR on climbs",
            Some(120),
            "Z2",
        ),
        0,
    )
    .expect("a training day renders");
    assert_eq!(zoned.kind, PlannedSessionKind::Workout);
    assert_eq!(zoned.sport, SportType::Ride);
    assert_eq!(zoned.name, "2h endurance, low HR on climbs");
    assert_eq!(zoned.duration_seconds, Some(7200));
    assert_eq!(zoned.notes, "2h endurance, low HR on climbs\nZ2");
    assert_eq!(zoned.steps.len(), 1, "an in-grammar intensity is one step");
    assert_eq!(zoned.steps[0].duration_seconds, 7200);
    assert_eq!(zoned.steps[0].target_zone, "Z2");
    assert_eq!(
        zoned.steps[0].label,
        SportType::Ride.display_name(),
        "the cue is the sport, never prose that could carry a duration"
    );
    assert_eq!(
        zoned.external_id,
        CalendarKey::plan_day(user, date("2026-09-07"), 0)
    );

    // "3x8min @ 88-93% FTP" is interval structure the plan does not carry as
    // steps, so the day goes out timed and un-targeted rather than as one
    // wrong 60-minute block at 88-93 %.
    let prose = plan_day_session(
        user,
        &day(
            "2026-09-08",
            "run",
            "Intervals. Keep the recoveries easy",
            Some(60),
            "3x8min @ 88-93% FTP",
        ),
        0,
    )
    .unwrap();
    assert!(prose.steps.is_empty());
    assert_eq!(prose.duration_seconds, Some(3600));
    assert_eq!(prose.name, "Intervals", "the title is the first clause");

    // No duration means no step even with a zone, and a rest day is nothing.
    let untimed =
        plan_day_session(user, &day("2026-09-09", "run", "Easy jog", None, "Z2"), 0).unwrap();
    assert!(untimed.steps.is_empty());
    assert_eq!(untimed.duration_seconds, None);
    assert!(plan_day_session(user, &rest("2026-09-10"), 0).is_none());
}

#[test]
fn a_structured_day_renders_its_steps_and_sums_its_duration() {
    let user = Uuid::new_v4();
    let workout = "Threshold 3x8. Keep the recoveries easy";
    let mut structured = day("2026-09-08", "vélo", workout, Some(61), "Z4");
    structured.steps = threshold_steps();
    assert_eq!(WorkoutStep::total_seconds(&structured.steps), 3660);

    let session = plan_day_session(user, &structured, 0).expect("a structured day renders");
    assert_eq!(
        session.steps.len(),
        4,
        "the coach's steps, not one derived from the intensity"
    );
    assert_eq!(
        session.steps[0].label, "Warm-up",
        "the coach's cue, not the sport"
    );
    assert_eq!(session.steps[1].repeat, 3);
    assert_eq!(session.steps[1].target_zone, "88-93% FTP");
    assert_eq!(session.steps[2].duration_seconds, 240);
    assert_eq!(
        session.duration_seconds,
        Some(3660),
        "15 + 3×8 + 3×4 + 10 minutes, from the steps"
    );
    assert_eq!(session.notes, format!("{workout}\nZ4"));
    assert_eq!(session.name, "Threshold 3x8");
    assert_eq!(session.sport, SportType::Ride);

    // Structure is content: the same day pushed as prose has a different
    // hash, so a re-push after the coach adds steps updates the entry —
    // and the prose day still gets its single intensity-derived step.
    let prose =
        plan_day_session(user, &day("2026-09-08", "vélo", workout, Some(61), "Z4"), 0).unwrap();
    assert_eq!(prose.steps.len(), 1);
    assert_eq!(prose.steps[0].label, SportType::Ride.display_name());
    assert_ne!(
        prose.payload_hash().unwrap(),
        session.payload_hash().unwrap()
    );
}

#[test]
fn titles_are_bounded_and_fall_back_to_the_sport() {
    let user = Uuid::new_v4();
    let long = "a".repeat(80);
    let session =
        plan_day_session(user, &day("2026-09-07", "run", &long, Some(30), ""), 0).unwrap();
    assert_eq!(session.name.chars().count(), 60);
    assert!(session.name.ends_with('…'));
    let blank = plan_day_session(user, &day("2026-09-07", "swim", "   ", Some(30), ""), 0).unwrap();
    assert_eq!(blank.name, SportType::Swim.display_name());
}

#[test]
fn desired_entries_skip_the_past_and_rest_and_key_double_days_by_ordinal() {
    let user = Uuid::new_v4();
    let from = date("2026-09-09");
    let weeks = vec![
        week(
            "w1",
            "2026-09-07",
            "Volume back up",
            vec![
                day("2026-09-07", "ride", "Endurance", Some(60), "Z2"),
                day("2026-09-08", "run", "Easy", Some(40), "Z1"),
                rest("2026-09-09"),
                day("2026-09-10", "swim", "Technique", Some(45), ""),
                day("2026-09-10", "ride", "Brick after the swim", Some(60), "Z2"),
                day("2026-09-11", "ride", "Openers", Some(30), "tempo"),
            ],
        ),
        week(
            "w2",
            "2026-09-14",
            "Sharpen",
            vec![
                day("2026-09-14", "ride", "Threshold", Some(75), "threshold"),
                rest("2026-09-15"),
            ],
        ),
        week(
            "w3",
            "2026-09-21",
            "   ",
            vec![day("2026-09-21", "run", "Long", Some(90), "Z2")],
        ),
    ];
    let entries = desired_entries(user, &weeks, from);
    let keys: Vec<&str> = entries
        .iter()
        .map(|e| e.session.external_id.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            // w1 started before `from`, so no week note; its two past days and
            // the rest day are gone; the brick day yields ordinals 0 and 1.
            CalendarKey::plan_day(user, date("2026-09-10"), 0),
            CalendarKey::plan_day(user, date("2026-09-10"), 1),
            CalendarKey::plan_day(user, date("2026-09-11"), 0),
            // w2 starts on or after `from`, so its focus becomes a week note.
            CalendarKey::plan_week_note(user, date("2026-09-14")),
            CalendarKey::plan_day(user, date("2026-09-14"), 0),
            // w3 has a blank focus: no note.
            CalendarKey::plan_day(user, date("2026-09-21"), 0),
        ]
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
    );
    let note = &entries[3];
    assert_eq!(note.source, CalendarEventSource::PlanWeekNote);
    assert_eq!(note.session.kind, PlannedSessionKind::WeekNote);
    assert_eq!(note.session.name, "Sharpen");
    assert_eq!(note.plan_week_id, "w2");
    assert!(entries
        .iter()
        .filter(|e| e.source == CalendarEventSource::PlanDay)
        .all(|e| e.session.date >= from));
}

#[test]
fn the_ledger_diff_counts_creates_updates_unchanged_and_removals() {
    let user = Uuid::new_v4();
    let from = date("2026-09-07");
    let first = desired_entries(
        user,
        &[week(
            "w1",
            "2026-09-07",
            "",
            vec![
                day("2026-09-07", "ride", "Endurance", Some(60), "Z2"),
                day("2026-09-08", "run", "Easy", Some(40), "Z1"),
                day("2026-09-09", "swim", "Technique", Some(45), ""),
            ],
        )],
        from,
    );
    assert_eq!(first.len(), 3);
    let ledger: Vec<PrescribedWorkout> = first.iter().map(|e| live_row(user, e)).collect();

    // Nothing changed: everything is unchanged.
    let same = diff_against_ledger(&first, &ledger).unwrap();
    assert_eq!(
        (same.create, same.update, same.unchanged, same.remove),
        (0, 0, 3, 0)
    );
    assert!(!same.is_stale());

    // The week re-saved: Monday longer, Tuesday now rest, Thursday added.
    let adjusted = desired_entries(
        user,
        &[week(
            "w1b",
            "2026-09-07",
            "",
            vec![
                day("2026-09-07", "ride", "Endurance", Some(90), "Z2"),
                rest("2026-09-08"),
                day("2026-09-09", "swim", "Technique", Some(45), ""),
                day("2026-09-10", "run", "Strides", Some(30), "Z3"),
            ],
        )],
        from,
    );
    let changed = diff_against_ledger(&adjusted, &ledger).unwrap();
    assert_eq!(
        (
            changed.create,
            changed.update,
            changed.unchanged,
            changed.remove
        ),
        (1, 1, 1, 1)
    );
    assert!(changed.is_stale());

    // Rows that are not plan rows never count: a single prescription on the
    // calendar is neither wanted nor unwanted by the plan.
    let mut prescription = live_row(user, &first[0]);
    prescription.source = CalendarEventSource::Prescription;
    prescription.external_id = Some(CalendarKey::prescription(prescription.id));
    let with_rx = diff_against_ledger(&first, &[prescription]).unwrap();
    assert_eq!((with_rx.create, with_rx.remove), (3, 0));
}

#[test]
fn the_intensity_grammar_is_closed() {
    use RelativeIntensity::{HeartRateZone, Percent, SweetSpot, Zone};
    let parsed = |s: &str| RelativeIntensity::parse(s);
    assert_eq!(parsed("Z2"), Some(Zone(2)));
    assert_eq!(parsed(" zone 4 "), Some(Zone(4)));
    assert_eq!(parsed("Z2 HR"), Some(HeartRateZone(2)));
    assert_eq!(parsed("Tempo"), Some(Zone(3)));
    assert_eq!(parsed("Threshold"), Some(Zone(4)));
    assert_eq!(parsed("VO2max"), Some(Zone(5)));
    assert_eq!(parsed("sweet spot"), Some(SweetSpot));
    assert_eq!(parsed("75%"), Some(Percent { low: 75, high: 75 }));
    assert_eq!(parsed("88-93% FTP"), Some(Percent { low: 88, high: 93 }));
    // A pace-family label names what the sport already decides, and an en
    // dash between a band's bounds is the hyphen a keyboard offered.
    assert_eq!(parsed("Z2 pace"), Some(Zone(2)));
    assert_eq!(parsed("zone 3 Pace"), Some(Zone(3)));
    assert_eq!(
        parsed("88\u{2013}93% FTP"),
        Some(Percent { low: 88, high: 93 })
    );
    // Outside the grammar: structure, inverted bands, absolute watts, prose.
    assert_eq!(parsed("3x8min @ 88-93% FTP"), None);
    assert_eq!(parsed("93-88%"), None);
    assert_eq!(parsed("250w"), None);
    assert_eq!(parsed("Z9"), None);
    assert_eq!(parsed("comfortably hard"), None);
    assert_eq!(parsed(""), None);
}

#[test]
fn calendar_keys_name_the_slot_not_the_row() {
    let user = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
    let rx = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
    assert_eq!(
        CalendarKey::plan_day(user, date("2026-09-07"), 1),
        "dravr:plan:11111111-2222-3333-4444-555555555555:2026-09-07:1"
    );
    assert_eq!(
        CalendarKey::plan_week_note(user, date("2026-09-07")),
        "dravr:plan:11111111-2222-3333-4444-555555555555:week:2026-09-07"
    );
    assert_eq!(
        CalendarKey::prescription(rx),
        "dravr:rx:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    );
    assert!(CalendarKey::plan_day(user, date("2026-09-07"), 0)
        .starts_with(&CalendarKey::plan_prefix(user)));
    assert!(!CalendarKey::prescription(rx).starts_with(&CalendarKey::plan_prefix(user)));
}

#[test]
fn the_payload_hash_moves_with_the_content_and_only_the_content() {
    let user = Uuid::new_v4();
    let a = plan_day_session(
        user,
        &day("2026-09-07", "ride", "Endurance", Some(60), "Z2"),
        0,
    )
    .unwrap();
    let b = plan_day_session(
        user,
        &day("2026-09-07", "ride", "Endurance", Some(60), "Z2"),
        0,
    )
    .unwrap();
    let c = plan_day_session(
        user,
        &day("2026-09-07", "ride", "Endurance", Some(90), "Z2"),
        0,
    )
    .unwrap();
    assert_eq!(a.payload_hash().unwrap(), b.payload_hash().unwrap());
    assert_ne!(a.payload_hash().unwrap(), c.payload_hash().unwrap());
    let _: PlannedSession = a;
}
