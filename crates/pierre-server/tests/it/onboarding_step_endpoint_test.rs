// ABOUTME: HTTP tests for PUT /api/me/onboarding/steps/{step_id} — status (incl. not_applicable) + chosen_channel validation
// ABOUTME: Pins the slug guard, the coach-only answer, the proposal's lead becoming selected and the team step joining it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use crate::helpers::agent_fixtures::publish_catalogue_agent;
use crate::helpers::notify_capture::{capture_notify, named, only};

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_contremaitre::messaging_strings::{
    format_template, KEY_AGENT_WELCOME_DISCOVER_LINE, KEY_AGENT_WELCOME_LIST_AND,
    KEY_AGENT_WELCOME_REACH_LINE, KEY_AGENT_WELCOME_TEAM_MANY, KEY_AGENT_WELCOME_TEAM_SOLO,
};
use pierre_core::models::agents::{
    Agent, AgentCategory, AgentVisibility, CreateAgentRequest, CreateSystemAgentRequest,
};
use pierre_core::models::{MessageRecord, TenantId};
use pierre_database::repositories::OnboardingResetScope;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_services::conversation_forge::selected_or_system_agent;
use pierre_services::default_agent::ROSTER_AGENT_HANDLE;
use pierre_services::intake::MAX_ONBOARDING_TEAM;
use pierre_services::locale::resolve_user_locale;
use pierre_services::personal_thread::find_personal_thread;
use uuid::Uuid;

async fn setup() -> (axum::Router, String) {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (_user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let token = generate_test_token(&resources, &user).await;
    let router = OnboardingRoutes::routes(Arc::clone(&resources));
    (router, format!("Bearer {token}"))
}

#[tokio::test]
async fn put_step_accepts_a_valid_channel_slug() {
    let (router, token) = setup().await;
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/messaging_channel")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete", "chosen_channel": "telegram" }))
        .send(router)
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::NO_CONTENT,
        "a valid slug ('telegram') must persist"
    );
}

#[tokio::test]
async fn put_step_rejects_a_non_slug_chosen_channel() {
    let (router, token) = setup().await;
    // 25 chars — under the length cap, so this exercises the slug charset guard,
    // not the length guard. Markup must never reach the stored column.
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/messaging_channel")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete", "chosen_channel": "<script>alert(1)</script>" }))
        .send(router)
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::BAD_REQUEST,
        "a non-slug chosen_channel (markup) must be rejected, not stored"
    );
}

#[tokio::test]
async fn put_step_rejects_an_unknown_status() {
    let (router, token) = setup().await;
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/profile_type")
        .header("Authorization", &token)
        .json(&json!({ "status": "bogus" }))
        .send(router)
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::BAD_REQUEST,
        "an unknown status must be rejected"
    );
}

/// A coach who does not train records the athlete steps as outside their
/// journey; the status round-trips so every surface reads it back.
#[tokio::test]
async fn put_step_accepts_not_applicable_and_status_reads_it_back() {
    let (router, token) = setup().await;
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/parq")
        .header("Authorization", &token)
        .json(&json!({ "status": "not_applicable" }))
        .send(router.clone())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::NO_CONTENT,
        "not_applicable must be an accepted status"
    );

    let status: serde_json::Value = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &token)
        .send(router)
        .await
        .assert_status(StatusCode::OK)
        .json();
    let steps = status["steps"].as_array().expect("steps array");
    assert!(
        steps
            .iter()
            .any(|s| s["step_id"] == "parq" && s["status"] == "not_applicable"),
        "the not_applicable parq row must be served back, got {steps:?}"
    );
}

#[tokio::test]
async fn status_reports_an_athlete_with_no_coach_signals() {
    let (router, token) = setup().await;
    let status: serde_json::Value = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &token)
        .send(router)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(status["coaches_others"], false);
}

