// ABOUTME: Pins the relative-day vocabulary — today and yesterday on the athlete's calendar
// ABOUTME: Regression for 2026-09-21, when the newest activity row became "ce matin"
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Two halves, like the weekday table: what a prompt surface *writes* next to
//! a date, and what the verifier *reads* out of a reply. The writing half is
//! one word per locale; the reading half is per locale on purpose, because
//! French `hier` is German `hier` (here).

use chrono::NaiveDate;
use pierre_core::civil_time::{
    format_civil_day, relative_day, relative_day_forms, relative_day_label, RelativeDay,
};

fn sept(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, d).unwrap()
}

#[test]
fn only_today_and_yesterday_are_relative_days() {
    let today = sept(21);
    assert_eq!(relative_day(sept(21), today), Some(RelativeDay::Today));
    assert_eq!(relative_day(sept(20), today), Some(RelativeDay::Yesterday));
    assert_eq!(
        relative_day(sept(19), today),
        None,
        "two days ago is a date"
    );
    assert_eq!(relative_day(sept(22), today), None, "tomorrow is a date");
}

#[test]
fn the_written_label_follows_the_locale() {
    for (locale, today, yesterday) in [
        ("fr", "aujourd'hui", "hier"),
        ("en", "today", "yesterday"),
        ("es", "hoy", "ayer"),
        ("de", "heute", "gestern"),
        ("pt", "hoje", "ontem"),
    ] {
        assert_eq!(relative_day_label(RelativeDay::Today, locale), today);
        assert_eq!(
            relative_day_label(RelativeDay::Yesterday, locale),
            yesterday
        );
    }
    assert_eq!(
        relative_day_label(RelativeDay::Yesterday, "xx"),
        "yesterday",
        "an unknown locale falls back to English like the weekday table"
    );
}

/// The reading table is per locale so that German `hier` is never yesterday,
/// and every locale still reads English because the model falls back to it.
#[test]
fn the_read_forms_are_per_locale_and_always_include_english() {
    assert!(
        !relative_day_forms(RelativeDay::Yesterday, "de").contains(&"hier"),
        "German `hier` means here"
    );
    assert!(relative_day_forms(RelativeDay::Yesterday, "fr").contains(&"hier"));
    for locale in ["fr", "en", "es", "de", "pt"] {
        assert!(
            relative_day_forms(RelativeDay::Yesterday, locale).contains(&"yesterday"),
            "{locale} must read English"
        );
        assert!(
            relative_day_forms(RelativeDay::Today, locale).contains(&"this morning"),
            "{locale} must read English times of day as today"
        );
    }
    assert!(
        relative_day_forms(RelativeDay::Today, "fr").contains(&"ce matin"),
        "a time of day asserts the current day"
    );
}

#[test]
fn a_civil_day_renders_with_its_weekday() {
    assert_eq!(format_civil_day(sept(21), "fr"), "2026-09-21 lun");
    assert_eq!(format_civil_day(sept(20), "en"), "2026-09-20 Sun");
}
