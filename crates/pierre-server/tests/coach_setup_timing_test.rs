// ABOUTME: Coach setup-time events (carnet#739): the onboarding group step's create and Later, and the first athlete joining
// ABOUTME: Pins which step fires each notify event, that it fires once, and the timing fields PostHog reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The validation cohort aims for a coach to go from a new account to a first
//! athlete in their group within an hour. Three notify events measure it:
//! `onboarding.coach_group_created` when the onboarding "Your group" step
//! creates the group, `onboarding.coach_group_skipped` when the coach taps
//! **Later**, and `group.first_athlete_joined` when the first person other
//! than the owner joins. These tests drive the same routes and service calls
//! the clients and `/group join` use, and read the events off the notify
//! target.

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{
    create_test_server_resources, create_test_user_with_email, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only};
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_routes_groups::GroupRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

/// A coach on a plan with group coaching, signed in, with an agent to give
/// their group.
struct Coach {
    res: Arc<ServerContext>,
    router: axum::Router,
    auth: String,
    user_id: Uuid,
    tenant_id: TenantId,
    agent_id: String,
}

async fn coach() -> Coach {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, user, _) =
        create_test_user_with_plan(&res.agent.database, "setup-coach@test.com", "enterprise")
            .await
            .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&res, &user).await);
    let tenant_id = res
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    let request: CreateAgentRequest = serde_json::from_value(
        json!({"title":"Group Agent","system_prompt":"Test.","category":"training","tags":["run"]}),
    )
    .unwrap();
    let agent_id = res
        .common
        .repos
        .agents
        .create(user_id, tenant_id, &request)
        .await
        .unwrap()
        .id
        .to_string();
    let router =
        GroupRoutes::routes(Arc::clone(&res)).merge(OnboardingRoutes::routes(Arc::clone(&res)));
    Coach {
        res,
        router,
        auth,
        user_id,
        tenant_id,
        agent_id,
    }
}

impl Coach {
    /// POST `/api/groups` as the coach and return the created group.
    async fn create_group(&self, coach_is_me: bool) -> Value {
        let resp = AxumTestRequest::post("/api/groups")
            .header("authorization", &self.auth)
            .json(&json!({"name": "Les Rouleurs", "agent_id": self.agent_id, "coach_is_me": coach_is_me}))
            .send(self.router.clone())
            .await;
        assert_eq!(resp.status_code(), StatusCode::CREATED);
        resp.json()
    }

    /// PUT an onboarding step's status as the coach.
    async fn put_step(&self, step: &str, status: &str) {
        let resp = AxumTestRequest::put(&format!("/api/me/onboarding/steps/{step}"))
            .header("authorization", &self.auth)
            .json(&json!({ "status": status }))
            .send(self.router.clone())
            .await;
        assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);
    }

    /// A member invite to `group_id`, as the onboarding step makes it.
    async fn invite(&self, group_id: &str) -> String {
        let resp = AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
            .header("authorization", &self.auth)
            .json(&json!({ "kind": "member" }))
            .send(self.router.clone())
            .await;
        assert_eq!(resp.status_code(), StatusCode::CREATED);
        resp.json::<Value>()["code"].as_str().unwrap().to_owned()
    }

    /// A new athlete account redeeming `code`, the service call `/group join` makes.
    async fn athlete_joins(&self, email: &str, code: &str) -> Uuid {
        let (athlete, _) = create_test_user_with_email(&self.res.agent.database, email)
            .await
            .unwrap();
        self.res
            .group_service()
            .join_group(code, athlete, self.tenant_id)
            .await
            .unwrap();
        athlete
    }
}

/// A field that must parse as a non-negative whole number of seconds.
fn seconds(event: &helpers::notify_capture::NotifyEvent, field: &str) -> i64 {
    let value: i64 = event.field(field).parse().unwrap();
    assert!(value >= 0, "{field} is negative: {value}");
    value
}

#[tokio::test]
async fn the_onboarding_group_step_times_the_group_from_signup() {
    let coach = coach().await;
    coach
        .res
        .common
        .repos
        .users
        .set_manages_roster(coach.user_id, true)
        .await
        .unwrap();

    let (events, _guard) = capture_notify();
    let group = coach.create_group(true).await;

    let created = only(&events, "onboarding.coach_group_created");
    assert_eq!(created.field("user_id"), coach.user_id.to_string());
    assert_eq!(created.field("tenant_id"), coach.tenant_id.to_string());
    assert_eq!(created.field("group_id"), group["id"].as_str().unwrap());
    assert_eq!(created.field("coach_attached"), "true");
    // The account was made moments ago, so the step is well inside the hour.
    assert!(seconds(&created, "seconds_since_signup") < 3600);
    // The generic adoption event still fires alongside it.
    only(&events, "group.created");
}

