// ABOUTME: Pins the requests a workout-library save sends Intervals.icu: folder lookup, folder create, workout create, and the no-op re-save
// ABOUTME: Also that a provider built for a coached athlete never writes a library, that an OAuth grant missing the library scope is told to reconnect, and that Intervals.icu declares one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::{Arc, Mutex};

use dravr_tronc::dates::CivilDate;
use pierre_providers::core::{CredentialKind, FitnessProvider, OAuth2Credentials};
use pierre_providers::errors::ErrorCode;
use pierre_providers::intervals_icu_provider::IntervalsIcuProvider;
use pierre_providers::models::{PlannedSession, PlannedSessionKind, SportType, WorkoutStep};
use pierre_providers::spi::IntervalsIcuDescriptor;
use pierre_test_support::providers::{offline_provider_config, offline_provider_registry};
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;

const COACH_ID: &str = "i100";
const COACH_KEY: &str = "coach-key";
/// An OAuth token granted before Dravr asked for `LIBRARY:WRITE`: the stub
/// refuses it every library call, as Intervals.icu does.
const GRANT_WITHOUT_LIBRARY: &str = "grant-without-library";

/// Every request the stub saw: method, target, and the JSON body a write sent.
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

fn session() -> PlannedSession {
    PlannedSession {
        external_id: "dravr:plan:u:2026-11-25:0".to_owned(),
        kind: PlannedSessionKind::Workout,
        date: CivilDate::from_ymd(2026, 11, 25).expect("a calendar date"),
        sport: SportType::VirtualRide,
        name: "VO2 30/30".to_owned(),
        duration_seconds: Some(3_600),
        notes: "Zwift, free ride".to_owned(),
        steps: vec![
            WorkoutStep {
                label: "Warm-up".to_owned(),
                duration_seconds: 600,
                distance_meters: None,
                target_zone: "Z2".to_owned(),
                repeat: 1,
                repeat_group: None,
                note: None,
            },
            WorkoutStep {
                label: "Short interval".to_owned(),
                duration_seconds: 30,
                distance_meters: None,
                target_zone: "Z5".to_owned(),
                repeat: 10,
                repeat_group: None,
                note: None,
            },
        ],
    }
}

fn coach_credentials() -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: COACH_ID.to_owned(),
        client_secret: SecretString::default(),
        access_token: Some(SecretString::from(COACH_KEY.to_owned())),
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
        kind: CredentialKind::ApiKey,
        request_budget: None,
    }
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0)
}

/// A loopback stand-in for the library endpoints: `GET /folders` answers
/// `folders`, and each create answers a fresh id.
async fn spawn_library(folders: Value, seen: Seen) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (seen, folders) = (Arc::clone(&seen), folders.clone());
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0_u8; 4096];
                let end = loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break raw.len();
                    }
                    raw.extend_from_slice(&buf[..n]);
                    let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
                    if raw.len() >= end + 4 + content_length(&head) {
                        break end;
                    }
                };
                let head = String::from_utf8_lossy(&raw[..end]).into_owned();
                let mut start = head.split_whitespace();
                let method = start.next().unwrap_or_default().to_owned();
                let target = start.next().unwrap_or_default().to_owned();
                let body: Value = serde_json::from_slice(raw.get(end + 4..).unwrap_or_default())
                    .unwrap_or(Value::Null);
                let path = format!("/api/v1/athlete/{COACH_ID}");
                let stale = format!("authorization: Bearer {GRANT_WITHOUT_LIBRARY}");
                let (status, answer) = if head.lines().any(|l| l.eq_ignore_ascii_case(&stale)) {
                    ("403 Forbidden", "{}".to_owned())
                } else {
                    match (method.as_str(), target.strip_prefix(&path)) {
                        ("GET", Some("/folders")) => ("200 OK", folders.to_string()),
                        ("POST", Some("/folders")) => ("200 OK", r#"{"id":31}"#.to_owned()),
                        ("POST", Some("/workouts")) => ("200 OK", r#"{"id":77}"#.to_owned()),
                        _ => ("404 Not Found", "{}".to_owned()),
                    }
                };
                seen.lock().unwrap().push((method, target, body));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                drop(socket.write_all(response.as_bytes()).await);
                drop(socket.flush().await);
            });
        }
    });
    format!("http://{addr}")
}

