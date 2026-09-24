// ABOUTME: Pins the planned-workout read end to end against a loopback stand-in for the scraper service
// ABOUTME: The wire request, the converted plan, the coach and roster refusals kept apart from a dead session, and the capability flag
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte planned-workout read contract.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `GET /api/planned-workouts` answers the plan, or one of the scraper's
//! refusals. The refusals of the athlete a read named are not a dead
//! session: a `401 session_expired` sends the athlete through a re-login,
//! while a coach account that named no athlete (`400 athlete_required`) or an
//! athlete off the roster (`403 athlete_not_accessible`) is the same after a
//! re-login. These pin that each lands as its own typed error, that the
//! TrainingPeaks provider reads and converts the plan, and that the Strava
//! and Garmin mirrors refuse the read rather than answering an empty
//! calendar.
//!
//! The scraper here is a loopback stand-in (a test double, per the repo's
//! mock rule): it answers by the `X-Session-Id` each seeded session sends and
//! records every request, so the test reads the bytes the client put on the
//! wire. The service side of the contract is pinned in dravr-sciotte's own
//! `planned_workouts_route_test` and `error_response_test`. The scraped
//! scenarios share one test because they share the process-wide
//! `DRAVR_SCIOTTE_REMOTE_URL`; the pure functions are tested on their own.

use std::env;
use std::sync::{Arc, Mutex};

