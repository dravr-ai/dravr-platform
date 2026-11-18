// ABOUTME: Pins that a date written through sqlx reads back as the same value on SQLite and on PostgreSQL
// ABOUTME: Each engine keeps its own precision: nanoseconds as SQLite text, microseconds in a PostgreSQL column
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Round-trip goldens for dates on both engines (carnet#1620, gate 4).
//!
//! These run on whichever backend `DATABASE_URL` names, so the same writes are
//! checked on `SQLite` locally and on `PostgreSQL` in its CI lane. What comes
//! back differs by engine and both answers are pinned:
//!
//! - `SQLite` stores text and keeps all nine fraction digits;
//! - `PostgreSQL` stores microseconds, so the last three digits are cut.
//!
//! The `PostgreSQL` expectations for the scratch table come from the driver's
//! source (it stores whole microseconds since 2000-01-01); every value here is
//! on or after that date, where cutting and rounding down agree. They are
//! first executed by the `PostgreSQL` lane, not by a local run.
//!
//! Everything that names the date engine lives in [`engine`].

use pierre_core::models::{ActivityBuilder, SportType, TenantId, User, UserOAuthToken};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_test_support::db::create_test_db;
use secrecy::SecretString;
use sqlx::Row;
use uuid::Uuid;

/// Everything in this file that names the date engine.
mod engine {
    use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};

    pub(super) type Instant = DateTime<Utc>;
    pub(super) type CivilDate = NaiveDate;
    pub(super) type CivilTime = NaiveTime;
    pub(super) type CivilDateTime = NaiveDateTime;

    const STAMP: &str = "%Y-%m-%dT%H:%M:%S%.9f";

    /// A zoneless date-time from `2026-11-17T10:00:05.120000000`.
    pub(super) fn civil(text: &str) -> CivilDateTime {
        NaiveDateTime::parse_from_str(text, STAMP).unwrap()
    }

    /// The UTC instant at that date and time.
    pub(super) fn instant(text: &str) -> Instant {
        civil(text).and_utc()
    }

    /// The same shape back.
    pub(super) fn show(value: Instant) -> String {
        value.format(STAMP).to_string()
    }

    /// The same shape for a zoneless date-time.
    pub(super) fn show_civil(value: CivilDateTime) -> String {
        value.format(STAMP).to_string()
    }

    /// The date part of a zoneless date-time.
    pub(super) fn date_of(value: CivilDateTime) -> CivilDate {
        value.date()
    }

    /// The time-of-day part of a zoneless date-time.
    pub(super) fn time_of(value: CivilDateTime) -> CivilTime {
        value.time()
    }

    /// A calendar date as `2026-11-17`.
    pub(super) fn show_date(value: CivilDate) -> String {
        value.to_string()
    }

    /// A time of day at nanosecond precision.
    pub(super) fn show_time(value: CivilTime) -> String {
        value.format("%H:%M:%S%.9f").to_string()
    }

    /// The instant a whole number of days away.
    pub(super) fn plus_days(value: Instant, days: i64) -> Instant {
        value + Duration::days(days)
    }

    /// The instant a whole number of milliseconds away.
    pub(super) fn plus_millis(value: Instant, millis: i64) -> Instant {
        value + Duration::milliseconds(millis)
    }
}

/// Whether the factory opened `SQLite`, which keeps nanoseconds.
fn keeps_nanoseconds(db: &Database) -> bool {
    match db.backend() {
        DatabaseBackend::SQLite(_) => true,
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(_) => false,
    }
}

/// The expected text for this engine: all nine digits, or cut to six.
fn at_engine_precision(db: &Database, nanoseconds: &str, microseconds: &str) -> String {
    if keeps_nanoseconds(db) {
        nanoseconds.to_owned()
    } else {
        microseconds.to_owned()
    }
}

