// ABOUTME: Wahoo provider tests against a loopback stub — the requests a push, re-push, delete and calendar read send
// ABOUTME: Pins reconcile-not-append, the neutral payload on the wire, and that a file off Wahoo's hosts is never fetched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Wahoo provider tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate, so without a surviving crate doc the command-line
// `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-wahoo")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::from_utf8;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use chrono::{Duration, NaiveDate, Utc};
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::errors::ErrorCode;
use pierre_providers::models::{PlannedSession, PlannedSessionKind, SportType, WorkoutStep};
use pierre_providers::pagination::PaginationParams;
use pierre_providers::wahoo_plan::opaque_id;
use pierre_providers::wahoo_provider::WahooProvider;
use pierre_providers::ProviderRegistry;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// One request the stub received.
#[derive(Debug)]
struct Captured {
    method: String,
    path: String,
    head: String,
    body: String,
}

impl Captured {
    /// The decoded value of a form field in the body.
    fn field(&self, name: &str) -> Option<String> {
        self.body.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (percent_decode(key) == name).then(|| percent_decode(value))
        })
    }

    fn has_bearer(&self, token: &str) -> bool {
        self.head.lines().any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    && value.trim() == format!("Bearer {token}")
            })
        })
    }
}

/// Decode `application/x-www-form-urlencoded`.
fn percent_decode(text: &str) -> String {
    let bytes = text.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// Serve `script` in order — one `(status, body)` per request — capturing each
/// request's method, path, head and body.
async fn stub(script: Vec<(&'static str, &'static str)>) -> (String, JoinHandle<Vec<Captured>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let mut captured = Vec::new();
        for (status, body) in script {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut raw = Vec::new();
            let mut buf = [0_u8; 4096];
            let head_end = loop {
                let n = socket.read(&mut buf).await.expect("read request");
                raw.extend_from_slice(&buf[..n]);
                if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break at + 4;
                }
                assert!(n > 0, "connection closed before the request head ended");
            };
            let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            while raw.len() < head_end + length {
                let n = socket.read(&mut buf).await.expect("read body");
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
            }
            let body_text = String::from_utf8_lossy(&raw[head_end..]).into_owned();
            let mut request_line = head.lines().next().unwrap_or_default().split(' ');
            let method = request_line.next().unwrap_or_default().to_owned();
            let path = request_line.next().unwrap_or_default().to_owned();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.expect("write");
            socket.flush().await.expect("flush");
            captured.push(Captured {
                method,
                path,
                head,
                body: body_text,
            });
        }
        captured
    });
    (format!("http://{addr}/v1"), handle)
}

const TOKEN: &str = "wahoo-access-token";

async fn provider(base_url: &str) -> WahooProvider {
    let registry = ProviderRegistry::new();
    let mut config: ProviderConfig = registry
        .default_config("wahoo")
        .cloned()
        .expect("wahoo is registered");
    config.api_base_url = base_url.to_owned();
    let provider = WahooProvider::with_config(config);
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "client".to_owned(),
            client_secret: "secret".to_owned(),
            access_token: Some(TOKEN.to_owned()),
            refresh_token: Some("refresh".to_owned()),
            expires_at: Some(Utc::now() + Duration::hours(2)),
            scopes: vec![],
            kind: pierre_providers::CredentialKind::OAuthBearer,
            request_budget: None,
        })
        .await
        .unwrap();
    provider
}

fn step(label: &str, minutes: u32, zone: &str, repeat: u32) -> WorkoutStep {
    WorkoutStep {
        label: label.to_owned(),
        duration_seconds: minutes * 60,
        distance_meters: None,
        target_zone: zone.to_owned(),
        repeat,
        repeat_group: (repeat > 1).then_some(1),
        note: Some("the athlete's knee is sore".to_owned()),
    }
}

const KEY: &str = "dravr:plan:4b0f3c1e-8a2d-4f6b-9c3e-2d1a7e5f9b10:2026-10-08:0";

