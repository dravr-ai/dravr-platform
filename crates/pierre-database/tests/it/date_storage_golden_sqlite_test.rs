// ABOUTME: Pins the exact text SQLite stores for a date, and that it sorts in time order
// ABOUTME: Recorded from the date engine in use on 2026-11-17; a SQLite-only suite, it reads raw column text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Storage goldens for dates on `SQLite` (carnet#1620, gates 4 and 7). The
//! decode half, which stored shapes read back as an instant, is
//! `date_decode_golden_sqlite_test`; both share [`engine`] and the scratch
//! table defined here.
//!
//! `SQLite` has no timestamp type. A timestamp column holds whatever text the
//! driver writes, and queries compare that text: `WHERE created_at > ?`,
//! `ORDER BY start_date`. So the bytes are the contract, in two directions.
//!
//! **Writing.** A bound instant is stored as `2026-11-17T10:00:05.120+00:00`:
//! the fraction padded to 3, 6 or 9 digits, and `+00:00`, never `Z`. Rows
//! written in any other shape sort wrongly against the rows already there. The
//! ordering test below shows it: within one second, the `+` of an offset sorts
//! before the `.` of a fraction, and a `Z` would sort after it.
//!
//! **Reading** is the decode half's.
//!
//! One column is written in another shape on purpose: a deletion receipt's
//! `deleted_at` is a string the repository builds itself, with six fraction
//! digits and a `Z`, because it is part of a hash. Its own unit test pins it
//! (`a_receipt_timestamp_keeps_microseconds_in_utc`).
//!
//! This suite is `SQLite` by construction: it asserts raw column text and
//! opens `SQLite` itself, whatever `DATABASE_URL` names. The `PostgreSQL` lane
//! runs this crate's whole test binary, so the suite runs there too, still
//! against `SQLite`. The round trip that must hold on both engines is
//! `date_round_trip_golden_test`.
//!
//! Everything that names the date engine lives in [`engine`].

use pierre_core::models::{ActivityBuilder, SportType, TenantId, User, UserOAuthToken};
use pierre_database::backends::factory::Database;
use pierre_test_support::db::create_sqlite_test_db;
use secrecy::SecretString;
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

/// Everything in the two `SQLite` date suites that names the date engine.
pub(crate) mod engine {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

    pub(crate) type Instant = DateTime<Utc>;
    pub(crate) type CivilDate = NaiveDate;
    pub(crate) type CivilTime = NaiveTime;
    pub(crate) type CivilDateTime = NaiveDateTime;

    const STAMP: &str = "%Y-%m-%dT%H:%M:%S%.9f";

    /// A zoneless date-time from `2026-11-17T10:00:05.120000000`.
    pub(crate) fn civil(text: &str) -> CivilDateTime {
        NaiveDateTime::parse_from_str(text, STAMP).unwrap()
    }

    /// The UTC instant at that date and time.
    pub(crate) fn instant(text: &str) -> Instant {
        civil(text).and_utc()
    }

    /// The date part.
    pub(crate) fn date(text: &str) -> CivilDate {
        civil(text).date()
    }

    /// The time-of-day part.
    pub(crate) fn time(text: &str) -> CivilTime {
        civil(text).time()
    }

    /// A calendar date as `2026-11-17`.
    pub(crate) fn show_date(value: CivilDate) -> String {
        value.to_string()
    }

    /// The same shape back, so a verdict reads the way the input was written.
    pub(crate) fn show(value: Instant) -> String {
        value.format(STAMP).to_string()
    }

    /// The same shape for a zoneless date-time.
    pub(crate) fn show_civil(value: CivilDateTime) -> String {
        value.format(STAMP).to_string()
    }

    /// A time of day at nanosecond precision.
    pub(crate) fn show_time(value: CivilTime) -> String {
        value.format("%H:%M:%S%.9f").to_string()
    }

