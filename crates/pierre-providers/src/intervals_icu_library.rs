// ABOUTME: Intervals.icu workout library rendering — a PlannedSession into the library workout body, and the folder listing a save matches against
// ABOUTME: Pure functions and wire shapes; the provider's library save sends the bodies and reads the listing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu workout library
//!
//! The account's own library, as `GET /api/v1/athlete/{id}/folders` lists it:
//! folders and plans, each with the workouts in it. A saved session is a
//! `POST /workouts` body with the same description the calendar gets — the
//! agent's prose, then the steps in the workout text DSL that Intervals.icu
//! parses into structure — plus the folder it goes in.
//!
//! A library workout carries no key of Dravr's, so a save recognises what is
//! already there by name and description alone: the same workout in the same
//! folder is not written twice, and a different workout under the same name
//! stays untouched beside the new one.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::intervals_icu_calendar::{intervals_event_type, render_description};
use crate::models::PlannedSession;

/// The `type` of a folder that holds loose workouts. The other kind, `PLAN`,
/// lays its workouts on days of a schedule, and a template dropped into one
/// would land on a day of somebody's plan.
const FOLDER_TYPE: &str = "FOLDER";

/// One entry of `GET /folders`: the fields a save reads.
#[derive(Debug, Deserialize)]
pub(crate) struct LibraryFolder {
    /// The folder's id, which a workout names as `folder_id`.
    pub(crate) id: i64,
    /// The name the account gave it.
    #[serde(default)]
    pub(crate) name: String,
    /// `FOLDER` or `PLAN`.
    #[serde(rename = "type", default)]
    pub(crate) kind: String,
    /// The workouts in it.
    #[serde(default)]
    pub(crate) children: Vec<LibraryWorkout>,
}

/// One workout in a [`LibraryFolder`]: the fields a save compares.
#[derive(Debug, Deserialize)]
pub(crate) struct LibraryWorkout {
    /// The workout's id.
    pub(crate) id: i64,
    /// Its name.
    #[serde(default)]
    pub(crate) name: String,
    /// Its description: the prose and the step DSL its structure is parsed
    /// from.
    #[serde(default)]
    pub(crate) description: Option<String>,
}

/// The id of a folder or workout a `POST` created.
#[derive(Debug, Deserialize)]
pub(crate) struct CreatedLibraryItem {
    /// The new item's id.
    pub(crate) id: i64,
}

/// The plain folder called `name`, matched the way a person names it: case
/// and surrounding whitespace aside. A plan of the same name is never it.
pub(crate) fn find_folder<'a>(
    folders: &'a [LibraryFolder],
    name: &str,
) -> Option<&'a LibraryFolder> {
    let wanted = name.trim().to_lowercase();
    folders
        .iter()
        .find(|folder| folder.kind == FOLDER_TYPE && folder.name.trim().to_lowercase() == wanted)
}

/// The workout in `folder` that already is `body` — the same name and the same
/// description, line endings and trailing spaces aside — or `None` when the
/// save has to write it.
pub(crate) fn find_same_workout<'a>(
    folder: &'a LibraryFolder,
    body: &Value,
) -> Option<&'a LibraryWorkout> {
    let name = body.get("name").and_then(Value::as_str).unwrap_or_default();
    let description = body
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let description = normalized(description);
    folder.children.iter().find(|workout| {
        workout.name.trim() == name.trim()
            && normalized(workout.description.as_deref().unwrap_or_default()) == description
    })
}