#[tokio::test]
async fn status_reports_a_coach_and_accepts_the_group_step() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = resources.agent.database.repositories();
    repos.users.set_coaches_others(user_id, true).await.unwrap();
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let router = OnboardingRoutes::routes(Arc::clone(&resources));

    let resp = AxumTestRequest::put("/api/me/onboarding/steps/coach_group")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete" }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    let status: serde_json::Value = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &token)
        .send(router)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(status["coaches_others"], true);
    let steps = status["steps"].as_array().expect("steps array");
    assert!(steps
        .iter()
        .any(|s| s["step_id"] == "coach_group" && s["status"] == "complete"));
}

/// A user holding a selected agent in their tenant, with a token for that
/// tenant: the state signup leaves every new account in.
async fn user_with_selected_agent() -> (Arc<ServerContext>, axum::Router, String, Uuid, TenantId) {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = resources.agent.database.repositories();
    let tenant = repos.tenants.list_for_user(user_id).await.unwrap()[0].id;
    let request: CreateAgentRequest = serde_json::from_value(
        json!({"title":"Endurance","system_prompt":"Test.","category":"training","tags":["run"]}),
    )
    .unwrap();
    let agent = repos
        .agents
        .create(user_id, tenant, &request)
        .await
        .unwrap();
    repos
        .tenants
        .set_selected_agent(tenant, user_id, Some(&agent.id.to_string()))
        .await
        .unwrap();
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let router = OnboardingRoutes::routes(Arc::clone(&resources));
    (resources, router, token, user_id, tenant)
}

/// A tenant system agent, created in order so the list (newest first) puts
/// later ones ahead.
async fn system_agent(
    resources: &ServerContext,
    owner: Uuid,
    tenant: TenantId,
    title: &str,
    tags: Vec<String>,
) -> String {
    resources
        .agent
        .database
        .repositories()
        .agents
        .create_system_agent(
            owner,
            tenant,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: None,
                system_prompt: "Test.".to_owned(),
                category: AgentCategory::Custom,
                tags,
                sample_prompts: vec![],
                visibility: AgentVisibility::Tenant,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

/// A catalogue agent published in the Store, as every agent the proposal
/// offers is: the team step joins it through the Store install.
async fn published_agent(
    resources: &ServerContext,
    owner: Uuid,
    tenant: TenantId,
    title: &str,
) -> String {
    publish_catalogue_agent(
        resources.agent.database.repositories(),
        owner,
        tenant,
        title,
        "Test.",
    )
    .await
    .to_string()
}

/// A coach who does not train runs their practice with the roster agent, and
/// an answer that changes later takes them back to an athlete's agent: the
/// selection is left empty and their thread resolves it from the answer.
#[tokio::test]
async fn the_coach_only_answer_resolves_the_roster_agent_until_they_train() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let repos = resources.agent.database.repositories();
    let athlete = system_agent(&resources, user_id, tenant, "Endurance", vec![]).await;
    // Seeded after the athlete agent, so it is first in the list.
    let roster_id = system_agent(
        &resources,
        user_id,
        tenant,
        "Roster Agent",
        vec![Agent::COACH_TOOL_TAG.to_owned()],
    )
    .await;
    let handle = repos
        .store_listings
        .assign_catalogue_handle(&roster_id, tenant)
        .await
        .unwrap();
    assert_eq!(handle, ROSTER_AGENT_HANDLE, "fixture precondition");
    let registry = &resources.common.repos;

    let resp = AxumTestRequest::put("/api/me/onboarding/steps/about_you")
        .header("Authorization", &token)
        .json(&json!({ "status": "not_applicable" }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);
    assert_eq!(
        repos
            .tenants
            .get_selected_agent(tenant, user_id)
            .await
            .unwrap(),
        None,
        "the coach-only answer leaves nothing selected"
    );
    assert_eq!(
        selected_or_system_agent(registry, tenant, user_id).await,
        Some(roster_id.clone()),
        "their own thread resolves the roster agent"
    );

    // An operator reset, then the person says they train.
    repos
        .user_onboarding
        .reset_onboarding(&user_id.to_string(), OnboardingResetScope::OnboardingOnly)
        .await
        .unwrap();
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/about_you")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete" }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);
    assert_eq!(
        selected_or_system_agent(registry, tenant, user_id).await,
        Some(athlete),
        "once they train, their thread is an athlete's again"
    );
}