async fn coach_provider(base_url: String) -> IntervalsIcuProvider {
    let mut config = offline_provider_config("intervals_icu").unwrap();
    config.api_base_url = base_url;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(coach_credentials())
        .await
        .expect("set creds");
    provider
}

fn requests(seen: &Seen) -> Vec<(String, String, Value)> {
    seen.lock().unwrap().clone()
}

#[tokio::test]
async fn a_missing_folder_is_created_then_the_workout_goes_in_it() {
    let seen = Seen::default();
    let base = spawn_library(
        json!([{ "id": 5, "name": "Dravr", "type": "PLAN", "children": [] }]),
        Arc::clone(&seen),
    )
    .await;
    let provider = coach_provider(base).await;

    let saved = provider
        .save_library_workout(&session(), "Dravr")
        .await
        .expect("the save lands");

    assert_eq!(
        saved.folder_id, "31",
        "a plan of the same name is not the folder"
    );
    assert_eq!(saved.workout_id, "77", "bug: the id the create answered");
    assert_eq!(saved.folder, "Dravr", "bug: the folder the save named");
    assert!(
        saved.folder_created,
        "bug: the folder was missing: {saved:?}"
    );
    assert!(
        !saved.already_saved,
        "bug: the folder held nothing: {saved:?}"
    );
    let seen = requests(&seen);
    let calls: Vec<(&str, &str)> = seen
        .iter()
        .map(|(method, target, _)| (method.as_str(), target.as_str()))
        .collect();
    assert_eq!(
        calls,
        [
            ("GET", "/api/v1/athlete/i100/folders"),
            ("POST", "/api/v1/athlete/i100/folders"),
            ("POST", "/api/v1/athlete/i100/workouts"),
        ],
        "bug: list, create the folder, create the workout, at the coach's own path"
    );
    assert_eq!(
        seen[1].2,
        json!({ "name": "Dravr", "type": "FOLDER" }),
        "bug: a plain folder, never a plan"
    );
    let workout = &seen[2].2;
    assert_eq!(workout["folder_id"], 31, "bug: into the created folder");
    assert_eq!(workout["name"], "VO2 30/30", "bug: {workout}");
    assert_eq!(workout["type"], "VirtualRide", "bug: {workout}");
    assert_eq!(workout["moving_time"], 3_600, "bug: {workout}");
    let description = workout["description"].as_str().expect("a description");
    assert!(
        description.contains("10x") && description.contains("- Short interval 30s"),
        "the steps go out in the workout DSL: {description}"
    );
}

#[tokio::test]
async fn an_existing_folder_is_reused_whatever_its_case() {
    let seen = Seen::default();
    let base = spawn_library(
        json!([{ "id": 9, "name": "vo2 sessions", "type": "FOLDER", "children": [] }]),
        Arc::clone(&seen),
    )
    .await;
    let provider = coach_provider(base).await;

    let saved = provider
        .save_library_workout(&session(), "VO2 Sessions")
        .await
        .expect("the save lands");

    assert_eq!(saved.folder_id, "9", "bug: the existing folder");
    assert_eq!(saved.folder, "vo2 sessions", "the library's own spelling");
    assert!(!saved.folder_created, "bug: the folder existed: {saved:?}");
    let seen = requests(&seen);
    assert_eq!(seen.len(), 2, "no folder create: {seen:?}");
    assert_eq!(
        seen[1].2["folder_id"], 9,
        "bug: into the existing folder: {seen:?}"
    );
}

