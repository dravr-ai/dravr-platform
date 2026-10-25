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
use serde_json::Value;

use crate::delegation::CoachRoster;
use crate::errors::{AppError, AppResult};
use crate::models::{RosterAccess, RosterAthlete};

/// Longest athlete id a delegated read accepts. Intervals.icu ids are short
/// (`i123456`); the cap only bounds a value someone filled with something
/// else.
const MAX_ATHLETE_ID_CHARS: usize = 32;

/// One entry of `GET /api/v1/athletes` (the `AthleteWithTags` schema of the
/// Intervals.icu `OpenAPI` spec), the fields a roster reads.
///
/// The roster arrives as one array, decoded whole, so the fields read off
/// the wire as loosely as their meaning allows: the access, the private
/// wellness fields and the standing are kept as raw JSON and read by
/// [`Self::access`], [`Self::private_wellness_keys`] and [`Self::active`],
/// where a missing, `null` or unexpected value reads as unreported instead
/// of failing every athlete's entry.
#[derive(Debug, Deserialize)]
pub(crate) struct IntervalsIcuRosterEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    firstname: Option<String>,
    #[serde(default)]
    lastname: Option<String>,
    #[serde(default)]
    email: Option<String>,
    /// The caller's access to the listed athlete: `WRITE` (coaches them),
    /// `READ` (follows them) or `NONE`.
    #[serde(default)]
    icu_permission: Option<Value>,
    /// The names of the wellness fields the athlete keeps private.
    #[serde(default)]
    private_wellness_keys: Option<Value>,
    /// The athlete's standing with the caller: `ACTIVE`, `DORMANT` or
    /// `ARCHIVED`.
    #[serde(default)]
    status: Option<Value>,
}

impl IntervalsIcuRosterEntry {
    /// The coach's access to the athlete: `WRITE` coaches, `READ` follows,
    /// `NONE` gives no access. Anything else, `null`, or no value at all
    /// reads as `None`, which grants nothing a known access would.
    fn access(&self) -> Option<RosterAccess> {
        match self.icu_permission.as_ref().and_then(Value::as_str)? {
            "WRITE" => Some(RosterAccess::Coach),
            "READ" => Some(RosterAccess::Follower),
            "NONE" => Some(RosterAccess::NoAccess),
            _ => None,
        }
    }

    /// The wellness fields the athlete keeps private, when the entry gives
    /// them as an array of names; `None` for anything else, `null` and an
    /// absent value included, so an unreadable answer never reads as
    /// "nothing private".
    fn private_wellness_keys(&self) -> Option<Vec<String>> {
        let Value::Array(keys) = self.private_wellness_keys.as_ref()? else {
            return None;
        };
        keys.iter()
            .map(|key| key.as_str().map(str::to_owned))
            .collect()
    }

    /// Whether the athlete is active with the caller: `ACTIVE` is,
    /// `DORMANT` and `ARCHIVED` are not. Anything else, `null`, or no value
    /// at all reads as `None`, which counts as neither.
    fn active(&self) -> Option<bool> {
        match self.status.as_ref().and_then(Value::as_str)? {
            "ACTIVE" => Some(true),
            "DORMANT" | "ARCHIVED" => Some(false),
            _ => None,
        }
    }

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
/// account's own email, and every other entry as an athlete, with the
/// coach's access to them, the wellness fields they keep private and whether
/// they are active.
///
/// An entry whose id a delegated read would refuse ([`check_athlete_id`]) is
/// left out: no link could ever read it.
pub(crate) fn coach_roster(own_id: &str, entries: Vec<IntervalsIcuRosterEntry>) -> CoachRoster {
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
        let name = entry.display_name();
        let access = entry.access();
        let private_wellness_keys = entry.private_wellness_keys();
        let active = entry.active();
        athletes.push(
            RosterAthlete::new(entry.id, name, entry.email)
                .with_access(access)
                .with_private_wellness_keys(private_wellness_keys)
                .with_active(active),
        );
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
pub(crate) fn check_athlete_id(athlete_id: &str) -> AppResult<()> {
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
            icu_permission: None,
            private_wellness_keys: None,
            status: None,
        }
    }

