// ABOUTME: The athlete's civil date, resolved from the timezone stored on their user row
// ABOUTME: The one read-then-resolve every plan surface shares, over pierre-core's civil clock
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Today, on the athlete's calendar.
//!
//! Every surface that projects a plan for "today" — the plan card the reply
//! carries, the coverage and readiness rails, the calendar-push preview, the
//! flavour resolver — needs the same value: the civil date the athlete is
//! living in, taken from the IANA zone on their user row and falling back to
//! UTC when there is none.
//!
//! The arithmetic belongs to [`pierre_core::civil_time`], which owns the
//! zone-resolution and clock-reading rules and the incidents behind them. What
//! lives here is the half that needs the database: the users-table read that
//! supplies the zone. That read is why the function cannot sit in
//! `pierre-core`, and it is the half that was being written out again at each
//! call site — once per crate, with the two copies differing only in whether
//! they took a `Uuid` or a `&str` they then parsed back into one.
//!
//! Taking the `Uuid` is deliberate. The `&str` form swallowed a malformed id
//! into the same UTC fallback that an athlete with no zone on file gets, so a
//! caller passing the wrong string got a plausible date instead of a type
//! error. Every caller already holds the `Uuid`.

use chrono::{NaiveDate, Utc};
use pierre_core::civil_time::{clock_date, resolve_zone};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// The civil date the athlete is living in.
///
/// Best-effort by design: an unreadable user row, an absent timezone and an
/// unparseable one all collapse to UTC, because every caller is projecting a
/// plan rather than answering a question, and a plan shown against the server's
/// day is better than no plan. That is the same fallback
/// [`resolve_zone`] applies, so this agrees with the date anchor and the
/// activity list rather than drifting from them.
pub async fn athlete_today(repos: &RepositoryRegistry, user_id: Uuid) -> NaiveDate {
    let timezone = repos
        .users
        .get_global(user_id)
        .await
        .ok()
        .flatten()
        .and_then(|user| user.timezone);
    clock_date(Utc::now(), resolve_zone(timezone.as_deref()))
}
