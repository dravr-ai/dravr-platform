// ABOUTME: An Intervals.icu coach's API key reads a coached athlete by id, and lists the athletes it coaches
// ABOUTME: Pins the URL and credential a delegated read sends, the scope it refuses, and the two refusals a caller acts on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Delegated Intervals.icu reads, against a loopback stand-in for the API.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate, so without a surviving crate doc `missing_docs` trips.
#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{NaiveDate, TimeZone, Utc};
use pierre_providers::core::{
    ActivityQueryParams, CredentialKind, FitnessProvider, OAuth2Credentials,
};
use pierre_providers::delegation::{is_athlete_off_roster, is_coach_credential_expired};
use pierre_providers::errors::ErrorCode;
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::models::{PlannedSession, PlannedSessionKind, RosterAthlete, SportType};
use pierre_providers::ProviderRegistry;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The coach's own Intervals.icu athlete id.
const COACH_ID: &str = "i100";
/// The coach's API key, which the stand-in honours.
const COACH_KEY: &str = "coach-key";
/// A key Intervals.icu no longer honours.
const DEAD_KEY: &str = "dead-key";
/// An athlete who shares with the coach.
const ALEX: &str = "i201";
/// A second athlete who shares with the coach.
const SAM: &str = "i202";
/// An athlete who does not share with the coach.
const STRANGER: &str = "i999";

/// The request heads the stand-in answered, in order.
type Seen = Arc<Mutex<Vec<String>>>;

fn activity(id: &str, athlete: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"Easy run","type":"Run","start_date_local":"2026-09-20T07:00:00","elapsed_time":1800,"distance":5000.0,"icu_athlete_id":"{athlete}"}}"#
    )
}

/// What the stand-in answers `target` with, under the API key `key`.
fn answer(target: &str, key: &str) -> (&'static str, String) {
    if key != COACH_KEY {
        return ("401 Unauthorized", "{}".to_owned());
    }
    let path = target.split('?').next().unwrap_or_default();
    match path {
        "/api/v1/athletes" => (
            "200 OK",
            format!(
                r#"[{{"id":"{COACH_ID}","name":"Casey Coach","email":"coach@links.test"}},
                    {{"id":"{ALEX}","name":"Alex Athlete","email":"alex@links.test"}},
                    {{"id":"{SAM}","firstname":"Sam","lastname":"Swimmer"}}]"#
            ),
        ),
        "/api/v1/athlete/i201" => (
            "200 OK",
            format!(r#"{{"id":"{ALEX}","name":"Alex Athlete","email":"alex@links.test"}}"#),
        ),
        "/api/v1/athlete/i201/activities" => ("200 OK", format!("[{}]", activity("a1", ALEX))),
        "/api/v1/activity/a1" => ("200 OK", activity("a1", ALEX)),
        "/api/v1/activity/a1/messages" => ("200 OK", "[]".to_owned()),
        "/api/v1/activity/a2" => ("200 OK", activity("a2", SAM)),
        _ if path.starts_with("/api/v1/athlete/i999") => ("403 Forbidden", "{}".to_owned()),
        _ => ("404 Not Found", "{}".to_owned()),
    }
}

/// The API key a captured request head carries, from its HTTP Basic pair.
fn basic_pair(head: &str) -> String {
    let encoded = head
        .lines()
        .find_map(|line| {
            line.strip_prefix("authorization: Basic ")
                .or_else(|| line.strip_prefix("Authorization: Basic "))
        })
        .expect("the request carries HTTP Basic credentials");
    String::from_utf8(STANDARD.decode(encoded.trim()).expect("base64 pair")).expect("utf-8 pair")
}

fn target(head: &str) -> &str {
    head.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("request head carries a target")
}

/// A loopback stand-in for the Intervals.icu API that answers every request
/// by path and key, and records each request head.
async fn spawn_api(seen: Seen) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0_u8; 2048];
                loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    head.extend_from_slice(&buf[..n]);
                    if head.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let key = basic_pair(&head)
                    .strip_prefix("API_KEY:")
                    .unwrap_or_default()
                    .to_owned();
                let (status, body) = answer(target(&head), &key);
                seen.lock().unwrap().push(head);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    format!("http://{addr}")
}

/// A registry whose Intervals.icu calls go to `base_url`.
fn registry(base_url: String) -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    let mut config = default_config();
    config.api_base_url = base_url;
    registry.set_default_config("intervals_icu", config);
    registry
}

