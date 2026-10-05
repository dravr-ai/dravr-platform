// ABOUTME: Intervals.icu coach roster — the athletes a coach's API key can read, from GET /api/v1/athletes
// ABOUTME: Pure wire shape and mapping, plus the athlete-id check a delegated read applies before an id reaches a URL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Intervals.icu coach roster
//!
//! `GET /api/v1/athletes` lists the athletes the caller follows or coaches,
//! the caller included. Intervals.icu answers it for an API key only: a
//! bearer token is refused. The caller's own entry is the coach account,
//! whose email binds it to the coach's Dravr account; every other entry is an
//! athlete the key can read by id.

use serde::Deserialize;

use crate::delegation::CoachRoster;
use crate::errors::{AppError, AppResult};
use crate::models::RosterAthlete;

/// Longest athlete id a delegated read accepts. Intervals.icu ids are short
/// (`i123456`); the cap only bounds a value someone filled with something
/// else.
const MAX_ATHLETE_ID_CHARS: usize = 32;

/// One entry of `GET /api/v1/athletes` (the `AthleteWithTags` schema of the
/// Intervals.icu `OpenAPI` spec), the fields a roster reads.
#[derive(Debug, Deserialize)]
pub struct IntervalsIcuRosterEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    firstname: Option<String>,
    #[serde(default)]
    lastname: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

impl IntervalsIcuRosterEntry {
    /// The name the entry gives: its full name, else its first and last
    /// names joined; `None` when it gives none.
    fn display_name(&self) -> Option<String> {
        let named = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        named(&self.name).or_else(|| {
            let parts: Vec<String> = [&self.firstname, &self.lastname]
                .into_iter()
                .filter_map(named)
                .collect();
            (!parts.is_empty()).then(|| parts.join(" "))
        })
    }
}

/// The roster `entries` describe for the coach account `own_id`: the
/// account's own email, and every other entry as an athlete.
///
/// An entry whose id a delegated read would refuse ([`check_athlete_id`]) is
/// left out: no link could ever read it.
pub fn coach_roster(own_id: &str, entries: Vec<IntervalsIcuRosterEntry>) -> CoachRoster {
    let mut account_email = None;
    let mut athletes = Vec::new();
    for entry in entries {
        if entry.id == own_id {
            account_email = entry.email;
            continue;
        }
        if check_athlete_id(&entry.id).is_err() {
            continue;
        }
        athletes.push(RosterAthlete {
            name: entry.display_name(),
            id: entry.id,
            email: entry.email,
        });
    }
    CoachRoster {
        account_email,
        athletes,
    }
}

/// Refuse an athlete id Intervals.icu could not have issued: anything but a
/// short run of ASCII letters and digits. The id becomes a URL path segment,
/// so nothing else may pass.
///
/// # Errors
///
/// Returns an invalid-input error for an empty, overlong or non-alphanumeric
/// id.
pub fn check_athlete_id(athlete_id: &str) -> AppResult<()> {
    let valid = !athlete_id.is_empty()
        && athlete_id.len() <= MAX_ATHLETE_ID_CHARS
        && athlete_id.bytes().all(|b| b.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(AppError::invalid_input(
            "That is not an Intervals.icu athlete id",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: Option<&str>, email: Option<&str>) -> IntervalsIcuRosterEntry {
        IntervalsIcuRosterEntry {
            id: id.to_owned(),
            name: name.map(str::to_owned),
            firstname: None,
            lastname: None,
            email: email.map(str::to_owned),
        }
    }

    #[test]
    fn the_coach_entry_gives_the_email_and_every_other_entry_is_an_athlete() {
        let roster = coach_roster(
            "i100",
            vec![
                entry("i100", Some("Casey Coach"), Some("coach@links.test")),
                entry("i201", Some("Alex Athlete"), Some("alex@links.test")),
                entry("i202", None, None),
            ],
        );
        assert_eq!(roster.account_email.as_deref(), Some("coach@links.test"));
        assert_eq!(
            roster.athletes,
            vec![
                RosterAthlete {
                    id: "i201".to_owned(),
                    name: Some("Alex Athlete".to_owned()),
                    email: Some("alex@links.test".to_owned()),
                },
                RosterAthlete {
                    id: "i202".to_owned(),
                    name: None,
                    email: None,
                },
            ]
        );
    }

    #[test]
    fn a_name_falls_back_to_first_and_last_names() {
        let named = IntervalsIcuRosterEntry {
            id: "i3".to_owned(),
            name: Some("  ".to_owned()),
            firstname: Some("Sam".to_owned()),
            lastname: Some("Swimmer".to_owned()),
            email: None,
        };
        assert_eq!(named.display_name().as_deref(), Some("Sam Swimmer"));
    }

    #[test]
    fn an_entry_no_read_could_address_is_left_off_the_roster() {
        let roster = coach_roster("i1", vec![entry("../i2", Some("Odd"), None)]);
        assert!(roster.athletes.is_empty());
        assert!(roster.account_email.is_none());
    }

    #[test]
    fn only_a_short_alphanumeric_id_is_an_athlete_id() {
        assert!(check_athlete_id("i123456").is_ok());
        assert!(check_athlete_id(&"9".repeat(MAX_ATHLETE_ID_CHARS)).is_ok());
        for refused in [
            "",
            "i1/../i2",
            "i 1",
            "i1?x=1",
            &"9".repeat(MAX_ATHLETE_ID_CHARS + 1),
        ] {
            assert!(check_athlete_id(refused).is_err(), "{refused:?}");
        }
    }
}