/// The athlete's agent signup picked is released by the coach-only answer,
/// whether or not the tenant has the roster agent to resolve to.
#[tokio::test]
async fn the_coach_only_answer_clears_the_selected_agent() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;

    let resp = AxumTestRequest::put("/api/me/onboarding/steps/about_you")
        .header("Authorization", &token)
        .json(&json!({ "status": "not_applicable" }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    let selected = resources
        .agent
        .database
        .repositories()
        .tenants
        .get_selected_agent(tenant, user_id)
        .await
        .unwrap();
    assert_eq!(selected, None, "a coach who does not train holds no agent");
}

#[tokio::test]
async fn an_athletes_answer_keeps_the_selected_agent() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let tenants = Arc::clone(&resources.agent.database.repositories().tenants);
    let held = tenants.get_selected_agent(tenant, user_id).await.unwrap();
    assert!(held.is_some(), "the fixture selects an agent");

    let resp = AxumTestRequest::put("/api/me/onboarding/steps/parq")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete" }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    let selected = tenants.get_selected_agent(tenant, user_id).await.unwrap();
    assert_eq!(selected, held, "an athlete keeps the agent they hold");
}

// ============================================================================
// The proposal's lead becomes the selected agent (carnet#1101)
// ============================================================================

async fn put_proposal(router: &axum::Router, token: &str, body: serde_json::Value) -> StatusCode {
    AxumTestRequest::put("/api/me/onboarding/steps/coach_proposal")
        .header("Authorization", token)
        .json(&body)
        .send(router.clone())
        .await
        .status_code()
}

/// Start and Skip both send the lead with the step write, and it replaces the
/// starter signup selected, so Home, a new thread and the DM answer with the
/// agent the athlete was proposed. A write with no lead (the error state's
/// Continue) leaves the selection alone.
#[tokio::test]
async fn start_and_skip_make_the_proposals_lead_the_selected_agent() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let tenants = Arc::clone(&resources.agent.database.repositories().tenants);
    let tempo = published_agent(&resources, user_id, tenant, "Tempo Coach").await;
    let hills = published_agent(&resources, user_id, tenant, "Hill Coach").await;

    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "complete", "agent_id": tempo })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        tenants.get_selected_agent(tenant, user_id).await.unwrap(),
        Some(tempo.clone()),
        "Start makes the tapped lead the selected agent"
    );

    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "skipped", "agent_id": hills })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        tenants.get_selected_agent(tenant, user_id).await.unwrap(),
        Some(hills.clone()),
        "Skip makes the proposed lead the selected agent"
    );

    assert_eq!(
        put_proposal(&router, &token, json!({ "status": "complete" })).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        tenants.get_selected_agent(tenant, user_id).await.unwrap(),
        Some(hills),
        "a step write with no lead keeps the selection"
    );
}