use chrono::{NaiveDate, TimeZone, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_providers::core::{
    planned_workouts_unsupported, FitnessProvider, OAuth2Credentials, ProviderConfig,
    ProviderFactory,
};
use pierre_providers::errors::{AppError, ErrorCode};
use pierre_providers::models::{SportType, WorkoutStep};
use pierre_providers::registry::global_registry;
use pierre_providers::sciotte_provider::{
    SciotteGarminProviderFactory, SciotteProviderFactory, SciotteTarget,
    SciotteTrainingPeaksProviderFactory,
};
use pierre_providers::sciotte_remote::{
    athlete_refusal_error, sciotte_refusal, AthleteId, RemoteActivityQuery, RemoteSciotteClient,
    ATHLETE_NOT_ACCESSIBLE, ATHLETE_REQUIRED, ENV_AUDIENCE, ENV_REMOTE_URL,
};
use pierre_providers::spi::{
    ProviderCapabilities, ProviderDescriptor, SciotteDescriptor, SciotteGarminDescriptor,
    SciotteTrainingPeaksDescriptor,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A TrainingPeaks athlete account whose plan the stand-in serves.
const ATHLETE_SESSION: &str = "tp-athlete";
/// A TrainingPeaks coach account: no calendar of its own.
const COACH_SESSION: &str = "tp-coach";
/// A session whose named athlete is off the roster.
const ROSTER_SESSION: &str = "tp-roster";
/// A session the provider no longer honours.
const DEAD_SESSION: &str = "tp-dead";
/// A session the stand-in answers `invalid_window` for.
const MALFORMED_SESSION: &str = "tp-malformed";
/// A session whose body announces more rows than it carries.
const TRUNCATED_SESSION: &str = "tp-truncated";
/// A Strava mirror session, which has no planned read.
const STRAVA_SESSION: &str = "strava-session";

/// Two planned workouts as the service puts them on the wire: a structured
/// ride and a day off.
fn planned_body() -> Value {
    json!({
        "count": 2,
        "planned_workouts": [
            {
                "id": "900001:910004",
                "date": "2026-09-28",
                "sport_type": "ride",
                "title": "Threshold 3x10",
                "description": "Aim for 5 more watts per block.",
                "planned_duration_seconds": 3600,
                "planned_training_stress_score": 78.3,
                "planned_intensity_factor": 0.85,
                "structure": {
                    "intensity_metric": "percent_of_ftp",
                    "target_kind": "range",
                    "length_metric": "duration",
                    "blocks": [
                        { "repeat": 1, "steps": [
                            { "name": "Warm up", "intensity_class": "warm_up",
                              "duration_seconds": 600, "target_min": 45.0, "target_max": 55.0 }
                        ]},
                        { "repeat": 3, "steps": [
                            { "name": "On", "intensity_class": "active",
                              "duration_seconds": 600, "target_min": 95.0, "target_max": 100.0 },
                            { "name": "Off", "intensity_class": "rest",
                              "duration_seconds": 300, "target_min": 50.0, "target_max": 55.0 }
                        ]}
                    ]
                }
            },
            {
                "id": "900001:910007",
                "date": "2026-10-01",
                "sport_type": { "other": "Day Off" },
                "title": "Day off",
                "description": "Rest. Sleep in if you can.",
                "completed_activity_id": "900001:910007"
            }
        ]
    })
}

/// The status and body the stand-in answers a planned read on `session`.
fn planned_answer(session: &str) -> (u16, Value) {
    match session {
        ATHLETE_SESSION => (200, planned_body()),
        ROSTER_SESSION => (
            403,
            json!({ "error": ATHLETE_NOT_ACCESSIBLE, "athlete": "900002" }),
        ),
        COACH_SESSION => (
            400,
            json!({ "error": ATHLETE_REQUIRED, "message": "coach accounts must name an athlete" }),
        ),
        DEAD_SESSION => (401, json!({ "error": "session_expired" })),
        MALFORMED_SESSION => (
            400,
            json!({ "error": "invalid_window", "message": "`before` precedes `after`" }),
        ),
        TRUNCATED_SESSION => {
            let mut body = planned_body();
            body["count"] = json!(3);
            (200, body)
        }
        _ => (404, json!({ "error": "session_not_found" })),
    }
}

/// The status and body the stand-in answers an activity list on `session`.
fn activities_answer(session: &str) -> (u16, Value) {
    if session == COACH_SESSION {
        return (400, json!({ "error": ATHLETE_REQUIRED }));
    }
    (
        200,
        json!({ "count": 0, "activities": [], "head_complete": true }),
    )
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

/// The `X-Session-Id` a request carried.
fn session_of(request: &str) -> String {
    request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("x-session-id")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default()
}

/// Serve the import, the planned read and the activity list, recording every
/// request. Any other path answers 404, so an unexpected call fails the test.
fn spawn_scraper_stub(listener: TcpListener, seen: Arc<Mutex<Vec<String>>>) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let request = read_request(&mut stream).await;
            seen.lock().unwrap().push(request.clone());
            let target = request
                .lines()
                .next()
                .and_then(|line| line.split(' ').nth(1))
                .unwrap_or_default()
                .to_owned();
            let path = target.split('?').next().unwrap_or_default();
            let session = session_of(&request);
            let (status, body) = match path {
                "/auth/import-session" => (200, json!({ "session_id": "stub-session" })),
                "/api/planned-workouts" => planned_answer(&session),
                "/api/activities" => activities_answer(&session),
                _ => (404, json!({ "error": "not_served" })),
            };
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
}

/// A provider built by `factory` under `name`, holding session `session_id`.
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
        created_at: Utc.with_ymd_and_hms(2026, 9, 23, 8, 0, 0).unwrap(), // Safe: literal instant
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

fn athlete(id: &str) -> AthleteId {
    id.parse().expect("a numeric athlete id is well formed") // Safe: callers pass digit literals
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date literal") // Safe: literal calendar date
}

/// The request line of every recorded request to `path` from `session`.
fn request_lines(seen: &Mutex<Vec<String>>, path: &str, session: &str) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|r| session_of(r) == session)
        .filter_map(|r| r.lines().next().map(str::to_owned))
        .filter(|line| line.contains(path))
        .collect()
}

async fn the_client_names_the_window_and_the_athlete(
    remote: &RemoteSciotteClient,
    seen: &Mutex<Vec<String>>,
) {
    let planned = remote
        .get_planned_workouts(
            ATHLETE_SESSION,
            day(2026, 9, 28),
            day(2026, 10, 4),
            Some(&athlete("900001")),
        )
        .await
        .expect("the stand-in serves the plan"); // Safe: ATHLETE_SESSION answers 200
    assert_eq!(planned.len(), 2);
    assert_eq!(planned[0].id, "900001:910004");
    assert_eq!(
        request_lines(seen, "/api/planned-workouts", ATHLETE_SESSION),
        vec![
            "GET /api/planned-workouts?after=2026-09-28&before=2026-10-04&athlete=900001 HTTP/1.1"
                .to_owned()
        ]
    );
}

async fn the_activity_query_sends_its_athlete(
    remote: &RemoteSciotteClient,
    seen: &Mutex<Vec<String>>,
) {
    let query = RemoteActivityQuery {
        limit: Some(5),
        athlete: Some(athlete("900001")),
        ..RemoteActivityQuery::default()
    };
    remote
        .get_activities(STRAVA_SESSION, &query)
        .await
        .expect("the stand-in serves an empty list"); // Safe: any non-coach session answers 200
    let lines = request_lines(seen, "/api/activities", STRAVA_SESSION);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("athlete=900001"),
        "the athlete rides on the list query: {lines:?}"
    );
}

