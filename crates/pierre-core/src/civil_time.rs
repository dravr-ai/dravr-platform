// ABOUTME: The athlete's civil clock — turns a UTC instant into the local day they actually lived
// ABOUTME: Owns localized weekday naming so no prompt surface makes the model derive a weekday itself
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's civil clock.
//!
//! Every surface that puts a date in front of the agent renders it here, in the
//! athlete's own zone and with the weekday spelled out. Two separate incidents
//! are the reason both halves exist.
//!
//! ## Why the zone
//!
//! On 2026-08-28 an athlete in `America/Toronto` was told "today is 2026-08-27
//! 22:59 (America/Toronto)" and handed an activity row stamped `2026-08-28` for
//! a hike started at 22:59 the previous evening. Two calendars in one prompt;
//! the agent reconciled them by inventing a time of day. Rendering every
//! instant through [`resolve_zone`] keeps the prompt in one frame.
//!
//! ## Why the weekday
//!
//! On 2026-09-02 an athlete spent the back half of a fifteen-turn conversation
//! correcting weekday claims — *"road 2 aus etait hier, mardi. T'es melé big"*,
//! *"date ride etait lundi. Ca va pas les dates"* — and the agent reassigned the
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

use dravr_tronc::date_pattern;
use dravr_tronc::dates::{CivilDate, CivilTime, Elapsed, Pattern, Timestamp, Weekday, Zone};

/// `2026-11-17 10:00 UTC`: an instant to the minute, in UTC, named as such —
/// the stamp admin and CLI surfaces print.
pub const UTC_MINUTE: Pattern<Timestamp> = date_pattern!(Timestamp, "%Y-%m-%d %H:%M UTC");

/// `2026-11-17 05:00`: an instant to the minute on a zone's clock, written
/// through `Zone::format_with`.
pub const LOCAL_MINUTE: Pattern<Timestamp> = date_pattern!(Timestamp, "%Y-%m-%d %H:%M");

/// RFC 3339 on a zone's clock, the fraction as `to_rfc3339` writes it.
const ZONED_RFC3339: Pattern<Timestamp> = date_pattern!(Timestamp, "%Y-%m-%dT%H:%M:%S%.f%:z");

/// Resolve a stored `users.timezone` into a zone, falling back to UTC.
///
/// An absent or unparseable zone collapses to UTC rather than erroring, so an
/// athlete with no timezone on file still gets one coherent frame across the
/// date anchor, the activity list and the roster card — they all fall back
/// together.
#[must_use]
pub fn resolve_zone(user_timezone: Option<&str>) -> Zone {
    parse_zone(user_timezone).unwrap_or_else(Zone::utc)
}

