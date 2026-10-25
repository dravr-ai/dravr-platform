// ABOUTME: Intervals.icu planned workouts — the calendar's WORKOUT and race events as PlannedWorkout, and the activities that completed them
// ABOUTME: Pure wire shapes and mapping; the provider reads GET /events and GET /activities?fields=id,paired_event_id and hands both here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu planned workouts
//!
//! What a human coach, an applied training plan or the athlete put on the
//! athlete's intervals.icu calendar, read back as [`PlannedWorkout`]s:
//!
//! - **Which events.** `WORKOUT` events, and `RACE_A` / `RACE_B` / `RACE_C`
//!   ones as a workout titled with the race's priority ("A race: …"). Dravr's
//!   own pushes (an `external_id` [`CalendarKey::is_dravr_key`] knows) are left out:
//!   they are the athlete's Dravr plan, not something planned elsewhere.
//! - **Who reads** ([`PlannedReader`]). An event the athlete's coach hides
//!   from the athlete is left out of the athlete's own read, through their
//!   own key or a delegated link. Their human coach, reading the calendar
//!   with the coach's own key, sees it, titled "Hidden from athlete: …" so it
//!   is never taken for a session on the athlete's calendar: the coach's key
//!   shows it on Intervals.icu all the same.
//! - **Planned-vs-done.** The pairing lives on the activity, not the event:
//!   `GET /activities?fields=id,paired_event_id` names the event each activity
//!   completed, and [`completed_by_event`] inverts it. A failed pairing read
//!   fails the planned read: an absent `completed_activity_id` means "not
//!   done", so it is never the answer to a read that did not happen.
//! - **Origin.** A plan is keyed on its provider alone (carnet#1001): an
//!   event another platform synced into intervals.icu reads as intervals.icu's.
//! - **Lenient decoding.** The events arrive as one array, so each entry is
//!   decoded on its own and each field reads as absent when its value is
//!   `null` or of an unexpected type: one odd entry never costs the athlete
//!   the rest of their calendar. The hidden-from-athlete flag is the one
//!   field read closed: only an absent, `null` or `false` value reads as
//!   shown, so an odd value never reveals a coach's draft to the athlete, and
//!   labels it hidden on the coach's read.
//!
//! Titles and descriptions cross verbatim: they are untrusted third-party
//! text, which the planned-workouts tool fences before a model reads it.

use std::collections::HashMap;

use chrono::{Days, NaiveDate};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use tracing::warn;

use crate::constants::oauth::INTERVALS_ICU;
use crate::intervals_icu_calendar::event_sport;
use crate::models::{CalendarKey, PlannedWorkout, PlannedWorkoutBuilder};

/// The `fields` a pairing read asks `GET /activities` for: the activity's id
/// and the planned event it completed.
pub(crate) const PAIRING_FIELDS: &str = "id,paired_event_id";

/// The words a hidden event's title starts with on a coach's read.
const HIDDEN_LABEL: &str = "Hidden from athlete";

/// Who reads a planned calendar, which decides what it shows of the events
/// the athlete's coach keeps hidden from the athlete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlannedReader {
    /// The athlete whose calendar it is, through their own key or a
    /// delegated link: an event hidden from them is left out.
    Athlete,
    /// The athlete's human coach, through the coach's own key: an event
    /// hidden from the athlete is kept, titled with [`HIDDEN_LABEL`].
    Coach,
}

/// What a planned read keeps of an event's category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// A planned session.
    Workout,
    /// A race, by the words its title is prefixed with.
    Race(&'static str),
}

/// The categories a planned read keeps, in the spelling Intervals.icu uses
/// both for the `category` query and on each event. The one table both the
/// query and the mapping read, so the two cannot disagree.
const PLANNED_CATEGORIES: [(&str, Entry); 4] = [
    ("WORKOUT", Entry::Workout),
    ("RACE_A", Entry::Race("A race")),
    ("RACE_B", Entry::Race("B race")),
    ("RACE_C", Entry::Race("C race")),
];

