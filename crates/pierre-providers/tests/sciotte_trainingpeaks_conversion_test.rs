// ABOUTME: Pins that a scraped TrainingPeaks workout reaches cageux with its load metrics and the athlete's self-report
// ABOUTME: Asserts TSS/IF/NP, feel and RPE in the platform's scales, the comment thread in order, and Strava rows left as they were
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte TrainingPeaks → cageux `Activity` conversion contract.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! TrainingPeaks is the one scraped source whose rows carry the coach's load
//! metrics — Training Stress Score, Intensity Factor and Normalized Power —
//! on every workout, and the athlete's own self-report: the "How did you
//! feel?" face, an RPE, and the notes and thread under the workout. This
//! binary drives the providers against a loopback scraper stub and pins that
//! all of it reaches the cageux `Activity` the analytics read — the feel as a
//! named rating (TrainingPeaks ranks it with 1 as the best), the RPE as a
//! CR-10 number, the comments in order with their authors and times — and
//! that a Strava row's self-report comes out exactly as it did before.
//!
//! The scraped scenarios live in one test because they share the process-wide
//! `DRAVR_SCIOTTE_REMOTE_URL`; separate `#[tokio::test]`s in this binary would
//! race. The pure mapping functions are tested on their own below.

use std::env;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory};
use pierre_providers::models::{ActivityComment, Feel};
use pierre_providers::sciotte_provider::{
    SciotteProviderFactory, SciotteTrainingPeaksProviderFactory,
};
use pierre_providers::sciotte_remote::{ENV_AUDIENCE, ENV_REMOTE_URL};
use pierre_providers::trainingpeaks_self_report::{
    feel_from_trainingpeaks, rpe_from_trainingpeaks,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A TrainingPeaks id is `athleteId:workoutId`: the API enforces the athlete
/// segment, so the workout id alone cannot address it. This one carries the
/// load metrics.
const LOAD_WORKOUT_ID: &str = "6642427:3301977";

/// A workout the athlete rated and the coach commented on.
const REPORTED_WORKOUT_ID: &str = "6642427:3301978";

/// A workout whose ratings sit outside the scales they claim.
const OUT_OF_RANGE_WORKOUT_ID: &str = "6642427:3301979";

/// A scraped Strava activity.
const STRAVA_ACTIVITY_ID: &str = "15523001";

/// Session ids, one per provider, so each import can be told apart.
const TRAININGPEAKS_SESSION: &str = "tp-session";
const STRAVA_SESSION: &str = "strava-session";

/// An activity the scraper would return, as the detail extract reports it.
fn scraped(id: &str, name: &str, sport: &str, provider: &str, extra: &Value) -> Value {
    let mut row = json!({
        "id": id,
        "name": name,
        "sport_type": sport,
        "start_date": "2026-09-18T11:30:00Z",
        "duration_seconds": 5_400,
        "provider": provider,
    });
    if let (Some(row), Some(extra)) = (row.as_object_mut(), extra.as_object()) {
        row.extend(extra.clone());
    }
    row
}

/// The body the stub answers `GET /api/activities/{id}` with, or `None` for
/// an id it does not serve.
fn activity_body(id: &str) -> Option<Value> {
    let body = match id {
        LOAD_WORKOUT_ID => scraped(
            id,
            "Sweet spot 3x15",
            "ride",
            "trainingpeaks",
            &json!({
                "distance_meters": 48_200.0,
                "average_heart_rate": 142,
                "max_heart_rate": 171,
                "average_power": 196,
                "max_power": 402,
                "normalized_power": 210,
                "average_cadence": 88,
                "training_stress_score": 88.1,
                "intensity_factor": 0.82,
            }),
        ),
        // The shape sciotte gives a rated workout: the two single-field
        // notes first, attributed by role and untimed, then the thread,
        // oldest first, under each commenter's display name.
        REPORTED_WORKOUT_ID => scraped(
            id,
            "Threshold 2x20",
            "ride",
            "trainingpeaks",
            &json!({
                "perceived_exertion": "6",
                "feel": 7,
                "comments": [
                    { "author": "coach", "text": "Hold cadence above 85 on both reps." },
                    { "author": "athlete", "text": "Legs heavy after the long week." },
                    {
                        "author": "Alex Athlete",
                        "text": "Second rep faded in the last 5 minutes.",
                        "created_at": "2026-09-18T14:05:00Z"
                    },
                    {
                        "author": "Coach Marie",
                        "text": "Fine — keep Thursday easy.",
                        "created_at": "2026-09-18T16:40:00Z"
                    }
                ]
            }),
        ),
        OUT_OF_RANGE_WORKOUT_ID => scraped(
            id,
            "Easy spin",
            "ride",
            "trainingpeaks",
            &json!({ "perceived_exertion": "11", "feel": 4 }),
        ),
        // A Strava extract's exertion is the slider's label. It is given a
        // feel rank here too, which no Strava extract produces, to show the
        // TrainingPeaks scale is never applied to another platform's rank.
        STRAVA_ACTIVITY_ID => scraped(
            id,
            "Morning Run",
            "run",
            "strava",
            &json!({ "perceived_exertion": "Hard", "feel": 1 }),
        ),
        _ => return None,
    };
    Some(body)
}

/// Read one whole HTTP request: the head, then as much body as its
/// `content-length` announces.
async fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&bytes);
        if let Some(head_end) = text.find("\r\n\r\n") {
            let announced = text[..head_end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if bytes.len() >= head_end + 4 + announced {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Serve `POST /auth/import-session` and `GET /api/activities/{id}` for the
/// ids [`activity_body`] knows, recording every request so the test can read
/// what the providers sent. An unknown path answers 404, so a request the
/// test did not expect fails it rather than reading as some other row.
fn spawn_scraper_stub(listener: TcpListener, seen: Arc<Mutex<Vec<String>>>) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let request = read_request(&mut stream).await;
            seen.lock().unwrap().push(request.clone());

            let request_line = request.lines().next().unwrap_or_default().to_owned();
            let path = request_line.split(' ').nth(1).unwrap_or_default();
            let body = if path == "/auth/import-session" {
                Some(json!({ "session_id": "stub-session" }))
            } else {
                path.strip_prefix("/api/activities/")
                    .map(|id| id.replace("%3A", ":"))
                    .and_then(|id| activity_body(&id))
            };
            let _ = stream.write_all(http_response(body).as_bytes()).await;
        }
    });
}

