// ABOUTME: carnet#1099 / carnet#1100 — the first-run done rules, the one write that marks them, and Getting started
// ABOUTME: Each flow is done once and reports flow.completed once; the check-in counts three distinct days

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The four first-run flows are done by rules met in different places — a
//! turn in the personal thread, a grounded answer, a complete dossier, a
//! reply on a day the agent checked in. These drive each rule against the
//! real store and the athlete's real state, and read the result back the way
//! Home does, through `GET /api/me/getting-started`.

use crate::common;
use crate::helpers;

use std::iter::once;
use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Duration, NaiveDate, Utc};
use serde_json::Value;
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only};
use pierre_core::models::{Pillar, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{HomePreferencesUpdate, UpsertUserFactParams};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::first_run::{
    after_athlete_turn, record, AthleteTurn, FirstRun, CHECK_IN, FINISH_PROFILE, LAST_WORKOUT,
    MEET_TEAM,
};

const WEB: &str = "web_chat";

struct Fixture {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    auth: String,
}

impl Fixture {
    fn repos(&self) -> &RepositoryRegistry {
        &self.resources.common.repos
    }

    fn turn(&self, personal_thread: bool, grounded: bool) -> AthleteTurn<'static> {
        AthleteTurn {
            user: self.user_id,
            tenant: self.tenant,
            personal_thread,
            grounded,
            surface: WEB,
        }
    }

    async fn first_run(&self) -> FirstRun {
        FirstRun::read(self.repos(), self.tenant, self.user_id)
            .await
            .unwrap()
    }

    async fn getting_started(&self) -> Value {
        AxumTestRequest::get("/api/me/getting-started")
            .header("Authorization", &self.auth)
            .send(OnboardingRoutes::routes(Arc::clone(&self.resources)))
            .await
            .assert_status(StatusCode::OK)
            .json()
    }
}

async fn setup() -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;
    let auth = format!("Bearer {}", generate_test_token(&resources, &user).await);
    Fixture {
        resources,
        user_id,
        tenant,
        auth,
    }
}

fn day(n: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, n).unwrap()
}

fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// Mark the agent's check-in posted at `at`, as the worker does once its
/// reply is stored.
async fn check_in_posted(fx: &Fixture, at: chrono::DateTime<Utc>) {
    let claims = &fx.repos().platform_post_claims;
    let lease_token = claims
        .claim_platform_post_since(
            fx.tenant,
            fx.user_id,
            "check_in",
            at - Duration::hours(1),
            at,
        )
        .await
        .unwrap()
        .expect("the hour's claim takes the post");
    assert!(claims
        .mark_platform_post_posted(fx.tenant, fx.user_id, "check_in", &lease_token, at)
        .await
        .unwrap());
}

