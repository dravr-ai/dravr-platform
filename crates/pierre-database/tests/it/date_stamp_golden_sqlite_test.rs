// ABOUTME: Pins the date parsing and stamping the repositories do themselves, through their public methods
// ABOUTME: Recorded from the date engine in use on 2026-11-17; a SQLite-only suite, it plants raw column text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Goldens for the dates the repositories compute (carnet#1620, gates 7 and 8).
//!
//! Most repositories hand an instant to the driver and take one back. Four
//! places do their own date work, in functions that are private to the crate.
//! Each is asserted here through the nearest public method that reaches it:
//!
//! | Private function | Reached through | What it does |
//! |---|---|---|
//! | `parse_step_time` | `UserOnboardingRepository::get_onboarding_steps` | reads `updated_at` text in three shapes |
//! | `next_setting_stamp` | `compare_and_set_system_setting` | cuts a stamp to microseconds and moves it past the prior one |
//! | `whole_seconds` | `SubscriptionsRepository::upsert_subscription` | cuts `last_event_at` to whole seconds |
//! | (public) | `analytics::next_utc_month_start` | the first instant of the next UTC month |
//!
//! `next_setting_stamp` reads the wall clock and has no clock seam, so only
//! its other branch is a golden: with a prior stamp in the future, the result
//! depends on the prior stamp alone. The branch that cuts the wall clock
//! itself is not pinned here: where the system clock ticks in microseconds
//! there is no digit for the cut to drop, and a test of it could not fail.
//!
//! This suite is `SQLite` by construction: it plants column text the way
//! another writer would have left it, and reads the raw column back.
//!
//! Everything that names the date engine lives in [`engine`].

use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, User, UserTier};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::analytics::next_utc_month_start;
use pierre_test_support::db::create_sqlite_test_db;
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

/// Everything in this file that names the date engine.
mod engine {
    use chrono::{DateTime, NaiveDateTime, Utc};

    pub(super) type Instant = DateTime<Utc>;

    /// A UTC instant from `2026-11-17T10:00:05.123456789`, any fraction or none.
    pub(super) fn instant(text: &str) -> Instant {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
            .unwrap()
            .and_utc()
    }

    /// The instant at nanosecond precision in UTC.
    pub(super) fn show(value: Instant) -> String {
        value.format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string()
    }
}

const REJECTED: &str = "rejected";

fn pool(db: &Database) -> &Pool<Sqlite> {
    db.sqlite_pool()
        .expect("create_sqlite_test_db opens SQLite")
}

async fn fresh_user(db: &Database) -> Uuid {
    let user = User::new(
        format!("date-stamp-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Date Stamp".to_owned()),
    );
    db.repositories().users.create(&user).await.unwrap()
}

/// Plants `text` as a step's `updated_at` and reads the step back through the
/// repository.
async fn step_time(db: &Database, user_id: &str, text: &str) -> String {
    sqlx::query("UPDATE user_onboarding SET updated_at = $1 WHERE user_id = $2")
        .bind(text)
        .bind(user_id)
        .execute(pool(db))
        .await
        .unwrap();
    let steps = db
        .repositories()
        .user_onboarding
        .get_onboarding_steps(user_id)
        .await
        .unwrap();
    assert_eq!(
        steps.len(),
        1,
        "the step is listed whatever its time reads as"
    );
    steps[0]
        .updated_at
        .map_or_else(|| REJECTED.to_owned(), engine::show)
}