/// The scraper names a detail pass by its scope (`every` or
/// `missing_location`) and refuses anything else with a 400, so a detail pass
/// asked for as `detail=true` failed the whole list read.
async fn the_detail_pass_is_asked_for_by_its_scope(
    remote: &RemoteSciotteClient,
    seen: &Mutex<Vec<String>>,
) {
    let query = RemoteActivityQuery {
        limit: Some(5),
        enrich_details: true,
        ..RemoteActivityQuery::default()
    };
    remote
        .get_activities(STRAVA_SESSION, &query)
        .await
        .expect("the stand-in serves an empty list"); // Safe: any non-coach session answers 200
    let lines: Vec<String> = request_lines(seen, "/api/activities", STRAVA_SESSION)
        .into_iter()
        .filter(|line| line.contains("detail="))
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("detail=every"),
        "the pass names its scope: {lines:?}"
    );
    assert!(!lines[0].contains("detail=true"), "{lines:?}");
}

async fn the_trainingpeaks_provider_reads_and_converts_the_plan(seen: &Mutex<Vec<String>>) {
    let trainingpeaks = connected(
        &SciotteTrainingPeaksProviderFactory,
        "sciotte_trainingpeaks",
        ATHLETE_SESSION,
    )
    .await;
    // Reset what the direct client call recorded for this session, so the
    // provider's own request is the one read below.
    seen.lock()
        .unwrap()
        .retain(|r| session_of(r) != ATHLETE_SESSION);

    let plan = trainingpeaks
        .list_planned_workouts(day(2026, 9, 28), day(2026, 10, 4))
        .await
        .expect("the TrainingPeaks provider reads the plan"); // Safe: ATHLETE_SESSION answers 200

    assert_eq!(plan.len(), 2);
    let ride = &plan[0];
    assert_eq!(ride.provider(), "trainingpeaks");
    assert_eq!(ride.provider_workout_id(), "900001:910004");
    assert_eq!(ride.sport_type(), &SportType::Ride);
    assert_eq!(ride.planned_duration_seconds(), Some(3600));
    assert_eq!(ride.description(), Some("Aim for 5 more watts per block."));
    let target = |label: &str, seconds: u32, zone: &str, repeat: u32, group: u32| WorkoutStep {
        label: label.to_owned(),
        duration_seconds: seconds,
        distance_meters: None,
        target_zone: zone.to_owned(),
        repeat,
        repeat_group: Some(group),
        note: None,
    };
    assert_eq!(
        ride.steps(),
        &[
            target("Warm up", 600, "45-55% FTP", 1, 1),
            target("On", 600, "95-100% FTP", 3, 2),
            target("Off", 300, "50-55% FTP", 3, 2),
        ][..]
    );
    let day_off = &plan[1];
    assert_eq!(
        day_off.sport_type(),
        &SportType::Other("Day Off".to_owned())
    );
    assert!(day_off.steps().is_empty());
    assert_eq!(day_off.completed_activity_id(), Some("900001:910007"));

    // The platform reads the signed-in account's own calendar: no athlete.
    assert_eq!(
        request_lines(seen, "/api/planned-workouts", ATHLETE_SESSION),
        vec!["GET /api/planned-workouts?after=2026-09-28&before=2026-10-04 HTTP/1.1".to_owned()]
    );
    let import = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.contains("/auth/import-session") && r.contains(ATHLETE_SESSION))
        .cloned()
        .expect("the session was imported before the read"); // Safe: asserted just above that the read ran
    assert!(import.contains(r#""provider":"trainingpeaks""#), "{import}");
}

async fn a_coach_account_is_told_it_has_no_calendar_not_sent_to_log_in() {
    let coach = connected(
        &SciotteTrainingPeaksProviderFactory,
        "sciotte_trainingpeaks",
        COACH_SESSION,
    )
    .await;
    for (surface, error) in [
        (
            "planned read",
            coach
                .list_planned_workouts(day(2026, 9, 28), day(2026, 10, 4))
                .await
                .expect_err("a coach account has no plan of its own"), // Safe: COACH_SESSION answers 400
        ),
        (
            "activity list",
            coach
                .get_activities(Some(5), None)
                .await
                .expect_err("a coach account has no activities of its own"), // Safe: COACH_SESSION answers 400
        ),
    ] {
        assert_eq!(error.code, ErrorCode::InvalidInput, "{surface}: {error:?}");
        assert_eq!(
            error.provider_auth_required_provider(),
            None,
            "{surface}: a re-login cannot turn a coach account into an athlete's"
        );
        assert_eq!(sciotte_refusal(&error), Some(ATHLETE_REQUIRED), "{surface}");
        assert_eq!(
            error.sanitized_message(),
            "This TrainingPeaks account is a coach account. TrainingPeaks keeps each \
             athlete's plan and training on that athlete's own calendar, and a coach \
             account has none of its own. To read an athlete's TrainingPeaks workouts, \
             link each athlete from a group you coach; the athlete confirms the link.",
            "{surface}: the athlete reads the refusal as written"
        );
        assert_eq!(
            error.sanitized_message(),
            SciotteTarget::TrainingPeaks.coach_account_refusal(),
            "{surface}: one text for the refusal, whichever path answers it"
        );
    }
}

async fn an_athlete_off_the_roster_is_a_permission_refusal(remote: &RemoteSciotteClient) {
    let error = remote
        .get_planned_workouts(
            ROSTER_SESSION,
            day(2026, 9, 28),
            day(2026, 10, 4),
            Some(&athlete("900002")),
        )
        .await
        .expect_err("the roster refusal is an error"); // Safe: ROSTER_SESSION answers 403
    assert_eq!(error.code, ErrorCode::PermissionDenied, "{error:?}");
    assert_eq!(sciotte_refusal(&error), Some(ATHLETE_NOT_ACCESSIBLE));
    assert_eq!(error.provider_auth_required_provider(), None);
    assert_eq!(
        error.sanitized_message(),
        "That athlete is not on this coach account's roster"
    );
}

async fn a_dead_session_still_asks_for_a_reconnect() {
    let dead = connected(
        &SciotteTrainingPeaksProviderFactory,
        "sciotte_trainingpeaks",
        DEAD_SESSION,
    )
    .await;
    let error = dead
        .list_planned_workouts(day(2026, 9, 28), day(2026, 10, 4))
        .await
        .expect_err("a dead session cannot read"); // Safe: DEAD_SESSION answers 401
    assert_eq!(error.code, ErrorCode::ProviderAuthRequired, "{error:?}");
    assert_eq!(
        error.provider_auth_required_provider().as_deref(),
        Some("sciotte_trainingpeaks"),
        "the reconnect link must open the TrainingPeaks login"
    );
    assert_eq!(sciotte_refusal(&error), None);
}

async fn a_query_the_platform_built_wrong_is_an_internal_fault(remote: &RemoteSciotteClient) {
    for session in [MALFORMED_SESSION, TRUNCATED_SESSION] {
        let error = remote
            .get_planned_workouts(session, day(2026, 9, 28), day(2026, 10, 4), None)
            .await
            .expect_err("neither body is a plan"); // Safe: both sessions answer a refusal or a mangled body
        assert_eq!(error.code, ErrorCode::InternalError, "{session}: {error:?}");
        assert_eq!(sciotte_refusal(&error), None, "{session}");
        assert_eq!(error.provider_auth_required_provider(), None, "{session}");
    }
}

async fn the_strava_and_garmin_mirrors_refuse_the_read(seen: &Mutex<Vec<String>>) {
    let strava = connected(&SciotteProviderFactory, "sciotte", STRAVA_SESSION).await;
    let garmin = connected(
        &SciotteGarminProviderFactory,
        "sciotte_garmin",
        STRAVA_SESSION,
    )
    .await;
    for (provider, name) in [(strava, "strava"), (garmin, "garmin")] {
        let error = provider
            .list_planned_workouts(day(2026, 9, 28), day(2026, 10, 4))
            .await
            .expect_err("no planned read on this mirror"); // Safe: only TrainingPeaks reads a plan
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(
            error.sanitized_message(),
            format!("{name} does not expose planned workouts")
        );
    }
    assert!(
        request_lines(seen, "/api/planned-workouts", STRAVA_SESSION).is_empty(),
        "a mirror without the capability never asks the scraper"
    );
}

#[tokio::test]
async fn the_planned_read_reaches_the_scraper_and_keeps_its_refusals_apart() {
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
    let remote = RemoteSciotteClient::require_from_env().expect("a loopback URL builds a client"); // Safe: URL set just above

    the_client_names_the_window_and_the_athlete(&remote, &seen).await;
    the_activity_query_sends_its_athlete(&remote, &seen).await;
    the_detail_pass_is_asked_for_by_its_scope(&remote, &seen).await;
    the_trainingpeaks_provider_reads_and_converts_the_plan(&seen).await;
    a_coach_account_is_told_it_has_no_calendar_not_sent_to_log_in().await;
    an_athlete_off_the_roster_is_a_permission_refusal(&remote).await;
    a_dead_session_still_asks_for_a_reconnect().await;
    a_query_the_platform_built_wrong_is_an_internal_fault(&remote).await;
    the_strava_and_garmin_mirrors_refuse_the_read(&seen).await;

    env::remove_var(ENV_REMOTE_URL);
}

#[test]
fn each_athlete_refusal_is_classified_by_status_and_marker() {
    let required = athlete_refusal_error(
        StatusCode::BAD_REQUEST,
        &json!({ "error": ATHLETE_REQUIRED }),
    )
    .expect("a 400 athlete_required is a refusal"); // Safe: the marker is the one classified
    assert_eq!(required.code, ErrorCode::InvalidInput);
    assert_eq!(sciotte_refusal(&required), Some(ATHLETE_REQUIRED));

    let off_roster = athlete_refusal_error(
        StatusCode::FORBIDDEN,
        &json!({ "error": ATHLETE_NOT_ACCESSIBLE, "athlete": "900002" }),
    )
    .expect("a 403 athlete_not_accessible is a refusal"); // Safe: the marker is the one classified
    assert_eq!(off_roster.code, ErrorCode::PermissionDenied);
    assert_eq!(sciotte_refusal(&off_roster), Some(ATHLETE_NOT_ACCESSIBLE));

    for marker in ["invalid_athlete", "invalid_window"] {
        let malformed = athlete_refusal_error(StatusCode::BAD_REQUEST, &json!({ "error": marker }))
            .expect("a malformed query is classified"); // Safe: both markers are classified
        assert_eq!(malformed.code, ErrorCode::InternalError, "{marker}");
        assert_eq!(sciotte_refusal(&malformed), None, "{marker}");
    }
}

#[test]
fn a_marker_under_the_wrong_status_or_an_unknown_one_is_not_a_refusal() {
    // The markers are read with their status: a 500 that happens to carry the
    // word is still a scraper fault.
    for (status, marker) in [
        (StatusCode::INTERNAL_SERVER_ERROR, ATHLETE_REQUIRED),
        (StatusCode::BAD_REQUEST, ATHLETE_NOT_ACCESSIBLE),
        (StatusCode::UNAUTHORIZED, "session_expired"),
        (StatusCode::BAD_REQUEST, "something_else"),
    ] {
        assert!(
            athlete_refusal_error(status, &json!({ "error": marker })).is_none(),
            "{status} {marker}"
        );
    }
    assert!(athlete_refusal_error(StatusCode::BAD_REQUEST, &json!({})).is_none());
    assert_eq!(sciotte_refusal(&AppError::internal("plain")), None);
}

#[test]
fn only_the_trainingpeaks_mirror_declares_a_planned_calendar() {
    let trainingpeaks = SciotteTrainingPeaksDescriptor.capabilities();
    assert!(trainingpeaks.supports_planned_workouts());
    assert!(trainingpeaks.supports_activities());
    assert!(!trainingpeaks.contains(ProviderCapabilities::CHEAP_ACTIVITY_DETAIL));
    assert!(!SciotteDescriptor.capabilities().supports_planned_workouts());
    assert!(!SciotteGarminDescriptor
        .capabilities()
        .supports_planned_workouts());
    assert!(!ProviderCapabilities::full_health().supports_planned_workouts());
    assert!(!ProviderCapabilities::activity_only().supports_planned_workouts());
    assert_eq!(ProviderCapabilities::PLANNED_WORKOUTS.bits(), 0b1000_0000);

    assert_eq!(
        global_registry().planned_workout_providers(),
        vec!["sciotte_trainingpeaks"],
        "the registry lists exactly the providers that declare the capability"
    );
}

#[test]
fn the_refusal_of_a_provider_without_a_plan_names_it_as_the_athlete_knows_it() {
    for (backend, named) in [
        ("sciotte_garmin", "garmin"),
        ("sciotte", "strava"),
        ("whoop", "whoop"),
    ] {
        let error = planned_workouts_unsupported(backend);
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(
            error.sanitized_message(),
            format!("{named} does not expose planned workouts")
        );
    }
}
