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

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};
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

/// Every UNAMBIGUOUS written form of `weekday`, lowercased, across the five
/// shipped locales.
///
/// For *reading* a weekday out of text rather than writing one: a claim that
/// says "dimanche" is asserting a day, and the athlete-data verifier has to
/// recognise it before it can check it (registre#249). Callers lowercase the
/// haystack and compare.
///
/// ## Why there are no abbreviations here
///
/// [`weekday_short`] writes `lun`/`mer`/`jeu`; this reads. The two jobs have
/// opposite failure costs, and the short forms are homographs of ordinary words
/// in every locale we ship: French `mon` is a possessive, `jeu` a game, `mer`
/// the sea; English `sun` is the star and `sat` a past tense; Spanish and
/// Portuguese `mar` is the sea, and the bare Portuguese ordinals `segunda`,
/// `quarta`, `quinta`, `sexta` are just numbers.
///
/// With them in the table, *"ta sortie Road 2 AUS confirme **mon** impression"*
/// read as a Monday claim and the layer contradicted a sentence that asserted
/// no day at all — a warning banner on a true reply, which is the exact
/// false-positive class this verifier exists to prevent (registre#258).
///
/// Missing a coach who writes "mar." costs one unchecked claim. Contradicting a
/// coach who wrote "la mer" costs the athlete's trust. Only full names, and the
/// Portuguese `-feira` compounds that disambiguate the ordinals, are listed.
#[must_use]
pub fn weekday_forms(weekday: Weekday) -> &'static [&'static str] {
    match weekday {
        Weekday::Mon => &["lundi", "monday", "lunes", "montag", "segunda-feira"],
        Weekday::Tue => &[
            "mardi",
            "tuesday",
            "martes",
            "dienstag",
            "terça-feira",
            "terca-feira",
        ],
        Weekday::Wed => &[
            "mercredi",
            "wednesday",
            "miércoles",
            "miercoles",
            "mittwoch",
            "quarta-feira",
        ],
        Weekday::Thu => &["jeudi", "thursday", "jueves", "donnerstag", "quinta-feira"],
        Weekday::Fri => &["vendredi", "friday", "viernes", "freitag", "sexta-feira"],
        Weekday::Sat => &["samedi", "saturday", "sábado", "sabado", "samstag"],
        Weekday::Sun => &["dimanche", "sunday", "domingo", "sonntag"],
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
    // A date-only row has no time of day to convert, so it keeps its own date
    // and renders 00:00 rather than being pulled back a day (registre#258).
    if is_date_only(instant) {
        let date = instant.date_naive();
        return format!(
            "{} {} 00:00",
            date.format("%Y-%m-%d"),
            weekday_short(date.weekday(), locale)
        );
    }
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
    // Via `local_date`, so a date-only row keeps the day its provider named
    // instead of being shifted back one (registre#258).
    let date = local_date(instant, zone);
    format!(
        "{} {}",
        date.format("%Y-%m-%d"),
        weekday_short(date.weekday(), locale)
    )
}

/// Whether an instant is a date-only provider row rather than a real clock
/// reading.
///
/// Several sources carry no time of day and render at midnight UTC of the
/// workout day — `RosterActivity::start` documents exactly that for
/// Strava-mirror scrapes. Such a value asserts a DAY, not a moment, and
/// converting it into a negative-offset zone moves it onto the previous
/// calendar day: midnight UTC is 20:00 the day before in `America/Toronto`.
///
/// A genuine session starting at exactly 00:00:00 UTC is indistinguishable and
/// vanishingly rare; treating it as date-only costs at most a day-boundary
/// nudge, where the alternative costs every date-only row a wrong weekday.
#[must_use]
pub fn is_date_only(instant: DateTime<Utc>) -> bool {
    instant.time() == NaiveTime::MIN
}

/// The athlete's local calendar date for a UTC instant.
///
/// The bucketing primitive: a 21:00 `America/Toronto` session belongs to the
/// day the athlete trained, not to the UTC day it lands in four hours later.
///
/// A date-only row ([`is_date_only`]) is returned unconverted. It already names
/// the day the provider means, and shifting it into the athlete's zone would
/// move it backwards — which is how the first version of this fix made
/// registre#200 worse on exactly the provider the 2026-09-02 athlete used
/// (registre#258).
#[must_use]
pub fn local_date(instant: DateTime<Utc>, zone: Tz) -> chrono::NaiveDate {
    if is_date_only(instant) {
        return instant.date_naive();
    }
    instant.with_timezone(&zone).date_naive()
}

/// Today, on the athlete's clock.
///
/// Deliberately NOT [`local_date`]. That one refuses to convert a midnight-UTC
/// instant, because a date-only provider row uses exactly that as its sentinel
/// for "a day, not a moment" (registre#258). A reading of the wall clock is
/// always a real moment, so it always converts — and a rollup window built with
/// `local_date` would rewind a day for every athlete ahead of UTC on the one
/// instant per day the sentinel guard fires.
///
/// It exists because the distinction cost a whole civil day of training data:
/// the daily rollup buckets each activity on the athlete's civil date but
/// bounded its window with `Utc::now().date_naive()`, so for any zone ahead of
/// UTC the athlete's current day sat past the end of the window and was
/// dropped — from the CTL/ATL/TSB series and from every answer built on it
/// (registre#260).
///
/// Takes the instant rather than reading the clock itself so the boundary it
/// exists for is testable.
#[must_use]
pub fn clock_date(instant: DateTime<Utc>, zone: Tz) -> chrono::NaiveDate {
    instant.with_timezone(&zone).date_naive()
}