/// `text` with every line's trailing whitespace and the blank lines at either
/// end removed, so a description reads equal however it was stored.
fn normalized(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// The `POST /folders` body of a plain folder called `name`.
pub(crate) fn folder_body(name: &str) -> Value {
    json!({ "name": name.trim(), "type": FOLDER_TYPE })
}

/// The `POST /workouts` body that keeps `session` in the folder `folder_id`.
pub(crate) fn library_workout_body(session: &PlannedSession, folder_id: i64) -> Value {
    let mut body = json!({
        "folder_id": folder_id,
        "name": session.name,
        "description": render_description(session),
        "type": intervals_event_type(&session.sport),
    });
    if let Some(seconds) = session.duration_seconds {
        body["moving_time"] = json!(seconds);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PlannedSessionKind, SportType, WorkoutStep};
    use dravr_tronc::dates::CivilDate;

    fn session() -> PlannedSession {
        PlannedSession {
            external_id: "dravr:plan:x".to_owned(),
            kind: PlannedSessionKind::Workout,
            date: CivilDate::from_ymd(2026, 11, 25).expect("a calendar date"),
            sport: SportType::VirtualRide,
            name: "VO2 30/30".to_owned(),
            duration_seconds: Some(3_600),
            notes: "Zwift, free ride".to_owned(),
            steps: vec![WorkoutStep {
                label: "Warm-up".to_owned(),
                duration_seconds: 600,
                distance_meters: None,
                target_zone: "Z2".to_owned(),
                repeat: 1,
                repeat_group: None,
                note: None,
            }],
        }
    }

    fn folder(kind: &str, name: &str, children: Vec<LibraryWorkout>) -> LibraryFolder {
        LibraryFolder {
            id: 7,
            name: name.to_owned(),
            kind: kind.to_owned(),
            children,
        }
    }

    #[test]
    fn the_body_carries_the_folder_the_type_and_the_calendar_description() {
        let s = session();
        let body = library_workout_body(&s, 7);
        assert_eq!(body["folder_id"], 7, "bug: the folder it goes in: {body}");
        assert_eq!(body["name"], "VO2 30/30", "bug: the session's name: {body}");
        assert_eq!(body["type"], "VirtualRide", "bug: the event type: {body}");
        assert_eq!(body["moving_time"], 3_600, "bug: the duration: {body}");
        assert_eq!(
            body["description"],
            render_description(&s),
            "bug: the calendar's description"
        );
        assert!(
            body["description"]
                .as_str()
                .is_some_and(|d| d.contains("- Warm-up 10m")),
            "the steps go out in the DSL Intervals.icu parses: {}",
            body["description"]
        );
    }

    #[test]
    fn a_session_with_no_duration_sends_no_moving_time() {
        let mut s = session();
        s.duration_seconds = None;
        assert!(
            library_workout_body(&s, 7).get("moving_time").is_none(),
            "bug: no duration, no moving_time"
        );
    }

    #[test]
    fn a_folder_is_found_by_name_whatever_its_case_and_a_plan_never_is() {
        let folders = vec![
            folder("PLAN", "Dravr", Vec::new()),
            folder("FOLDER", "  dravr ", Vec::new()),
        ];
        let found = find_folder(&folders, "Dravr").expect("the plain folder");
        assert_eq!(found.kind, "FOLDER", "bug: a plan is never the folder");
        assert!(
            find_folder(&folders[..1], "Dravr").is_none(),
            "bug: a plan of the name is not a folder"
        );
        assert!(
            find_folder(&folders, "VO2").is_none(),
            "bug: no folder of that name"
        );
    }

    #[test]
    fn the_same_workout_is_recognised_and_a_namesake_with_other_steps_is_not() {
        let s = session();
        let body = library_workout_body(&s, 7);
        let description = render_description(&s);
        let same = LibraryWorkout {
            id: 1,
            name: "VO2 30/30".to_owned(),
            description: Some(format!("{}  \r\n\n", description.replace('\n', "\r\n"))),
        };
        let namesake = LibraryWorkout {
            id: 2,
            name: "VO2 30/30".to_owned(),
            description: Some("- 10m Z1".to_owned()),
        };
        let with_namesake = folder("FOLDER", "Dravr", vec![namesake]);
        assert!(
            find_same_workout(&with_namesake, &body).is_none(),
            "bug: a namesake with other steps is another workout"
        );
        let with_same = folder(
            "FOLDER",
            "Dravr",
            vec![
                LibraryWorkout {
                    id: 2,
                    name: "VO2 30/30".to_owned(),
                    description: Some("- 10m Z1".to_owned()),
                },
                same,
            ],
        );
        assert_eq!(
            find_same_workout(&with_same, &body).map(|w| w.id),
            Some(1),
            "bug: the same name and description, line endings aside"
        );
    }

    #[test]
    fn the_folder_body_is_a_plain_folder() {
        assert_eq!(
            folder_body(" Dravr "),
            json!({ "name": "Dravr", "type": "FOLDER" }),
            "bug: a trimmed name, a plain folder"
        );
    }
}