#[tokio::test]
async fn the_same_workout_already_in_the_folder_is_not_written_again() {
    // What the first save sent is what the folder now lists: a second save
    // finds it and writes nothing, so asking twice never doubles a template.
    let first = Seen::default();
    let base = spawn_library(json!([]), Arc::clone(&first)).await;
    coach_provider(base)
        .await
        .save_library_workout(&session(), "Dravr")
        .await
        .expect("the first save lands");
    let sent = requests(&first)[2].2.clone();

    let second = Seen::default();
    let listing = json!([{
        "id": 31,
        "name": "Dravr",
        "type": "FOLDER",
        "children": [
            { "id": 12, "name": "VO2 30/30", "description": "- 10m Z1" },
            { "id": 77, "name": sent["name"], "description": sent["description"] },
        ],
    }]);
    let base = spawn_library(listing, Arc::clone(&second)).await;
    let saved = coach_provider(base)
        .await
        .save_library_workout(&session(), "Dravr")
        .await
        .expect("the re-save answers");

    assert!(
        saved.already_saved,
        "bug: the folder holds this workout: {saved:?}"
    );
    assert_eq!(
        saved.workout_id, "77",
        "the namesake with other steps is not it"
    );
    let seen = requests(&second);
    assert_eq!(
        seen.iter().map(|(m, _, _)| m.as_str()).collect::<Vec<_>>(),
        ["GET"],
        "the re-save only lists"
    );
}

#[tokio::test]
async fn a_provider_built_for_a_coached_athlete_never_writes_a_library() {
    let seen = Seen::default();
    let base = spawn_library(json!([]), Arc::clone(&seen)).await;
    let mut registry = offline_provider_registry();
    let mut config = offline_provider_config("intervals_icu").unwrap();
    config.api_base_url = base;
    registry.set_default_config("intervals_icu", config);
    let delegated = registry
        .create_delegated_provider("intervals_icu", "i201", coach_credentials())
        .await
        .expect("a coach's key reads a coached athlete");

    let error = delegated
        .save_library_workout(&session(), "Dravr")
        .await
        .expect_err("an athlete's library is not the connected account's");

    assert!(error.message.contains("own"), "{}", error.message);
    assert!(requests(&seen).is_empty(), "refused before any request");
}

#[tokio::test]
async fn a_blank_folder_name_is_refused_before_any_request() {
    let seen = Seen::default();
    let base = spawn_library(json!([]), Arc::clone(&seen)).await;
    let error = coach_provider(base)
        .await
        .save_library_workout(&session(), "   ")
        .await
        .expect_err("a folder needs a name");
    assert!(error.message.contains("folder"), "{}", error.message);
    assert!(
        requests(&seen).is_empty(),
        "bug: refused before any request: {:?}",
        requests(&seen)
    );
}

#[tokio::test]
async fn an_oauth_grant_without_the_library_scope_is_told_to_reconnect() {
    let seen = Seen::default();
    let base = spawn_library(json!([]), Arc::clone(&seen)).await;
    let mut config = offline_provider_config("intervals_icu").unwrap();
    config.api_base_url = base;
    let provider = IntervalsIcuProvider::with_config(config);
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            access_token: Some(SecretString::from(GRANT_WITHOUT_LIBRARY.to_owned())),
            scopes: vec!["ACTIVITY:READ".to_owned(), "CALENDAR:WRITE".to_owned()],
            kind: CredentialKind::OAuthBearer,
            ..coach_credentials()
        })
        .await
        .expect("set creds");

    let error = provider
        .save_library_workout(&session(), "Dravr")
        .await
        .expect_err("Intervals.icu keeps this grant out of the library");

    assert_eq!(
        error.code,
        ErrorCode::PermissionDenied,
        "bug: a refusal of its own, never a broken connection: {}",
        error.message
    );
    assert!(
        error.message.contains("Reconnect"),
        "bug: the coach is told how to fix it: {}",
        error.message
    );
    let seen = requests(&seen);
    let calls: Vec<(&str, &str)> = seen
        .iter()
        .map(|(method, target, _)| (method.as_str(), target.as_str()))
        .collect();
    assert_eq!(
        calls,
        [("GET", "/api/v1/athlete/0/folders")],
        "bug: nothing is written after the listing is refused"
    );
}

#[test]
fn intervals_icu_declares_its_workout_library() {
    use pierre_providers::spi::ProviderDescriptor;

    assert!(
        IntervalsIcuDescriptor
            .capabilities()
            .supports_workout_library(),
        "bug: Intervals.icu keeps a library Dravr saves to"
    );
}