fn session() -> PlannedSession {
    PlannedSession {
        external_id: KEY.to_owned(),
        kind: PlannedSessionKind::Workout,
        date: NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
        sport: SportType::Ride,
        name: "Threshold for Julie, HRV is back up".to_owned(),
        duration_seconds: Some(3600),
        notes: "Julie: fuel 60 g/h".to_owned(),
        steps: vec![
            step("Warm-up", 15, "Z2", 1),
            step("On", 8, "Z4", 3),
            step("Off", 4, "Z1", 3),
            step("Cool-down", 9, "Z1", 1),
        ],
    }
}

const POWER_ZONES: &str =
    r#"[{"id":1,"ftp":"250","workout_type_family_id":0,"updated_at":"2026-09-01T00:00:00.000Z"}]"#;

#[tokio::test]
async fn a_push_uploads_the_plan_then_schedules_a_workout_attached_to_it() {
    let (base, stub) = stub(vec![
        ("200 OK", POWER_ZONES),
        ("200 OK", "[]"),
        ("201 Created", r#"{"id":41}"#),
        ("201 Created", r#"{"id":77}"#),
    ])
    .await;
    let provider = provider(&base).await;

    let event_id = provider.push_planned_session(&session()).await.unwrap();
    assert_eq!(
        event_id, "77",
        "the scheduled workout is the calendar entry"
    );

    let requests = stub.await.unwrap();
    let opaque = opaque_id(KEY);
    assert_eq!(requests[0].path, "/v1/power_zones");
    assert_eq!(requests[1].path, format!("/v1/plans?external_id={opaque}"));
    assert_eq!(
        (requests[2].method.as_str(), requests[2].path.as_str()),
        ("POST", "/v1/plans")
    );
    assert_eq!(requests[2].field("plan[external_id]"), Some(opaque.clone()));
    assert_eq!(
        requests[2].field("plan[filename]").as_deref(),
        Some("plan.json")
    );
    assert!(requests[2]
        .field("plan[file]")
        .is_some_and(|file| file.starts_with("data:application/json;base64,")));
    assert_eq!(
        (requests[3].method.as_str(), requests[3].path.as_str()),
        ("POST", "/v1/workouts")
    );
    assert_eq!(requests[3].field("workout[plan_id]").as_deref(), Some("41"));
    assert_eq!(requests[3].field("workout[workout_token]"), Some(opaque));
    assert_eq!(
        requests[3].field("workout[name]").as_deref(),
        Some("Ride 1 h · 3×8 min")
    );
    assert_eq!(
        requests[3].field("workout[starts]").as_deref(),
        Some("2026-10-08T12:00:00.000Z")
    );
    assert!(requests.iter().all(|request| request.has_bearer(TOKEN)));
}

#[tokio::test]
async fn nothing_the_coach_or_the_athlete_wrote_reaches_the_wire() {
    let (base, stub) = stub(vec![
        ("200 OK", POWER_ZONES),
        ("200 OK", "[]"),
        ("201 Created", r#"{"id":41}"#),
        ("201 Created", r#"{"id":77}"#),
    ])
    .await;
    provider(&base)
        .await
        .push_planned_session(&session())
        .await
        .unwrap();
    for request in stub.await.unwrap() {
        let mut sent = format!("{} {}", request.path, percent_decode(&request.body));
        if let Some(file) = request.field("plan[file]") {
            let encoded = file.trim_start_matches("data:application/json;base64,");
            let decoded = BASE64.decode(encoded).unwrap();
            sent.push_str(&String::from_utf8(decoded).unwrap());
        }
        for forbidden in ["Julie", "HRV", "knee", "fuel", "4b0f3c1e", "dravr:plan"] {
            assert!(!sent.contains(forbidden), "'{forbidden}' was sent: {sent}");
        }
    }
}

#[tokio::test]
async fn a_repush_updates_dravrs_plan_in_place_rather_than_adding_one() {
    let (base, stub) = stub(vec![
        ("200 OK", POWER_ZONES),
        ("200 OK", r#"[{"id":41,"deleted":false,"external_id":"x"}]"#),
        ("200 OK", r#"{"id":41}"#),
        ("200 OK", r#"{"id":77}"#),
    ])
    .await;
    provider(&base)
        .await
        .update_planned_session("77", &session())
        .await
        .unwrap();
    let requests = stub.await.unwrap();
    assert_eq!(
        (requests[2].method.as_str(), requests[2].path.as_str()),
        ("PUT", "/v1/plans/41"),
        "the plan Dravr already put in the library is updated"
    );
    assert_eq!(
        requests[2].field("plan[external_id]"),
        None,
        "an update cannot change the external id"
    );
    assert_eq!(
        (requests[3].method.as_str(), requests[3].path.as_str()),
        ("PUT", "/v1/workouts/77")
    );
    assert_eq!(requests[3].field("workout[plan_id]").as_deref(), Some("41"));
    assert!(requests.iter().all(|request| request.method != "POST"));
}

#[tokio::test]
async fn a_delete_removes_the_workout_and_its_plan_and_skips_an_unknown_one() {
    let (base, stub) = stub(vec![
        (
            "200 OK",
            r#"{"id":77,"starts":"2026-10-08T12:00:00.000Z","plan_id":41,"workout_token":"dravr-x"}"#,
        ),
        ("204 No Content", ""),
        ("204 No Content", ""),
        ("404 Not Found", r#"{"error":"not found"}"#),
    ])
    .await;
    let deleted = provider(&base)
        .await
        .delete_planned_sessions(&["77".to_owned(), "78".to_owned()])
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    let requests = stub.await.unwrap();
    assert_eq!(
        (requests[1].method.as_str(), requests[1].path.as_str()),
        ("DELETE", "/v1/workouts/77")
    );
    assert_eq!(
        (requests[2].method.as_str(), requests[2].path.as_str()),
        ("DELETE", "/v1/plans/41")
    );
    assert_eq!(requests[3].path, "/v1/workouts/78");
}

#[tokio::test]
async fn the_calendar_lists_only_dravrs_workouts_inside_the_window() {
    let page = r#"{"workouts":[
        {"id":5,"starts":"2026-10-20T12:00:00.000Z","workout_token":"dravr-late","updated_at":"2026-10-07T00:00:00.000Z"},
        {"id":4,"starts":"2026-10-09T12:00:00.000Z","workout_token":"dravr-in","updated_at":"2026-10-07T00:00:00.000Z"},
        {"id":3,"starts":"2026-10-08T17:00:00.000Z","workout_token":"123","updated_at":"2026-10-07T00:00:00.000Z"},
        {"id":2,"starts":"2026-10-01T12:00:00.000Z","workout_token":"dravr-past","updated_at":"2026-10-01T00:00:00.000Z"},
        {"id":1,"starts":"2026-09-30T12:00:00.000Z","workout_token":"dravr-older"}
    ],"total":40,"page":1,"per_page":50}"#;
    let (base, stub) = stub(vec![("200 OK", page)]).await;
    let events = provider(&base)
        .await
        .list_calendar_events(
            NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].provider_event_id, "4");
    assert_eq!(events[0].external_id.as_deref(), Some("dravr-in"));
    let requests = stub.await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "the walk stops at the first workout before the window"
    );
}

/// A pushed plan puts every future session at the head of the listing, so an
/// activity fetch walks past whole pages of scheduled workouts rather than
/// answering short as if the athlete's history ended there.
#[tokio::test]
async fn an_activity_fetch_walks_past_pages_of_scheduled_workouts() {
    let (base, stub) = stub(vec![
        (
            "200 OK",
            r#"{"workouts":[{"id":30,"starts":"2026-12-01T12:00:00.000Z","workout_token":"dravr-a"}],"total":150}"#,
        ),
        (
            "200 OK",
            r#"{"workouts":[{"id":20,"starts":"2026-11-01T12:00:00.000Z","workout_token":"dravr-b"}],"total":150}"#,
        ),
        (
            "200 OK",
            r#"{"workouts":[{"id":10,"starts":"2026-10-01T09:00:00.000Z","workout_type_id":0,
                "workout_summary":{"duration_total_accum":"3600.0"}}],"total":150}"#,
        ),
    ])
    .await;
    let activities = provider(&base)
        .await
        .get_activities_with_params(&ActivityQueryParams::with_pagination(Some(1), None))
        .await
        .unwrap();
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].id(), "10");
    assert_eq!(activities[0].duration_seconds(), 3600);
    let requests = stub.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].path, "/v1/workouts?page=3&per_page=50");
}