/// The personal thread the athlete lands on answers with the lead when it
/// still runs on the agent that was selected before; a thread the athlete had
/// already pointed at another agent keeps it.
#[tokio::test]
async fn the_personal_thread_follows_the_lead_unless_the_athlete_moved_it() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let repos = resources.agent.database.repositories();
    let starter = repos
        .tenants
        .get_selected_agent(tenant, user_id)
        .await
        .unwrap()
        .expect("the fixture selects an agent");
    let thread = repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "Home",
            "test-model",
            Some(&starter),
            None,
        )
        .await
        .unwrap()
        .id;
    let tempo = published_agent(&resources, user_id, tenant, "Tempo Coach").await;

    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "skipped", "agent_id": tempo })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let rebound = repos
        .chat
        .get_conversation(&thread, &user_id.to_string(), tenant)
        .await
        .unwrap()
        .expect("the thread exists");
    assert_eq!(rebound.agent_id.as_deref(), Some(tempo.as_str()));

    // The athlete points the thread at an agent of their own choosing.
    let hills = published_agent(&resources, user_id, tenant, "Hill Coach").await;
    let mine = system_agent(&resources, user_id, tenant, "My Pick", vec![]).await;
    repos
        .chat
        .set_conversation_agent_id(&thread, Some(&mine), tenant)
        .await
        .unwrap();
    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "complete", "agent_id": hills })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let kept = repos
        .chat
        .get_conversation(&thread, &user_id.to_string(), tenant)
        .await
        .unwrap()
        .expect("the thread exists");
    assert_eq!(
        kept.agent_id.as_deref(),
        Some(mine.as_str()),
        "a thread the athlete moved keeps its agent"
    );
    assert_eq!(
        repos
            .tenants
            .get_selected_agent(tenant, user_id)
            .await
            .unwrap(),
        Some(hills),
        "the selection still takes the lead"
    );
}

/// The step write is never an unguarded setter: an agent the athlete cannot
/// read is not found, a coach-facing one is refused as `/agent add` refuses
/// it, and either way the selection is untouched.
#[tokio::test]
async fn a_lead_the_athlete_cannot_be_given_is_refused() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let tenants = Arc::clone(&resources.agent.database.repositories().tenants);
    let held = tenants.get_selected_agent(tenant, user_id).await.unwrap();
    let coach_tool = system_agent(
        &resources,
        user_id,
        tenant,
        "Roster Agent",
        vec![Agent::COACH_TOOL_TAG.to_owned()],
    )
    .await;

    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "complete", "agent_id": Uuid::new_v4().to_string() })
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "complete", "agent_id": coach_tool })
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        tenants.get_selected_agent(tenant, user_id).await.unwrap(),
        held
    );
    let steps = resources
        .agent
        .database
        .repositories()
        .user_onboarding
        .get_onboarding_steps(&user_id.to_string())
        .await
        .unwrap();
    assert!(
        steps.iter().all(|step| step.step_id != "coach_proposal"),
        "a refused lead leaves the step unwritten"
    );
}

/// A coach who does not train keeps nothing selected: their own thread
/// resolves the roster agent from that answer, and a proposal's lead does not
/// read them as an athlete.
#[tokio::test]
async fn a_coach_who_does_not_train_is_not_given_the_lead() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let tenants = Arc::clone(&resources.agent.database.repositories().tenants);
    let tempo = published_agent(&resources, user_id, tenant, "Tempo Coach").await;
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/about_you")
        .header("Authorization", &token)
        .json(&json!({ "status": "not_applicable" }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "complete", "agent_id": tempo })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        tenants.get_selected_agent(tenant, user_id).await.unwrap(),
        None
    );
    assert!(
        !is_member(&resources, &tempo, user_id).await,
        "a coach who does not train joins no athlete's agent"
    );
}

// ============================================================================
// The team step joins every agent it names (carnet#1095)
// ============================================================================

async fn is_member(resources: &ServerContext, agent_id: &str, user_id: Uuid) -> bool {
    resources
        .agent
        .database
        .repositories()
        .agents
        .get_user_preferences(agent_id, user_id)
        .await
        .unwrap()
        .is_member
}

async fn install_count(resources: &ServerContext, agent_id: &str) -> u32 {
    resources
        .agent
        .database
        .repositories()
        .store_listings
        .get_published_agent(agent_id)
        .await
        .unwrap()
        .expect("the agent is published")
        .listing
        .install_count
}

async fn proposal_step_written(resources: &ServerContext, user_id: Uuid) -> bool {
    resources
        .agent
        .database
        .repositories()
        .user_onboarding
        .get_onboarding_steps(&user_id.to_string())
        .await
        .unwrap()
        .iter()
        .any(|step| step.step_id == "coach_proposal")
}

