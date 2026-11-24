// ABOUTME: Pins which date shapes stored in SQLite read back as an instant, a date, a date-time or a time of day
// ABOUTME: Recorded from the date engine in use on 2026-11-17; a SQLite-only suite, it reads raw column text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Decode goldens for dates on `SQLite` (carnet#1620, gates 4 and 7): the
//! reading half of `date_storage_golden_sqlite_test`, whose [`engine`],
//! scratch table and pool it shares.
//!
//! Rows exist in several shapes, because columns are also filled by SQL
//! defaults (`datetime('now')`, `CURRENT_TIMESTAMP`) that write
//! `2026-11-17 10:00:05` with no offset. Every shape that decodes today must
//! keep decoding, to the same instant.
//!
//! The decode tables answer the one inbound input list shared with
//! `pierre-core` (`date_inbound_inputs.rs`), then the driver's other shapes:
//! its minute-only fallbacks, text into a zoneless date-time, text into a
//! time of day, and second 60.

use pierre_database::backends::factory::Database;
use pierre_test_support::db::create_sqlite_test_db;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::date_storage_golden_sqlite_test::{engine, pool, raw_rows, scratch_table};
use inputs::INBOUND_INPUTS;

/// The one inbound input list, shared with `pierre-core`'s JSON and
/// strict-parser goldens. Included by path: one crate's tests cannot import
/// another's.
#[path = "../../../pierre-core/tests/it/date_inbound_inputs.rs"]
mod inputs;

const TEN: &str = "2026-11-17T10:00:05.000000000";
const REJECTED: &str = "rejected";

/// A value as it sits in a column: text, a number, or nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stored {
    Text(&'static str),
    Integer(i64),
    Real(f64),
    Null,
}

/// Stores `value` in both columns and hands back the one row holding it.
async fn stored_row(db: &Database, value: Stored) -> SqliteRow {
    sqlx::query("DELETE FROM date_golden")
        .execute(pool(db))
        .await
        .unwrap();
    let insert = sqlx::query("INSERT INTO date_golden (as_text, as_datetime) VALUES ($1, $1)");
    let insert = match value {
        Stored::Text(text) => insert.bind(text),
        Stored::Integer(number) => insert.bind(number),
        Stored::Real(number) => insert.bind(number),
        Stored::Null => insert.bind(None::<String>),
    };
    insert.execute(pool(db)).await.unwrap();

    sqlx::query("SELECT as_text, as_datetime FROM date_golden")
        .fetch_one(pool(db))
        .await
        .unwrap()
}

/// Stores `value` in both columns and decodes each as an instant: the verdict
/// for the `TEXT` column, then for the `DATETIME` one.
async fn decode_verdicts(db: &Database, value: Stored) -> [String; 2] {
    let row = stored_row(db, value).await;
    let verdict = |index: usize| {
        row.try_get::<engine::Instant, _>(index)
            .map_or_else(|_| REJECTED.to_owned(), engine::show)
    };
    [verdict(0), verdict(1)]
}