/// A cursor page asks Wahoo for the caller's limit, so no listed workout is
/// passed over, and its cursor keeps counting pages of that size.
#[tokio::test]
async fn a_cursor_page_reads_the_limit_and_the_next_page_after_it() {
    let completed = r#"{"workouts":[
        {"id":12,"starts":"2026-10-03T09:00:00.000Z","workout_type_id":0,"workout_summary":{"duration_total_accum":"1800.0"}},
        {"id":11,"starts":"2026-10-02T09:00:00.000Z","workout_type_id":1,"workout_summary":{"duration_total_accum":"2400.0"}}
    ],"total":5}"#;
    let (base, stub) = stub(vec![("200 OK", completed), ("200 OK", completed)]).await;
    let provider = provider(&base).await;

    let first = provider
        .get_activities_cursor(&PaginationParams::forward(None, 2))
        .await
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert!(first.has_more);
    let next = first.next_cursor.expect("a next page");
    assert_eq!(next.decode().map(|(_, at)| at).as_deref(), Some("2:2"));

    // A different limit on the way does not shift the pages under the cursor.
    provider
        .get_activities_cursor(&PaginationParams::forward(Some(next), 5))
        .await
        .unwrap();
    let requests = stub.await.unwrap();
    assert_eq!(requests[0].path, "/v1/workouts?page=1&per_page=2");
    assert_eq!(requests[1].path, "/v1/workouts?page=2&per_page=2");
}