async fn fresh_user(db: &Database, created_at: engine::Instant) -> User {
    let mut user = User::new(
        format!("date-round-trip-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Date Round Trip".to_owned()),
    );
    user.created_at = created_at;
    user.last_active = engine::instant("2026-11-17T23:59:59.000000000");
    user.approved_at = Some(engine::instant("2026-11-17T10:00:05.100000000"));
    db.repositories().users.create(&user).await.unwrap();
    user
}

#[tokio::test]
async fn a_user_reads_back_with_the_timestamps_it_was_written_with() {
    let db = create_test_db().await.unwrap();
    let written = fresh_user(&db, engine::instant("2026-11-17T10:00:05.123456789")).await;

    let read = db
        .repositories()
        .users
        .get_global(written.id)
        .await
        .unwrap()
        .expect("the user just written must read back");

    assert_eq!(
        engine::show(read.created_at),
        at_engine_precision(
            &db,
            "2026-11-17T10:00:05.123456789",
            "2026-11-17T10:00:05.123456000"
        ),
        "created_at keeps the engine's precision"
    );
    assert_eq!(
        engine::show(read.last_active),
        "2026-11-17T23:59:59.000000000",
        "last_active reads back on a whole second"
    );
    assert_eq!(
        read.approved_at.map(engine::show),
        Some("2026-11-17T10:00:05.100000000".to_owned()),
        "approved_at reads back with its milliseconds"
    );
}

#[tokio::test]
async fn an_oauth_token_reads_back_with_the_timestamps_it_was_written_with() {
    let db = create_test_db().await.unwrap();
    let user = fresh_user(&db, engine::instant("2026-11-17T10:00:05.000000000")).await;
    let tenant_id = TenantId::generate();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id: user.id,
        tenant_id: tenant_id.to_string(),
        provider: "strava".to_owned(),
        access_token: SecretString::from("access-date-round-trip".to_owned()),
        refresh_token: None,
        token_type: "Bearer".to_owned(),
        expires_at: Some(engine::instant("2026-11-17T16:00:05.123400000")),
        scope: None,
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at: engine::instant("2026-11-17T10:00:05.120000000"),
        updated_at: engine::instant("2024-02-29T23:59:59.999999000"),
    };
    db.repositories()
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();

    let read = db
        .repositories()
        .oauth_tokens
        .get_token(user.id, tenant_id, "strava")
        .await
        .unwrap()
        .expect("the token just written must read back");

    assert_eq!(
        read.expires_at.map(engine::show),
        Some("2026-11-17T16:00:05.123400000".to_owned()),
        "expires_at reads back with four fraction digits"
    );
    assert_eq!(
        engine::show(read.created_at),
        "2026-11-17T10:00:05.120000000",
        "created_at reads back with its milliseconds"
    );
    assert_eq!(
        engine::show(read.updated_at),
        "2024-02-29T23:59:59.999999000",
        "microseconds survive on both engines"
    );
}

#[tokio::test]
async fn a_cached_activity_reads_back_with_the_start_it_was_written_with() {
    let db = create_test_db().await.unwrap();
    let start = engine::instant("2026-10-27T07:00:00.120000000");
    let user = fresh_user(&db, start).await;
    let tenant_id = TenantId::generate();
    let activity = ActivityBuilder::new(
        "route-1",
        "Lunch run",
        SportType::Run,
        start,
        1_800,
        "sciotte_strava",
    )
    .distance_meters(5_000.0)
    .build();
    db.repositories()
        .activity_cache
        .upsert_activities(user.id, &tenant_id, "strava", &[activity])
        .await
        .unwrap();

    let window = |from: i64, to: i64| {
        let repos = db.repositories();
        async move {
            repos
                .activity_cache
                .get_cached_activities(
                    user.id,
                    &tenant_id,
                    Some("strava"),
                    engine::plus_days(start, from),
                    engine::plus_days(start, to),
                    10,
                )
                .await
                .unwrap()
        }
    };

    let around = window(-1, 1).await;
    assert_eq!(around.len(), 1, "a window around the start finds the row");
    assert_eq!(
        engine::show(around[0].start_date()),
        "2026-10-27T07:00:00.120000000",
        "the cached start reads back with its milliseconds"
    );
    assert_eq!(
        window(1, 2).await.len(),
        0,
        "a window after the start does not"
    );
    assert_eq!(
        window(-2, -1).await.len(),
        0,
        "a window before the start does not"
    );
}