    /// The shape SQL defaults write: `2026-11-17 10:00:05`.
    pub(crate) fn sql_default_shape(value: Instant) -> String {
        value.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

/// The value table, as wall-clock text in UTC at nanosecond precision.
const SAMPLES: [&str; 11] = [
    "2026-11-17T10:00:05.000000000",
    "2026-11-17T10:00:05.100000000",
    "2026-11-17T10:00:05.120000000",
    "2026-11-17T10:00:05.123000000",
    "2026-11-17T10:00:05.123400000",
    "2026-11-17T10:00:05.123456000",
    "2026-11-17T10:00:05.123456789",
    "2026-11-17T10:00:05.000001000",
    "2026-11-17T23:59:59.999999999",
    "1969-07-20T20:17:40.250000000",
    "2024-02-29T23:59:59.999000000",
];

/// What each sample is stored as when bound as an instant.
const STORED: [&str; 11] = [
    "2026-11-17T10:00:05+00:00",
    "2026-11-17T10:00:05.100+00:00",
    "2026-11-17T10:00:05.120+00:00",
    "2026-11-17T10:00:05.123+00:00",
    "2026-11-17T10:00:05.123400+00:00",
    "2026-11-17T10:00:05.123456+00:00",
    "2026-11-17T10:00:05.123456789+00:00",
    "2026-11-17T10:00:05.000001+00:00",
    "2026-11-17T23:59:59.999999999+00:00",
    "1969-07-20T20:17:40.250+00:00",
    "2024-02-29T23:59:59.999+00:00",
];

pub(crate) fn pool(db: &Database) -> &Pool<Sqlite> {
    db.sqlite_pool()
        .expect("create_sqlite_test_db opens SQLite")
}

/// A scratch table with one column per declared type the schema uses for
/// dates: `TEXT` (408 columns) and `DATETIME` (71), whose affinity differs.
pub(crate) async fn scratch_table(db: &Database) {
    sqlx::query(
        "CREATE TABLE date_golden (
            position INTEGER PRIMARY KEY,
            as_text TEXT,
            as_datetime DATETIME
        )",
    )
    .execute(pool(db))
    .await
    .unwrap();
}

/// The raw text and storage class of both columns, in insertion order.
pub(crate) async fn raw_rows(db: &Database) -> Vec<[String; 4]> {
    sqlx::query(
        "SELECT CAST(as_text AS TEXT), typeof(as_text),
                CAST(as_datetime AS TEXT), typeof(as_datetime)
         FROM date_golden ORDER BY position",
    )
    .fetch_all(pool(db))
    .await
    .unwrap()
    .iter()
    .map(|row| {
        let column = |index: usize| {
            row.get::<Option<String>, _>(index)
                .unwrap_or_else(|| "NULL".to_owned())
        };
        [column(0), column(1), column(2), column(3)]
    })
    .collect()
}

#[tokio::test]
async fn a_bound_instant_is_stored_as_padded_text_ending_in_plus_zero() {
    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;

    for sample in SAMPLES {
        let value = engine::instant(sample);
        sqlx::query("INSERT INTO date_golden (as_text, as_datetime) VALUES ($1, $2)")
            .bind(value)
            .bind(value)
            .execute(pool(&db))
            .await
            .unwrap();
    }

    let wanted: Vec<[String; 4]> = STORED
        .iter()
        .map(|text| {
            [
                (*text).to_owned(),
                "text".to_owned(),
                (*text).to_owned(),
                "text".to_owned(),
            ]
        })
        .collect();
    assert_eq!(
        raw_rows(&db).await,
        wanted,
        "the text and storage class of each bound instant, in both columns"
    );
}

#[tokio::test]
async fn the_other_date_types_are_stored_as_these_texts() {
    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;
    let sample = "2026-11-17T10:00:05.120000000";
    let whole = "2024-02-29T23:59:59.000000000";

    let insert = "INSERT INTO date_golden (as_text, as_datetime) VALUES ($1, $2)";
    let pool = pool(&db);
    sqlx::query(insert)
        .bind(engine::date(sample))
        .bind(engine::date(whole))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(insert)
        .bind(engine::time(sample))
        .bind(engine::time(whole))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(insert)
        .bind(engine::civil(sample))
        .bind(engine::civil(whole))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(insert)
        .bind(None::<engine::Instant>)
        .bind(Some(engine::instant(sample)))
        .execute(pool)
        .await
        .unwrap();

    let text = |left: &str, right: &str| {
        [
            left.to_owned(),
            "text".to_owned(),
            right.to_owned(),
            "text".to_owned(),
        ]
    };
    assert_eq!(
        raw_rows(&db).await,
        vec![
            text("2026-11-17", "2024-02-29"),
            text("10:00:05.120", "23:59:59"),
            text("2026-11-17 10:00:05.120", "2024-02-29 23:59:59"),
            [
                "NULL".to_owned(),
                "null".to_owned(),
                "2026-11-17T10:00:05.120+00:00".to_owned(),
                "text".to_owned(),
            ],
        ],
        "a date, a time, a zoneless date-time and a NULL instant as stored text"
    );
}

/// The reason the stored shape matters: queries compare the text. Stored as
/// today, text order is time order. The same instants ending in `Z` are not.
#[tokio::test]
async fn stored_text_sorts_in_time_order() {
    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;

    // Out of order on purpose; positions are the time order.
    let by_time = [
        (1, "1969-07-20T20:17:40.250000000"),
        (2, "2024-02-29T23:59:59.999000000"),
        (3, "2026-11-17T10:00:05.000000000"),
        (4, "2026-11-17T10:00:05.000001000"),
        (5, "2026-11-17T10:00:05.100000000"),
        (6, "2026-11-17T10:00:05.120000000"),
        (7, "2026-11-17T10:00:05.123000000"),
        (8, "2026-11-17T10:00:05.123400000"),
        (9, "2026-11-17T10:00:05.123456000"),
        (10, "2026-11-17T10:00:05.123456789"),
        (11, "2026-11-17T10:00:06.000000000"),
        (12, "2026-11-17T23:59:59.999999999"),
    ];
    let shuffled = [by_time[6], by_time[2], by_time[11], by_time[0]]
        .into_iter()
        .chain(by_time);
    for (position, sample) in shuffled {
        sqlx::query(
            "INSERT OR IGNORE INTO date_golden (position, as_text, as_datetime)
             VALUES ($1, $2, $2)",
        )
        .bind(position)
        .bind(engine::instant(sample))
        .execute(pool(&db))
        .await
        .unwrap();
    }

    for by_column in [
        "SELECT position FROM date_golden ORDER BY as_text",
        "SELECT position FROM date_golden ORDER BY as_datetime",
    ] {
        let order: Vec<i64> = sqlx::query_scalar(by_column)
            .fetch_all(pool(&db))
            .await
            .unwrap();
        assert_eq!(order, (1..=12).collect::<Vec<i64>>(), "{by_column}");
    }

    let after: Vec<i64> =
        sqlx::query_scalar("SELECT position FROM date_golden WHERE as_text > $1 ORDER BY position")
            .bind(engine::instant("2026-11-17T10:00:05.120000000"))
            .fetch_all(pool(&db))
            .await
            .unwrap();
    assert_eq!(
        after,
        [7, 8, 9, 10, 11, 12],
        "WHERE as_text > a bound instant"
    );
}

async fn fresh_user(
    db: &Database,
    created_at: engine::Instant,
    last_active: engine::Instant,
) -> Uuid {
    let mut user = User::new(
        format!("date-golden-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Date Golden".to_owned()),
    );
    user.created_at = created_at;
    user.last_active = last_active;
    user.approved_at = Some(created_at);
    db.repositories().users.create(&user).await.unwrap()
}

#[tokio::test]
async fn the_users_repository_stores_its_timestamps_in_that_shape() {
    let db = create_sqlite_test_db().await.unwrap();
    let user_id = fresh_user(
        &db,
        engine::instant("2026-11-17T10:00:05.120000000"),
        engine::instant("2026-11-17T23:59:59.000000000"),
    )
    .await;

    let row = sqlx::query(
        "SELECT created_at, typeof(created_at), last_active, typeof(last_active),
                approved_at, typeof(approved_at)
         FROM users WHERE id = $1",
    )
    .bind(user_id.to_string())
    .fetch_one(pool(&db))
    .await
    .unwrap();
    let stored: Vec<String> = (0..6).map(|index| row.get::<String, _>(index)).collect();

    assert_eq!(
        stored,
        [
            "2026-11-17T10:00:05.120+00:00",
            "text",
            "2026-11-17T23:59:59+00:00",
            "text",
            "2026-11-17T10:00:05.120+00:00",
            "text",
        ],
        "created_at, last_active and approved_at text with their storage class"
    );
}

#[tokio::test]
async fn the_oauth_token_repository_stores_its_timestamps_in_that_shape() {
    let db = create_sqlite_test_db().await.unwrap();
    let created_at = engine::instant("2026-11-17T10:00:05.100000000");
    let user_id = fresh_user(&db, created_at, created_at).await;
    let tenant_id = TenantId::generate();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: "strava".to_owned(),
        access_token: SecretString::from("access-date-golden".to_owned()),
        refresh_token: None,
        token_type: "Bearer".to_owned(),
        expires_at: Some(engine::instant("2026-11-17T16:00:05.123400000")),
        scope: None,
        provider_user_id: None,
        oauth_app_client_id: None,
        created_at,
        updated_at: engine::instant("2026-11-18T00:00:00.000000000"),
    };
    db.repositories()
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();

