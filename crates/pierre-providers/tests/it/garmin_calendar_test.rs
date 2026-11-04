// ABOUTME: Garmin training-calendar tests against a loopback stub — the Training API calls a push, update, delete and listing send
// ABOUTME: Pins the WORKOUT_IMPORT gate, the workout-then-schedule pair as one entry id, and the neutral payload on the wire
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin training-calendar tests.

use chrono::{Duration, NaiveDate, Utc};
use pierre_providers::core::{CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig};
use pierre_providers::garmin_provider::GarminProvider;
use pierre_providers::models::{PlannedSession, PlannedSessionKind, SportType, WorkoutStep};
use pierre_providers::ProviderRegistry;
use secrecy::SecretString;
use serde_json::Value;
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
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{} body: {e}", self.path))
    }

    fn is(&self, method: &str, path: &str) -> bool {
        self.method == method && self.path == path
    }
}

/// Serve `script` in order — one `(status, body)` per request — capturing
/// each request. The base URL ends in the Health API's path, as Garmin's
/// does, so a Training API call landing on the host root is visible.
async fn stub(script: Vec<(&'static str, &'static str)>) -> (String, JoinHandle<Vec<Captured>>) {
    serve(
        script
            .into_iter()
            .map(|(status, body)| (status, body, body.len()))
            .collect(),
    )
    .await
}

/// [`stub`], each reply declaring its own `Content-Length`: a length past the
/// body sent is an answer cut short when the connection closes.
async fn serve(
    script: Vec<(&'static str, &'static str, usize)>,
) -> (String, JoinHandle<Vec<Captured>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let mut captured = Vec::new();
        for (status, body, declared) in script {
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
            let mut request_line = head.lines().next().unwrap_or_default().split(' ');
            let method = request_line.next().unwrap_or_default().to_owned();
            let path = request_line.next().unwrap_or_default().to_owned();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n{body}"
            );
            socket.write_all(response.as_bytes()).await.expect("write");
            socket.flush().await.expect("flush");
            captured.push(Captured {
                method,
                path,
                body: String::from_utf8_lossy(&raw[head_end..]).into_owned(),
                head,
            });
        }
        captured
    });
    (format!("http://{addr}/wellness-api/rest"), handle)
}

const TOKEN: &str = "garmin-access-token";

const GRANTED: (&str, &str) = ("200 OK", r#"["ACTIVITY_EXPORT","WORKOUT_IMPORT"]"#);

async fn provider(base_url: &str) -> GarminProvider {
    let registry = ProviderRegistry::new();
    let mut config: ProviderConfig = registry
        .default_config("garmin")
        .cloned()
        .expect("garmin is registered");
    config.api_base_url = base_url.to_owned();
    let provider = GarminProvider::with_config(config);
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "client".to_owned(),
            client_secret: SecretString::from("secret"),
            access_token: Some(SecretString::from(TOKEN.to_owned())),
            refresh_token: Some(SecretString::from("refresh")),
            expires_at: Some(Utc::now() + Duration::hours(20)),
            scopes: vec![],
            kind: CredentialKind::OAuthBearer,
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

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 12).unwrap()
}

