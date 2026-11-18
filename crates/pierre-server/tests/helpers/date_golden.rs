// ABOUTME: Shared mechanics of the response date goldens: every date in a JSON body with its path, seeded athletes, one request
// ABOUTME: A suite compares what `dates_in` returns with a literal table, so no date text in a response changes unseen
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// Every top-level test binary compiles `helpers`, and only the `it` binary's
// date goldens call this module, as with the sibling helpers.
#![allow(dead_code)]

//! Response date goldens (carnet#1620, gate 3): the shared half.
//!
//! A golden seeds rows at instants the test chooses, calls a real handler
//! (an HTTP router, the tool dispatch or `POST /mcp`) and compares **every**
//! date-looking text in the body with a literal table:
//!
//! - [`dates_in`] walks the body and returns `(JSON pointer, text)` for every
//!   date it finds, sorted. A string that is a date is one row; a date inside
//!   prose is a row at `pointer#n`, with the weekday word and the wall-clock
//!   time written after it. A field that appears, disappears or
//!   changes one character (`Z` to `+00:00`, a fraction digit, a zoned
//!   offset) changes the table.
//! - A number or a flag computed from dates (`days_spanned`, a phase's
//!   `current`) is not date text, so the walk does not see it: a suite names
//!   its pointer and [`with_values`] adds it as a row.
//! - A field the handler stamps with the wall clock cannot be a literal. The
//!   suite names its pointer in `stamped_now`, and the row carries the
//!   stamp's [`clock_shape`] instead: digits as `d`, and the clock's own
//!   fraction as `.f`. A stamp that lost its fraction, or was cut to a width
//!   no clock writes, has another shape. Where the row can be moved to an
//!   instant the suite chooses ([`restamp`] and its siblings), it is, and the
//!   value is compared.
//! - The athlete's own "today" is compared by value: [`Today`] reads the
//!   clock once and gives the day in two zones whose offsets never change and
//!   sit a day apart, so a day computed in UTC is wrong in at least one.
//!
//! # Where the chrono is
//!
//! The suites name no chrono type and call no chrono method; this file does
//! all of it. [`instant`], [`day`] and [`epoch`] build the seeded values;
//! [`utc_days_before_today`], [`whole_days_since`], [`Today`], [`Instant`]
//! and [`CLOCK_DIGITS`] read the clock; [`noon_in`] places a session on an
//! athlete's own day; [`lies_days_after`] parses an answered instant;
//! [`shifted`], [`days_between`], [`monday_of`], [`sunday_of`] and
//! [`first_of_next_month`] do the calendar arithmetic; [`seed_account`]
//! takes instants; [`restamp`] binds one through sqlx. A suite holds a date
//! only as a value these functions gave it: it compares two, orders two, and
//! hands them back here or to a fixture.
//!
//! A migration off chrono rewrites this file and leaves the suites' tables
//! untouched. After it, [`restamp`] writes whatever the new type's sqlx
//! encoding produces, which is part of what the restamped goldens compare.

use std::future::Future;
use std::panic::{panic_any, resume_unwind, AssertUnwindSafe};
use std::sync::{Arc, LazyLock};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{
    DateTime, Datelike, Days, Duration, FixedOffset, NaiveDate, NaiveDateTime, SubsecRound, Utc,
};
use futures_util::FutureExt;
use pierre_core::models::{CreateUserMcpTokenRequest, Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::permissions::UserRole;
use pierre_core::transport::Transport;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use regex::Regex;
use secrecy::ExposeSecret;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;

/// An instant (a date, a time of day, any fraction, and a zone written `Z`,
/// `UTC` or as a numeric offset with or without its colon), or a calendar
/// date with the weekday word and the wall-clock time that follow it in
/// prose (`2026-08-30 Sun 08:00`). The word is any whole word of two to five
/// letters, so a date in a sentence keeps a short word that follows it
/// (`2026-08-20 and`).
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?: ?(?:Z|UTC|[+-]\d{2}:?\d{2}))?",
        r"|\d{4}-\d{2}-\d{2}(?: \p{L}{2,5}\b\.?)?(?: \d{2}:\d{2}(?::\d{2})?)?",
    ))
    .unwrap()
});