/// Parse a stored `users.timezone`, or `None` when it is absent or names no
/// zone.
///
/// [`resolve_zone`] is this plus its UTC fallback. A caller choosing between
/// several people's zones needs the refusal instead of the fallback, so that
/// an unparseable zone is passed over rather than read as UTC.
#[must_use]
pub fn parse_zone(user_timezone: Option<&str>) -> Option<Zone> {
    user_timezone.and_then(|tz| tz.parse::<Zone>().ok())
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
/// agent scans; the abbreviation is unambiguous in every shipped locale. The
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
/// Missing an agent who writes "mar." costs one unchecked claim. Contradicting a
/// agent who wrote "la mer" costs the athlete's trust. Only full names, and the
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
pub fn format_local_stamp(instant: Timestamp, zone: &Zone, locale: &str) -> String {
    // A date-only row has no time of day to convert, so it keeps its own date
    // and renders 00:00 rather than being pulled back a day (registre#258).
    if is_date_only(instant) {
        let date = instant.utc_date();
        return format!("{} {} 00:00", date, weekday_short(date.weekday(), locale));
    }
    format_clock_stamp(instant, zone, locale)
}

/// `2026-09-01 lun 18:30` for a reading of the wall clock.
///
/// Deliberately NOT [`format_local_stamp`], and the same split [`clock_date`]
/// makes against [`local_date`], for the same reason. That one treats a
/// midnight-UTC instant as a date-only provider row and renders the UTC date
/// unconverted (registre#258); a clock reading is always a real moment, so
/// this always converts.
///
/// The `{{CURRENT_DATE}}` anchor is the caller that needed it. It floors `now`
/// to a five-minute quantum so the prompt prefix stays byte-identical between
/// requests, and that floor lands exactly on midnight UTC once a day, every
/// day. Rendered through the sentinel-guessing stamp, the agent read the UTC
/// weekday for those five minutes: a Toronto athlete at 20:00 on a Friday was
/// told it was Saturday, in the block that LEADS the system prompt.
///
/// Takes the instant rather than reading the clock itself, so the boundary it
/// exists for is testable.
#[must_use]
pub fn format_clock_stamp(instant: Timestamp, zone: &Zone, locale: &str) -> String {
    let local = zone.civil_at(instant);
    format!(
        "{} {} {:02}:{:02}",
        local.date(),
        weekday_short(local.date().weekday(), locale),
        local.time().hour(),
        local.time().minute()
    )
}

/// `instant` as RFC 3339 on `zone`'s clock, with that clock's offset:
/// `2026-11-17T10:00:05.120-05:00`, the fraction written as `to_rfc3339`
/// writes it (none, or 3, 6 or 9 digits) and a zero offset as `+00:00`.
///
/// What `start_date_local` and the other zoned instants carry.
#[must_use]
pub fn zoned_rfc3339(instant: Timestamp, zone: &Zone) -> String {
    zone.format_with(instant, &ZONED_RFC3339)
}

/// `2026-09-01 lun` — the day-only shape, for surfaces that quote no time.
#[must_use]
pub fn format_local_day(instant: Timestamp, zone: &Zone, locale: &str) -> String {
    // Via `local_date`, so a date-only row keeps the day its provider named
    // instead of being shifted back one (registre#258).
    format_civil_day(local_date(instant, zone), locale)
}

/// `2026-09-01 lun` for a date that is already on the athlete's calendar.
#[must_use]
pub fn format_civil_day(date: CivilDate, locale: &str) -> String {
    format!("{} {}", date, weekday_short(date.weekday(), locale))
}

/// A day named by its distance from the athlete's today rather than by its date.
///
/// Only the two an athlete actually says. "Ce matin", "aujourd'hui", "hier" are
/// how a session is referred to in conversation, and they are the two the model
/// gets wrong: on 2026-09-21 a Telegram athlete asked whether a 30-minute ride
/// *this morning* would be too much, and the coach answered about the ride he
/// had done the previous afternoon, calling it "ce matin" and moving every
/// other day of the week back by one with it. The newest row in the activity
/// list had become "today" in the model's reading, exactly as the platform
/// contract tells it never to do — and the row said `2026-09-20 dim 16:59`, so
/// the calendar arithmetic it skipped was one subtraction.
///
/// So the server does the subtraction, the same way it names the weekday.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeDay {
    /// The athlete's current civil day.
    Today,
    /// The civil day before it.
    Yesterday,
}

/// Which relative day `date` is on a calendar whose current day is `today`, or
/// `None` when it is neither — two days ago and tomorrow both read as plain
/// dates.
#[must_use]
pub fn relative_day(date: CivilDate, today: CivilDate) -> Option<RelativeDay> {
    match today.signed_duration_since(date).num_days() {
        0 => Some(RelativeDay::Today),
        1 => Some(RelativeDay::Yesterday),
        _ => None,
    }
}

/// The one word a prompt surface *writes* for a relative day, in `locale`.
///
/// Rides next to the date on an activity row — `2026-09-20 dim 16:59 (hier)` —
/// so the localized form matches the language the rest of the row is in.
#[must_use]
pub fn relative_day_label(day: RelativeDay, locale: &str) -> &'static str {
    // [fr, en, es, de, pt]
    let names: [&str; 5] = match day {
        RelativeDay::Today => ["aujourd'hui", "today", "hoy", "heute", "hoje"],
        RelativeDay::Yesterday => ["hier", "yesterday", "ayer", "gestern", "ontem"],
    };
    names[locale_index(locale)]
}