/// The `category` query of a planned read: every kept category,
/// comma-separated.
pub(crate) fn planned_categories() -> String {
    PLANNED_CATEGORIES.map(|(category, _)| category).join(",")
}

impl Entry {
    /// The entry an event of `category` is, or `None` for a category a
    /// planned read leaves out (or an event that states none).
    fn of(category: Option<&str>) -> Option<Self> {
        let category = category?.trim();
        PLANNED_CATEGORIES
            .iter()
            .find(|(name, _)| *name == category)
            .map(|(_, entry)| *entry)
    }

    /// The title the event reads under: its name, a race's prefixed with the
    /// race's priority.
    fn title(self, name: Option<&str>) -> String {
        let name = name.map(str::trim).filter(|name| !name.is_empty());
        match (self, name) {
            (Self::Workout, name) => name.unwrap_or_default().to_owned(),
            (Self::Race(label), Some(name)) => format!("{label}: {name}"),
            (Self::Race(label), None) => label.to_owned(),
        }
    }
}

/// `T` when the field's value is one, else `None`: a `null` or a value of
/// another type reads as absent rather than failing the entry. Shared by
/// every Intervals.icu wire shape decoded entry by entry (the planned events,
/// the sport settings).
pub(crate) fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    Ok(serde_json::from_value(Value::deserialize(deserializer)?).ok())
}

/// One entry of `GET /api/v1/athlete/{id}/events` (the `Event` schema of the
/// Intervals.icu `OpenAPI` spec), the fields a planned read uses. Every field
/// is optional and read through [`lenient`], save `hide_from_athlete`, kept
/// raw so [`PlannedEvent::kept`] reads it closed.
#[derive(Debug, Deserialize)]
struct IntervalsIcuPlannedEvent {
    #[serde(default, deserialize_with = "lenient")]
    id: Option<i64>,
    #[serde(default, deserialize_with = "lenient")]
    category: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    description: Option<String>,
    /// The activity type the event is planned as (`Ride`, `Run`, …).
    #[serde(default, rename = "type", deserialize_with = "lenient")]
    event_type: Option<String>,
    /// The athlete-local date (and time) the event is on.
    #[serde(default, deserialize_with = "lenient")]
    start_date_local: Option<String>,
    /// Planned moving time, in seconds.
    #[serde(default, deserialize_with = "lenient")]
    moving_time: Option<u64>,
    /// Planned distance, in metres.
    #[serde(default, deserialize_with = "lenient")]
    distance: Option<f64>,
    /// The athlete's coach keeps the event off the athlete's calendar: the
    /// raw value, since any value but `false` hides the event.
    #[serde(default)]
    hide_from_athlete: Option<Value>,
    /// The writer's own key for the event, when one was set.
    #[serde(default, deserialize_with = "lenient")]
    external_id: Option<String>,
}

/// A calendar event a planned read keeps: a workout or a race with an id and
/// a civil date, not Dravr's own, and shown to its reader.
#[derive(Debug)]
pub(crate) struct PlannedEvent {
    id: i64,
    date: NaiveDate,
    entry: Entry,
    /// The coach keeps it hidden from the athlete; kept only on the coach's
    /// read.
    hidden: bool,
    event: IntervalsIcuPlannedEvent,
}