/// Start puts the lead and every ticked agent on the team and selects the
/// lead; sending the same write again changes nothing, and each agent's
/// install count grew once.
#[tokio::test]
async fn start_joins_every_ticked_agent_and_selects_the_lead() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let lead = published_agent(&resources, user_id, tenant, "Endurance Coach").await;
    let fuel = published_agent(&resources, user_id, tenant, "Fuelling Coach").await;
    let rest = published_agent(&resources, user_id, tenant, "Recovery Coach").await;
    let unticked = published_agent(&resources, user_id, tenant, "Mobility Coach").await;
    let body = json!({ "status": "complete", "agent_id": lead, "team_agent_ids": [fuel, rest] });

    assert_eq!(
        put_proposal(&router, &token, body.clone()).await,
        StatusCode::NO_CONTENT
    );
    for agent in [&lead, &fuel, &rest] {
        assert!(
            is_member(&resources, agent, user_id).await,
            "{agent} joined"
        );
        assert_eq!(install_count(&resources, agent).await, 1);
    }
    assert!(
        !is_member(&resources, &unticked, user_id).await,
        "an unticked agent stays off the team"
    );
    let tenants = &resources.agent.database.repositories().tenants;
    assert_eq!(
        tenants.get_selected_agent(tenant, user_id).await.unwrap(),
        Some(lead.clone()),
        "the lead is selected"
    );

    assert_eq!(
        put_proposal(&router, &token, body).await,
        StatusCode::NO_CONTENT,
        "a retried Start converges"
    );
    for agent in [&lead, &fuel, &rest] {
        assert_eq!(
            install_count(&resources, agent).await,
            1,
            "a retry installs nothing twice"
        );
    }
}

/// Skip sends the lead alone, and the lead still joins: the team is never
/// empty.
#[tokio::test]
async fn skip_joins_the_lead_only() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let lead = published_agent(&resources, user_id, tenant, "Endurance Coach").await;
    let fuel = published_agent(&resources, user_id, tenant, "Fuelling Coach").await;

    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "skipped", "agent_id": lead, "team_agent_ids": [lead] })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert!(is_member(&resources, &lead, user_id).await);
    assert!(!is_member(&resources, &fuel, user_id).await);
}

/// A member the athlete cannot read is not found and a coach-facing one is
/// refused — and either way nothing is written: no agent joins, the
/// selection stays, the step stays unanswered.
#[tokio::test]
async fn a_team_member_that_cannot_join_writes_nothing() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let held = resources
        .agent
        .database
        .repositories()
        .tenants
        .get_selected_agent(tenant, user_id)
        .await
        .unwrap();
    let lead = published_agent(&resources, user_id, tenant, "Endurance Coach").await;
    let fuel = published_agent(&resources, user_id, tenant, "Fuelling Coach").await;
    let coach_tool = system_agent(
        &resources,
        user_id,
        tenant,
        "Roster Agent",
        vec![Agent::COACH_TOOL_TAG.to_owned()],
    )
    .await;
    let unpublished = system_agent(&resources, user_id, tenant, "Draft Coach", vec![]).await;

    for (member, expected) in [
        (Uuid::new_v4().to_string(), StatusCode::NOT_FOUND),
        (unpublished, StatusCode::NOT_FOUND),
        (coach_tool, StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(
            put_proposal(
                &router,
                &token,
                json!({ "status": "complete", "agent_id": lead, "team_agent_ids": [fuel, member] })
            )
            .await,
            expected
        );
    }
    assert!(
        !is_member(&resources, &lead, user_id).await,
        "the lead did not join"
    );
    assert!(
        !is_member(&resources, &fuel, user_id).await,
        "no member joined"
    );
    assert_eq!(
        resources
            .agent
            .database
            .repositories()
            .tenants
            .get_selected_agent(tenant, user_id)
            .await
            .unwrap(),
        held,
        "the selection is untouched"
    );
    assert!(
        !proposal_step_written(&resources, user_id).await,
        "a refused team leaves the step unwritten"
    );
}