/// Every written form that asserts `day` in `locale`, lowercased, for *reading*
/// a relative day out of a reply. English is included under every locale
/// because it is the language the model falls back to.
///
/// ## Why the table is per locale
///
/// [`weekday_forms`] scans all five locales at once because no full weekday
/// name is a homograph of anything. Relative days are not so lucky: French
/// `hier` (yesterday) is German `hier` (here), one of the commonest words in
/// the language. Scanned for a German athlete it would put a contradiction on
/// every sentence that says "here is your ride" — the registre#258 failure
/// class, a warning on a true sentence. So a locale reads only its own forms.
///
/// Times of day count as today: "ce matin" and "this evening" assert the
/// current day and nothing else this layer can check.
#[must_use]
pub fn relative_day_forms(day: RelativeDay, locale: &str) -> &'static [&'static str] {
    const TODAY_EN: &[&str] = &[
        "today",
        "this morning",
        "this afternoon",
        "this evening",
        "tonight",
    ];
    const TODAY_FR: &[&str] = &[
        "aujourd'hui",
        "aujourd’hui",
        "ce matin",
        "ce midi",
        "cet après-midi",
        "cet apres-midi",
        "ce soir",
        "today",
        "this morning",
        "this afternoon",
        "this evening",
        "tonight",
    ];
    const TODAY_ES: &[&str] = &[
        "hoy",
        "esta mañana",
        "esta manana",
        "esta tarde",
        "esta noche",
        "today",
        "this morning",
        "this afternoon",
        "this evening",
        "tonight",
    ];
    const TODAY_DE: &[&str] = &[
        "heute",
        "today",
        "this morning",
        "this afternoon",
        "this evening",
        "tonight",
    ];
    const TODAY_PT: &[&str] = &[
        "hoje",
        "today",
        "this morning",
        "this afternoon",
        "this evening",
        "tonight",
    ];
    const YESTERDAY_EN: &[&str] = &["yesterday"];
    const YESTERDAY_FR: &[&str] = &["hier", "yesterday"];
    const YESTERDAY_ES: &[&str] = &["ayer", "yesterday"];
    const YESTERDAY_DE: &[&str] = &["gestern", "yesterday"];
    const YESTERDAY_PT: &[&str] = &["ontem", "yesterday"];

    // [fr, en, es, de, pt]
    let forms: [&[&str]; 5] = match day {
        RelativeDay::Today => [TODAY_FR, TODAY_EN, TODAY_ES, TODAY_DE, TODAY_PT],
        RelativeDay::Yesterday => [
            YESTERDAY_FR,
            YESTERDAY_EN,
            YESTERDAY_ES,
            YESTERDAY_DE,
            YESTERDAY_PT,
        ],
    };
    forms[locale_index(locale)]
}

