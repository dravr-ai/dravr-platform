// ABOUTME: The one-time notice to a member sharing with a human coach that the coach's AI tools read them, on the coach-sharing route
// ABOUTME: Due until the member answers in Dravr's app; keep or stop there records the answer, the same write over an API key does not

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! EFVP "AI Assistant Connections", decision 9.2b. AI tools a human coach
//! connects to Dravr read a member by name while the member shares their
//! training with the coach. Every member sharing with a human coach is told
//! once, whenever they joined, as the wellness widening is told
//! (`coach_consents_test`): `GET …/members/me/coach-sharing` carries
//! `assistant_notice_due` until the member answers. The answer is the PUT
//! from Dravr's own app, keep sharing or stop; the same PUT over an API key
//! changes the sharing and leaves the notice due.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_test_support::delegation::add_group_member;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::coach_athlete_reads_test::{fixture, person, Fixture, Person};
use crate::coach_consents_test::api_key;
use crate::common;

/// The coach-sharing route of the fixture's group.
fn sharing_path(f: &Fixture) -> String {
    format!("/api/groups/{}/members/me/coach-sharing", f.group)
}

/// `who`'s session in Dravr's app, as the `authorization` header carries it.
async fn app_session(f: &Fixture, who: Person) -> String {
    let user = f
        .repos()
        .users
        .get_global(who.id)
        .await
        .unwrap()
        .expect("the person has an account");
    format!(
        "Bearer {}",
        common::generate_test_token(&f.res, &user).await
    )
}

/// A `PUT` of coach sharing in the fixture's group under `authorization`.
async fn put_sharing(f: &Fixture, authorization: &str, consent: bool) -> (StatusCode, Value) {
    let app = ProviderToolRouter::build_http_app(&f.res);
    let request = Request::put(sharing_path(f))
        .header("authorization", authorization)
        .header("content-type", "application/json")
        .body(Body::from(json!({ "consent": consent }).to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// `who`'s first-party `GET` of their coach sharing in the fixture's group.
async fn get_sharing(f: &Fixture, who: Person) -> (StatusCode, Value) {
    f.get_as(who, &sharing_path(f)).await
}

/// Whether `who` is due the notice, read from the app.
async fn due(f: &Fixture, who: Person) -> Value {
    let (status, body) = get_sharing(f, who).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["assistant_notice_due"].clone()
}

#[tokio::test]
async fn a_member_who_joined_after_the_notice_landed_is_told_too() {
    let f = fixture().await;
    let other = person(&f.res, "other", "Otto Other").await;
    add_group_member(f.repos(), f.group, other.id, other.tenant)
        .await
        .unwrap();

    let (status, body) = get_sharing(&f, other).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["consent"], true, "joining grants the sharing: {body}");
    assert_eq!(
        body["assistant_notice_due"], true,
        "bug: a member who joined today is told of the coach's tools too: {body}"
    );
    assert_eq!(
        due(&f, f.athlete).await,
        true,
        "bug: the member the fixture enrolled is told too"
    );

    // The coach holds no membership: there is no sharing of theirs to read.
    let (status, _) = get_sharing(&f, f.coach).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "bug: the coach is never asked about their own tools"
    );
}

#[tokio::test]
async fn keeping_over_an_api_key_leaves_it_due_and_keeping_in_the_app_clears_it() {
    let f = fixture().await;
    let key = api_key(&f.res, f.athlete.id).await;

    let (status, body) = put_sharing(&f, &key, true).await;
    assert_eq!(status, StatusCode::OK, "the write works anywhere: {body}");
    assert_eq!(body["consent"], true, "{body}");
    assert_eq!(
        body["assistant_notice_due"], true,
        "bug: an answer from outside Dravr's app is no answer to the notice: {body}"
    );
    assert_eq!(
        due(&f, f.athlete).await,
        true,
        "bug: the API-key write left the notice due, so the app still shows it"
    );

    let session = app_session(&f, f.athlete).await;
    let (status, body) = put_sharing(&f, &session, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["consent"], true, "{body}");
    assert_eq!(
        body["assistant_notice_due"], false,
        "bug: keeping it in the app answers the notice: {body}"
    );
    assert_eq!(due(&f, f.athlete).await, false, "the notice stays gone");
}

#[tokio::test]
async fn stopping_in_the_app_ends_the_sharing_and_answers_the_notice() {
    let f = fixture().await;
    let session = app_session(&f, f.athlete).await;

    let (status, body) = put_sharing(&f, &session, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["consent"], false, "{body}");
    assert_eq!(
        body["assistant_notice_due"], false,
        "bug: nothing to keep once stopped: {body}"
    );

    // Resuming over an API key does not ask again: the member answered.
    let key = api_key(&f.res, f.athlete.id).await;
    let (status, body) = put_sharing(&f, &key, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["consent"], true, "{body}");
    assert_eq!(
        body["assistant_notice_due"], false,
        "bug: a stop in the app is the member's answer, once: {body}"
    );
}

#[tokio::test]
async fn a_group_with_no_human_coach_tells_nothing() {
    let f = fixture().await;
    let member = f
        .repos()
        .groups
        .get_member(&f.group.to_string(), f.athlete.id)
        .await
        .unwrap()
        .expect("a live member");
    assert!(
        member.coach_sharing_consent,
        "test setup: the member shares"
    );
    assert!(
        f.repos()
            .groups
            .set_group_coach_user(&f.group.to_string(), None, f.coach.tenant)
            .await
            .unwrap(),
        "test setup: the group's coach is detached"
    );

    let (status, body) = get_sharing(&f, f.athlete).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["assistant_notice_due"], false,
        "bug: a group with no human coach asks nothing: {body}"
    );
}