fn coach_credentials(key: &str) -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: COACH_ID.to_owned(),
        client_secret: String::new(),
        access_token: Some(key.to_owned()),
        refresh_token: None,
        expires_at: None,
        scopes: Vec::new(),
        kind: CredentialKind::ApiKey,
        request_budget: None,
    }
}

async fn delegated(
    registry: &ProviderRegistry,
    athlete: &str,
    key: &str,
) -> Box<dyn FitnessProvider> {
    registry
        .create_delegated_provider("intervals_icu", athlete, coach_credentials(key))
        .await
        .expect("Intervals.icu reads for a coached athlete through an API key")
}

fn recent() -> ActivityQueryParams {
    ActivityQueryParams {
        limit: Some(10),
        offset: None,
        before: Some(
            Utc.with_ymd_and_hms(2026, 9, 30, 0, 0, 0)
                .unwrap()
                .timestamp(),
        ),
        after: Some(
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0)
                .unwrap()
                .timestamp(),
        ),
    }
}

fn last_target(seen: &Seen) -> String {
    target(seen.lock().unwrap().last().expect("a request was sent")).to_owned()
}

#[tokio::test]
async fn a_roster_athlete_is_read_at_their_own_path_with_the_coachs_key() {
    let seen = Seen::default();
    let registry = registry(spawn_api(Arc::clone(&seen)).await);
    let provider = delegated(&registry, ALEX, COACH_KEY).await;

    let activities = provider
        .get_activities_with_params(&recent())
        .await
        .expect("the coach reads the athlete's activities");
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].id(), "a1");
    {
        let heads = seen.lock().unwrap();
        let head = heads.last().expect("the list was requested");
        assert!(
            target(head).starts_with("/api/v1/athlete/i201/activities?"),
            "{}",
            target(head)
        );
        assert_eq!(basic_pair(head), format!("API_KEY:{COACH_KEY}"));
    }

    let athlete = provider.get_athlete().await.expect("the athlete's profile");
    assert_eq!(athlete.id, ALEX);
    assert_eq!(last_target(&seen), "/api/v1/athlete/i201");

    let detail = provider
        .get_activity_detailed("a1")
        .await
        .expect("the athlete's own activity reads");
    assert_eq!(detail.id(), "a1");
}

#[tokio::test]
async fn another_athletes_activity_is_not_found_through_a_link() {
    let seen = Seen::default();
    let registry = registry(spawn_api(Arc::clone(&seen)).await);
    let provider = delegated(&registry, ALEX, COACH_KEY).await;

    // The coach's key reads Sam's activity too; Alex's link must not.
    for read in [
        provider.get_activity("a2").await,
        provider.get_activity_detailed("a2").await,
        provider.get_activity_with_streams("a2").await,
    ] {
        let error = read.expect_err("Sam's activity is outside Alex's link");
        assert_eq!(error.code, ErrorCode::ResourceNotFound, "{error}");
    }
    // Nothing past the activity itself was fetched: no comments, no streams.
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|head| target(head) == "/api/v1/activity/a2"));
}

#[tokio::test]
async fn an_athlete_who_does_not_share_with_the_coach_is_off_roster() {
    let registry = registry(spawn_api(Seen::default()).await);
    let provider = delegated(&registry, STRANGER, COACH_KEY).await;

    let error = provider
        .get_activities_with_params(&recent())
        .await
        .expect_err("Intervals.icu refuses an athlete who does not share");
    assert!(is_athlete_off_roster(&error), "{error:?}");
    assert!(!is_coach_credential_expired(&error));
}

#[tokio::test]
async fn a_dead_coach_key_is_the_coachs_to_renew_not_the_readers() {
    let registry = registry(spawn_api(Seen::default()).await);
    let provider = delegated(&registry, ALEX, DEAD_KEY).await;

    let error = provider
        .get_athlete()
        .await
        .expect_err("a dead key fails the read");
    assert!(is_coach_credential_expired(&error), "{error:?}");
    assert!(
        error.provider_auth_required_provider().is_none(),
        "the reader is never sent to reconnect: {error:?}"
    );
}

