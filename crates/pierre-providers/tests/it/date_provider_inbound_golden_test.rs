// ABOUTME: Pins how provider payloads are read into dates, through the provider code that reads them
// ABOUTME: Recorded from the date engine in use on 2026-11-17: Intervals.icu, WHOOP and Terra
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Inbound goldens for provider dates (carnet#1620, gate 7).
//!
//! Three providers read dates with code of their own. The functions are
//! private to the crate, so each is asserted through the nearest public
//! entry that reaches it, against a stub listening on the loopback address:
//!
//! | Private function | Reached through | What it reads |
//! |---|---|---|
//! | `start_instant`, `parse_local_dt`, `wall_clock_in_zone` | `IntervalsIcuProvider::get_activity` | RFC 3339, then the offset-less wall clock `%Y-%m-%dT%H:%M:%S` placed in the athlete's zone or read as UTC |
//! | `convert_workout`, `whoop_utc_offset` | `WhoopProvider::get_activity` | RFC 3339 start and end; `±HH:MM` trimmed and parsed alone |
//! | (public) | `TerraMetadata`, `TerraConverters::activity_from_terra` | JSON into a value that keeps its offset |
//!
//! The verdict is the whole function's: it trims, falls back, or drops an
//! offset outside every civil zone. The same parsers alone, on a wider list,
//! are in `pierre-core` (`date_hand_parser_golden_test`).
//!
//! Zone rows use Toronto and Kolkata, on which the zone database in use and
//! the one that replaces it agree.
//!
//! Everything that names the date engine lives in [`engine`].

use pierre_providers::core::{CredentialKind, FitnessProvider, OAuth2Credentials};
use pierre_providers::intervals_icu_provider::{default_config, IntervalsIcuProvider};
use pierre_providers::models::Activity;
use pierre_providers::terra::models::{TerraActivity, TerraMetadata, TerraUser};
use pierre_providers::terra::TerraConverters;
use pierre_providers::whoop_provider::WhoopProvider;
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Everything in this file that names the date engine.
mod engine {
    use chrono::{DateTime, FixedOffset, Utc};

    /// The instant at nanosecond precision in UTC.
    pub(super) fn show(value: DateTime<Utc>) -> String {
        value.format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string()
    }

    /// The instant in UTC and the offset kept, as seconds east.
    pub(super) fn show_kept(value: DateTime<FixedOffset>) -> String {
        format!(
            "{} @ {}",
            show(value.with_timezone(&Utc)),
            value.offset().local_minus_utc()
        )
    }
}

const REJECTED: &str = "rejected";

/// A one-shot HTTP stub on the loopback address that answers one request with
/// `body` as JSON; returns its base URL.
async fn stub(body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            head.extend_from_slice(&buffer[..read]);
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.flush().await.unwrap();
    });
    format!("http://{address}")
}

fn credentials(kind: CredentialKind) -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: "i123456".to_owned(),
        client_secret: SecretString::default(),
        access_token: Some(SecretString::from("test-token")),
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
        kind,
        request_budget: None,
    }
}

/// An Intervals.icu activity with these date fields, read through the
/// provider: the start it is given, or `rejected` when it maps to nothing.
async fn intervals_start(start_date: Option<&str>, local: &str, zone: Option<&str>) -> String {
    let body = json!({
        "id": "i901",
        "type": "Ride",
        "start_date": start_date,
        "start_date_local": local,
        "timezone": zone,
        "elapsed_time": 3600,
    });
    let mut config = default_config();
    config.api_base_url = stub(body.to_string()).await;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(credentials(CredentialKind::ApiKey))
        .await
        .unwrap();
    provider.get_activity("i901").await.map_or_else(
        |_| REJECTED.to_owned(),
        |activity| engine::show(activity.start_date()),
    )
}

