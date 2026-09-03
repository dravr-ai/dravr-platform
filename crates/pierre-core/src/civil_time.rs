// ABOUTME: The athlete's civil clock — turns a UTC instant into the local day they actually lived
// ABOUTME: Owns localized weekday naming so no prompt surface makes the model derive a weekday itself
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's civil clock.
//!
//! Every surface that puts a date in front of the coach renders it here, in the
//! athlete's own zone and with the weekday spelled out. Two separate incidents
//! are the reason both halves exist.
//!
//! ## Why the zone
//!
//! On 2026-08-28 an athlete in `America/Toronto` was told "today is 2026-08-27
//! 22:59 (America/Toronto)" and handed an activity row stamped `2026-08-28` for
//! a hike started at 22:59 the previous evening. Two calendars in one prompt;
//! the coach reconciled them by inventing a time of day. Rendering every
//! instant through [`resolve_zone`] keeps the prompt in one frame.
//!
//! ## Why the weekday
//!
//! On 2026-09-02 an athlete spent the back half of a fifteen-turn conversation
//! correcting weekday claims — *"road 2 aus etait hier, mardi. T'es melé big"*,
//! *"date ride etait lundi. Ca va pas les dates"* — and the coach reassigned the
//! same five activities three times before he left. Nothing in the prompt named
//! a weekday: rows carried a bare `%Y-%m-%d` and the date anchor carried a bare
//! date plus a zone name, so the model derived every weekday by mental calendar
//! arithmetic and got them wrong.
//!
//! That is the same error class the epoch table in
//! `pierre_chat_pipeline::stages::prompt_assembly` already exists to remove,
//! whose own instruction reads *"never convert a date to an epoch yourself (it
//! is error-prone)"*. This module applies the identical reasoning to weekdays:
//! the server knows the answer, so the server says it.

use chrono::{DateTime, Datelike, Utc, Weekday};
use chrono_tz::Tz;

/// Resolve a stored `users.timezone` into a zone, falling back to UTC.
///
/// An absent or unparseable zone collapses to UTC rather than erroring, so an
/// athlete with no timezone on file still gets one coherent frame across the
/// date anchor, the activity list and the roster card — they all fall back
/// together.
#[must_use]
pub fn resolve_zone(user_timezone: Option<&str>) -> Tz {
    user_timezone
        .and_then(|tz| tz.parse::<Tz>().ok())
        .unwrap_or(Tz::UTC)
}

/// Column index into the five-locale tables, shared with the sport-name table
/// in `pierre_tool_runtime::implementations::fitness_support`.
fn locale_index(locale: &str) -> usize {
    match locale.get(0..2).unwrap_or("en") {
        "fr" => 0,
        "es" => 2,
        "de" => 3,
        "pt" => 4,
        _ => 1,
    }
}

/// Short localized weekday name, keyed by BCP-47 locale.
///
/// Short rather than full because these ride inside dense activity rows the
/// coach scans; the abbreviation is unambiguous in every shipped locale. The
/// match has no wildcard arm, so the table stays exhaustive over `Weekday`.
#[must_use]
pub fn weekday_short(weekday: Weekday, locale: &str) -> &'static str {
    // [fr, en, es, de, pt]
    let names: [&str; 5] = match weekday {
        Weekday::Mon => ["lun", "Mon", "lun", "Mo", "seg"],
        Weekday::Tue => ["mar", "Tue", "mar", "Di", "ter"],
        Weekday::Wed => ["mer", "Wed", "mié", "Mi", "qua"],
        Weekday::Thu => ["jeu", "Thu", "jue", "Do", "qui"],
        Weekday::Fri => ["ven", "Fri", "vie", "Fr", "sex"],
        Weekday::Sat => ["sam", "Sat", "sáb", "Sa", "sáb"],
        Weekday::Sun => ["dim", "Sun", "dom", "So", "dom"],
    };
    names[locale_index(locale)]
}

/// Every written form of `weekday`, lowercased, across the five shipped
/// locales — long and short.
///
/// For *reading* a weekday out of text rather than writing one: a claim that
/// says "dimanche" is asserting a day, and the athlete-data verifier has to
/// recognise it before it can check it (registre#249). Lowercased and
/// unaccented-tolerant only insofar as the source is, so callers lowercase the
/// haystack and compare.
#[must_use]
pub fn weekday_forms(weekday: Weekday) -> &'static [&'static str] {
    match weekday {
        Weekday::Mon => &[
            "lundi", "monday", "lunes", "montag", "segunda", "lun", "mon",
        ],
        Weekday::Tue => &[
            "mardi", "tuesday", "martes", "dienstag", "terça", "terca", "mar", "tue",
        ],
        Weekday::Wed => &[
            "mercredi",
            "wednesday",
            "miércoles",
            "miercoles",
            "mittwoch",
            "quarta",
            "mer",
            "wed",
        ],
        Weekday::Thu => &[
            "jeudi",
            "thursday",
            "jueves",
            "donnerstag",
            "quinta",
            "jeu",
            "thu",
        ],
        Weekday::Fri => &[
            "vendredi", "friday", "viernes", "freitag", "sexta", "ven", "fri",
        ],
        Weekday::Sat => &[
            "samedi", "saturday", "sábado", "sabado", "samstag", "sáb", "sam", "sat",
        ],
        Weekday::Sun => &["dimanche", "sunday", "domingo", "sonntag", "dim", "sun"],
    }
}

/// Every weekday, for callers that scan text for any of them.
pub const ALL_WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

/// `2026-09-01 lun 18:30` — the stamp shape every dated prompt surface shares.
///
/// The weekday sits between the date and the time deliberately: the model reads
/// the row left to right, and the weekday is the field it was previously forced
/// to compute.
#[must_use]
pub fn format_local_stamp(instant: DateTime<Utc>, zone: Tz, locale: &str) -> String {
    let local = instant.with_timezone(&zone);
    format!(
        "{} {} {}",
        local.format("%Y-%m-%d"),
        weekday_short(local.weekday(), locale),
        local.format("%H:%M")
    )
}

/// `2026-09-01 lun` — the day-only shape, for surfaces that quote no time.
#[must_use]
pub fn format_local_day(instant: DateTime<Utc>, zone: Tz, locale: &str) -> String {
    let local = instant.with_timezone(&zone);
    format!(
        "{} {}",
        local.format("%Y-%m-%d"),
        weekday_short(local.weekday(), locale)
    )
}

/// The athlete's local calendar date for a UTC instant.
///
/// The bucketing primitive: a 21:00 `America/Toronto` session belongs to the
/// day the athlete trained, not to the UTC day it lands in four hours later.
#[must_use]
pub fn local_date(instant: DateTime<Utc>, zone: Tz) -> chrono::NaiveDate {
    instant.with_timezone(&zone).date_naive()
}