/// The response for a stub request: 200 carrying `body` as JSON, or 404 when
/// the stub serves nothing at that path.
fn http_response(body: Option<Value>) -> String {
    body.map_or_else(
        || "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned(),
        |body| {
            let body = body.to_string();
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
        },
    )
}

/// A provider built by `factory` under `name`, holding a session `session_id`.
async fn connected(
    factory: &dyn ProviderFactory,
    name: &str,
    session_id: &str,
) -> Box<dyn FitnessProvider> {
    let provider = factory
        .create(ProviderConfig {
            name: name.to_owned(),
            auth_url: String::new(),
            token_url: String::new(),
            api_base_url: String::new(),
            revoke_url: None,
            default_scopes: vec![],
        })
        .expect("sciotte provider construction is infallible"); // Safe: factory returns Ok unconditionally
    let session = AuthSession {
        session_id: session_id.to_owned(),
        cookies: vec![],
        created_at: Utc.with_ymd_and_hms(2026, 9, 18, 11, 0, 0).unwrap(), // Safe: literal calendar date is valid
        expires_at: None,
    };
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Some(serde_json::to_string(&session).expect("AuthSession serializes")), // Safe: plain data struct
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
        })
        .await
        .expect("a serialized session is accepted"); // Safe: the JSON above is a valid AuthSession
    provider
}

/// The import request that carried `session_id`.
fn import_for(seen: &Mutex<Vec<String>>, session_id: &str) -> String {
    seen.lock()
        .unwrap()
        .iter()
        .find(|r| r.contains("/auth/import-session") && r.contains(session_id))
        .cloned()
        .unwrap_or_else(|| panic!("no session import carried {session_id}"))
}

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 18, hour, minute, 0).unwrap() // Safe: literal calendar date is valid
}