// LIMITATION(registre#1036): `planned_events` keeps WORKOUT and RACE_A/B/C events only, and
// `PlannedEvent::into_workout` maps no `workout_doc` steps, no `compliance`, no entry kind and
// no planned load: NOTE and TARGET entries never reach the agent, and an intervals.icu workout
// reads as its title, sport, duration, distance and description. cageux `PlannedWorkout` has no
// kind or compliance field, `workout_doc`'s step shape is undocumented, and planned load needs a
// Methodology rule.
/// The events of a planned read's raw `GET /events` array that a planned
/// calendar shows `reader`, in the order they arrived.
///
/// An entry that is not an event object, or an event with no id or no civil
/// date, is left out with a warning naming no content; one outside
/// [`PLANNED_CATEGORIES`] (the upstream filter is not relied on), one of
/// Dravr's own pushes, and, on the athlete's read, one hidden from the
/// athlete are left out silently.
pub(crate) fn planned_events(entries: Vec<Value>, reader: PlannedReader) -> Vec<PlannedEvent> {
    entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            serde_json::from_value::<IntervalsIcuPlannedEvent>(entry).map_or_else(
                |_| {
                    warn!(
                        index,
                        "intervals_icu planned read left out an entry that is not an event"
                    );
                    None
                },
                |event| PlannedEvent::kept(event, reader),
            )
        })
        .collect()
}

impl PlannedEvent {
    /// `event` when a planned calendar shows it to `reader`.
    fn kept(event: IntervalsIcuPlannedEvent, reader: PlannedReader) -> Option<Self> {
        let entry = Entry::of(event.category.as_deref())?;
        // Read closed: a flag of any other type than `false` is a coach's
        // draft, left out of the athlete's read and labelled on the coach's.
        let hidden = !matches!(event.hide_from_athlete, None | Some(Value::Bool(false)));
        if (hidden && reader == PlannedReader::Athlete)
            || event
                .external_id
                .as_deref()
                .is_some_and(CalendarKey::is_dravr_key)
        {
            return None;
        }
        let Some(id) = event.id else {
            warn!("intervals_icu planned event has no id; left out of the plan");
            return None;
        };
        let Some(date) = civil_date(event.start_date_local.as_deref()) else {
            warn!(
                event_id = id,
                "intervals_icu planned event has no civil date; left out of the plan"
            );
            return None;
        };
        Some(Self {
            id,
            date,
            entry,
            hidden,
            event,
        })
    }

    /// The event as a [`PlannedWorkout`], completed by the activity
    /// `completed` names for its id, if any. An event hidden from the
    /// athlete, which only the coach's read keeps, is titled as hidden.
    ///
    /// A zero planned duration or distance states nothing and reads as
    /// absent. The start time is not set: Intervals.icu states a naive
    /// athlete-local time, which no UTC instant can be read from without the
    /// athlete's offset.
    pub(crate) fn into_workout(self, completed: &HashMap<i64, String>) -> PlannedWorkout {
        let event = self.event;
        let title = self.entry.title(event.name.as_deref());
        PlannedWorkoutBuilder::new(
            INTERVALS_ICU,
            self.id.to_string(),
            self.date,
            event_sport(event.event_type.as_deref()),
            if self.hidden {
                hidden_title(&title)
            } else {
                title
            },
        )
        .description_opt(event.description.filter(|text| !text.trim().is_empty()))
        .planned_duration_seconds_opt(event.moving_time.filter(|seconds| *seconds > 0))
        .planned_distance_meters_opt(
            event
                .distance
                .filter(|metres| metres.is_finite() && *metres > 0.0),
        )
        .completed_activity_id_opt(completed.get(&self.id).cloned())
        .build()
    }
}

/// `title` as the coach's read shows an event hidden from the athlete.
fn hidden_title(title: &str) -> String {
    if title.is_empty() {
        HIDDEN_LABEL.to_owned()
    } else {
        format!("{HIDDEN_LABEL}: {title}")
    }
}

/// The civil date `start_date_local` begins with (`YYYY-MM-DD`, for a race;
/// a full local date-time for a session).
fn civil_date(start_date_local: Option<&str>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(start_date_local?.get(..10)?, "%Y-%m-%d").ok()
}

/// Whether a window starting `after` can hold a workout already done, as of
/// `today` in UTC: a day of slack covers an athlete whose own calendar runs
/// ahead of UTC. A window wholly in the future spends no pairing request.
pub(crate) fn may_hold_done_workouts(after: NaiveDate, today: NaiveDate) -> bool {
    today
        .checked_add_days(Days::new(1))
        .is_none_or(|tomorrow| after <= tomorrow)
}

