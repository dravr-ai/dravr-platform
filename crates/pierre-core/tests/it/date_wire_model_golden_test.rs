// ABOUTME: Pins that the wire models carrying dates read their JSON and write it back to the same bytes
// ABOUTME: Recorded from the date engine in use on 2026-11-17; the stored columns are pinned in pierre-database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Wire-model round trips for dates (carnet#1620, gate 5, the model half).
//!
//! These four models are what providers are mapped into and what tools hand
//! back. A sleep session, a recovery reading and a health reading are never
//! written to a column as they stand: the database keeps its own `Stored*`
//! rows and serializes only parts of them. The activity is a type of the
//! `dravr-cageux` satellite, and the activity cache rewrites the document
//! before storing it. So this file does not pin stored bytes. It pins that a
//! model read from JSON and written again gives the same text, which is what
//! a tool result, a cache entry in memory and a provider fixture rely on.
//!
//! The documents that really sit in columns are pinned where they are
//! written, in `pierre-database` (`date_stored_column_golden_sqlite_test`): the sleep
//! stages document and the cached activity as stored.
//!
//! Each document below is the text the serializer produces today, with
//! fractions that end in zeros because those are the ones a trimming writer
//! changes.
//!
//! No date type is named here: the documents are text in and text out.

use pierre_core::models::{Activity, HealthMetrics, RecoveryMetrics, SleepSession};
use serde::de::DeserializeOwned;
use serde::Serialize;

const ACTIVITY: &str = concat!(
    r#"{"id":"route-1","name":"Lunch run","sport_type":"run","#,
    r#""start_date":"2026-10-27T07:00:00.120Z","duration_seconds":1800,"#,
    r#""distance_meters":5000.0,"provider":"sciotte_strava"}"#,
);

const SLEEP_SESSION: &str = concat!(
    r#"{"id":"sleep-1","start_time":"2026-11-16T22:41:07.100Z","#,
    r#""end_time":"2026-11-17T06:02:30Z","time_in_bed":441,"total_sleep_time":402,"#,
    r#""sleep_efficiency":91.5,"sleep_score":82.0,"stages":["#,
    r#"{"stage_type":"light","start_time":"2026-11-16T22:41:07.100Z","duration_minutes":38},"#,
    r#"{"stage_type":"deep","start_time":"2026-11-16T23:19:07.123400Z","duration_minutes":64},"#,
    r#"{"stage_type":"rem","start_time":"2026-11-17T00:23:07.123456789Z","duration_minutes":51}"#,
    r#"],"hrv_during_sleep":58.5,"provider":"garmin"}"#,
);

const RECOVERY_METRICS: &str = concat!(
    r#"{"date":"2024-02-29T23:59:59.999Z","recovery_score":71.5,"readiness_score":80.0,"#,
    r#""resting_heart_rate":47,"provider":"whoop"}"#,
);

const HEALTH_METRICS: &str = concat!(
    r#"{"date":"1969-07-20T20:17:40.250Z","weight":72.5,"blood_pressure":[118,76],"#,
    r#""provider":"withings"}"#,
);

/// Reads a document into its model and writes the model back out.
fn load_and_save<T: DeserializeOwned + Serialize>(document: &str) -> String {
    let model: T = serde_json::from_str(document).unwrap();
    serde_json::to_string(&model).unwrap()
}

#[test]
fn an_activity_wire_model_round_trips_to_the_same_bytes() {
    assert_eq!(
        load_and_save::<Activity>(ACTIVITY),
        ACTIVITY,
        "the activity document after a read and a write"
    );
}

#[test]
fn a_sleep_session_wire_model_round_trips_to_the_same_bytes() {
    assert_eq!(
        load_and_save::<SleepSession>(SLEEP_SESSION),
        SLEEP_SESSION,
        "the sleep session document after a read and a write"
    );
}

#[test]
fn a_recovery_reading_wire_model_round_trips_to_the_same_bytes() {
    assert_eq!(
        load_and_save::<RecoveryMetrics>(RECOVERY_METRICS),
        RECOVERY_METRICS,
        "the recovery document after a read and a write"
    );
}

#[test]
fn a_health_reading_wire_model_round_trips_to_the_same_bytes() {
    assert_eq!(
        load_and_save::<HealthMetrics>(HEALTH_METRICS),
        HEALTH_METRICS,
        "the health document after a read and a write"
    );
}

/// A document whose dates were written by something else — an offset instead
/// of `Z`, a short fraction, another zone — is saved in the one shape we write.
#[test]
fn a_document_in_another_shape_is_saved_in_ours() {
    let rows = [
        ("2026-10-27T07:00:00.120+00:00", "2026-10-27T07:00:00.120Z"),
        ("2026-10-27T07:00:00.12Z", "2026-10-27T07:00:00.120Z"),
        ("2026-10-27T07:00:00.1Z", "2026-10-27T07:00:00.100Z"),
        ("2026-10-27T07:00:00.000Z", "2026-10-27T07:00:00Z"),
        ("2026-10-27T07:00:00.1200Z", "2026-10-27T07:00:00.120Z"),
        ("2026-10-27T07:00:00.12345Z", "2026-10-27T07:00:00.123450Z"),
        ("2026-10-27T03:00:00.120-04:00", "2026-10-27T07:00:00.120Z"),
        ("2026-10-27 07:00:00.120 UTC", "2026-10-27T07:00:00.120Z"),
    ];
    for (written, saved) in rows {
        let document = ACTIVITY.replace("2026-10-27T07:00:00.120Z", written);
        assert_eq!(
            load_and_save::<Activity>(&document),
            ACTIVITY.replace("2026-10-27T07:00:00.120Z", saved),
            "{written}"
        );
    }
}

/// A document with no usable date is refused, not read as some default day.
#[test]
fn a_document_without_a_usable_date_is_refused() {
    for broken in ["2026-10-27T07:00:00.120", "2026-10-27", "1793084400", ""] {
        let document = ACTIVITY.replace("2026-10-27T07:00:00.120Z", broken);
        assert!(
            serde_json::from_str::<Activity>(&document).is_err(),
            "{broken:?} was accepted as a start date"
        );
    }
    let missing = ACTIVITY.replace(r#""start_date":"2026-10-27T07:00:00.120Z","#, "");
    assert!(
        serde_json::from_str::<Activity>(&missing).is_err(),
        "an activity with no start date was accepted"
    );
}