/// `start_date` wins when it reads as RFC 3339 or as an offset-less clock
/// taken as UTC. Without it the wall clock is placed in the named zone; with
/// no zone, an unknown one, or a clock that falls in a spring gap, the wall
/// clock is read as UTC.
#[tokio::test]
async fn an_intervals_icu_start_is_read_from_the_instant_or_the_wall_clock() {
    const TORONTO: Option<&str> = Some("America/Toronto");
    // start_date, start_date_local, timezone, the start read
    let expected = [
        // The instant is there.
        (
            Some("2026-08-30T14:00:00Z"),
            "2026-08-30T10:00:00",
            TORONTO,
            "2026-08-30T14:00:00.000000000Z",
        ),
        (
            Some("2026-08-30T10:00:00-04:00"),
            "2026-08-30T10:00:00",
            TORONTO,
            "2026-08-30T14:00:00.000000000Z",
        ),
        (
            Some("2026-08-30T14:00:00.250Z"),
            "2026-08-30T10:00:00",
            TORONTO,
            "2026-08-30T14:00:00.250000000Z",
        ),
        (
            Some("2026-08-30 14:00:00Z"),
            "2026-08-30T10:00:00",
            TORONTO,
            "2026-08-30T14:00:00.000000000Z",
        ),
        (
            Some("2026-08-30T14:00:00"),
            "2026-08-30T10:00:00",
            TORONTO,
            "2026-08-30T14:00:00.000000000Z",
        ),
        // The instant is unreadable: the wall clock takes over.
        (
            Some("2026-08-30"),
            "2026-08-30T10:00:00",
            TORONTO,
            "2026-08-30T14:00:00.000000000Z",
        ),
        (
            Some(""),
            "2026-08-30T10:00:00",
            TORONTO,
            "2026-08-30T14:00:00.000000000Z",
        ),
        // No instant: the wall clock in the athlete's zone.
        (
            None,
            "2026-08-30T10:00:00",
            TORONTO,
            "2026-08-30T14:00:00.000000000Z",
        ),
        (
            None,
            "2026-01-30T10:00:00",
            TORONTO,
            "2026-01-30T15:00:00.000000000Z",
        ),
        (
            None,
            "2026-08-30T10:00:00",
            Some("Asia/Kolkata"),
            "2026-08-30T04:30:00.000000000Z",
        ),
        (
            None,
            "2026-08-30T10:00:00",
            Some("Etc/UTC"),
            "2026-08-30T10:00:00.000000000Z",
        ),
        // No zone, or one that is not a zone: the wall clock as UTC.
        (
            None,
            "2026-08-30T10:00:00",
            None,
            "2026-08-30T10:00:00.000000000Z",
        ),
        (
            None,
            "2026-08-30T10:00:00",
            Some("Mars/Olympus"),
            "2026-08-30T10:00:00.000000000Z",
        ),
        (
            None,
            "2026-08-30T10:00:00",
            Some("america/toronto"),
            "2026-08-30T10:00:00.000000000Z",
        ),
        (
            None,
            "2026-08-30T10:00:00",
            Some(""),
            "2026-08-30T10:00:00.000000000Z",
        ),
        // Spring 2025: 02:30 does not exist in Toronto.
        (
            None,
            "2025-03-09T02:30:00",
            TORONTO,
            "2025-03-09T02:30:00.000000000Z",
        ),
        // Autumn 2025: 01:30 happens twice in Toronto.
        (
            None,
            "2025-11-02T01:30:00",
            TORONTO,
            "2025-11-02T05:30:00.000000000Z",
        ),
        // Wall clocks in other shapes.
        (
            None,
            "2026-08-30T10:00:00Z",
            TORONTO,
            "2026-08-30T10:00:00.000000000Z",
        ),
        (
            None,
            "2026-08-30T10:00:00-04:00",
            None,
            "2026-08-30T14:00:00.000000000Z",
        ),
        (None, "2026-08-30T10:00:00.5", TORONTO, "rejected"),
        (None, "2026-08-30 10:00:00", TORONTO, "rejected"),
        (None, "2026-08-30T10:00", TORONTO, "rejected"),
        (
            None,
            "2026-08-30T10:00:60",
            TORONTO,
            "2026-08-30T14:00:60.000000000Z",
        ),
        (
            None,
            "2026-8-30T9:0:0",
            TORONTO,
            "2026-08-30T13:00:00.000000000Z",
        ),
        (None, "2026-08-30", TORONTO, "rejected"),
        (None, "", TORONTO, "rejected"),
    ];

    let mut actual = Vec::new();
    for (start_date, local, zone, _) in expected {
        actual.push((
            start_date.unwrap_or("-"),
            local,
            zone.unwrap_or("-"),
            intervals_start(start_date, local, zone).await,
        ));
    }
    let wanted: Vec<(&str, &str, &str, String)> = expected
        .iter()
        .map(|(start_date, local, zone, start)| {
            (
                start_date.unwrap_or("-"),
                *local,
                zone.unwrap_or("-"),
                (*start).to_owned(),
            )
        })
        .collect();
    assert_eq!(actual, wanted, "the start Intervals.icu gives each row");
}