async fn a_workout_keeps_its_load_metrics(
    trainingpeaks: &dyn FitnessProvider,
    seen: &Mutex<Vec<String>>,
) {
    let activity = trainingpeaks
        .get_activity(LOAD_WORKOUT_ID)
        .await
        .expect("the stub answers a parseable workout"); // Safe: the stub body is built from the same models

    assert_eq!(activity.training_stress_score(), Some(88.1));
    assert_eq!(activity.intensity_factor(), Some(0.82));
    assert_eq!(activity.normalized_power(), Some(210));
    assert_eq!(activity.average_power(), Some(196));
    assert_eq!(activity.max_power(), Some(402));
    assert_eq!(activity.average_heart_rate(), Some(142));
    assert_eq!(activity.duration_seconds(), 5_400);
    // A workout nobody rated carries no self-report.
    assert_eq!(activity.perceived_exertion(), None);
    assert_eq!(activity.feel(), None);
    assert_eq!(activity.comments(), None);

    let import = import_for(seen, TRAININGPEAKS_SESSION);
    assert!(
        import.contains(r#""provider":"trainingpeaks""#),
        "the session must be imported under the scraper's trainingpeaks provider, \
         or the service routes the scrape to Strava: {import}"
    );
}

async fn the_self_report_crosses_in_the_platform_scales(trainingpeaks: &dyn FitnessProvider) {
    let activity = trainingpeaks
        .get_activity(REPORTED_WORKOUT_ID)
        .await
        .expect("the stub answers a parseable workout"); // Safe: the stub body is built from the same models

    assert_eq!(activity.perceived_exertion(), Some(6.0));
    // TrainingPeaks' 7 is its fourth face of five, Weak — 1 is the best.
    assert_eq!(activity.feel(), Some(Feel::Poor));
    assert_eq!(
        activity.comments(),
        Some(
            &[
                ActivityComment {
                    author: Some("coach".to_owned()),
                    text: "Hold cadence above 85 on both reps.".to_owned(),
                    created_at: None,
                },
                ActivityComment {
                    author: Some("athlete".to_owned()),
                    text: "Legs heavy after the long week.".to_owned(),
                    created_at: None,
                },
                ActivityComment {
                    author: Some("Alex Athlete".to_owned()),
                    text: "Second rep faded in the last 5 minutes.".to_owned(),
                    created_at: Some(at(14, 5)),
                },
                ActivityComment {
                    author: Some("Coach Marie".to_owned()),
                    text: "Fine — keep Thursday easy.".to_owned(),
                    created_at: Some(at(16, 40)),
                },
            ][..]
        ),
        "every comment, in order, its author as the provider named it"
    );
    assert_eq!(
        activity.description(),
        None,
        "a workout's description is the coach's prescription, not the athlete's"
    );
}

async fn ratings_outside_their_scales_are_dropped(trainingpeaks: &dyn FitnessProvider) {
    let activity = trainingpeaks
        .get_activity(OUT_OF_RANGE_WORKOUT_ID)
        .await
        .expect("the stub answers a parseable workout"); // Safe: the stub body is built from the same models

    assert_eq!(activity.perceived_exertion(), None, "RPE 11 is past CR-10");
    assert_eq!(activity.feel(), None, "4 is not one of the five faces");
    assert_eq!(activity.comments(), None);
}

async fn a_strava_row_keeps_its_self_report_as_before(
    strava: &dyn FitnessProvider,
    seen: &Mutex<Vec<String>>,
) {
    let activity = strava
        .get_activity(STRAVA_ACTIVITY_ID)
        .await
        .expect("the stub answers a parseable activity"); // Safe: the stub body is built from the same models

    assert_eq!(
        activity.perceived_exertion(),
        None,
        "a Strava label is a band, not a CR-10 rating"
    );
    assert_eq!(
        activity.feel(),
        None,
        "the TrainingPeaks scale never reads another platform's rank"
    );
    assert_eq!(activity.comments(), None);

    let import = import_for(seen, STRAVA_SESSION);
    assert!(import.contains(r#""provider":"strava""#), "{import}");
}

#[tokio::test]
async fn scraped_rows_reach_cageux_with_their_metrics_and_self_report() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback"); // Safe: ephemeral port on loopback
    let addr = listener
        .local_addr()
        .expect("bound listener has an address"); // Safe: listener is bound
    let seen = Arc::new(Mutex::new(Vec::new()));
    spawn_scraper_stub(listener, seen.clone());

    env::set_var(ENV_REMOTE_URL, format!("http://{addr}"));
    env::remove_var(ENV_AUDIENCE);

    let trainingpeaks = connected(
        &SciotteTrainingPeaksProviderFactory,
        "sciotte_trainingpeaks",
        TRAININGPEAKS_SESSION,
    )
    .await;
    let strava = connected(&SciotteProviderFactory, "sciotte", STRAVA_SESSION).await;

    a_workout_keeps_its_load_metrics(trainingpeaks.as_ref(), &seen).await;
    the_self_report_crosses_in_the_platform_scales(trainingpeaks.as_ref()).await;
    ratings_outside_their_scales_are_dropped(trainingpeaks.as_ref()).await;
    a_strava_row_keeps_its_self_report_as_before(strava.as_ref(), &seen).await;

    env::remove_var(ENV_REMOTE_URL);
}

#[test]
fn each_trainingpeaks_face_maps_to_the_feel_at_its_position() {
    // 1 is TrainingPeaks' best face (Very Strong), 9 its worst (Very Weak).
    assert_eq!(feel_from_trainingpeaks(1), Some(Feel::Strong));
    assert_eq!(feel_from_trainingpeaks(3), Some(Feel::Good));
    assert_eq!(feel_from_trainingpeaks(5), Some(Feel::Normal));
    assert_eq!(feel_from_trainingpeaks(7), Some(Feel::Poor));
    assert_eq!(feel_from_trainingpeaks(9), Some(Feel::Weak));
}

#[test]
fn a_rank_that_is_not_a_face_is_not_a_feel() {
    for rank in [0, 2, 4, 6, 8, 10, 11, u8::MAX] {
        assert_eq!(feel_from_trainingpeaks(rank), None, "rank {rank}");
    }
}

#[test]
fn a_trainingpeaks_rpe_string_is_a_cr10_rating_inside_one_to_ten() {
    assert_eq!(rpe_from_trainingpeaks("6"), Some(6.0));
    assert_eq!(rpe_from_trainingpeaks("1"), Some(1.0));
    assert_eq!(rpe_from_trainingpeaks("10"), Some(10.0));
    assert_eq!(rpe_from_trainingpeaks(" 7 "), Some(7.0));
    assert_eq!(rpe_from_trainingpeaks("6.5"), Some(6.5));
}

#[test]
fn an_rpe_that_is_not_a_rating_on_the_scale_is_none() {
    for rpe in ["0", "11", "-3", "Hard", "Moderate", "", "NaN", "inf"] {
        assert_eq!(rpe_from_trainingpeaks(rpe), None, "{rpe:?}");
    }
}