/// The date or instant alone, without the words around it.
static STAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?: ?(?:Z|UTC|[+-]\d{2}:?\d{2}))?)?",
    )
    .unwrap()
});

/// The fraction of a second in a stamp.
static FRACTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.\d+").unwrap());

/// A UTC instant from `2026-08-28T02:59:03.500`, with any fraction or none.
pub(crate) fn instant(text: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .unwrap()
        .and_utc()
}

/// Seconds since the epoch of `2026-08-29T00:00:00`, for the tools that take
/// an instant as a number.
pub(crate) fn epoch(text: &str) -> i64 {
    instant(text).timestamp()
}

/// The UTC calendar dates each of `days` days before today, as `2026-11-16`,
/// from one reading of the clock: for the few reads that only serve rows of
/// the last days, where the day cannot be chosen and the time of day can.
pub(crate) fn utc_days_before_today<const N: usize>(days: [u64; N]) -> [String; N] {
    let today = Utc::now().date_naive();
    days.map(|back| (today - Days::new(back)).to_string())
}

/// A calendar date from `2026-08-28`.
pub(crate) fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

/// What a stamp looks like with its value taken out: every digit is `d`, the
/// fraction's digits included, and a weekday word after it is dropped.
/// `2026-11-17T15:00:05.250123+00:00` is `dddd-dd-ddTdd:dd:dd.dddddd+dd:dd`;
/// `2026-11-17 Tue` is `dddd-dd-dd`.
pub(crate) fn shape(stamp: &str) -> String {
    let stamp = STAMP.find(stamp).map_or(stamp, |found| found.as_str());
    digits_out(stamp)
}

fn digits_out(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_digit() { 'd' } else { c })
        .collect()
}

/// The fraction digits this machine's clock writes: 6 where it counts
/// microseconds (macOS), 9 where it counts nanoseconds (Linux). Read from the
/// clock itself, not from the platform's name.
pub(crate) static CLOCK_DIGITS: LazyLock<usize> = LazyLock::new(|| {
    let nanos = (0..256).any(|_| !Utc::now().timestamp_subsec_nanos().is_multiple_of(1_000));
    if nanos {
        9
    } else {
        6
    }
});

/// The fraction widths a reading of now can be answered at: the clock's own,
/// and six digits, which is what a PostgreSQL `TIMESTAMPTZ` keeps of a
/// nanosecond reading. Three digits is never one of them, so a stamp cut to
/// milliseconds is seen.
fn clock_widths() -> [usize; 2] {
    [*CLOCK_DIGITS, 6]
}

/// A reading of now whose trailing groups of zeros happened to fall away:
/// one read in a thousand on a microsecond clock is answered `.250`, not
/// `.250000`, by a formatter that writes the clock's whole width. One such
/// stamp cannot be told from a stamp cut to milliseconds, so the golden runs
/// again ([`on_the_clock`]); a format cut to milliseconds is narrow every
/// time, and the third narrow reading fails with this message.
#[derive(Debug)]
pub(crate) struct NarrowClockReading(pub(crate) String);

/// [`shape`] of a stamp taken from the wall clock and written at the clock's
/// own width: a fraction of one of [`clock_widths`] is `.f`.
///
/// So `2026-11-17T15:00:05.250123+00:00` is `dddd-dd-ddTdd:dd:dd.f+dd:dd`
/// on a microsecond clock. A fraction narrower than the clock's (none at all
/// included) is either a lucky reading or a format that cut it: it raises
/// [`NarrowClockReading`], which [`on_the_clock`] answers by running the
/// golden again. Any wider width keeps its digits.
pub(crate) fn clock_shape(stamp: &str) -> String {
    let stamp = STAMP.find(stamp).map_or(stamp, |found| found.as_str());
    if !stamp.contains(':') {
        return digits_out(stamp);
    }
    let width = FRACTION
        .find(stamp)
        .map_or(0, |found| found.as_str().len() - 1);
    let widths = clock_widths();
    if widths.contains(&width) {
        return digits_out(&FRACTION.replace(stamp, ".f"));
    }
    if width < *CLOCK_DIGITS && width.is_multiple_of(3) {
        panic_any(NarrowClockReading(format!(
            "{stamp} is a reading of now written with {width} fraction digits; this \
             clock writes {} (or 6 once stored)",
            *CLOCK_DIGITS
        )));
    }
    digits_out(stamp)
}