#[tokio::test]
async fn a_workout_file_off_wahoos_hosts_is_never_fetched() {
    let workout = r#"{"id":9,"starts":"2026-10-06T10:00:00.000Z","workout_type_id":0,
        "workout_summary":{"duration_total_accum":"3600.0","file":{"url":"http://127.0.0.1:9/ride.fit"}}}"#;
    let (base, stub) = stub(vec![("200 OK", workout)]).await;
    let series = provider(&base)
        .await
        .get_activity_streams("9")
        .await
        .unwrap();
    assert!(series.is_none(), "an off-host file yields no series");
    assert_eq!(
        stub.await.unwrap().len(),
        1,
        "only the workout was requested"
    );
}

#[tokio::test]
async fn a_rejected_access_token_asks_the_athlete_to_reconnect() {
    let (base, stub) = stub(vec![("401 Unauthorized", r#"{"error":"revoked"}"#)]).await;
    let err = provider(&base).await.get_athlete().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ProviderAuthRequired);
    assert_eq!(
        err.provider_auth_required_provider().as_deref(),
        Some("wahoo")
    );
    stub.await.unwrap();
}

#[test]
fn wahoo_is_registered_with_its_calendar_and_its_terms() {
    let registry = ProviderRegistry::new();
    let caps = registry.get_capabilities("wahoo").expect("registered");
    assert!(caps.supports_calendar_write());
    assert!(
        !caps.supports_calendar_week_notes(),
        "a workout calendar holds no notes"
    );
    let descriptor = registry.get_descriptor("wahoo").expect("descriptor");
    assert!(descriptor.refused_refresh_is_revocation());
    assert!(descriptor.bars_cross_athlete_learning());
    assert!(descriptor.owner_id_from_api());
    let config = registry.default_config("wahoo").expect("config");
    assert_eq!(config.token_url, "https://api.wahooligan.com/oauth/token");
    assert_eq!(
        config.revoke_url.as_deref(),
        Some("https://api.wahooligan.com/v1/permissions")
    );
    assert!(config
        .default_scopes
        .iter()
        .any(|scope| scope == "plans_write"));
    assert!(!config
        .default_scopes
        .iter()
        .any(|scope| scope == "user_write"));
}