/// The list is the athlete's own input, so its length is bounded.
#[tokio::test]
async fn an_oversized_team_is_refused() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let lead = published_agent(&resources, user_id, tenant, "Endurance Coach").await;
    let members: Vec<String> = (0..=MAX_ONBOARDING_TEAM)
        .map(|_| Uuid::new_v4().to_string())
        .collect();
    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "complete", "agent_id": lead, "team_agent_ids": members })
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert!(!is_member(&resources, &lead, user_id).await);
    assert!(!proposal_step_written(&resources, user_id).await);
}

// ============================================================================
// The funnel's step event (carnet#1100) and the lead's welcome (carnet#1099)
// ============================================================================

async fn put_step(router: &axum::Router, token: &str, step: &str, status: &str) -> StatusCode {
    AxumTestRequest::put(&format!("/api/me/onboarding/steps/{step}"))
        .header("Authorization", token)
        .json(&json!({ "status": status }))
        .send(router.clone())
        .await
        .status_code()
}

/// A step write reports `onboarding.step_completed` once per stored change:
/// the first write and a changed status do, a repeat of the same status and
/// a refused write do not.
#[tokio::test]
async fn a_step_reports_completed_only_when_its_stored_status_changes() {
    let (_resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let (events, _guard) = capture_notify();

    assert_eq!(
        put_step(&router, &token, "parq", "complete").await,
        StatusCode::NO_CONTENT
    );
    let first = only(&events, "onboarding.step_completed");
    assert_eq!(first.field("step_id"), "parq");
    assert_eq!(first.field("status"), "complete");
    assert_eq!(first.field("user_id"), user_id.to_string());
    assert_eq!(first.field("tenant_id"), tenant.to_string());
    let seconds: i64 = first.field("seconds_since_signup").parse().unwrap();
    assert!((0..3600).contains(&seconds), "timed from signup: {seconds}");

    assert_eq!(
        put_step(&router, &token, "parq", "complete").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        named(&events, "onboarding.step_completed").len(),
        1,
        "the same status again is not a new step"
    );

    assert_eq!(
        put_step(&router, &token, "parq", "skipped").await,
        StatusCode::NO_CONTENT
    );
    let steps = named(&events, "onboarding.step_completed");
    assert_eq!(steps.len(), 2, "a changed status is");
    assert_eq!(steps[1].field("status"), "skipped");

    assert_eq!(
        put_step(&router, &token, "parq", "bogus").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put_step(&router, &token, "nope", "complete").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        named(&events, "onboarding.step_completed").len(),
        2,
        "a refused write reports nothing"
    );
}

/// A coach's Later on the group step reports both its own event and the
/// funnel's step, from one user read.
#[tokio::test]
async fn the_coach_group_skip_reports_its_event_and_the_step() {
    let (_resources, router, token, _user_id, _tenant) = user_with_selected_agent().await;
    let (events, _guard) = capture_notify();
    assert_eq!(
        put_step(&router, &token, "coach_group", "skipped").await,
        StatusCode::NO_CONTENT
    );
    only(&events, "onboarding.coach_group_skipped");
    assert_eq!(
        only(&events, "onboarding.step_completed").field("step_id"),
        "coach_group"
    );
}

/// The athlete's own words for `key`, as the welcome was written in them.
async fn said(resources: &ServerContext, user_id: Uuid, key: &str) -> String {
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), user_id).await;
    resources.mcp.messaging_strings_registry.get(key, &locale)
}

/// The newest welcome row of the athlete's personal thread.
async fn personal_welcome(
    resources: &ServerContext,
    user_id: Uuid,
    tenant: TenantId,
) -> Option<String> {
    let repos = resources.agent.database.repositories();
    let thread = find_personal_thread(&resources.common.repos, &user_id.to_string(), tenant)
        .await
        .unwrap()?;
    repos
        .chat
        .get_messages(&thread.id, &user_id.to_string(), tenant)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(MessageRecord::is_agent_welcome)
        .map(|row| row.content)
}