/// Run a golden that compares a reading of now by [`clock_shape`], again
/// when a reading came out narrower than the clock writes, three times at
/// most: a format that cuts the fraction fails on the third.
pub(crate) async fn on_the_clock<F, Fut>(golden: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    for attempt in 1..=3 {
        let Err(panic) = AssertUnwindSafe(golden()).catch_unwind().await else {
            return;
        };
        match panic.downcast::<NarrowClockReading>() {
            Ok(narrow) if attempt == 3 => panic!("{}", narrow.0),
            Ok(_) => {}
            Err(other) => resume_unwind(other),
        }
    }
}

/// Whether `at` (an instant as a response writes it) lies between `before`
/// plus `days` and `after` plus `days`: a lifetime counted from a reading of
/// now the test cannot choose, pinned between two readings of its own.
pub(crate) fn lies_days_after(at: &str, before: Instant, after: Instant, days: i64) -> bool {
    lies_after(at, before, after, Duration::days(days))
}

/// [`lies_days_after`], for a lifetime counted in minutes.
pub(crate) fn lies_minutes_after(at: &str, before: Instant, after: Instant, minutes: i64) -> bool {
    lies_after(at, before, after, Duration::minutes(minutes))
}

fn lies_after(at: &str, before: Instant, after: Instant, span: Duration) -> bool {
    let at = DateTime::parse_from_rfc3339(at)
        .unwrap()
        .with_timezone(&Utc);
    // The first reading cut to its microsecond: a stamp stored in
    // PostgreSQL comes back cut so.
    before.0.trunc_subsecs(6) + span <= at && at <= after.0 + span
}

/// Whether `at` (an instant as a response writes it) lies between two
/// readings of the clock, the first cut to its millisecond: a handler may cut
/// its own reading to the microsecond or the millisecond, so `at` can sit
/// just under a reading of nanoseconds taken before it.
pub(crate) fn lies_between(at: &str, before: Instant, after: Instant) -> bool {
    let at = DateTime::parse_from_rfc3339(at)
        .unwrap()
        .with_timezone(&Utc);
    let floor =
        before.0 - Duration::nanoseconds(i64::from(before.0.timestamp_subsec_nanos() % 1_000_000));
    floor <= at && at <= after.0
}

/// One reading of the wall clock, held to compare with an instant a handler
/// stamped.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Instant(DateTime<Utc>);

impl Instant {
    pub(crate) fn now() -> Self {
        Self(Utc::now())
    }
}

/// The zone fourteen hours ahead of UTC all year.
pub(crate) const KIRITIMATI: &str = "Pacific/Kiritimati";
/// The zone twelve hours behind UTC all year (the sign is POSIX's).
pub(crate) const WEST_OF_THE_DATE_LINE: &str = "Etc/GMT+12";

/// The calendar day of one reading of the clock, in UTC and in the two
/// fixed-offset zones: [`KIRITIMATI`] and [`WEST_OF_THE_DATE_LINE`] are 26
/// hours apart, so their days always differ, and at any hour at least one of
/// them is not UTC's day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Today {
    pub(crate) utc: NaiveDate,
    pub(crate) kiritimati: NaiveDate,
    pub(crate) west: NaiveDate,
}

