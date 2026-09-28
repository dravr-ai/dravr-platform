// ABOUTME: Pins that the platform reads sciotte-server's HTTP contract from dravr_sciotte::wire, not a copy
// ABOUTME: Each wire marker is classified as the platform means it, and a body sciotte's own type wrote decodes as sent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte wire-contract client side.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `dravr_sciotte::wire` is the one declaration of what the scraper service
//! answers with: the server serializes its response types and writes its
//! `error` markers, and the platform deserializes the same types and matches
//! the same markers. These tests drive the platform's classifiers with the
//! markers sciotte exports and its client with a body sciotte's own
//! `ActivitiesResponse` serialized, so a marker or field renamed upstream
//! reaches the platform on the dependency bump instead of drifting.
//!
//! The scraper here is a loopback stand-in (a test double, per the repo's mock
//! rule) that answers `/api/activities` by the `X-Session-Id` a read sends.

use std::env;

use dravr_sciotte::models::{Activity, ActivityList};
use dravr_sciotte::wire::{
    ActivitiesResponse, ATHLETE_NOT_ACCESSIBLE, ATHLETE_REQUIRED, INVALID_ATHLETE, INVALID_WINDOW,
    SCRAPER_BUSY, SESSION_EXPIRED, SESSION_NOT_FOUND,
};
use pierre_providers::errors::ErrorCode;
use pierre_providers::sciotte_remote::{
    athlete_refusal_error, auth_required_error, backpressure_error, sciotte_refusal,
    RemoteActivityQuery, RemoteSciotteClient, ENV_AUDIENCE, ENV_REMOTE_URL,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A session whose list read the stand-in answers with a capture that missed
/// the list's head, serialized by sciotte's own response type.
const HEAD_MISSED_SESSION: &str = "head-missed";
/// A session whose list read the stand-in answers with a body that does not
/// say whether it read the head.
const UNSTATED_HEAD_SESSION: &str = "unstated-head";

fn activity(id: &str) -> Activity {
    serde_json::from_value(json!({
        "id": id,
        "name": "Morning run",
        "sport_type": "run",
        "start_date": "2026-09-20T07:00:00Z",
        "duration_seconds": 3600,
        "provider": "strava"
    }))
    .expect("a minimal activity") // Safe: literal activity with every required field
}

#[test]
fn every_wire_marker_is_classified_as_the_platform_means_it() {
    let shed = backpressure_error(StatusCode::OK, &json!({ "error": SCRAPER_BUSY }))
        .expect("the shed marker is backpressure whatever the status");
    assert_eq!(shed.code, ErrorCode::ResourceUnavailable);

    for marker in [SESSION_NOT_FOUND, SESSION_EXPIRED] {
        let dead = auth_required_error(StatusCode::UNAUTHORIZED, &json!({ "error": marker }))
            .unwrap_or_else(|| panic!("a 401 {marker} asks the athlete to sign in again"));
        assert_eq!(dead.code, ErrorCode::ProviderAuthRequired, "{marker}");
    }

    let required = athlete_refusal_error(
        StatusCode::BAD_REQUEST,
        &json!({ "error": ATHLETE_REQUIRED }),
    )
    .expect("a 400 athlete_required is a refusal");
    assert_eq!(required.code, ErrorCode::InvalidInput);
    assert_eq!(sciotte_refusal(&required), Some(ATHLETE_REQUIRED));

    let off_roster = athlete_refusal_error(
        StatusCode::FORBIDDEN,
        &json!({ "error": ATHLETE_NOT_ACCESSIBLE }),
    )
    .expect("a 403 athlete_not_accessible is a refusal");
    assert_eq!(off_roster.code, ErrorCode::PermissionDenied);
    assert_eq!(sciotte_refusal(&off_roster), Some(ATHLETE_NOT_ACCESSIBLE));

    for marker in [INVALID_ATHLETE, INVALID_WINDOW] {
        let malformed = athlete_refusal_error(StatusCode::BAD_REQUEST, &json!({ "error": marker }))
            .unwrap_or_else(|| panic!("a 400 {marker} is classified"));
        assert_eq!(malformed.code, ErrorCode::InternalError, "{marker}");
        assert_eq!(sciotte_refusal(&malformed), None, "{marker}");
    }
}

/// The body the stand-in answers a list read of `session` with.
fn activities_body(session: &str) -> Value {
    match session {
        HEAD_MISSED_SESSION => serde_json::to_value(ActivitiesResponse::from(ActivityList {
            activities: vec![activity("a1")],
            head_complete: false,
        }))
        .expect("sciotte's response type serializes"),
        UNSTATED_HEAD_SESSION => json!({ "count": 1, "activities": [activity("a1")] }),
        _ => json!({ "error": SESSION_NOT_FOUND }),
    }
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

fn spawn_scraper_stub(listener: TcpListener) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut chunk = [0_u8; 8192];
            let n = stream.read(&mut chunk).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&chunk[..n]).into_owned();
            let body = activities_body(&session_of(&request)).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
}

#[tokio::test]
async fn the_client_reads_the_body_sciottes_own_type_wrote() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    spawn_scraper_stub(listener);
    env::remove_var(ENV_AUDIENCE);
    env::set_var(ENV_REMOTE_URL, &base);
    let remote = RemoteSciotteClient::require_from_env().expect("loopback needs no audience");

    let list = remote
        .get_activities(HEAD_MISSED_SESSION, &RemoteActivityQuery::default())
        .await
        .expect("a body sciotte's server type wrote decodes");
    let ids: Vec<&str> = list.activities.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["a1"]);
    assert_eq!(list.count, 1);
    assert!(
        !list.head_complete,
        "a capture that missed the list's head is read as one"
    );

    // A body that does not say whether it read the head is not taken as
    // complete: assuming so is the masking that lost activities (carnet#149).
    let unstated = remote
        .get_activities(UNSTATED_HEAD_SESSION, &RemoteActivityQuery::default())
        .await
        .expect_err("a body without head_complete is not the contract");
    assert_eq!(unstated.code, ErrorCode::InternalError);

    env::remove_var(ENV_REMOTE_URL);
}