#[tokio::test]
async fn a_linked_athletes_calendar_is_never_written() {
    let seen = Seen::default();
    let registry = registry(spawn_api(Arc::clone(&seen)).await);
    let provider = delegated(&registry, ALEX, COACH_KEY).await;

    let deleted = provider
        .delete_planned_sessions(&["42".to_owned()])
        .await
        .expect_err("a delegated provider writes nothing");
    assert_eq!(deleted.code, ErrorCode::InvalidInput);
    let updated = provider
        .update_planned_session("42", &sample_session())
        .await
        .expect_err("a delegated provider writes nothing");
    assert_eq!(updated.code, ErrorCode::InvalidInput);
    let pushed = provider
        .push_planned_session(&sample_session())
        .await
        .expect_err("a delegated provider writes nothing");
    assert_eq!(pushed.code, ErrorCode::InvalidInput);
    assert!(seen.lock().unwrap().is_empty(), "nothing reached the API");
}

fn sample_session() -> PlannedSession {
    PlannedSession {
        external_id: "dravr:rx:delegated".to_owned(),
        kind: PlannedSessionKind::Workout,
        date: NaiveDate::from_ymd_opt(2026, 10, 1).expect("valid date"),
        sport: SportType::Run,
        name: "Easy run".to_owned(),
        duration_seconds: Some(1800),
        notes: String::new(),
        steps: Vec::new(),
    }
}

#[tokio::test]
async fn an_id_intervals_icu_could_not_have_issued_is_refused_before_any_read() {
    let registry = registry("http://127.0.0.1:9".to_owned());
    for refused in ["", "i1/../i2", "i1?athlete=i2", "i 1"] {
        let Err(error) = registry
            .create_delegated_provider("intervals_icu", refused, coach_credentials(COACH_KEY))
            .await
        else {
            panic!("{refused:?} must be refused");
        };
        assert_eq!(error.code, ErrorCode::InvalidInput, "{refused:?}");
        assert!(registry
            .check_delegated_athlete("intervals_icu", refused)
            .is_err());
    }
    assert!(registry
        .check_delegated_athlete("intervals_icu", ALEX)
        .is_ok());
}

#[tokio::test]
async fn a_coach_key_lists_the_athletes_it_coaches_and_its_own_email() {
    let seen = Seen::default();
    let registry = registry(spawn_api(Arc::clone(&seen)).await);
    let provider = registry
        .create_provider("intervals_icu")
        .expect("Intervals.icu is registered");
    provider
        .set_credentials(coach_credentials(COACH_KEY))
        .await
        .expect("an API key credential is accepted");

    let roster = provider
        .read_coach_roster()
        .await
        .expect("the coach's roster reads");
    assert_eq!(roster.account_email.as_deref(), Some("coach@links.test"));
    assert_eq!(
        roster.athletes,
        vec![
            RosterAthlete {
                id: ALEX.to_owned(),
                name: Some("Alex Athlete".to_owned()),
                email: Some("alex@links.test".to_owned()),
            },
            RosterAthlete {
                id: SAM.to_owned(),
                name: Some("Sam Swimmer".to_owned()),
                email: None,
            },
        ]
    );
    let head = seen.lock().unwrap().last().cloned().expect("one request");
    assert_eq!(target(&head), "/api/v1/athletes");
    assert_eq!(basic_pair(&head), format!("API_KEY:{COACH_KEY}"));
}

#[tokio::test]
async fn an_oauth_grant_cannot_list_a_coachs_athletes() {
    let seen = Seen::default();
    let registry = registry(spawn_api(Arc::clone(&seen)).await);
    let provider = registry
        .create_provider("intervals_icu")
        .expect("Intervals.icu is registered");
    let mut bearer = coach_credentials("oauth-token");
    bearer.kind = CredentialKind::OAuthBearer;
    provider
        .set_credentials(bearer)
        .await
        .expect("a bearer credential is accepted");

    let error = provider
        .read_coach_roster()
        .await
        .expect_err("Intervals.icu answers the roster for an API key only");
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(seen.lock().unwrap().is_empty(), "nothing reached the API");
}

#[tokio::test]
async fn a_provider_with_no_roster_says_so() {
    let registry = ProviderRegistry::new();
    let provider = registry
        .create_provider("strava")
        .expect("Strava is registered");
    let error = provider
        .read_coach_roster()
        .await
        .expect_err("Strava lists no coach's athletes");
    assert_eq!(error.code, ErrorCode::InvalidInput);
}