impl Today {
    pub(crate) fn read() -> Self {
        let now = Utc::now();
        let ahead = FixedOffset::east_opt(14 * 3_600).unwrap();
        let behind = FixedOffset::west_opt(12 * 3_600).unwrap();
        let today = Self {
            utc: now.date_naive(),
            kiritimati: now.with_timezone(&ahead).date_naive(),
            west: now.with_timezone(&behind).date_naive(),
        };
        assert_ne!(
            today.kiritimati, today.west,
            "zones 26 hours apart always read different days"
        );
        today
    }

    /// Whether the clock still reads the same three days: a golden that
    /// compares "today" reads the clock before its request and asks this
    /// after, and runs again when a midnight fell between the two.
    pub(crate) fn holds(&self) -> bool {
        Self::read() == *self
    }

    /// The day in one of the two zones.
    pub(crate) fn in_zone(&self, zone: &str) -> NaiveDate {
        match zone {
            KIRITIMATI => self.kiritimati,
            WEST_OF_THE_DATE_LINE => self.west,
            other => panic!("{other} is not one of the two fixed-offset zones"),
        }
    }
}

/// The UTC instant of 12:00:00.500250 on `date` where the athlete lives, in
/// one of the two fixed-offset zones, as [`instant`] reads it
/// (`2026-11-15T22:00:00.500250` for Kiritimati's 2026-11-16): a session
/// seeded on the athlete's own day, so how far it lies behind their today
/// does not hang on the hour the suite runs.
pub(crate) fn noon_in(date: NaiveDate, zone: &str) -> String {
    let hours_ahead = match zone {
        KIRITIMATI => 14,
        WEST_OF_THE_DATE_LINE => -12,
        other => panic!("{other} is not one of the two fixed-offset zones"),
    };
    let at =
        date.and_hms_micro_opt(12, 0, 0, 500_250).unwrap().and_utc() - Duration::hours(hours_ahead);
    at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

/// The whole days from `since` (`2026-11-16T11:30:00`, UTC) to now: what a
/// handler that subtracts instants counts, whatever the athlete's zone.
pub(crate) fn whole_days_since(since: &str) -> i64 {
    (Utc::now() - instant(since)).num_days()
}

/// `date` moved by `days`, as `2026-11-16`.
pub(crate) fn shifted(date: NaiveDate, days: i64) -> String {
    let moved = if days < 0 {
        date - Days::new(days.unsigned_abs())
    } else {
        date + Days::new(days.unsigned_abs())
    };
    moved.to_string()
}

/// The days from `from` to `to`, negative when `to` is the earlier.
pub(crate) fn days_between(from: NaiveDate, to: NaiveDate) -> i64 {
    (to - from).num_days()
}

/// The Sunday on or before `date`.
pub(crate) fn sunday_of(date: NaiveDate) -> NaiveDate {
    date - Days::new(u64::from(date.weekday().num_days_from_sunday()))
}

/// The first day of `date`'s month.
pub(crate) fn first_of_month(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap()
}

/// The instant `minutes` before a reading of the clock, for a row whose age
/// a handler measures against now.
pub(crate) fn minutes_before_now(minutes: i64) -> DateTime<Utc> {
    Utc::now() - Duration::minutes(minutes)
}

/// The first day of the month after `date`'s.
pub(crate) fn first_of_next_month(date: NaiveDate) -> NaiveDate {
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1).unwrap()
}

/// The weekday of `date` (`2026-11-16`) as the activity list writes it in
/// `locale`, from a table of its own: `Mon` in English, `lun` in French.
pub(crate) fn weekday_word(date: &str, locale: &str) -> &'static str {
    let words = match locale {
        "en" => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        "fr" => ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"],
        other => panic!("no weekday words for {other}"),
    };
    words[day(date).weekday().num_days_from_monday() as usize]
}

/// The weekday of `date` in full, in English.
pub(crate) fn weekday_name(date: &str) -> &'static str {
    [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ][day(date).weekday().num_days_from_monday() as usize]
}

/// The Monday of the week `date` falls in.
pub(crate) fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Days::new(u64::from(date.weekday().num_days_from_monday()))
}