/// A WHOOP workout with these fields, read through the provider.
async fn whoop_workout(start: &str, end: &str, offset: Option<&str>) -> Option<Activity> {
    let body = json!({
        "id": "w901",
        "start": start,
        "end": end,
        "sport_name": "running",
        "timezone_offset": offset,
    });
    let mut config = WhoopProvider::new().config().clone();
    config.api_base_url = stub(body.to_string()).await;
    let provider = WhoopProvider::with_config(config);
    provider
        .set_credentials(credentials(CredentialKind::OAuthBearer))
        .await
        .unwrap();
    provider.get_activity("w901").await.ok()
}

/// The stated offset is trimmed and read as `±HH:MM`; anything else, and an
/// offset outside every civil zone, leaves the workout with no offset.
#[tokio::test]
async fn a_whoop_offset_is_read_as_seconds_east_or_dropped() {
    // timezone_offset, seconds east
    let expected = [
        (Some("-05:00"), "-18000"),
        (Some("-04:00"), "-14400"),
        (Some("+05:30"), "19800"),
        (Some("+05:45"), "20700"),
        (Some("+00:00"), "0"),
        (Some("-00:00"), "0"),
        (Some("+14:00"), "50400"),
        (Some("-12:00"), "-43200"),
        (Some("+14:01"), "none"),
        (Some("-12:01"), "none"),
        (Some("+23:59"), "none"),
        (Some("  -05:00  "), "-18000"),
        (Some("-0500"), "-18000"),
        (Some("-05"), "none"),
        (Some("-5:00"), "none"),
        (Some("05:00"), "none"),
        (Some("Z"), "none"),
        (Some("UTC"), "none"),
        (Some("America/Toronto"), "none"),
        (Some(""), "none"),
        (None, "none"),
    ];

    let mut actual = Vec::new();
    for (offset, _) in expected {
        let verdict = whoop_workout("2026-11-17T15:00:05Z", "2026-11-17T15:30:05Z", offset)
            .await
            .map_or_else(
                || REJECTED.to_owned(),
                |workout| {
                    workout
                        .utc_offset_seconds()
                        .map_or_else(|| "none".to_owned(), |seconds| seconds.to_string())
                },
            );
        actual.push((offset.unwrap_or("-"), verdict));
    }
    let wanted: Vec<(&str, String)> = expected
        .iter()
        .map(|(offset, seconds)| (offset.unwrap_or("-"), (*seconds).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "the WHOOP offset read from each timezone_offset"
    );
}

/// The start and end are strict RFC 3339; the duration is the whole seconds
/// between them, truncated.
#[tokio::test]
async fn a_whoop_start_and_end_are_strict_rfc_3339() {
    // start, end, the start read and the duration in seconds
    let expected = [
        (
            "2026-11-17T15:00:05Z",
            "2026-11-17T15:30:05Z",
            "2026-11-17T15:00:05.000000000Z for 1800s",
        ),
        (
            "2026-11-17T10:00:05-05:00",
            "2026-11-17T10:30:05-05:00",
            "2026-11-17T15:00:05.000000000Z for 1800s",
        ),
        (
            "2026-11-17T15:00:05.250Z",
            "2026-11-17T15:30:05.249Z",
            "2026-11-17T15:00:05.250000000Z for 1799s",
        ),
        (
            "2026-11-17T15:00:05.123456789Z",
            "2026-11-17T15:00:06.123456788Z",
            "2026-11-17T15:00:05.123456789Z for 0s",
        ),
        (
            "2026-11-17 15:00:05Z",
            "2026-11-17T15:30:05Z",
            "2026-11-17T15:00:05.000000000Z for 1800s",
        ),
        (
            "2026-11-17T15:30:05Z",
            "2026-11-17T15:00:05Z",
            "2026-11-17T15:30:05.000000000Z for 1800s",
        ),
        ("2026-11-17T15:00:05", "2026-11-17T15:30:05Z", "rejected"),
        ("2026-11-17T15:00:05Z", "2026-11-17T15:30:05", "rejected"),
        (
            "2026-11-17T15:00:05 UTC",
            "2026-11-17T15:30:05Z",
            "rejected",
        ),
        ("2026-11-17", "2026-11-17T15:30:05Z", "rejected"),
    ];

    let mut actual = Vec::new();
    for (start, end, _) in expected {
        let verdict = whoop_workout(start, end, None).await.map_or_else(
            || REJECTED.to_owned(),
            |workout| {
                format!(
                    "{} for {}s",
                    engine::show(workout.start_date()),
                    workout.duration_seconds()
                )
            },
        );
        actual.push((start, end, verdict));
    }
    let wanted: Vec<(&str, &str, String)> = expected
        .iter()
        .map(|(start, end, verdict)| (*start, *end, (*verdict).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "the WHOOP start and duration read from each start and end"
    );
}

/// Terra's `start_time` as JSON `text`, read into the payload model.
fn terra_start(text: &str) -> String {
    serde_json::from_value::<TerraMetadata>(json!({ "start_time": text }))
        .ok()
        .and_then(|metadata| metadata.start_time)
        .map_or_else(|| REJECTED.to_owned(), engine::show_kept)
}

/// The payload model keeps the offset Terra sent, which is the athlete's own
/// and dates a day or a night.
#[test]
fn a_terra_time_keeps_the_offset_it_was_sent_with() {
    let expected = [
        (
            "2024-01-15T08:00:00-05:00",
            "2024-01-15T13:00:00.000000000Z @ -18000",
        ),
        (
            "2024-01-15T08:00:00.000000-05:00",
            "2024-01-15T13:00:00.000000000Z @ -18000",
        ),
        (
            "2024-01-15T08:00:00.120-05:00",
            "2024-01-15T13:00:00.120000000Z @ -18000",
        ),
        (
            "2024-01-15T18:30:00+05:30",
            "2024-01-15T13:00:00.000000000Z @ 19800",
        ),
        ("2024-01-15T13:00:00Z", "2024-01-15T13:00:00.000000000Z @ 0"),
        (
            "2024-01-15T13:00:00+00:00",
            "2024-01-15T13:00:00.000000000Z @ 0",
        ),
        (
            "2024-01-15T13:00:00-00:00",
            "2024-01-15T13:00:00.000000000Z @ 0",
        ),
        (
            "2024-01-15T08:00:00-0500",
            "2024-01-15T13:00:00.000000000Z @ -18000",
        ),
        (
            "2024-01-15 08:00:00-05:00",
            "2024-01-15T13:00:00.000000000Z @ -18000",
        ),
        (
            "2024-01-15T13:00:00 UTC",
            "2024-01-15T13:00:00.000000000Z @ 0",
        ),
        ("2024-01-15T08:00:00-05", "rejected"),
        ("2024-01-15T08:00:00", "rejected"),
        ("2024-01-15", "rejected"),
        ("", "rejected"),
    ];
    let actual: Vec<(&str, String)> = expected
        .iter()
        .map(|(text, _)| (*text, terra_start(text)))
        .collect();
    let wanted: Vec<(&str, String)> = expected
        .iter()
        .map(|(text, verdict)| (*text, (*verdict).to_owned()))
        .collect();
    assert_eq!(
        actual, wanted,
        "the instant and offset Terra's start_time keeps"
    );
}

/// The workout Terra's payload maps to starts at the instant sent, whatever
/// its offset, and lasts the whole seconds to its end.
#[test]
fn a_terra_workout_starts_at_the_instant_and_lasts_to_its_end() {
    let workout = |start: &str, end: Value| {
        let terra: TerraActivity = serde_json::from_value(json!({
            "metadata": {
                "summary_id": "act_1",
                "start_time": start,
                "end_time": end,
                "type": 1,
            },
        }))
        .unwrap();
        let user = TerraUser {
            user_id: "terra_user_1".to_owned(),
            provider: Some("GARMIN".to_owned()),
            last_webhook_update: None,
            reference_id: None,
        };
        TerraConverters::activity_from_terra(&terra, &user).map_or_else(
            || REJECTED.to_owned(),
            |activity| {
                format!(
                    "{} for {}s",
                    engine::show(activity.start_date()),
                    activity.duration_seconds()
                )
            },
        )
    };
    let actual = [
        workout(
            "2024-01-15T08:00:00-05:00",
            json!("2024-01-15T09:00:00-05:00"),
        ),
        workout(
            "2024-01-15T08:00:00.250-05:00",
            json!("2024-01-15T14:00:00.249Z"),
        ),
        workout(
            "2024-01-15T18:30:00+05:30",
            json!("2024-01-15T08:00:00-05:00"),
        ),
        workout("2024-01-15T08:00:00-05:00", Value::Null),
    ];
    assert_eq!(
        actual,
        [
            "2024-01-15T13:00:00.000000000Z for 3600s",
            "2024-01-15T13:00:00.250000000Z for 3599s",
            "2024-01-15T13:00:00.000000000Z for 0s",
            "2024-01-15T13:00:00.000000000Z for 0s"
        ],
        "the start and duration of each Terra workout"
    );
}