    let row = sqlx::query(
        "SELECT expires_at, typeof(expires_at), created_at, updated_at
         FROM user_oauth_tokens WHERE id = $1 AND tenant_id = $2",
    )
    .bind(&token.id)
    .bind(&token.tenant_id)
    .fetch_one(pool(&db))
    .await
    .unwrap();
    let stored: Vec<String> = (0..4).map(|index| row.get::<String, _>(index)).collect();

    assert_eq!(
        stored,
        [
            "2026-11-17T16:00:05.123400+00:00",
            "text",
            "2026-11-17T10:00:05.100+00:00",
            "2026-11-18T00:00:00+00:00",
        ],
        "expires_at with its storage class, then created_at and updated_at text"
    );
}

/// The activity cache stores the start twice: as a column the reads sort and
/// filter on, and inside the JSON document. The two are different shapes.
#[tokio::test]
async fn the_activity_cache_stores_the_start_as_a_column_and_inside_its_document() {
    let db = create_sqlite_test_db().await.unwrap();
    let start = engine::instant("2026-10-27T07:00:00.120000000");
    let user_id = fresh_user(&db, start, start).await;
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
        .upsert_activities(user_id, &tenant_id, "strava", &[activity])
        .await
        .unwrap();

    let row = sqlx::query(
        "SELECT start_date, typeof(start_date), data_json
         FROM cached_activities WHERE user_id = $1 AND tenant_id = $2",
    )
    .bind(user_id.to_string())
    .bind(tenant_id.to_string())
    .fetch_one(pool(&db))
    .await
    .unwrap();

    assert_eq!(
        row.get::<String, _>(0),
        "2026-10-27T07:00:00.120+00:00",
        "the start_date column text"
    );
    assert_eq!(
        row.get::<String, _>(1),
        "text",
        "the start_date column storage class"
    );
    // The field alone, byte for byte. The whole document is pinned in
    // `date_stored_column_golden_sqlite_test`.
    let document: serde_json::Value = serde_json::from_str(&row.get::<String, _>(2)).unwrap();
    assert_eq!(
        document["start_date"],
        serde_json::Value::String("2026-10-27T07:00:00.120Z".to_owned()),
        "the stored document writes the start with a padded fraction and a Z"
    );
}