/// Every date in `body`, as `(JSON pointer, text)`, sorted by pointer.
pub(crate) fn dates_in(body: &Value) -> Vec<(String, String)> {
    dates_in_stamped(body, &[])
}

/// [`dates_in`], with the rows at `stamped_now` carried as their
/// [`clock_shape`].
///
/// A pointer in `stamped_now` that matches no row fails: a suite cannot keep
/// excusing a field the response no longer carries.
pub(crate) fn dates_in_stamped(body: &Value, stamped_now: &[&str]) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    walk(body, String::new(), &mut rows);
    for pointer in stamped_now {
        assert!(
            rows.iter().any(|(at, _)| at == pointer),
            "no date at {pointer}, which the suite says is stamped with now: {rows:#?}"
        );
    }
    for (pointer, text) in &mut rows {
        if stamped_now.contains(&pointer.as_str()) {
            *text = clock_shape(text);
        }
    }
    rows.sort();
    rows
}

fn walk(value: &Value, pointer: String, rows: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                walk(child, format!("{pointer}/{key}"), rows);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                walk(child, format!("{pointer}/{index}"), rows);
            }
        }
        Value::String(text) => {
            let found: Vec<&str> = DATE.find_iter(text).map(|m| m.as_str()).collect();
            if found.len() == 1 && found[0] == text {
                rows.push((pointer, text.clone()));
            } else {
                for (index, date) in found.into_iter().enumerate() {
                    rows.push((format!("{pointer}#{index}"), date.to_owned()));
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// `rows` with the text at each of `pointers` carried as its [`shape`], every
/// digit kept as `d`: for a stamp the database writes from its own clock
/// (SQLite's `CURRENT_TIMESTAMP` is whole seconds), which no reading of the
/// test's clock can predict and whose width is the database's, not the
/// machine's. A pointer that matches no row fails.
pub(crate) fn shaped(mut rows: Vec<(String, String)>, pointers: &[&str]) -> Vec<(String, String)> {
    for pointer in pointers {
        let row = rows
            .iter_mut()
            .find(|(at, _)| at == pointer)
            .unwrap_or_else(|| panic!("no date at {pointer}"));
        row.1 = shape(&row.1);
    }
    rows
}

/// `rows` with the text at each of `pointers` carried as the shape of a
/// stamp the database writes from its own clock: SQLite's
/// `CURRENT_TIMESTAMP` is whole seconds, so every digit is kept
/// ([`shape`]); PostgreSQL's `NOW()` keeps microseconds, so the fraction is
/// held to the clock's widths ([`clock_shape`]).
pub(crate) fn on_database_clock(
    database: &Database,
    rows: Vec<(String, String)>,
    pointers: &[&str],
) -> Vec<(String, String)> {
    match database.backend() {
        DatabaseBackend::SQLite(_) => shaped(rows, pointers),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(_) => {
            let mut rows = rows;
            for pointer in pointers {
                let row = rows
                    .iter_mut()
                    .find(|(at, _)| at == pointer)
                    .unwrap_or_else(|| panic!("no date at {pointer}"));
                row.1 = clock_shape(&row.1);
            }
            rows
        }
    }
}

/// `stamp`, an instant a handler answered from its own reading of the
/// clock, as the database hands it back once stored: SQLite keeps the text
/// whole, PostgreSQL's `TIMESTAMPTZ` keeps microseconds, and the stamp is
/// then written as the handler writes it (`Z` or `+00:00`, trailing groups
/// of zeros dropped).
pub(crate) fn as_stored(database: &Database, stamp: &str) -> String {
    match database.backend() {
        DatabaseBackend::SQLite(_) => stamp.to_owned(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(_) => {
            let kept = DateTime::parse_from_rfc3339(stamp)
                .unwrap()
                .with_timezone(&Utc)
                .trunc_subsecs(6)
                .to_rfc3339();
            if stamp.ends_with('Z') {
                kept.replace("+00:00", "Z")
            } else {
                kept
            }
        }
    }
}

/// The literal table a golden compares [`dates_in`] with.
pub(crate) fn table(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = rows
        .iter()
        .map(|(pointer, text)| ((*pointer).to_owned(), (*text).to_owned()))
        .collect();
    rows.sort();
    rows
}

/// `rows` with the value at each of `pointers` added as a row, written as
/// JSON: the numbers and flags a handler computes from dates, which the walk
/// does not see. A pointer the body does not carry fails.
pub(crate) fn with_values(
    mut rows: Vec<(String, String)>,
    body: &Value,
    pointers: &[&str],
) -> Vec<(String, String)> {
    for pointer in pointers {
        let value = body
            .pointer(pointer)
            .unwrap_or_else(|| panic!("nothing at {pointer}: {body}"));
        rows.push(((*pointer).to_owned(), value.to_string()));
    }
    rows.sort();
    rows
}

/// One athlete: the account's id, the tenant they act in and a bearer for it.
pub(crate) struct Athlete {
    pub(crate) user_id: Uuid,
    pub(crate) tenant: TenantId,
    pub(crate) token: String,
}

/// An athlete on the starter plan who reads in `locale`, with `zone` on file
/// when one is given.
pub(crate) async fn seed_athlete(
    resources: &Arc<ServerContext>,
    label: &str,
    zone: Option<&str>,
    locale: &str,
) -> Athlete {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let token = common::generate_test_token(resources, &user).await;
    let users = &resources.common.repos.users;
    users.update_locale(user_id, locale).await.unwrap();
    if let Some(zone) = zone {
        users.set_timezone(user_id, zone).await.unwrap();
    }
    Athlete {
        user_id,
        tenant,
        token,
    }
}

/// An account whose own stamps the suite chooses: created and approved at
/// `created`, last active at `last_active`, owning one tenant created at
/// `created`.
pub(crate) async fn seed_account(
    resources: &Arc<ServerContext>,
    label: &str,
    role: UserRole,
    created: DateTime<Utc>,
    last_active: DateTime<Utc>,
) -> Athlete {
    let mut user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        bcrypt::hash("password123", 4).unwrap(),
        Some(label.to_owned()),
    );
    user.role = role;
    user.user_status = UserStatus::Active;
    user.created_at = created;
    user.last_active = last_active;
    user.approved_at = Some(created);
    let repos = &resources.common.repos;
    repos.users.create(&user).await.unwrap();
    let tenant = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: created,
            updated_at: created,
        })
        .await
        .unwrap();
    repos.users.update_tenant_id(user.id, tenant).await.unwrap();
    let token = common::generate_test_token(resources, &user).await;
    Athlete {
        user_id: user.id,
        tenant,
        token,
    }
}

/// Run `sql` with `at` as `$1`, `id` as `$2` and, when given, `tenant` as
/// `$3`: a row's instant moved to one the suite chooses, where the repository
/// stamps it with the wall clock and takes none from its caller.
///
/// A statement on a table with a `tenant_id` column filters on it through
/// `$3`, bound to the tenant the suite seeded the row under; `None` is only
/// for a table without one.
pub(crate) async fn restamp(
    database: &Database,
    sql: &'static str,
    at: DateTime<Utc>,
    id: &str,
    tenant: Option<TenantId>,
) {
    let tenant = tenant.map(|tenant| tenant.to_string());
    let changed = match database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            let query = sqlx::query(sql).bind(at).bind(id);
            match &tenant {
                Some(tenant) => query.bind(tenant.as_str()),
                None => query,
            }
            .execute(sqlite.pool())
            .await
            .unwrap()
            .rows_affected()
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            let query = sqlx::query(sql).bind(at).bind(id);
            match &tenant {
                Some(tenant) => query.bind(tenant.as_str()),
                None => query,
            }
            .execute(postgres.pool())
            .await
            .unwrap()
            .rows_affected()
        }
    };
    assert_eq!(changed, 1, "{sql} moved one row ({id})");
}