/// Cover the North Star and every pillar, which makes the dossier complete.
async fn complete_the_dossier(fx: &Fixture) {
    let user = fx.user_id.to_string();
    let topics = once((FactKind::NorthStar, None)).chain(
        Pillar::ALL
            .iter()
            .map(|pillar| (FactKind::Goal, Some(*pillar))),
    );
    for (kind, pillar) in topics {
        fx.repos()
            .memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id: fx.tenant,
                user_id: &user,
                agent_id: None,
                scope: MemoryScope::User,
                kind,
                pillar,
                predicate_code: PredicateCode::States,
                object: "an answer",
                confidence: 0.9,
                source: FactSource::Onboarding,
                valid_until: None,
                source_msg_id: None,
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_flow_is_done_on_its_first_day_and_reports_completed_once() {
    let fx = setup().await;
    let (events, _guard) = capture_notify();

    let first = record(fx.repos(), fx.user_id, fx.tenant, MEET_TEAM, day(1), WEB)
        .await
        .unwrap();
    assert!(first.done && first.became_done);
    let completed = only(&events, "flow.completed");
    assert_eq!(completed.field("flow"), MEET_TEAM);
    assert_eq!(completed.field("surface"), WEB);
    assert_eq!(completed.field("user_id"), fx.user_id.to_string());
    assert_eq!(completed.field("tenant_id"), fx.tenant.to_string());

    for date in [day(1), day(2)] {
        let again = record(fx.repos(), fx.user_id, fx.tenant, MEET_TEAM, date, WEB)
            .await
            .unwrap();
        assert!(again.done && !again.became_done, "{date}");
    }
    assert_eq!(
        named(&events, "flow.completed").len(),
        1,
        "once per athlete"
    );
}

#[tokio::test]
async fn the_check_in_is_done_on_its_third_distinct_day() {
    let fx = setup().await;
    let (events, _guard) = capture_notify();
    let mark = |date| record(fx.repos(), fx.user_id, fx.tenant, CHECK_IN, date, WEB);

    assert_eq!(mark(day(1)).await.unwrap().days, 1);
    assert_eq!(mark(day(1)).await.unwrap().days, 1, "one row a day");
    let second = mark(day(3)).await.unwrap();
    assert_eq!((second.days, second.done), (2, false));
    assert!(named(&events, "flow.completed").is_empty());

    let third = mark(day(8)).await.unwrap();
    assert!(third.became_done, "three distinct days, not three in a row");
    assert_eq!(only(&events, "flow.completed").field("flow"), CHECK_IN);
    assert!(!mark(day(9)).await.unwrap().became_done);
    assert_eq!(named(&events, "flow.completed").len(), 1);
}

#[tokio::test]
async fn first_run_days_stay_in_their_tenant() {
    let fx = setup().await;
    record(fx.repos(), fx.user_id, fx.tenant, LAST_WORKOUT, day(1), WEB)
        .await
        .unwrap();
    let elsewhere = fx
        .repos()
        .first_run_progress
        .first_run_days(TenantId::generate(), fx.user_id)
        .await
        .unwrap();
    assert!(elsewhere.is_empty(), "another tenant reads none of it");
    assert!(fx.first_run().await.is_done(LAST_WORKOUT));
}

/// A turn in the personal thread meets the team; a grounded answer, in any
/// thread, reads the last workout; a turn elsewhere and ungrounded meets
/// nothing.
#[tokio::test]
async fn an_athletes_turn_marks_the_team_and_the_grounded_answer() {
    let fx = setup().await;
    let (events, _guard) = capture_notify();

    after_athlete_turn(fx.repos(), fx.turn(false, false)).await;
    assert!(!fx.first_run().await.is_done(MEET_TEAM));
    assert!(named(&events, "flow.completed").is_empty());

    after_athlete_turn(fx.repos(), fx.turn(true, false)).await;
    let first_run = fx.first_run().await;
    assert!(first_run.is_done(MEET_TEAM));
    assert!(
        !first_run.is_done(LAST_WORKOUT),
        "an ungrounded answer is not one"
    );

    after_athlete_turn(fx.repos(), fx.turn(false, true)).await;
    assert!(fx.first_run().await.is_done(LAST_WORKOUT));
    let flows: Vec<String> = named(&events, "flow.completed")
        .iter()
        .map(|e| e.field("flow").to_owned())
        .collect();
    assert_eq!(flows, [MEET_TEAM, LAST_WORKOUT]);
}

/// A reply in the personal thread answers the check-in only on a day the
/// agent checked in, at or before it — never one posted on another day.
#[tokio::test]
async fn a_reply_counts_toward_the_check_in_only_on_a_day_it_was_posted() {
    let fx = setup().await;
    after_athlete_turn(fx.repos(), fx.turn(true, false)).await;
    assert!(
        !fx.first_run().await.met_on(CHECK_IN, today()),
        "no check-in yet"
    );

    check_in_posted(&fx, Utc::now() - Duration::days(2)).await;
    after_athlete_turn(fx.repos(), fx.turn(true, false)).await;
    assert!(
        !fx.first_run().await.met_on(CHECK_IN, today()),
        "a check-in two days ago is not today's"
    );

    check_in_posted(&fx, Utc::now() - Duration::seconds(5)).await;
    after_athlete_turn(fx.repos(), fx.turn(false, false)).await;
    assert!(
        !fx.first_run().await.met_on(CHECK_IN, today()),
        "the answer comes in the personal thread"
    );
    after_athlete_turn(fx.repos(), fx.turn(true, false)).await;
    let first_run = fx.first_run().await;
    assert!(first_run.met_on(CHECK_IN, today()));
    assert!(!first_run.is_done(CHECK_IN), "one day of three");
}

/// Finish your profile is the dossier covering every topic, noticed by the
/// next turn — not by a walk that ended.
#[tokio::test]
async fn a_complete_dossier_finishes_the_profile_flow() {
    let fx = setup().await;
    after_athlete_turn(fx.repos(), fx.turn(false, false)).await;
    assert!(!fx.first_run().await.is_done(FINISH_PROFILE));

    complete_the_dossier(&fx).await;
    let (events, _guard) = capture_notify();
    after_athlete_turn(fx.repos(), fx.turn(false, false)).await;
    assert!(fx.first_run().await.is_done(FINISH_PROFILE));
    assert_eq!(
        only(&events, "flow.completed").field("flow"),
        FINISH_PROFILE
    );
}

fn row<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("no {id} row in {body}"))
}

