// ABOUTME: Pins the JSON documents the repositories really write into columns, dates included
// ABOUTME: Recorded from the date engine in use on 2026-11-17; a SQLite-only suite, it reads raw column text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stored-document goldens for dates (carnet#1620, gate 5).
//!
//! A JSON document in a column outlives the code that wrote it. A row written
//! last year is read into a model and, on the next write, serialized again.
//! If the date engine writes an instant in another shape, every such row
//! changes bytes the first time it is touched: content hashes move and a
//! comparison between an old row and a new one stops meaning anything.
//!
//! The fixtures here are taken from columns, not from models:
//!
//! - `sleep_sessions.stages_json`, written by `upsert_sleep_session` from the
//!   session's stages alone. It is a list of structs, so its bytes are fixed
//!   and compared whole;
//! - `cached_activities.data_json`, written by `upsert_activities`, which
//!   serializes the activity through a JSON value and removes the samples.
//!   The order of its keys follows how the JSON library was built (a feature
//!   another crate of the build may switch on), so the document is compared
//!   field by field: every key, and every value byte for byte.
//!
//! Each is then read back through the repository and written again, and the
//! column must hold the same bytes.
//!
//! This suite is `SQLite` by construction: it asserts raw column text.
//!
//! Everything that names the date engine lives in [`engine`].

use pierre_core::models::{
    ActivityBuilder, DataSource, DeviceType, SportType, StoredSleepSession, StoredSleepStage,
    StoredSleepStageType, TenantId, User,
};
use pierre_database::backends::factory::Database;
use pierre_test_support::db::create_sqlite_test_db;
use serde_json::{json, Value};
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

/// Everything in this file that names the date engine.
mod engine {
    use chrono::{DateTime, Duration, NaiveDateTime, Utc};

    pub(super) type Instant = DateTime<Utc>;

    /// A UTC instant from `2026-11-17T10:00:05.123456789`, any fraction or none.
    pub(super) fn instant(text: &str) -> Instant {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
            .unwrap()
            .and_utc()
    }

    /// The instant plus a whole number of days, negative to go back.
    pub(super) fn plus_days(value: Instant, days: i64) -> Instant {
        value + Duration::days(days)
    }
}

use engine::instant;

fn pool(db: &Database) -> &Pool<Sqlite> {
    db.sqlite_pool()
        .expect("create_sqlite_test_db opens SQLite")
}

async fn fresh_user(db: &Database) -> Uuid {
    let user = User::new(
        format!("date-column-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Date Column".to_owned()),
    );
    db.repositories().users.create(&user).await.unwrap()
}

fn stage(kind: StoredSleepStageType, start: &str, end: &str, seconds: u32) -> StoredSleepStage {
    StoredSleepStage {
        stage_type: kind,
        start: instant(start),
        end: instant(end),
        duration_seconds: seconds,
    }
}

/// The raw date columns and stages document of a user's one sleep row.
async fn sleep_row(db: &Database, user_id: Uuid, tenant_id: &TenantId) -> [String; 4] {
    let row = sqlx::query(
        "SELECT start_time, end_time, synced_at, stages_json
         FROM sleep_sessions WHERE user_id = $1 AND tenant_id = $2",
    )
    .bind(user_id.to_string())
    .bind(tenant_id.to_string())
    .fetch_one(pool(db))
    .await
    .unwrap();
    [row.get(0), row.get(1), row.get(2), row.get(3)]
}