/// [`restamp`] for a column that holds whole seconds since the epoch.
pub(crate) async fn restamp_seconds(
    database: &Database,
    sql: &'static str,
    seconds: i64,
    id: &str,
    tenant: Option<TenantId>,
) {
    let tenant = tenant.map(|tenant| tenant.to_string());
    let changed = match database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            let query = sqlx::query(sql).bind(seconds).bind(id);
            match &tenant {
                Some(tenant) => query.bind(tenant.as_str()),
                None => query,
            }
            .execute(sqlite.pool())
            .await
            .unwrap()
            .rows_affected()
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            let query = sqlx::query(sql).bind(seconds).bind(id);
            match &tenant {
                Some(tenant) => query.bind(tenant.as_str()),
                None => query,
            }
            .execute(postgres.pool())
            .await
            .unwrap()
            .rows_affected()
        }
    };
    assert_eq!(changed, 1, "{sql} moved one row ({id})");
}

/// One request through `router`, answered as status and JSON body (`Null`
/// when the body is not JSON).
pub(crate) async fn send(router: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A bearer `GET`.
pub(crate) fn get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

/// A bearer request with a JSON body.
pub(crate) fn with_json(method: &str, uri: &str, token: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// One tool call as the agent makes it for `athlete`, in their tenant: the
/// response, or the refusal's text when the call is refused before it runs.
pub(crate) async fn try_tool(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    tool: &str,
    parameters: Value,
) -> Result<UniversalResponse, String> {
    UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters,
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .map_err(|refusal| refusal.to_string())
}

/// [`try_tool`], for a call that is not refused.
pub(crate) async fn call_tool(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    tool: &str,
    parameters: Value,
) -> UniversalResponse {
    try_tool(resources, athlete, tool, parameters)
        .await
        .unwrap_or_else(|refusal| panic!("{tool} did not dispatch: {refusal}"))
}

/// The payload of a tool call that succeeded.
pub(crate) async fn tool_payload(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    tool: &str,
    parameters: Value,
) -> Value {
    let response = call_tool(resources, athlete, tool, parameters).await;
    assert!(response.success, "{tool} ran: {:?}", response.error);
    response
        .result
        .unwrap_or_else(|| panic!("{tool} returned no payload"))
}

/// One tool call as an MCP client makes it: JSON-RPC `tools/call` on
/// `POST /mcp` of the server's whole HTTP app, under a personal MCP token
/// minted for `athlete`. Answers the JSON-RPC envelope.
pub(crate) async fn mcp_call(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    tool: &str,
    arguments: Value,
) -> Value {
    let token = resources
        .common
        .repos
        .user_mcp_tokens
        .create_token(
            athlete.user_id,
            &CreateUserMcpTokenRequest {
                name: "date golden".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap()
        .token_value
        .expose_secret()
        .to_owned();
    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    });
    let (status, envelope) = send(
        ProviderToolRouter::build_http_app(resources),
        with_json("POST", "/mcp", &token, &call),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tool}: {envelope}");
    assert_eq!(envelope["jsonrpc"], "2.0", "{envelope}");
    assert_eq!(envelope["id"], 7, "{envelope}");
    envelope
}

/// The payload of a `tools/call` that succeeded, with its envelope checked:
/// one text block, whose text is the structured content serialized as it is,
/// so every date reaches the client twice and character for character alike.
pub(crate) fn mcp_payload(envelope: &Value) -> Value {
    let result = &envelope["result"];
    assert_eq!(result["isError"], false, "{envelope}");
    let blocks = result["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 1, "{envelope}");
    assert_eq!(blocks[0]["type"], "text", "{envelope}");
    let structured = result["structuredContent"].clone();
    assert_eq!(
        blocks[0]["text"].as_str().unwrap(),
        structured.to_string(),
        "the text block is the structured content"
    );
    structured
}

/// The text of a `tools/call` the server refused.
pub(crate) fn mcp_refusal(envelope: &Value) -> String {
    let result = &envelope["result"];
    assert_eq!(result["isError"], true, "{envelope}");
    result["content"][0]["text"].as_str().unwrap().to_owned()
}