/// A window whose edge falls in the same second as the start. On `SQLite`
/// the window is a comparison of text, so it only answers correctly while the
/// column and the bound edges are written in one shape: within one second,
/// the `+` of `+00:00` sorts before the `.` of a fraction and a `Z` sorts
/// after both. A start stored in any other shape drops out of a window that
/// ends on it. `PostgreSQL` compares instants and gives the same answers.
#[tokio::test]
async fn a_window_edge_in_the_same_second_as_the_start_finds_the_row() {
    let db = create_test_db().await.unwrap();
    let whole = engine::instant("2026-10-27T07:00:00.000000000");
    let fraction = engine::instant("2026-10-28T07:00:00.120000000");
    let user = fresh_user(&db, whole).await;
    let tenant_id = TenantId::generate();
    let activities = [("whole-second", whole), ("fraction", fraction)].map(|(id, start)| {
        ActivityBuilder::new(
            id,
            "Lunch run",
            SportType::Run,
            start,
            1_800,
            "sciotte_strava",
        )
        .build()
    });
    db.repositories()
        .activity_cache
        .upsert_activities(user.id, &tenant_id, "strava", &activities)
        .await
        .unwrap();

    // start, window edges in milliseconds from the start, rows found
    let rows = [
        // The window is the start itself, inclusive at both ends.
        (whole, 0, 0, 1),
        (fraction, 0, 0, 1),
        // The start is the upper edge.
        (whole, -500, 0, 1),
        (fraction, -120, 0, 1),
        (fraction, -1_000, 0, 1),
        // The start is the lower edge.
        (whole, 0, 500, 1),
        (fraction, 0, 380, 1),
        (fraction, 0, 1_000, 1),
        // Both edges inside the start's second, around it.
        (whole, -1, 1, 1),
        (fraction, -1, 1, 1),
        (fraction, -120, 880, 1),
        // One millisecond off, on either side.
        (whole, 1, 500, 0),
        (whole, -500, -1, 0),
        (fraction, 1, 500, 0),
        (fraction, -120, -1, 0),
    ];
    let mut actual = Vec::new();
    for (start, from, to, _) in rows {
        let found = db
            .repositories()
            .activity_cache
            .get_cached_activities(
                user.id,
                &tenant_id,
                Some("strava"),
                engine::plus_millis(start, from),
                engine::plus_millis(start, to),
                10,
            )
            .await
            .unwrap();
        actual.push((engine::show(start), from, to, found.len()));
    }
    let wanted: Vec<(String, i64, i64, usize)> = rows
        .iter()
        .map(|(start, from, to, found)| (engine::show(*start), *from, *to, *found))
        .collect();
    assert_eq!(
        actual, wanted,
        "start, window edges in milliseconds, rows found"
    );
}

/// One row of every date type bound and read back through a raw statement.
struct ScratchRow {
    at: engine::Instant,
    day: engine::CivilDate,
    time: engine::CivilTime,
    wall: engine::CivilDateTime,
}

impl ScratchRow {
    fn shown(&self) -> [String; 4] {
        [
            engine::show(self.at),
            engine::show_date(self.day),
            engine::show_time(self.time),
            engine::show_civil(self.wall),
        ]
    }
}

const SCRATCH_INSERT: &str =
    "INSERT INTO date_golden (slot, stamp_at, civil_day, clock_time, wall_clock)
     VALUES ($1, $2, $3, $4, $5)";