#[tokio::test]
async fn the_sleep_stages_document_is_stored_as_these_bytes_and_saved_back_unchanged() {
    let db = create_sqlite_test_db().await.unwrap();
    let user_id = fresh_user(&db).await;
    let tenant_id = TenantId::generate();
    let data_source_id = db
        .repositories()
        .data_sources
        .upsert_data_source(
            &tenant_id,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: "garmin".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap();

    let night = StoredSleepSession {
        id: "sleep-date-golden".to_owned(),
        user_id: user_id.to_string(),
        data_source_id,
        is_nap: false,
        start_datetime: instant("2026-11-16T22:41:07.100"),
        end_datetime: instant("2026-11-17T06:02:30"),
        total_sleep_seconds: Some(24_120),
        deep_sleep_seconds: None,
        light_sleep_seconds: None,
        rem_sleep_seconds: None,
        awake_seconds: Some(2_340),
        sleep_efficiency: Some(91.5),
        avg_heart_rate: None,
        min_heart_rate: None,
        avg_hrv: None,
        sleep_score: Some(82),
        avg_spo2: None,
        stages: vec![
            stage(
                StoredSleepStageType::Light,
                "2026-11-16T22:41:07.100",
                "2026-11-16T23:19:07.120",
                2_280,
            ),
            stage(
                StoredSleepStageType::Deep,
                "2026-11-16T23:19:07.123400",
                "2026-11-17T00:23:07",
                3_840,
            ),
            stage(
                StoredSleepStageType::Rem,
                "2026-11-17T00:23:07.123456789",
                "2026-11-17T01:14:07.000001",
                3_060,
            ),
        ],
        source_name: "garmin".to_owned(),
    };
    let synced_at = instant("2026-11-17T06:30:00.250");
    db.repositories()
        .sleep
        .upsert_sleep_session(&tenant_id, &night, synced_at)
        .await
        .unwrap();

    let stored = sleep_row(&db, user_id, &tenant_id).await;
    assert_eq!(
        stored,
        [
            "2026-11-16T22:41:07.100+00:00",
            "2026-11-17T06:02:30+00:00",
            "2026-11-17T06:30:00.250+00:00",
            r#"[{"stage_type":"Light","start":"2026-11-16T22:41:07.100Z","end":"2026-11-16T23:19:07.120Z","duration_seconds":2280},{"stage_type":"Deep","start":"2026-11-16T23:19:07.123400Z","end":"2026-11-17T00:23:07Z","duration_seconds":3840},{"stage_type":"Rem","start":"2026-11-17T00:23:07.123456789Z","end":"2026-11-17T01:14:07.000001Z","duration_seconds":3060}]"#
        ],
        "start, end and sync columns, then the stages document, as stored text"
    );

    // Read through the repository and written again at a later sync, the
    // stages hold the same bytes: they survive a load and a save. The sync
    // time moves, which shows the second write did rewrite the row.
    let read = db
        .repositories()
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant_id,
            engine::plus_days(night.start_datetime, -1),
            engine::plus_days(night.end_datetime, 1),
        )
        .await
        .unwrap();
    assert_eq!(read.len(), 1, "the night just written reads back");
    db.repositories()
        .sleep
        .upsert_sleep_session(&tenant_id, &read[0], engine::plus_days(synced_at, 1))
        .await
        .unwrap();
    let saved = sleep_row(&db, user_id, &tenant_id).await;
    assert_eq!(
        saved[2], "2026-11-18T06:30:00.250+00:00",
        "the second write moved the sync time"
    );
    assert_eq!(
        [&saved[0], &saved[1], &saved[3]],
        [&stored[0], &stored[1], &stored[3]],
        "start, end and stages hold the same bytes after a load and a save"
    );
}

/// The raw start column and document of a user's one cached activity.
async fn cached_row(db: &Database, user_id: Uuid, tenant_id: &TenantId) -> (String, Value) {
    let row = sqlx::query(
        "SELECT start_date, data_json FROM cached_activities WHERE user_id = $1 AND tenant_id = $2",
    )
    .bind(user_id.to_string())
    .bind(tenant_id.to_string())
    .fetch_one(pool(db))
    .await
    .unwrap();
    let document: String = row.get(1);
    (row.get(0), serde_json::from_str(&document).unwrap())
}

#[tokio::test]
async fn the_cached_activity_document_is_stored_with_these_fields_and_saved_back_unchanged() {
    let db = create_sqlite_test_db().await.unwrap();
    let user_id = fresh_user(&db).await;
    let tenant_id = TenantId::generate();
    let start = instant("2026-10-27T07:00:00.120");
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

    let (column, document) = cached_row(&db, user_id, &tenant_id).await;
    assert_eq!(
        column, "2026-10-27T07:00:00.120+00:00",
        "the start_date column text"
    );
    assert_eq!(
        document,
        json!({
            "id": "route-1",
            "name": "Lunch run",
            "sport_type": "run",
            "start_date": "2026-10-27T07:00:00.120Z",
            "duration_seconds": 1800,
            "distance_meters": 5000.0,
            "provider": "sciotte_strava",
        }),
        "every key of the stored document, the start with a padded fraction and a Z"
    );

    // Read through the repository and written again, both hold the same
    // text: the start is not re-shaped by a load and a save.
    let read = db
        .repositories()
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("strava"),
            engine::plus_days(start, -1),
            engine::plus_days(start, 1),
            10,
        )
        .await
        .unwrap();
    assert_eq!(read.len(), 1, "the activity just written reads back");
    let rewritten = db
        .repositories()
        .activity_cache
        .upsert_activities(user_id, &tenant_id, "strava", &read)
        .await
        .unwrap();
    assert_eq!(rewritten, 1, "the second write did rewrite the row");
    assert_eq!(
        cached_row(&db, user_id, &tenant_id).await,
        (column, document),
        "the column and document hold the same text after a load and a save"
    );
}
