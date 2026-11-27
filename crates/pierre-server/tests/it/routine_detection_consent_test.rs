// ABOUTME: The athlete's routine-detection consent: off by default, granted only in the app, every answer stamped
// ABOUTME: Pins the one-time card, "not now" clearing it, the API-key refusal and where the consent is not offered

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routines P3b (carnet#1792). A member may let their group's human coach be
//! told when their current break is longer than any in their own last eight
//! weeks while nothing is planned for them. Learning their pattern is
//! profiling (Law 25 art. 8.1), so the consent is off until granted, and a
//! grant is taken only from Dravr's own app; a revoke from anywhere. It is
//! offered where the group has a human coach other than the member, and a
//! sharing member is asked once: any answer, "not now" (a revoke while
//! already off) included, is stamped, and the stamp is what clears the card.

use std::sync::Arc;

use axum::middleware::from_fn;
use axum::Router;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::groups::CoachConsents;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::request_transport::declare_request_transport;
use pierre_routes_groups::DelegatedConnectionRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::coach_consents_test::api_key;
use crate::common::create_test_server_resources;
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::coach_roster_fixtures::{
    coach, coached_group, get, group_agent, join, person, Person,
};

/// A group whose human coach is `coach`, with `athlete` sharing their
/// training with them.
struct Squad {
    res: Arc<ServerContext>,
    router: Router,
    group: Uuid,
    coach: Person,
    athlete: Person,
}

async fn squad(label: &str) -> Squad {
    let res = create_test_server_resources().await.unwrap();
    let coach = coach(&res, &format!("coach-{label}@routine.test"), "Casey Coach").await;
    let athlete = person(
        &res,
        &format!("athlete-{label}@routine.test"),
        "Alex Athlete",
    )
    .await;
    let agent = group_agent(&res, &coach).await;
    let group = coached_group(&res, coach.tenant, coach.id, &agent, "Squad").await;
    join(&res, group, athlete.id, athlete.tenant, true).await;
    // Every request is served over the transport its credential names, as
    // the production app declares it.
    let router = DelegatedConnectionRoutes::routes(Arc::clone(&res))
        .layer(from_fn(declare_request_transport));
    Squad {
        res,
        router,
        group,
        coach,
        athlete,
    }
}

impl Squad {
    fn path(&self) -> String {
        format!("/api/groups/{}/members/me/coach-consents", self.group)
    }

    /// The caller's coach consents, as the apps read them.
    async fn read(&self, authorization: &str) -> Value {
        let (status, body) = get(&self.router, &self.path(), authorization).await;
        assert_eq!(
            status, 200,
            "bug: a member cannot read their consents: {body:#}"
        );
        body
    }

    /// The caller answers with `body`.
    async fn answer(&self, authorization: &str, body: &Value) -> (u16, Value) {
        let response = AxumTestRequest::put(&self.path())
            .header("authorization", authorization)
            .json(body)
            .send(self.router.clone())
            .await;
        let parsed = serde_json::from_str(&response.body_text()).unwrap_or(Value::Null);
        (response.status(), parsed)
    }

    /// `member`'s coach consents as stored on their membership.
    async fn stored(&self, member: Uuid) -> CoachConsents {
        self.res
            .common
            .repos
            .groups
            .get_member(&self.group.to_string(), member)
            .await
            .unwrap()
            .expect("a live member")
            .coach_consents
    }
}

#[tokio::test]
async fn routine_detection_is_off_by_default_and_asked_once_of_a_sharing_member() {
    let s = squad("default").await;
    let read = s.read(&s.athlete.auth).await;
    assert_eq!(
        read["routine_detection"], false,
        "bug: routine detection, profiling, is on before the member granted it: {read:#}"
    );
    assert_eq!(
        read["routine_detection_at"],
        Value::Null,
        "bug: an answer is recorded the member never gave: {read:#}"
    );
    assert_eq!(
        read["routine_detection_supported"], true,
        "bug: a group with a human coach other than the member does not offer it: {read:#}"
    );
    assert_eq!(
        read["routine_detection_notice_due"], true,
        "bug: a sharing member who never answered is not asked: {read:#}"
    );
    let stored = s.stored(s.athlete.id).await;
    assert!(
        !stored.routine_detection && stored.routine_detection_at.is_none(),
        "bug: a new membership stores routine detection on or answered: {stored:?}"
    );
}

#[tokio::test]
async fn a_grant_from_the_app_turns_routine_detection_on_and_clears_the_card() {
    let s = squad("grant").await;
    let (status, body) = s
        .answer(&s.athlete.auth, &json!({ "routine_detection": true }))
        .await;
    assert_eq!(status, 200, "bug: the app's grant was refused: {body:#}");
    assert_eq!(
        body["routine_detection"], true,
        "bug: the grant did not turn routine detection on: {body:#}"
    );
    assert!(
        body["routine_detection_at"].is_string(),
        "bug: the grant was not stamped: {body:#}"
    );
    assert_eq!(
        body["routine_detection_notice_due"], false,
        "bug: the card is still asked after a grant: {body:#}"
    );
    let stored = s.stored(s.athlete.id).await;
    assert!(
        stored.routine_detection && stored.routine_detection_at.is_some(),
        "bug: the grant was not stored with its stamp: {stored:?}"
    );
    assert!(
        !stored.assistant_read && stored.missed_session_check,
        "bug: granting routine detection moved another consent: {stored:?}"
    );
    let read = s.read(&s.athlete.auth).await;
    assert_eq!(
        (
            &read["routine_detection"],
            &read["routine_detection_notice_due"]
        ),
        (&json!(true), &json!(false)),
        "bug: a later read does not show the grant: {read:#}"
    );
}

#[tokio::test]
async fn not_now_stamps_the_answer_and_clears_the_card() {
    let s = squad("not-now").await;
    let (status, body) = s
        .answer(&s.athlete.auth, &json!({ "routine_detection": false }))
        .await;
    assert_eq!(status, 200, "bug: \"not now\" was refused: {body:#}");
    assert_eq!(
        body["routine_detection"], false,
        "bug: \"not now\" turned routine detection on: {body:#}"
    );
    assert!(
        body["routine_detection_at"].is_string(),
        "bug: \"not now\" (false while already false) was not stamped: {body:#}"
    );
    assert_eq!(
        body["routine_detection_notice_due"], false,
        "bug: the card is still asked after \"not now\": {body:#}"
    );
    let read = s.read(&s.athlete.auth).await;
    assert_eq!(
        read["routine_detection_notice_due"], false,
        "bug: the card comes back after \"not now\": {read:#}"
    );

    // Every answer is stamped, a repeat of the same value included.
    let groups = &s.res.common.repos.groups;
    let group = s.group.to_string();
    let first = Timestamp::UNIX_EPOCH + Elapsed::seconds(1_795_000_000);
    let later = first + Elapsed::days(3);
    for at in [first, later] {
        let member = groups
            .set_coach_routine_detection(&group, s.athlete.id, false, at)
            .await
            .unwrap()
            .expect("a live member");
        assert_eq!(
            member.coach_consents.routine_detection_at,
            Some(at),
            "bug: a repeated answer was not stamped again"
        );
        assert!(
            !member.coach_consents.routine_detection,
            "bug: a revoke turned routine detection on"
        );
    }
    assert!(
        groups
            .set_coach_routine_detection(&group, Uuid::new_v4(), true, later)
            .await
            .unwrap()
            .is_none(),
        "bug: an answer was written for someone who is not a member"
    );
}

#[tokio::test]
async fn an_api_key_revokes_routine_detection_but_never_grants_it() {
    let s = squad("api-key").await;
    let key = api_key(&s.res, s.athlete.id).await;
    let (status, body) = s.answer(&key, &json!({ "routine_detection": true })).await;
    assert_eq!(
        status, 403,
        "bug: a grant over an API key was not refused: {body:#}"
    );
    assert_eq!(
        body["details"]["reason"], "consent_grant_needs_app",
        "bug: the refusal does not say the grant needs the app: {body:#}"
    );
    let stored = s.stored(s.athlete.id).await;
    assert!(
        !stored.routine_detection && stored.routine_detection_at.is_none(),
        "bug: a refused grant wrote something: {stored:?}"
    );

    let (status, body) = s
        .answer(&s.athlete.auth, &json!({ "routine_detection": true }))
        .await;
    assert_eq!(status, 200, "bug: the app's grant was refused: {body:#}");
    let (status, body) = s.answer(&key, &json!({ "routine_detection": false })).await;
    assert_eq!(
        status, 200,
        "bug: a revoke over an API key was refused: {body:#}"
    );
    assert_eq!(
        body["routine_detection"], false,
        "bug: the revoke over an API key left routine detection on: {body:#}"
    );
    assert!(
        !s.stored(s.athlete.id).await.routine_detection,
        "bug: the revoke over an API key was not stored"
    );
}

#[tokio::test]
async fn routine_detection_is_not_offered_without_a_human_coach_other_than_the_member() {
    // The coach, a member of their own group, is offered nothing about
    // themselves and cannot change a member's consent.
    let s = squad("not-offered").await;
    join(&s.res, s.group, s.coach.id, s.coach.tenant, true).await;
    let read = s.read(&s.coach.auth).await;
    assert_eq!(
        (
            &read["routine_detection_supported"],
            &read["routine_detection_notice_due"]
        ),
        (&json!(false), &json!(false)),
        "bug: the coach is offered or asked routine detection of themselves: {read:#}"
    );
    let (status, body) = s
        .answer(&s.coach.auth, &json!({ "routine_detection": true }))
        .await;
    assert_eq!(
        status, 403,
        "bug: the coach set a consent on their own group: {body:#}"
    );
    assert!(
        !s.stored(s.coach.id).await.routine_detection,
        "bug: the coach's refused grant was stored"
    );

    // With no human coach at all, a grant from the app is ignored.
    assert!(
        s.res
            .common
            .repos
            .groups
            .set_group_coach_user(&s.group.to_string(), None, s.coach.tenant)
            .await
            .unwrap(),
        "the group's coach is detached"
    );
    let read = s.read(&s.athlete.auth).await;
    assert_eq!(
        (
            &read["routine_detection_supported"],
            &read["routine_detection_notice_due"]
        ),
        (&json!(false), &json!(false)),
        "bug: a group with no human coach offers or asks routine detection: {read:#}"
    );
    let (status, body) = s
        .answer(&s.athlete.auth, &json!({ "routine_detection": true }))
        .await;
    assert_eq!(status, 200, "bug: an ignored grant was refused: {body:#}");
    assert_eq!(
        (&body["routine_detection"], &body["routine_detection_at"]),
        (&json!(false), &Value::Null),
        "bug: a grant the group does not offer was stored: {body:#}"
    );
    let stored = s.stored(s.athlete.id).await;
    assert!(
        !stored.routine_detection && stored.routine_detection_at.is_none(),
        "bug: a grant the group does not offer was stored: {stored:?}"
    );
}