const SCRATCH_SELECT: &str =
    "SELECT stamp_at, civil_day, clock_time, wall_clock FROM date_golden WHERE slot = $1";

/// Writes the row into a scratch table typed the way each engine's schema
/// types dates, and reads it back.
async fn through_a_scratch_table(db: &Database, position: i32, row: &ScratchRow) -> ScratchRow {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS date_golden (
                    slot INTEGER PRIMARY KEY, stamp_at TEXT, civil_day TEXT, clock_time TEXT,
                    wall_clock TEXT
                )",
            )
            .execute(sqlite.pool())
            .await
            .unwrap();
            sqlx::query(SCRATCH_INSERT)
                .bind(position)
                .bind(row.at)
                .bind(row.day)
                .bind(row.time)
                .bind(row.wall)
                .execute(sqlite.pool())
                .await
                .unwrap();
            let read = sqlx::query(SCRATCH_SELECT)
                .bind(position)
                .fetch_one(sqlite.pool())
                .await
                .unwrap();
            ScratchRow {
                at: read.get(0),
                day: read.get(1),
                time: read.get(2),
                wall: read.get(3),
            }
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS date_golden (
                    slot INTEGER PRIMARY KEY, stamp_at TIMESTAMPTZ, civil_day DATE,
                    clock_time TIME, wall_clock TIMESTAMP
                )",
            )
            .execute(pg.pool())
            .await
            .unwrap();
            sqlx::query(SCRATCH_INSERT)
                .bind(position)
                .bind(row.at)
                .bind(row.day)
                .bind(row.time)
                .bind(row.wall)
                .execute(pg.pool())
                .await
                .unwrap();
            let read = sqlx::query(SCRATCH_SELECT)
                .bind(position)
                .fetch_one(pg.pool())
                .await
                .unwrap();
            ScratchRow {
                at: read.get(0),
                day: read.get(1),
                time: read.get(2),
                wall: read.get(3),
            }
        }
    }
}

#[tokio::test]
async fn every_date_type_round_trips_through_a_raw_statement() {
    let db = create_test_db().await.unwrap();

    // written, read back on SQLite, read back on PostgreSQL
    let rows = [
        (
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T10:00:05.100000000",
            "2026-11-17T10:00:05.100000000",
            "2026-11-17T10:00:05.100000000",
        ),
        (
            "2026-11-17T10:00:05.123456000",
            "2026-11-17T10:00:05.123456000",
            "2026-11-17T10:00:05.123456000",
        ),
        (
            "2026-11-17T10:00:05.123456789",
            "2026-11-17T10:00:05.123456789",
            "2026-11-17T10:00:05.123456000",
        ),
        (
            "2026-11-17T23:59:59.999999999",
            "2026-11-17T23:59:59.999999999",
            "2026-11-17T23:59:59.999999000",
        ),
        (
            "2024-02-29T23:59:59.999000000",
            "2024-02-29T23:59:59.999000000",
            "2024-02-29T23:59:59.999000000",
        ),
        (
            "2000-01-01T00:00:00.000000000",
            "2000-01-01T00:00:00.000000000",
            "2000-01-01T00:00:00.000000000",
        ),
    ];

    let numbered = (1..).zip(rows);
    for (position, (written, on_sqlite, on_postgres)) in numbered {
        let wall = engine::civil(written);
        let row = ScratchRow {
            at: engine::instant(written),
            day: engine::date_of(wall),
            time: engine::time_of(wall),
            wall,
        };
        let read = through_a_scratch_table(&db, position, &row).await;

        let expected = at_engine_precision(&db, on_sqlite, on_postgres);
        let (date, time) = expected.split_once('T').unwrap();
        assert_eq!(
            read.shown(),
            [
                expected.clone(),
                date.to_owned(),
                time.to_owned(),
                expected.clone()
            ],
            "{written}"
        );
    }
}