/// Home's list: connect, then the four flows in order; the first open row
/// is current and carries its gs: postback; the rows offered are reported
/// shown under the `getting_started` surface and never counted toward
/// retiring a starter.
#[tokio::test]
async fn getting_started_lists_the_rows_and_reports_them_shown_without_counting_them() {
    let fx = setup().await;
    let (events, _guard) = capture_notify();
    let body = fx.getting_started().await;

    let ids: Vec<&str> = body["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "connect",
            "meet_team",
            "last_workout",
            "about_me_more",
            "check_in"
        ]
    );
    assert_eq!(body["total"], 5);
    assert_eq!(body["done_count"], 0);
    assert_eq!(body["hidden"], false);
    assert!(body["folded_at"].is_null());
    let connect = row(&body, "connect");
    assert_eq!(
        connect["current"], true,
        "nothing connected: connect is current"
    );
    assert!(connect["postback"].is_null(), "connect opens Settings");
    assert_eq!(row(&body, "meet_team")["postback"], "gs:1:meet_team");
    assert_eq!(row(&body, "meet_team")["current"], false);
    let last_workout = row(&body, "last_workout");
    assert_eq!(last_workout["eligible"], false, "no activities yet");
    assert!(last_workout["postback"].is_null());

    let shown = named(&events, "suggestion.shown");
    assert!(!shown.is_empty());
    for event in &shown {
        assert_eq!(event.field("surface"), "getting_started");
        assert_eq!(event.field("channel"), WEB);
    }
    assert!(shown
        .iter()
        .any(|e| e.field("use_case") == MEET_TEAM && e.field("position") == "1"));
    assert!(
        fx.repos()
            .use_case_exposures
            .use_case_exposures(fx.tenant, fx.user_id)
            .await
            .unwrap()
            .is_empty(),
        "a Home read never counts toward retiring a starter"
    );
}

#[tokio::test]
async fn getting_started_ticks_a_done_flow_and_says_when_it_is_hidden() {
    let fx = setup().await;
    after_athlete_turn(fx.repos(), fx.turn(true, false)).await;
    let body = fx.getting_started().await;
    let meet_team = row(&body, "meet_team");
    assert_eq!(meet_team["done"], true);
    assert!(meet_team["postback"].is_null(), "a done row links nowhere");
    assert_eq!(body["done_count"], 1);

    complete_the_dossier(&fx).await;
    let body = fx.getting_started().await;
    assert_eq!(
        row(&body, "about_me_more")["done"],
        true,
        "a dossier filled outside any turn is noticed on the Home read"
    );

    fx.repos()
        .home_preferences
        .set_home_preferences(
            fx.user_id,
            HomePreferencesUpdate {
                getting_started_hidden: Some(true),
                ..HomePreferencesUpdate::default()
            },
        )
        .await
        .unwrap();
    let (events, _guard) = capture_notify();
    let body = fx.getting_started().await;
    assert_eq!(body["hidden"], true);
    assert!(
        named(&events, "suggestion.shown").is_empty(),
        "a hidden list shows nothing"
    );
}