/// An athlete who holds no agent yet, with a token for their tenant.
async fn bare_athlete() -> (Arc<ServerContext>, axum::Router, String, Uuid, TenantId) {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let router = OnboardingRoutes::routes(Arc::clone(&resources));
    (resources, router, token, user_id, tenant)
}

/// Skip posts the lead's welcome into the personal thread, forged for it, as
/// Start does: the first-run Meet your team turn is never missing. A lead
/// alone says it is the whole team; there is nobody to reach yet.
#[tokio::test]
async fn skip_opens_the_personal_thread_on_the_leads_welcome() {
    let (resources, router, token, user_id, tenant) = bare_athlete().await;
    let lead = published_agent(&resources, user_id, tenant, "Tempo Coach").await;
    assert!(personal_welcome(&resources, user_id, tenant)
        .await
        .is_none());

    assert_eq!(
        put_proposal(
            &router,
            &token,
            json!({ "status": "skipped", "agent_id": lead, "team_agent_ids": [lead] })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let welcome = personal_welcome(&resources, user_id, tenant)
        .await
        .expect("Skip opens the thread on the lead's welcome");
    assert!(welcome.contains("Tempo Coach"), "{welcome}");
    let solo = said(&resources, user_id, KEY_AGENT_WELCOME_TEAM_SOLO).await;
    let discover = said(&resources, user_id, KEY_AGENT_WELCOME_DISCOVER_LINE).await;
    let reach = said(&resources, user_id, KEY_AGENT_WELCOME_REACH_LINE).await;
    assert!(welcome.contains(&solo), "{welcome}");
    assert!(welcome.contains(&discover), "{welcome}");
    assert!(!welcome.contains(&reach), "nobody to reach: {welcome}");
}

/// Start names the rest of the team in the lead's welcome — the members it
/// ticked and the agent the athlete already held — says how to reach them in
/// the app, and a second write posts no second welcome.
#[tokio::test]
async fn start_names_the_team_in_the_leads_welcome_once() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let lead = published_agent(&resources, user_id, tenant, "Tempo Coach").await;
    let fuel = published_agent(&resources, user_id, tenant, "Fuel Coach").await;
    let body = json!({ "status": "complete", "agent_id": lead, "team_agent_ids": [lead, fuel] });

    assert_eq!(
        put_proposal(&router, &token, body.clone()).await,
        StatusCode::NO_CONTENT
    );
    let welcome = personal_welcome(&resources, user_id, tenant)
        .await
        .expect("Start opens the thread on the lead's welcome");
    let and = said(&resources, user_id, KEY_AGENT_WELCOME_LIST_AND).await;
    let names = [
        format_template(&and, &["Fuel Coach", "Endurance"]),
        format_template(&and, &["Endurance", "Fuel Coach"]),
    ];
    let many = said(&resources, user_id, KEY_AGENT_WELCOME_TEAM_MANY).await;
    assert!(
        names
            .iter()
            .any(|names| welcome.contains(&format_template(&many, &[names]))),
        "both teammates, named once: {welcome}"
    );
    let reach = said(&resources, user_id, KEY_AGENT_WELCOME_REACH_LINE).await;
    assert!(
        welcome.contains(&reach),
        "the web app says how to reach a teammate: {welcome}"
    );

    assert_eq!(
        put_proposal(&router, &token, body).await,
        StatusCode::NO_CONTENT
    );
    let thread = find_personal_thread(&resources.common.repos, &user_id.to_string(), tenant)
        .await
        .unwrap()
        .expect("the thread");
    let welcomes = resources
        .agent
        .database
        .repositories()
        .chat
        .get_messages(&thread.id, &user_id.to_string(), tenant)
        .await
        .unwrap()
        .into_iter()
        .filter(MessageRecord::is_agent_welcome)
        .count();
    assert_eq!(welcomes, 1, "the introduction ledger keeps it once");
}