fn session() -> PlannedSession {
    PlannedSession {
        external_id: "dravr:plan:4b0f3c1e-8a2d-4f6b-9c3e-2d1a7e5f9b10:2026-11-12:0".to_owned(),
        kind: PlannedSessionKind::Workout,
        date: date(),
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

#[tokio::test]
async fn a_push_creates_the_workout_then_schedules_it_and_names_the_pair() {
    let (base, stub) = stub(vec![
        GRANTED,
        (
            "200 OK",
            r#"{"workoutId":41,"ownerId":9000,"workoutName":"x"}"#,
        ),
        ("200 OK", "77"),
    ])
    .await;
    let event_id = provider(&base)
        .await
        .push_planned_session(&session())
        .await
        .unwrap();
    assert_eq!(
        event_id, "41:77",
        "the entry is the workout and its schedule"
    );

    let requests = stub.await.unwrap();
    assert!(requests[0].is("GET", "/wellness-api/rest/user/permissions"));
    assert!(requests[1].is("POST", "/workoutportal/workout/v2"));
    let workout = requests[1].json();
    assert_eq!(workout["workoutName"], "Ride 1 h · 3×8 min");
    assert_eq!(workout["sport"], "CYCLING");
    assert_eq!(workout["workoutProvider"], "Dravr");
    assert_eq!(
        workout["segments"][0]["steps"][1]["type"],
        "WorkoutRepeatStep"
    );
    assert!(requests[2].is("POST", "/training-api/schedule/"));
    assert_eq!(
        requests[2].json(),
        serde_json::json!({"workoutId": 41, "date": "2026-11-12"})
    );
    for request in &requests {
        assert!(
            request.head.lines().any(|line| line
                .to_ascii_lowercase()
                .starts_with("authorization: bearer garmin-access-token")),
            "{} carries the athlete's token",
            request.path
        );
    }
}

#[tokio::test]
async fn nothing_the_coach_or_the_athlete_wrote_reaches_the_wire() {
    let (base, stub) = stub(vec![
        GRANTED,
        ("200 OK", r#"{"workoutId":41,"ownerId":9000}"#),
        (
            "200 OK",
            r#"{"scheduleId":77,"workoutId":41,"date":"2026-11-12"}"#,
        ),
    ])
    .await;
    provider(&base)
        .await
        .push_planned_session(&session())
        .await
        .unwrap();
    for request in stub.await.unwrap() {
        let sent = format!("{} {}", request.path, request.body);
        for forbidden in ["Julie", "HRV", "knee", "fuel", "4b0f3c1e", "dravr:plan"] {
            assert!(!sent.contains(forbidden), "'{forbidden}' was sent: {sent}");
        }
    }
}

#[tokio::test]
async fn an_athlete_who_did_not_grant_workout_import_is_refused_before_anything_is_written() {
    let (base, stub) = stub(vec![("200 OK", r#"["ACTIVITY_EXPORT","HEALTH_EXPORT"]"#)]).await;
    let provider = provider(&base).await;
    let err = provider
        .push_planned_session(&session())
        .await
        .expect_err("no workout import, no push");
    assert!(err.to_string().contains("WORKOUT_IMPORT"), "{err}");
    let err = provider
        .list_calendar_events(date(), date())
        .await
        .expect_err("the refusal is remembered");
    assert!(err.to_string().contains("WORKOUT_IMPORT"), "{err}");
    let requests = stub.await.unwrap();
    assert_eq!(requests.len(), 1, "only the permission read was sent");
}

#[tokio::test]
async fn a_grant_withdrawn_since_the_read_is_refused_by_name() {
    let (base, stub) = stub(vec![
        GRANTED,
        (
            "412 Precondition Failed",
            r#"{"errorMessage":"permission"}"#,
        ),
    ])
    .await;
    let err = provider(&base)
        .await
        .push_planned_session(&session())
        .await
        .expect_err("Garmin refused the write");
    assert!(err.to_string().contains("WORKOUT_IMPORT"), "{err}");
    assert_eq!(stub.await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_schedule_that_fails_takes_its_workout_with_it() {
    let (base, stub) = stub(vec![
        GRANTED,
        ("200 OK", r#"{"workoutId":41,"ownerId":9000}"#),
        ("400 Bad Request", r#"{"errorMessage":"bad date"}"#),
        ("204 No Content", ""),
    ])
    .await;
    provider(&base)
        .await
        .push_planned_session(&session())
        .await
        .expect_err("the schedule was refused");
    let requests = stub.await.unwrap();
    assert!(requests[3].is("DELETE", "/training-api/workout/v2/41"));
}

#[tokio::test]
async fn a_schedule_create_that_names_no_id_is_found_on_the_days_listing() {
    let (base, stub) = stub(vec![
        GRANTED,
        ("200 OK", r#"{"workoutId":41,"ownerId":9000}"#),
        ("204 No Content", ""),
        (
            "200 OK",
            r#"[{"scheduleId":5,"workoutId":3,"date":"2026-11-12"},{"scheduleId":77,"workoutId":41,"date":"2026-11-12"}]"#,
        ),
    ])
    .await;
    let event_id = provider(&base)
        .await
        .push_planned_session(&session())
        .await
        .unwrap();
    assert_eq!(event_id, "41:77");
    let requests = stub.await.unwrap();
    assert!(requests[3].is(
        "GET",
        "/training-api/schedule?startDate=2026-11-12&endDate=2026-11-12"
    ));
}

#[tokio::test]
async fn an_update_replaces_the_workout_in_place_and_moves_its_schedule() {
    let (base, stub) = stub(vec![
        GRANTED,
        (
            "200 OK",
            r#"{"workoutId":41,"ownerId":9000,"workoutName":"old"}"#,
        ),
        ("200 OK", r#"{"workoutId":41,"ownerId":9000}"#),
        (
            "200 OK",
            r#"{"scheduleId":77,"workoutId":41,"date":"2026-11-12"}"#,
        ),
    ])
    .await;
    provider(&base)
        .await
        .update_planned_session("41:77", &session())
        .await
        .unwrap();
    let requests = stub.await.unwrap();
    assert!(requests[1].is("GET", "/training-api/workout/v2/41"));
    assert!(requests[2].is("PUT", "/training-api/workout/v2/41"));
    let workout = requests[2].json();
    assert_eq!(workout["workoutId"], 41);
    assert_eq!(
        workout["ownerId"], 9000,
        "Garmin requires the owner on an update"
    );
    assert!(requests[3].is("PUT", "/training-api/schedule/77"));
    assert_eq!(
        requests[3].json(),
        serde_json::json!({"scheduleId": 77, "workoutId": 41, "date": "2026-11-12"})
    );
    assert!(requests.iter().all(|request| request.method != "POST"));
}

#[tokio::test]
async fn a_delete_removes_each_schedule_and_its_workout_and_skips_one_already_gone() {
    let (base, stub) = stub(vec![
        GRANTED,
        ("204 No Content", ""),
        ("204 No Content", ""),
        ("404 Not Found", ""),
        ("404 Not Found", ""),
    ])
    .await;
    let deleted = provider(&base)
        .await
        .delete_planned_sessions(&["41:77".to_owned(), "42:78".to_owned()])
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    let requests = stub.await.unwrap();
    assert!(requests[1].is("DELETE", "/training-api/schedule/77"));
    assert!(requests[2].is("DELETE", "/training-api/workout/v2/41"));
    assert!(requests[3].is("DELETE", "/training-api/schedule/78"));
    assert!(requests[4].is("DELETE", "/training-api/workout/v2/42"));
}

#[tokio::test]
async fn an_id_garmin_never_gave_is_refused_before_any_call() {
    let (base, stub) = stub(vec![]).await;
    let provider = provider(&base).await;
    for bad in ["77", "41:../x", "dravr-123"] {
        assert!(provider
            .delete_planned_sessions(&[bad.to_owned()])
            .await
            .is_err());
        assert!(provider
            .update_planned_session(bad, &session())
            .await
            .is_err());
    }
    let mut note = session();
    note.kind = PlannedSessionKind::WeekNote;
    assert!(provider
        .push_planned_session(&note)
        .await
        .expect_err("a week note")
        .to_string()
        .contains("week notes"));
    assert!(stub.await.unwrap().is_empty(), "nothing was sent");
}

#[tokio::test]
async fn the_listing_names_each_schedule_in_the_window_by_its_pair() {
    let (base, stub) = stub(vec![
        GRANTED,
        (
            "200 OK",
            r#"[
                {"scheduleId":77,"workoutId":41,"date":"2026-11-12"},
                {"scheduleId":78,"workoutId":42,"date":"2026-11-14"},
                {"scheduleId":79,"workoutId":43,"date":"2026-12-01"}
            ]"#,
        ),
        ("200 OK", ""),
    ])
    .await;
    let provider = provider(&base).await;
    let to = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
    let events = provider.list_calendar_events(date(), to).await.unwrap();
    let ids: Vec<&str> = events
        .iter()
        .map(|e| e.provider_event_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["41:77", "42:78"],
        "a schedule outside the window is left out"
    );
    assert_eq!(
        events[1].date,
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
    );
    assert!(events
        .iter()
        .all(|e| e.external_id.is_none() && e.updated_at.is_none()));

    let empty = provider.list_calendar_events(date(), to).await.unwrap();
    assert!(empty.is_empty(), "an empty answer is an empty calendar");

    let requests = stub.await.unwrap();
    assert_eq!(
        requests.len(),
        3,
        "the permission is read once per provider"
    );
    assert!(requests[1].is(
        "GET",
        "/training-api/schedule?startDate=2026-11-12&endDate=2026-11-20"
    ));
}

#[tokio::test]
async fn a_listing_cut_short_is_an_error_never_an_empty_calendar() {
    // Read as empty, the reconcile would take every live entry for one the
    // athlete removed and write it again.
    let partial = r#"[{"scheduleId":77,"workoutId":41,"#;
    let (base, stub) = serve(vec![
        (GRANTED.0, GRANTED.1, GRANTED.1.len()),
        ("200 OK", partial, partial.len() + 64),
    ])
    .await;
    let err = provider(&base)
        .await
        .list_calendar_events(date(), date())
        .await
        .expect_err("bug: a listing whose body never arrived read as a calendar");
    assert!(err.to_string().contains("unreadable"), "{err}");
    assert_eq!(
        stub.await.unwrap().len(),
        2,
        "the permission and the listing were each read once"
    );
}