    /// The one entry `json`, an `/api/v1/athletes` element, decodes to.
    fn decoded(json: &str) -> IntervalsIcuRosterEntry {
        serde_json::from_str(json).unwrap()
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
                RosterAthlete::new(
                    "i201",
                    Some("Alex Athlete".to_owned()),
                    Some("alex@links.test".to_owned()),
                ),
                RosterAthlete::new("i202", None, None),
            ]
        );
    }

    #[test]
    fn a_name_falls_back_to_first_and_last_names() {
        let named = IntervalsIcuRosterEntry {
            firstname: Some("Sam".to_owned()),
            lastname: Some("Swimmer".to_owned()),
            ..entry("i3", Some("  "), None)
        };
        assert_eq!(named.display_name().as_deref(), Some("Sam Swimmer"));
    }

    #[test]
    fn each_permission_reads_as_the_coachs_access_and_anything_else_as_unreported() {
        for (permission, access) in [
            (r#""WRITE""#, Some(RosterAccess::Coach)),
            (r#""READ""#, Some(RosterAccess::Follower)),
            (r#""NONE""#, Some(RosterAccess::NoAccess)),
            ("null", None),
            (r#""write""#, None),
            (r#""OWNER""#, None),
            ("7", None),
            (r#"["WRITE"]"#, None),
        ] {
            let entry = decoded(&format!(r#"{{"id":"i1","icu_permission":{permission}}}"#));
            assert_eq!(entry.access(), access, "{permission}");
        }
        assert_eq!(decoded(r#"{"id":"i1"}"#).access(), None, "missing");
    }

    #[test]
    fn each_status_reads_as_active_or_not_and_anything_else_as_unreported() {
        for (status, active) in [
            (r#""ACTIVE""#, Some(true)),
            (r#""DORMANT""#, Some(false)),
            (r#""ARCHIVED""#, Some(false)),
            ("null", None),
            (r#""active""#, None),
            (r#""PAUSED""#, None),
            ("true", None),
            ("1", None),
            (r#"["ACTIVE"]"#, None),
        ] {
            let entry = decoded(&format!(r#"{{"id":"i1","status":{status}}}"#));
            assert_eq!(entry.active(), active, "{status}");
        }
        assert_eq!(decoded(r#"{"id":"i1"}"#).active(), None, "missing");
    }

    #[test]
    fn only_an_array_of_names_reads_as_the_private_wellness_fields() {
        for (keys, read) in [
            ("[]", Some(vec![])),
            (r#"["hrv"]"#, Some(vec!["hrv".to_owned()])),
            (
                r#"["hrv","weight"]"#,
                Some(vec!["hrv".to_owned(), "weight".to_owned()]),
            ),
            ("null", None),
            (r#""hrv""#, None),
            ("[1]", None),
            (r#"["hrv",null]"#, None),
            (r#"{"hrv":true}"#, None),
        ] {
            let entry = decoded(&format!(r#"{{"id":"i1","private_wellness_keys":{keys}}}"#));
            assert_eq!(entry.private_wellness_keys(), read, "{keys}");
        }
        assert_eq!(
            decoded(r#"{"id":"i1"}"#).private_wellness_keys(),
            None,
            "missing"
        );
    }

    #[test]
    fn an_odd_entry_in_the_middle_still_lets_every_athlete_through() {
        let entries: Vec<IntervalsIcuRosterEntry> = serde_json::from_str(
            r#"[
                {"id":"i100","name":"Casey Coach","email":"coach@links.test","icu_permission":"WRITE","status":"ACTIVE"},
                {"id":"i201","name":"Alex Athlete","icu_permission":"WRITE","private_wellness_keys":[],"status":"ACTIVE"},
                {"id":"i202","name":null,"email":null,"icu_permission":null,"private_wellness_keys":null,"status":null},
                {"id":"i203","icu_permission":42,"private_wellness_keys":"hrv","status":7},
                {"id":"i204","icu_permission":"READ","private_wellness_keys":["hrv"],"status":"ARCHIVED"}
            ]"#,
        )
        .unwrap();
        let roster = coach_roster("i100", entries);
        assert_eq!(roster.account_email.as_deref(), Some("coach@links.test"));
        assert_eq!(
            roster.athletes,
            vec![
                RosterAthlete::new("i201", Some("Alex Athlete".to_owned()), None)
                    .with_access(Some(RosterAccess::Coach))
                    .with_private_wellness_keys(Some(vec![]))
                    .with_active(Some(true)),
                RosterAthlete::new("i202", None, None),
                RosterAthlete::new("i203", None, None),
                RosterAthlete::new("i204", None, None)
                    .with_access(Some(RosterAccess::Follower))
                    .with_private_wellness_keys(Some(vec!["hrv".to_owned()]))
                    .with_active(Some(false)),
            ]
        );
    }

    #[test]
    fn an_entry_no_read_could_address_is_left_off_the_roster() {
        let roster = coach_roster("i1", vec![entry("../i2", Some("Odd"), None)]);
        assert_eq!(roster.athletes, []);
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