/// Whether an instant is a date-only provider row rather than a real clock
/// reading.
///
/// Several sources carry no time of day and render at midnight UTC of the
/// workout day — `RosterActivity::start` documents exactly that for
/// Strava-mirror session reads. Such a value asserts a DAY, not a moment, and
/// converting it into a negative-offset zone moves it onto the previous
/// calendar day: midnight UTC is 20:00 the day before in `America/Toronto`.
///
/// A genuine session starting at exactly 00:00:00 UTC is indistinguishable and
/// vanishingly rare; treating it as date-only costs at most a day-boundary
/// nudge, where the alternative costs every date-only row a wrong weekday.
#[must_use]
pub fn is_date_only(instant: Timestamp) -> bool {
    instant.utc_time() == CivilTime::MIDNIGHT
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
pub fn local_date(instant: Timestamp, zone: &Zone) -> CivilDate {
    if is_date_only(instant) {
        return instant.utc_date();
    }
    zone.date_at(instant)
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
/// bounded its window with `Timestamp::now().utc_date()`, so for any zone ahead of
/// UTC the athlete's current day sat past the end of the window and was
/// dropped — from the CTL/ATL/TSB series and from every answer built on it
/// (registre#260).
///
/// Takes the instant rather than reading the clock itself so the boundary it
/// exists for is testable.
#[must_use]
pub fn clock_date(instant: Timestamp, zone: &Zone) -> CivilDate {
    zone.date_at(instant)
}

/// The Monday of the week `date` falls in: weeks run Monday to Sunday on the
/// athlete's calendar, as Home's plan strip and its volume weeks do.
#[must_use]
pub fn monday_of(date: CivilDate) -> CivilDate {
    date.checked_sub_days(i64::from(date.weekday().num_days_from_monday()))
        .unwrap_or(date)
}

/// The instant the athlete's `date` begins in `zone`.
///
/// Where the zone skips its midnight (a DST gap, as Beirut and Cairo spring
/// forward), the day begins the instant the gap opens: midnight under the
/// offset in force the day before.
#[must_use]
pub fn day_start_utc(date: CivilDate, zone: &Zone) -> Timestamp {
    let midnight = date.at_midnight();
    zone.resolve(midnight).earliest().unwrap_or_else(|| {
        // At the calendar's first day there is no day before to read an
        // offset from: midnight UTC is the only instant left.
        let start = midnight
            .checked_sub(Elapsed::days(1))
            .and_then(|before| before.and_utc().ok())
            .map(|before| zone.offset_seconds_at(before))
            .and_then(|offset| midnight.checked_sub(Elapsed::seconds(i64::from(offset))))
            .unwrap_or(midnight);
        // A reading UTC cannot name lies at the start of the range.
        start.and_utc().unwrap_or(Timestamp::MIN)
    })
}

#[cfg(test)]
mod tests {

    use super::*;

    fn ymd(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::from_ymd(year, month, day)
            .ok()
            .unwrap_or(CivilDate::MIN)
    }

    #[test]
    fn a_week_starts_on_its_monday() {
        assert_eq!(
            monday_of(ymd(2026, 11, 15)),
            ymd(2026, 11, 9),
            "bug: a Sunday is not in the week of the Monday before it"
        );
        assert_eq!(
            monday_of(ymd(2026, 11, 9)),
            ymd(2026, 11, 9),
            "bug: a Monday is not its own week's start"
        );
        assert_eq!(
            monday_of(ymd(2027, 1, 1)),
            ymd(2026, 12, 28),
            "bug: a week that straddles new year does not start in the old year"
        );
    }

    #[test]
    fn a_day_starts_at_the_athletes_midnight() {
        let montreal = resolve_zone(Some("America/Montreal"));
        assert_eq!(
            day_start_utc(ymd(2026, 1, 1), &montreal),
            Timestamp::parse_from_rfc3339("2026-01-01T05:00:00Z")
                .ok()
                .unwrap_or_default(),
            "bug: Montreal's midnight is not 05:00 UTC in winter"
        );
    }

    #[test]
    fn a_skipped_midnight_east_of_utc_starts_the_day_when_the_gap_opens() {
        // Beirut springs forward from 00:00 (UTC+2) to 01:00 (UTC+3) on the
        // last Sunday of March, so the 29th begins at 22:00 UTC on the 28th.
        let beirut = resolve_zone(Some("Asia/Beirut"));
        let date = ymd(2026, 3, 29);
        assert!(
            beirut
                .resolve(date.and_time(CivilTime::MIDNIGHT))
                .earliest()
                .is_none(),
            "bug: the fixture zone no longer skips this midnight, so the gap is untested"
        );
        let start = day_start_utc(date, &beirut);
        assert_eq!(
            start,
            Timestamp::parse_from_rfc3339("2026-03-28T22:00:00Z")
                .ok()
                .unwrap_or_default(),
            "bug: a day whose midnight is skipped does not begin when the gap opens"
        );
        assert_eq!(
            beirut.date_at(start - Elapsed::nanoseconds(1)),
            ymd(2026, 3, 28),
            "bug: the instant before the day begins is not the day before"
        );
    }

    #[test]
    fn the_calendars_first_day_starts_east_of_utc_without_a_panic() {
        // Tokyo's midnight on the calendar's first day falls before the first
        // instant the date types hold, so it has no local start to map.
        let tokyo = resolve_zone(Some("Asia/Tokyo"));
        assert_eq!(
            day_start_utc(CivilDate::MIN, &tokyo),
            CivilDate::MIN
                .and_time(CivilTime::MIDNIGHT)
                .and_utc()
                .unwrap_or(Timestamp::MIN),
            "bug: a day with no local start and no day before is not read from midnight UTC"
        );
    }
}