/// The verdict is the whole function's: it trims, tries RFC 3339, then the
/// shape `PostgreSQL` renders, then the shape `SQLite` defaults write, read as
/// UTC. A time it cannot read leaves the step listed with no time.
#[tokio::test]
async fn an_onboarding_step_time_reads_in_three_shapes() {
    let db = create_sqlite_test_db().await.unwrap();
    let user_id = fresh_user(&db).await.to_string();
    db.repositories()
        .user_onboarding
        .set_onboarding_step(&user_id, "connect_provider", "done", None, None)
        .await
        .unwrap();

    let expected = [
        // What the SQLite default writes.
        ("2026-11-13 10:04:05", "2026-11-13T10:04:05.000000000Z"),
        ("2026-11-13 10:04:05.250", "2026-11-13T10:04:05.250000000Z"),
        (
            "2026-11-13 10:04:05.123456789",
            "2026-11-13T10:04:05.123456789Z",
        ),
        // What PostgreSQL renders a TIMESTAMPTZ as.
        ("2026-11-13 10:04:05+00", "2026-11-13T10:04:05.000000000Z"),
        ("2026-11-13 05:04:05-05", "2026-11-13T10:04:05.000000000Z"),
        (
            "2026-11-13 15:34:05+05:30",
            "2026-11-13T10:04:05.000000000Z",
        ),
        (
            "2026-11-13 10:04:05.250000+00",
            "2026-11-13T10:04:05.250000000Z",
        ),
        (
            "2026-11-13 10:04:05.25+00",
            "2026-11-13T10:04:05.250000000Z",
        ),
        // RFC 3339, tried first.
        ("2026-11-13T10:04:05Z", "2026-11-13T10:04:05.000000000Z"),
        (
            "2026-11-13T10:04:05+00:00",
            "2026-11-13T10:04:05.000000000Z",
        ),
        (
            "2026-11-13T05:04:05.120-05:00",
            "2026-11-13T10:04:05.120000000Z",
        ),
        ("2026-11-13 10:04:05Z", "2026-11-13T10:04:05.000000000Z"),
        ("2026-11-13t10:04:05z", "2026-11-13T10:04:05.000000000Z"),
        // Trimmed before any of them is tried.
        ("  2026-11-13 10:04:05  ", "2026-11-13T10:04:05.000000000Z"),
        ("\t2026-11-13T10:04:05Z\n", "2026-11-13T10:04:05.000000000Z"),
        // Shapes one parser takes and the function still reads or refuses.
        ("2026-11-13T10:04:05", "rejected"),
        ("2026-11-13 10:04:05 UTC", "rejected"),
        ("2026-11-13 10:04:05 +00", "2026-11-13T10:04:05.000000000Z"),
        ("2026-11-13 10:04:05+0000", "2026-11-13T10:04:05.000000000Z"),
        ("2026-11-13 10:04", "rejected"),
        ("2026-11-13", "rejected"),
        ("2026-1-3 9:4:5", "2026-01-03T09:04:05.000000000Z"),
        ("2026-11-13 10:04:60", "2026-11-13T10:04:60.000000000Z"),
        ("2026-11-13 24:00:00", "rejected"),
        ("1794909605", "rejected"),
        ("yesterday", "rejected"),
        ("", "rejected"),
    ];

    let mut actual = Vec::new();
    for (text, _) in expected {
        actual.push((text, step_time(&db, &user_id, text).await));
    }
    let wanted: Vec<(&str, String)> = expected
        .iter()
        .map(|(text, verdict)| (*text, (*verdict).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "planted step text, then the time it reads as"
    );
}

/// A compare-and-set whose prior stamp is not in the past answers with the
/// prior stamp cut to microseconds plus one microsecond, and stores it.
#[tokio::test]
async fn a_setting_stamp_is_whole_microseconds_past_the_prior_one() {
    let db = create_sqlite_test_db().await.unwrap();

    // the prior stamp, the stamp the write answers with
    let expected = [
        ("2099-01-01T00:00:00", "2099-01-01T00:00:00.000001000Z"),
        (
            "2099-01-01T00:00:00.123456789",
            "2099-01-01T00:00:00.123457000Z",
        ),
        (
            "2099-01-01T00:00:00.123456",
            "2099-01-01T00:00:00.123457000Z",
        ),
        (
            "2099-01-01T00:00:00.999999999",
            "2099-01-01T00:00:01.000000000Z",
        ),
        (
            "2099-12-31T23:59:59.999999500",
            "2100-01-01T00:00:00.000000000Z",
        ),
    ];

    let mut actual = Vec::new();
    let numbered = expected.iter().enumerate();
    for (position, (prior, _)) in numbered {
        let key = format!("date-golden-{position}");
        db.compare_and_set_system_setting(&key, "first", None)
            .await
            .unwrap()
            .expect("an absent row is created");
        let prior_stamp = engine::instant(prior);
        sqlx::query("UPDATE system_settings SET updated_at = $1 WHERE key = $2")
            .bind(prior_stamp)
            .bind(&key)
            .execute(pool(&db))
            .await
            .unwrap();

        let stamp = db
            .compare_and_set_system_setting(&key, "second", Some(prior_stamp))
            .await
            .unwrap()
            .expect("the row is as the prior stamp read it");
        let stored = db.get_system_setting(&key).await.unwrap().unwrap();
        assert_eq!(stored.updated_at, stamp, "the answer is what was stored");
        actual.push((*prior, engine::show(stamp)));
    }
    let wanted: Vec<(&str, String)> = expected
        .iter()
        .map(|(prior, stamp)| (*prior, (*stamp).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "prior stamp, then the stamp the write answers with"
    );
}

/// A subscription's `last_event_at` is cut to whole seconds before it is
/// bound, so the stored text never carries a fraction and never rounds up.
#[tokio::test]
async fn a_subscription_event_time_is_stored_at_whole_seconds() {
    let db = create_sqlite_test_db().await.unwrap();
    let user_id = fresh_user(&db).await;

    // the event time, the stored text
    let expected = [
        ("2026-11-17T10:00:05", "2026-11-17T10:00:05+00:00"),
        ("2026-11-17T10:00:05.250", "2026-11-17T10:00:05+00:00"),
        ("2026-11-17T10:00:05.999999999", "2026-11-17T10:00:05+00:00"),
        ("2026-11-17T23:59:59.999999999", "2026-11-17T23:59:59+00:00"),
    ];

    let created = engine::instant("2026-11-17T09:00:00.120");
    let mut actual = Vec::new();
    let numbered = expected.iter().enumerate();
    for (position, (event, _)) in numbered {
        let subscription = Subscription {
            id: Uuid::new_v4(),
            tenant_id: TenantId::generate(),
            user_id,
            provider: "dummy".to_owned(),
            provider_customer_id: format!("cus_date_golden_{position}"),
            provider_subscription_id: Some(format!("sub_date_golden_{position}")),
            status: SubscriptionStatus::Active,
            plan_tier: UserTier::Professional,
            current_period_start: Some(created),
            current_period_end: None,
            cancel_at_period_end: false,
            canceled_at: None,
            trial_end: None,
            metadata: None,
            created_at: created,
            updated_at: created,
            last_event_at: Some(engine::instant(event)),
        };
        db.repositories()
            .subscriptions
            .upsert_subscription(&subscription)
            .await
            .unwrap();
        let row = sqlx::query(
            "SELECT last_event_at, current_period_start FROM subscriptions
             WHERE id = $1 AND tenant_id = $2",
        )
        .bind(subscription.id.to_string())
        .bind(subscription.tenant_id)
        .fetch_one(pool(&db))
        .await
        .unwrap();
        assert_eq!(
            row.get::<String, _>(1),
            "2026-11-17T09:00:00.120+00:00",
            "the other timestamps keep their fraction"
        );
        actual.push((*event, row.get::<String, _>(0)));
    }
    let wanted: Vec<(&str, String)> = expected
        .iter()
        .map(|(event, stored)| (*event, (*stored).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "event time, then the text stored for it");
}

/// The monthly request budget resets at the first instant of the next UTC
/// month, whatever day and time of the month it is asked on.
#[test]
fn the_next_utc_month_starts_at_midnight_on_the_first() {
    let expected = [
        (
            "2026-11-17T10:00:05.123456789",
            "2026-12-01T00:00:00.000000000Z",
        ),
        ("2026-11-01T00:00:00", "2026-12-01T00:00:00.000000000Z"),
        (
            "2026-11-30T23:59:59.999999999",
            "2026-12-01T00:00:00.000000000Z",
        ),
        (
            "2026-12-31T23:59:59.999999999",
            "2027-01-01T00:00:00.000000000Z",
        ),
        ("2026-01-31T12:00:00", "2026-02-01T00:00:00.000000000Z"),
        ("2024-01-31T12:00:00", "2024-02-01T00:00:00.000000000Z"),
        ("2024-02-29T12:00:00", "2024-03-01T00:00:00.000000000Z"),
        ("2026-03-31T12:00:00", "2026-04-01T00:00:00.000000000Z"),
    ];
    let actual: Vec<(&str, String)> = expected
        .iter()
        .map(|(now, _)| {
            (
                *now,
                engine::show(next_utc_month_start(engine::instant(now))),
            )
        })
        .collect();
    let wanted: Vec<(&str, String)> = expected
        .iter()
        .map(|(now, start)| (*now, (*start).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "asked at, then the next UTC month's first instant"
    );
}