/// One activity of `GET /activities?fields=id,paired_event_id`: its id, and
/// the planned event it completed when the calendar paired it. `fields=`
/// leaves null values out, so an unpaired activity carries no
/// `paired_event_id` at all.
#[derive(Debug, Deserialize)]
pub(crate) struct IntervalsIcuActivityPairing {
    id: String,
    #[serde(default)]
    paired_event_id: Option<i64>,
}

/// The activity that completed each planned event, by event id, under the
/// activity id Dravr knows the activity by. When two activities name one
/// event, the first listed wins.
pub(crate) fn completed_by_event(
    pairings: Vec<IntervalsIcuActivityPairing>,
) -> HashMap<i64, String> {
    let mut completed = HashMap::new();
    for pairing in pairings {
        if let Some(event) = pairing.paired_event_id {
            completed.entry(event).or_insert(pairing.id);
        }
    }
    completed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SportType;
    use serde_json::json;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("a calendar date")
    }

    fn read_by(
        reader: PlannedReader,
        entries: Value,
        completed: &HashMap<i64, String>,
    ) -> Vec<PlannedWorkout> {
        let Value::Array(entries) = entries else {
            panic!("an array of events");
        };
        planned_events(entries, reader)
            .into_iter()
            .map(|event| event.into_workout(completed))
            .collect()
    }

    fn workouts(entries: Value, completed: &HashMap<i64, String>) -> Vec<PlannedWorkout> {
        read_by(PlannedReader::Athlete, entries, completed)
    }

    /// Dravr's own pushes, then one event per value the hidden flag can
    /// carry on the wire.
    fn hidden_flag_calendar() -> Value {
        json!([
            { "id": 1, "category": "WORKOUT", "start_date_local": "2026-10-05",
              "external_id": "dravr:plan:4b0f3c1e:2026-10-05:0" },
            { "id": 2, "category": "WORKOUT", "start_date_local": "2026-10-05",
              "external_id": "dravr:rx:9c3e" },
            { "id": 3, "category": "WORKOUT", "start_date_local": "2026-10-05",
              "name": "Draft", "hide_from_athlete": true },
            { "id": 4, "category": "WORKOUT", "start_date_local": "2026-10-05",
              "name": "Shown", "hide_from_athlete": false, "external_id": null },
            { "id": 5, "category": "WORKOUT", "start_date_local": "2026-10-05",
              "name": "Shown", "hide_from_athlete": null },
            { "id": 6, "category": "WORKOUT", "start_date_local": "2026-10-05",
              "name": "Draft", "hide_from_athlete": "yes" },
            { "id": 7, "category": "WORKOUT", "start_date_local": "2026-10-05",
              "name": "Draft", "hide_from_athlete": 1 },
            { "id": 8, "category": "WORKOUT", "start_date_local": "2026-10-05",
              "name": "Draft", "hide_from_athlete": "false" }
        ])
    }

    #[test]
    fn a_workout_event_maps_onto_a_planned_workout() {
        let completed = HashMap::from([(42, "i101".to_owned())]);
        let planned = workouts(
            json!([{
                "id": 42,
                "category": "WORKOUT",
                "name": "Threshold 3x10",
                "description": "Hold the line.\n- 3x10m Z4",
                "type": "Ride",
                "start_date_local": "2026-10-05T00:00:00",
                "moving_time": 3600,
                "distance": 30000.0,
                "icu_training_load": 78,
                "external_id": "tp:123"
            }]),
            &completed,
        );
        assert_eq!(planned.len(), 1);
        let ride = &planned[0];
        assert_eq!(ride.provider(), "intervals_icu");
        assert_eq!(ride.provider_workout_id(), "42");
        assert_eq!(ride.date(), day(5));
        assert_eq!(ride.sport_type(), &SportType::Ride);
        assert_eq!(ride.title(), "Threshold 3x10");
        assert_eq!(ride.description(), Some("Hold the line.\n- 3x10m Z4"));
        assert_eq!(ride.planned_duration_seconds(), Some(3600));
        assert_eq!(ride.planned_distance_meters(), Some(30000.0));
        assert_eq!(ride.completed_activity_id(), Some("i101"));
        assert_eq!(ride.steps(), []);
        assert_eq!(ride.start_time(), None);
        assert_eq!(
            ride.planned_training_stress_score(),
            None,
            "intervals.icu's planned load is on its own scale"
        );
    }

    #[test]
    fn a_race_is_titled_with_its_priority() {
        let planned = workouts(
            json!([
                { "id": 1, "category": "RACE_A", "name": "Ironman Quebec", "type": "Run",
                  "start_date_local": "2026-10-10" },
                { "id": 2, "category": "RACE_B", "name": "  ", "start_date_local": "2026-10-11" },
                { "id": 3, "category": "RACE_C", "start_date_local": "2026-10-12" }
            ]),
            &HashMap::new(),
        );
        let titles: Vec<&str> = planned.iter().map(PlannedWorkout::title).collect();
        assert_eq!(titles, ["A race: Ironman Quebec", "B race", "C race"]);
        assert_eq!(planned[0].date(), day(10));
        assert_eq!(planned[1].sport_type(), &SportType::Workout);
    }

    #[test]
    fn only_workouts_and_races_are_read_whatever_the_upstream_filter_did() {
        let planned = workouts(
            json!([
                { "id": 1, "category": "NOTE", "start_date_local": "2026-10-05" },
                { "id": 2, "category": "TARGET", "start_date_local": "2026-10-05" },
                { "id": 3, "category": "SICK", "start_date_local": "2026-10-05" },
                { "id": 4, "category": "workout", "start_date_local": "2026-10-05" },
                { "id": 5, "start_date_local": "2026-10-05" },
                { "id": 6, "category": null, "start_date_local": "2026-10-05" },
                { "id": 7, "category": "WORKOUT", "start_date_local": "2026-10-05" }
            ]),
            &HashMap::new(),
        );
        let ids: Vec<&str> = planned
            .iter()
            .map(PlannedWorkout::provider_workout_id)
            .collect();
        assert_eq!(ids, ["7"]);
    }

    #[test]
    fn dravrs_own_pushes_and_hidden_events_are_left_out() {
        let planned = workouts(hidden_flag_calendar(), &HashMap::new());
        let ids: Vec<&str> = planned
            .iter()
            .map(PlannedWorkout::provider_workout_id)
            .collect();
        assert_eq!(
            ids,
            ["4", "5"],
            "only an absent, null or false flag shows the event: an odd one keeps the draft hidden"
        );
    }

    #[test]
    fn the_coachs_read_keeps_hidden_events_titled_as_hidden() {
        let planned = read_by(
            PlannedReader::Coach,
            hidden_flag_calendar(),
            &HashMap::from([(3, "i101".to_owned())]),
        );
        let read: Vec<(&str, &str)> = planned
            .iter()
            .map(|workout| (workout.provider_workout_id(), workout.title()))
            .collect();
        assert_eq!(
            read,
            [
                ("3", "Hidden from athlete: Draft"),
                ("4", "Shown"),
                ("5", "Shown"),
                ("6", "Hidden from athlete: Draft"),
                ("7", "Hidden from athlete: Draft"),
                ("8", "Hidden from athlete: Draft"),
            ],
            "Dravr's own pushes stay out; an odd flag reads as hidden, never as shown"
        );
        assert_eq!(
            planned[0].completed_activity_id(),
            Some("i101"),
            "a hidden session the athlete did still names the activity"
        );
    }

    #[test]
    fn a_hidden_race_or_untitled_draft_keeps_the_hidden_label_first() {
        let planned = read_by(
            PlannedReader::Coach,
            json!([
                { "id": 1, "category": "RACE_A", "name": "Ironman Quebec",
                  "start_date_local": "2026-10-10", "hide_from_athlete": true },
                { "id": 2, "category": "WORKOUT", "name": "  ",
                  "start_date_local": "2026-10-11", "hide_from_athlete": true }
            ]),
            &HashMap::new(),
        );
        let titles: Vec<&str> = planned.iter().map(PlannedWorkout::title).collect();
        assert_eq!(
            titles,
            [
                "Hidden from athlete: A race: Ironman Quebec",
                "Hidden from athlete"
            ]
        );
    }

    #[test]
    fn an_event_with_no_id_or_no_civil_date_is_left_out_and_the_rest_kept() {
        let planned = workouts(
            json!([
                { "category": "WORKOUT", "start_date_local": "2026-10-05" },
                { "id": "12", "category": "WORKOUT", "start_date_local": "2026-10-05" },
                { "id": 2, "category": "WORKOUT" },
                { "id": 3, "category": "WORKOUT", "start_date_local": "soon" },
                { "id": 4, "category": "WORKOUT", "start_date_local": null },
                { "id": 5, "category": "WORKOUT", "start_date_local": "2026-10-06T07:30:00" }
            ]),
            &HashMap::new(),
        );
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].provider_workout_id(), "5");
        assert_eq!(planned[0].date(), day(6));
    }

    #[test]
    fn nulls_and_odd_values_read_as_absent_without_failing_the_calendar() {
        let planned = workouts(
            json!([
                "not an event",
                null,
                7,
                { "id": 1, "category": "WORKOUT", "start_date_local": "2026-10-05",
                  "name": null, "description": null, "type": null,
                  "moving_time": null, "distance": null },
                { "id": 2, "category": "WORKOUT", "start_date_local": "2026-10-05",
                  "name": 12, "description": ["x"], "type": { "a": 1 },
                  "moving_time": "3600", "distance": "far", "external_id": 99 },
                { "id": 3, "category": "WORKOUT", "start_date_local": "2026-10-05",
                  "moving_time": 0, "distance": 0.0, "description": "   " },
                { "id": 4, "category": "WORKOUT", "start_date_local": "2026-10-05",
                  "moving_time": -60 }
            ]),
            &HashMap::new(),
        );
        assert_eq!(planned.len(), 4);
        for workout in &planned {
            assert_eq!(workout.title(), "");
            assert_eq!(workout.description(), None);
            assert_eq!(workout.sport_type(), &SportType::Workout);
            assert_eq!(workout.planned_duration_seconds(), None);
            assert_eq!(workout.planned_distance_meters(), None);
        }
    }

    #[test]
    fn the_category_query_names_every_kept_category() {
        assert_eq!(planned_categories(), "WORKOUT,RACE_A,RACE_B,RACE_C");
        for (category, _) in PLANNED_CATEGORIES {
            assert!(Entry::of(Some(category)).is_some(), "{category}");
        }
    }

    #[test]
    fn the_first_activity_paired_with_an_event_completes_it() {
        let pairings: Vec<IntervalsIcuActivityPairing> = serde_json::from_value(json!([
            { "id": "i101", "paired_event_id": 42 },
            { "id": "i102" },
            { "id": "i103", "paired_event_id": null },
            { "id": "i104", "paired_event_id": 42 },
            { "id": "i105", "paired_event_id": 43 }
        ]))
        .expect("the null-excluded pairing shape decodes");
        let completed = completed_by_event(pairings);
        assert_eq!(
            completed,
            HashMap::from([(42, "i101".to_owned()), (43, "i105".to_owned())])
        );
    }

    #[test]
    fn a_pairing_read_runs_only_for_a_window_that_can_hold_a_done_workout() {
        let today = day(15);
        assert!(may_hold_done_workouts(day(1), today));
        assert!(may_hold_done_workouts(day(15), today));
        assert!(
            may_hold_done_workouts(day(16), today),
            "an athlete east of UTC is already on the 16th"
        );
        assert!(!may_hold_done_workouts(day(17), today));
    }
}