/// Every stored shape and what it reads back as. Both columns are listed even
/// where they agree, because a `DATETIME` column turns numeric-looking text
/// into a number before the driver sees it.
#[tokio::test]
async fn each_stored_shape_decodes_to_the_recorded_instant_or_is_refused() {
    use Stored::{Integer, Null, Real, Text};

    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;

    // stored value, verdict from the TEXT column, verdict from the DATETIME column
    let rows = [
        // What we write.
        (Text("2026-11-17T10:00:05+00:00"), TEN, TEN),
        (
            Text("2026-11-17T10:00:05.120+00:00"),
            "2026-11-17T10:00:05.120000000",
            "2026-11-17T10:00:05.120000000",
        ),
        (
            Text("2026-11-17T10:00:05.123456789+00:00"),
            "2026-11-17T10:00:05.123456789",
            "2026-11-17T10:00:05.123456789",
        ),
        // Other RFC 3339 forms.
        (Text("2026-11-17T10:00:05Z"), TEN, TEN),
        (
            Text("2026-11-17T10:00:05.1Z"),
            "2026-11-17T10:00:05.100000000",
            "2026-11-17T10:00:05.100000000",
        ),
        (Text("2026-11-17T05:00:05-05:00"), TEN, TEN),
        (Text("2026-11-17 10:00:05+00:00"), TEN, TEN),
        (Text("2026-11-17 10:00:05Z"), TEN, TEN),
        (Text("2026-11-17T10:00:05+0000"), TEN, TEN),
        (Text("2026-11-17 10:00:05 UTC"), REJECTED, REJECTED),
        // What SQL defaults write: no offset, read as UTC.
        (Text("2026-11-17 10:00:05"), TEN, TEN),
        (
            Text("2026-11-17 10:00:05.123"),
            "2026-11-17T10:00:05.123000000",
            "2026-11-17T10:00:05.123000000",
        ),
        (Text("2026-11-17T10:00:05"), TEN, TEN),
        (
            Text("2026-11-17T10:00:05.123"),
            "2026-11-17T10:00:05.123000000",
            "2026-11-17T10:00:05.123000000",
        ),
        (
            Text("2026-11-17 10:00"),
            "2026-11-17T10:00:00.000000000",
            "2026-11-17T10:00:00.000000000",
        ),
        (
            Text("2026-11-17T10:00"),
            "2026-11-17T10:00:00.000000000",
            "2026-11-17T10:00:00.000000000",
        ),
        (Text("2026-11-17"), REJECTED, REJECTED),
        (Text("10:00:05"), REJECTED, REJECTED),
        // Numbers. A TEXT column turns them into text, which is refused; a
        // DATETIME column keeps them, and reads an integer as epoch seconds
        // and a real as a Julian day.
        (Integer(1_794_909_605), REJECTED, TEN),
        (Integer(0), REJECTED, "1970-01-01T00:00:00.000000000"),
        (Integer(-1), REJECTED, "1969-12-31T23:59:59.000000000"),
        (Real(1_794_909_605.5), REJECTED, REJECTED),
        (Real(2_461_362.5), REJECTED, "2026-11-18T00:00:00.000000000"),
        (Text("1794909605"), REJECTED, TEN),
        // Nothing usable.
        (Text(""), REJECTED, REJECTED),
        (Text("yesterday"), REJECTED, REJECTED),
        (Null, REJECTED, REJECTED),
    ];

    let mut actual = Vec::new();
    for (value, _, _) in rows {
        let [text, datetime] = decode_verdicts(&db, value).await;
        actual.push((value, text, datetime));
    }
    let wanted: Vec<(Stored, String, String)> = rows
        .iter()
        .map(|(value, text, datetime)| (*value, (*text).to_owned(), (*datetime).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "stored shape, then its TEXT and DATETIME column verdicts as an instant"
    );
}

/// An empty column reads as "no instant" through an optional field.
#[tokio::test]
async fn a_null_column_decodes_as_no_instant() {
    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;
    sqlx::query(
        "INSERT INTO date_golden (as_text, as_datetime) VALUES (NULL, '2026-11-17 10:00:05')",
    )
    .execute(pool(&db))
    .await
    .unwrap();

    let row = sqlx::query("SELECT as_text, as_datetime FROM date_golden")
        .fetch_one(pool(&db))
        .await
        .unwrap();
    let read = |index: usize| {
        row.get::<Option<engine::Instant>, _>(index)
            .map(engine::show)
    };
    assert_eq!(read(0), None, "a NULL TEXT column reads as no instant");
    assert_eq!(
        read(1),
        Some(TEN.to_owned()),
        "the filled DATETIME column beside it still reads its instant"
    );
}

/// The two SQL defaults the migrations use, as the engine writes them now.
#[tokio::test]
async fn what_the_sql_defaults_write_decodes_as_utc() {
    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;
    sqlx::query(
        "INSERT INTO date_golden (as_text, as_datetime)
         VALUES (datetime('now'), CURRENT_TIMESTAMP)",
    )
    .execute(pool(&db))
    .await
    .unwrap();

    let row = sqlx::query(
        "SELECT as_text, as_datetime, CAST(as_text AS TEXT), CAST(as_datetime AS TEXT),
                typeof(as_text), typeof(as_datetime)
         FROM date_golden",
    )
    .fetch_one(pool(&db))
    .await
    .unwrap();

    for (decoded, raw, class) in [(0, 2, 4), (1, 3, 5)] {
        let raw_text: String = row.get(raw);
        let value: engine::Instant = row.get(decoded);
        assert_eq!(
            row.get::<String, _>(class),
            "text",
            "a SQL default is stored with the text storage class"
        );
        assert_eq!(raw_text.len(), "2026-11-17 10:00:05".len(), "{raw_text}");
        assert_eq!(
            engine::sql_default_shape(value),
            raw_text,
            "the offset-less text is read as UTC, digit for digit"
        );
    }
}

/// A calendar date column reads back from the text we store.
#[tokio::test]
async fn a_stored_calendar_date_decodes() {
    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;
    sqlx::query(
        "INSERT INTO date_golden (as_text, as_datetime) VALUES ('2024-02-29', '2024-02-29')",
    )
    .execute(pool(&db))
    .await
    .unwrap();

    let row = sqlx::query("SELECT as_text, as_datetime FROM date_golden")
        .fetch_one(pool(&db))
        .await
        .unwrap();
    assert_eq!(
        engine::show_date(row.get::<engine::CivilDate, _>(0)),
        "2024-02-29",
        "a calendar date from the TEXT column"
    );
    assert_eq!(
        engine::show_date(row.get::<engine::CivilDate, _>(1)),
        "2024-02-29",
        "a calendar date from the DATETIME column"
    );
    assert!(
        row.try_get::<engine::CivilTime, _>(0).is_err(),
        "a date is not a time of day"
    );
}

/// Walks `rows` through a decode and compares both columns' verdicts at once.
async fn assert_decodes<const N: usize>(
    db: &Database,
    decode: fn(&SqliteRow, usize) -> String,
    rows: [(Stored, &str, &str); N],
) {
    let mut actual = Vec::new();
    for (value, _, _) in rows {
        let row = stored_row(db, value).await;
        actual.push((value, decode(&row, 0), decode(&row, 1)));
    }
    let wanted: Vec<(Stored, String, String)> = rows
        .iter()
        .map(|(value, text, datetime)| (*value, (*text).to_owned(), (*datetime).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "stored value, then its TEXT and DATETIME column verdicts"
    );
}

fn as_instant(row: &SqliteRow, index: usize) -> String {
    row.try_get::<engine::Instant, _>(index)
        .map_or_else(|_| REJECTED.to_owned(), engine::show)
}

fn as_civil(row: &SqliteRow, index: usize) -> String {
    row.try_get::<engine::CivilDateTime, _>(index)
        .map_or_else(|_| REJECTED.to_owned(), engine::show_civil)
}

fn as_time(row: &SqliteRow, index: usize) -> String {
    row.try_get::<engine::CivilTime, _>(index)
        .map_or_else(|_| REJECTED.to_owned(), engine::show_time)
}

/// The third path of the inbound gate: the one input list the JSON and
/// strict-parser goldens answer in `pierre-core`, stored as text and decoded
/// as an instant. Each row: the input, the verdict from the `TEXT` column,
/// the verdict from the `DATETIME` column.
#[tokio::test]
async fn the_shared_inbound_inputs_decode_from_stored_text() {
    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;

    let expected: [(&str, &str, &str); 54] = [
        (
            "2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T10:00:05+00:00",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T05:00:05-05:00",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T15:30:05+05:30",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T10:00:05-00:00",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T10:00:05+0000",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T05:00:05-0500",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        ("2026-11-17T10:00:05+00", "rejected", "rejected"),
        (
            "2026-11-17T10:00:05 +00:00",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17t10:00:05z",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T10:00:05z",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17t10:00:05Z",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17 10:00:05Z",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17 10:00:05+00:00",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17  10:00:05Z",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        ("2026-11-17_10:00:05Z", "rejected", "rejected"),
        ("2026-11-17 10:00:05 UTC", "rejected", "rejected"),
        ("2026-11-17T10:00:05 UTC", "rejected", "rejected"),
        ("2026-11-17T10:00:05UTC", "rejected", "rejected"),
        ("2026-11-17T10:00:05 utc", "rejected", "rejected"),
        ("2026-11-17T10:00:05 GMT", "rejected", "rejected"),
        ("2026-11-17T05:00:05 EST", "rejected", "rejected"),
        (
            "2026-1-7T9:0:5Z",
            "2026-01-07T09:00:05.000000000",
            "2026-01-07T09:00:05.000000000",
        ),
        (
            "2026-11-17T9:00:05Z",
            "2026-11-17T09:00:05.000000000",
            "2026-11-17T09:00:05.000000000",
        ),
        (
            "2026-11-17T10:00:05",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17 10:00:05",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        (
            "2026-11-17T10:00Z",
            "2026-11-17T10:00:00.000000000",
            "2026-11-17T10:00:00.000000000",
        ),
        ("2026-11-17", "rejected", "rejected"),
        ("2026-11-17Z", "rejected", "rejected"),
        ("", "rejected", "rejected"),
        (
            "2026-11-17T10:00:05.1Z",
            "2026-11-17T10:00:05.100000000",
            "2026-11-17T10:00:05.100000000",
        ),
        (
            "2026-11-17T10:00:05.12Z",
            "2026-11-17T10:00:05.120000000",
            "2026-11-17T10:00:05.120000000",
        ),
        (
            "2026-11-17T10:00:05.123Z",
            "2026-11-17T10:00:05.123000000",
            "2026-11-17T10:00:05.123000000",
        ),
        (
            "2026-11-17T10:00:05.1234Z",
            "2026-11-17T10:00:05.123400000",
            "2026-11-17T10:00:05.123400000",
        ),
        (
            "2026-11-17T10:00:05.12345Z",
            "2026-11-17T10:00:05.123450000",
            "2026-11-17T10:00:05.123450000",
        ),
        (
            "2026-11-17T10:00:05.123456Z",
            "2026-11-17T10:00:05.123456000",
            "2026-11-17T10:00:05.123456000",
        ),
        (
            "2026-11-17T10:00:05.1234567Z",
            "2026-11-17T10:00:05.123456700",
            "2026-11-17T10:00:05.123456700",
        ),
        (
            "2026-11-17T10:00:05.12345678Z",
            "2026-11-17T10:00:05.123456780",
            "2026-11-17T10:00:05.123456780",
        ),
        (
            "2026-11-17T10:00:05.123456789Z",
            "2026-11-17T10:00:05.123456789",
            "2026-11-17T10:00:05.123456789",
        ),
        (
            "2026-11-17T10:00:05.1234567891Z",
            "2026-11-17T10:00:05.123456789",
            "2026-11-17T10:00:05.123456789",
        ),
        (
            "2026-11-17T10:00:05.100+00:00",
            "2026-11-17T10:00:05.100000000",
            "2026-11-17T10:00:05.100000000",
        ),
        ("2026-11-17T10:00:05,123Z", "rejected", "rejected"),
        ("2026-11-17T10:00:05.Z", "rejected", "rejected"),
        (
            "2016-12-31T23:59:60Z",
            "2016-12-31T23:59:59.000000000",
            "2016-12-31T23:59:59.000000000",
        ),
        (
            "2026-11-17T10:00:60Z",
            "2026-11-17T10:00:59.000000000",
            "2026-11-17T10:00:59.000000000",
        ),
        ("2026-11-17T24:00:00Z", "rejected", "rejected"),
        ("2026-02-30T10:00:05Z", "rejected", "rejected"),
        ("2025-02-29T10:00:05Z", "rejected", "rejected"),
        ("2026-13-01T10:00:05Z", "rejected", "rejected"),
        (
            " 2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05.000000000",
            "2026-11-17T10:00:05.000000000",
        ),
        ("2026-11-17T10:00:05Z ", "rejected", "rejected"),
        ("20261117T100005Z", "rejected", "rejected"),
        ("Tue, 17 Nov 2026 10:00:05 GMT", "rejected", "rejected"),
        ("1794909605", "rejected", "2026-11-17T10:00:05.000000000"),
    ];
    let named: Vec<&str> = expected.iter().map(|(input, ..)| *input).collect();
    assert_eq!(
        named, INBOUND_INPUTS,
        "the SQLite decode does not answer the shared inputs"
    );

    let mut actual = Vec::new();
    for (input, _, _) in expected {
        let [text, datetime] = decode_verdicts(&db, Stored::Text(input)).await;
        actual.push((input, text, datetime));
    }
    let wanted: Vec<(&str, String, String)> = expected
        .iter()
        .map(|(input, text, datetime)| (*input, (*text).to_owned(), (*datetime).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "shared input, then its TEXT and DATETIME column verdicts as an instant"
    );
}

/// The driver's fallback shapes with a minute and no seconds, followed by an
/// offset: four of its twelve, and their near misses.
#[tokio::test]
async fn the_minute_only_fallback_shapes_decode_with_their_offset() {
    use Stored::Text;

    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;
    assert_decodes(
        &db,
        as_instant,
        [
            (
                Text("2026-11-17 10:00Z"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (
                Text("2026-11-17 10:00+00:00"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (
                Text("2026-11-17 05:00-05:00"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (
                Text("2026-11-17T10:00Z"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (
                Text("2026-11-17T10:00+00:00"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (
                Text("2026-11-17T15:30+05:30"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (Text("2026-11-17 10:00z"), "rejected", "rejected"),
            (
                Text("2026-11-17 10:00+0000"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (
                Text("2026-11-17T10:00 +00:00"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
        ],
    )
    .await;
}

/// Text decoded as a zoneless date-time. Text that carries an offset is read
/// as its own wall clock, not converted to UTC: `05:00:05-05:00` reads as
/// five o'clock.
#[tokio::test]
async fn stored_text_decodes_as_a_zoneless_date_time() {
    use Stored::{Integer, Null, Real, Text};

    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;
    assert_decodes(
        &db,
        as_civil,
        [
            (
                Text("2026-11-17 10:00:05.120"),
                "2026-11-17T10:00:05.120000000",
                "2026-11-17T10:00:05.120000000",
            ),
            (
                Text("2026-11-17 10:00:05"),
                "2026-11-17T10:00:05.000000000",
                "2026-11-17T10:00:05.000000000",
            ),
            (
                Text("2026-11-17T10:00:05.120"),
                "2026-11-17T10:00:05.120000000",
                "2026-11-17T10:00:05.120000000",
            ),
            (
                Text("2026-11-17T10:00:05"),
                "2026-11-17T10:00:05.000000000",
                "2026-11-17T10:00:05.000000000",
            ),
            (
                Text("2026-11-17 10:00:05.123456789"),
                "2026-11-17T10:00:05.123456789",
                "2026-11-17T10:00:05.123456789",
            ),
            (
                Text("2026-11-17 10:00"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (
                Text("2026-11-17T10:00"),
                "2026-11-17T10:00:00.000000000",
                "2026-11-17T10:00:00.000000000",
            ),
            (
                Text("2026-11-17T10:00:05+00:00"),
                "2026-11-17T10:00:05.000000000",
                "2026-11-17T10:00:05.000000000",
            ),
            (
                Text("2026-11-17T10:00:05.120+00:00"),
                "2026-11-17T10:00:05.120000000",
                "2026-11-17T10:00:05.120000000",
            ),
            (
                Text("2026-11-17T10:00:05Z"),
                "2026-11-17T10:00:05.000000000",
                "2026-11-17T10:00:05.000000000",
            ),
            (
                Text("2026-11-17T05:00:05-05:00"),
                "2026-11-17T05:00:05.000000000",
                "2026-11-17T05:00:05.000000000",
            ),
            (
                Text("2026-11-17T15:30:05+05:30"),
                "2026-11-17T15:30:05.000000000",
                "2026-11-17T15:30:05.000000000",
            ),
            (
                Text("2026-11-17 10:00:05+00:00"),
                "2026-11-17T10:00:05.000000000",
                "2026-11-17T10:00:05.000000000",
            ),
            (Text("2026-11-17 10:00:05 UTC"), "rejected", "rejected"),
            (Text("2026-11-17"), "rejected", "rejected"),
            (Text("10:00:05"), "rejected", "rejected"),
            (
                Text("2026-11-17 10:00:60"),
                "2026-11-17T10:00:59.000000000",
                "2026-11-17T10:00:59.000000000",
            ),
            (Text("2026-11-17 24:00:00"), "rejected", "rejected"),
            (
                Integer(1_794_909_605),
                "rejected",
                "2026-11-17T10:00:05.000000000",
            ),
            (
                Real(2_461_362.5),
                "rejected",
                "2026-11-18T00:00:00.000000000",
            ),
            (
                Text("1794909605"),
                "rejected",
                "2026-11-17T10:00:05.000000000",
            ),
            (Text(""), "rejected", "rejected"),
            (Null, "rejected", "rejected"),
        ],
    )
    .await;
}

/// Text decoded as a time of day.
#[tokio::test]
async fn stored_text_decodes_as_a_time_of_day() {
    use Stored::{Integer, Null, Real, Text};

    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;
    assert_decodes(
        &db,
        as_time,
        [
            (
                Text("10:00:05.120"),
                "10:00:05.120000000",
                "10:00:05.120000000",
            ),
            (Text("10:00:05"), "10:00:05.000000000", "10:00:05.000000000"),
            (
                Text("23:59:59.999999999"),
                "23:59:59.999999999",
                "23:59:59.999999999",
            ),
            (Text("10:00"), "10:00:00.000000000", "10:00:00.000000000"),
            (
                Text("10:00:05.1"),
                "10:00:05.100000000",
                "10:00:05.100000000",
            ),
            (
                Text("10:00:05.1234567891"),
                "10:00:05.123456789",
                "10:00:05.123456789",
            ),
            (
                Text("10:00:05Z"),
                "10:00:05.000000000",
                "10:00:05.000000000",
            ),
            (
                Text("10:00:05+00:00"),
                "10:00:05.000000000",
                "10:00:05.000000000",
            ),
            (Text("T10:00:05"), "rejected", "rejected"),
            (Text("2026-11-17 10:00:05"), "rejected", "rejected"),
            (Text("2026-11-17T10:00:05+00:00"), "rejected", "rejected"),
            (Text("9:0:5"), "09:00:05.000000000", "09:00:05.000000000"),
            (Text("10:00:60"), "10:00:59.000000000", "10:00:59.000000000"),
            (Text("24:00:00"), "rejected", "rejected"),
            (Text("10:00 AM"), "rejected", "rejected"),
            (
                Text(" 10:00:05"),
                "10:00:05.000000000",
                "10:00:05.000000000",
            ),
            (Text("10:00:05 "), "rejected", "rejected"),
            (Integer(36_005), "rejected", "rejected"),
            (Real(0.5), "rejected", "rejected"),
            (Text("36005"), "rejected", "rejected"),
            (Text(""), "rejected", "rejected"),
            (Null, "rejected", "rejected"),
        ],
    )
    .await;
}

/// Second 60 through the database: second 60 is read as second 59 of its
/// minute (carnet#1620, accepted 2026-11-18), so it is stored as `:59` text,
/// reads back as second 59, and sorts with it before the next minute.
#[tokio::test]
async fn a_leap_second_is_stored_as_second_sixty_and_reads_back() {
    let db = create_sqlite_test_db().await.unwrap();
    scratch_table(&db).await;
    let insert = "INSERT INTO date_golden (as_text, as_datetime) VALUES ($1, $1)";
    for sample in [
        "2016-12-31T23:59:59.500000000",
        "2016-12-31T23:59:60.000000000",
        "2016-12-31T23:59:60.500000000",
        "2017-01-01T00:00:00.000000000",
    ] {
        sqlx::query(insert)
            .bind(engine::instant(sample))
            .execute(pool(&db))
            .await
            .unwrap();
    }

    let stored: Vec<String> = raw_rows(&db)
        .await
        .into_iter()
        .map(|[text, ..]| text)
        .collect();
    assert_eq!(
        stored,
        [
            "2016-12-31T23:59:59.500+00:00",
            "2016-12-31T23:59:59+00:00",
            "2016-12-31T23:59:59.500+00:00",
            "2017-01-01T00:00:00+00:00"
        ],
        "the text an instant writes, second 60 read as 59"
    );

    let rows = sqlx::query("SELECT as_text FROM date_golden ORDER BY as_text")
        .fetch_all(pool(&db))
        .await
        .unwrap();
    let read: Vec<String> = rows.iter().map(|row| as_instant(row, 0)).collect();
    assert_eq!(
        read,
        [
            "2016-12-31T23:59:59.000000000",
            "2016-12-31T23:59:59.500000000",
            "2016-12-31T23:59:59.500000000",
            "2017-01-01T00:00:00.000000000"
        ],
        "second 60 reads back as second 59 and sorts before the next minute"
    );
}