#[tokio::test]
async fn a_coach_without_coach_access_is_timed_with_the_coach_unattached() {
    let coach = coach().await;

    let (events, _guard) = capture_notify();
    let group = coach.create_group(true).await;
    assert!(group["coach_user_id"].is_null());

    let created = only(&events, "onboarding.coach_group_created");
    assert_eq!(created.field("coach_attached"), "false");
}

#[tokio::test]
async fn a_group_made_outside_the_onboarding_step_is_not_timed() {
    let coach = coach().await;

    let (events, _guard) = capture_notify();
    coach.create_group(false).await;

    assert!(named(&events, "onboarding.coach_group_created").is_empty());
    only(&events, "group.created");
}

#[tokio::test]
async fn later_on_the_group_step_fires_the_skip_and_nothing_else_does() {
    let coach = coach().await;

    let (events, _guard) = capture_notify();
    coach.put_step("coach_group", "complete").await;
    coach.put_step("profile_type", "skipped").await;
    assert!(
        named(&events, "onboarding.coach_group_skipped").is_empty(),
        "only Later on the group step is a skip"
    );

    coach.put_step("coach_group", "skipped").await;
    let skipped = only(&events, "onboarding.coach_group_skipped");
    assert_eq!(skipped.field("user_id"), coach.user_id.to_string());
    assert_eq!(skipped.field("tenant_id"), coach.tenant_id.to_string());
    assert!(seconds(&skipped, "seconds_since_signup") < 3600);
}

#[tokio::test]
async fn the_first_athlete_to_join_ends_the_setup_once() {
    let coach = coach().await;
    let group = coach.create_group(false).await;
    let group_id = group["id"].as_str().unwrap().to_owned();
    let code = coach.invite(&group_id).await;

    let (events, _guard) = capture_notify();
    let first = coach.athlete_joins("first-athlete@test.com", &code).await;

    let joined = only(&events, "group.first_athlete_joined");
    // The event is the coach's: the owner, not the athlete who joined.
    assert_eq!(joined.field("user_id"), coach.user_id.to_string());
    assert_ne!(joined.field("user_id"), first.to_string());
    assert_eq!(joined.field("tenant_id"), coach.tenant_id.to_string());
    assert_eq!(joined.field("group_id"), group_id);
    let since_group = seconds(&joined, "seconds_since_group_created");
    let since_signup = seconds(&joined, "seconds_since_coach_signup");
    assert!(
        since_signup >= since_group,
        "the coach signed up before making the group: {since_signup} < {since_group}"
    );

    // A second athlete, and a third after the first one leaves, are not first.
    coach.athlete_joins("second-athlete@test.com", &code).await;
    coach
        .res
        .group_service()
        .leave_group(&group_id, first)
        .await
        .unwrap();
    coach.athlete_joins("third-athlete@test.com", &code).await;
    assert_eq!(
        named(&events, "group.first_athlete_joined").len(),
        1,
        "only the group's first athlete ends the setup"
    );
    assert_eq!(named(&events, "group.joined").len(), 3);
}

#[tokio::test]
async fn a_channel_bound_group_times_its_first_enrolled_member() {
    let coach = coach().await;
    let group = coach.create_group(false).await;
    let group_id = Uuid::parse_str(group["id"].as_str().unwrap()).unwrap();
    let stored = coach
        .res
        .common
        .repos
        .groups
        .get_group(&group_id.to_string(), coach.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let (athlete, _) =
        create_test_user_with_email(&coach.res.agent.database, "chat-athlete@test.com")
            .await
            .unwrap();

    let (events, _guard) = capture_notify();
    assert!(coach
        .res
        .group_service()
        .enroll_channel_member(&stored, athlete)
        .await
        .unwrap());
    // Enrolling again is a no-op, and fires nothing.
    assert!(coach
        .res
        .group_service()
        .enroll_channel_member(&stored, athlete)
        .await
        .unwrap());

    let joined = only(&events, "group.first_athlete_joined");
    assert_eq!(joined.field("user_id"), coach.user_id.to_string());
    assert_eq!(joined.field("group_id"), group_id.to_string());
}
