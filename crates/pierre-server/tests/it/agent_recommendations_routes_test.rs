// ABOUTME: Integration tests for GET /api/agents?personalize=true end-to-end
// ABOUTME: Verifies match_score/recommended wiring, cold start, the rule-picked team, and its catalogue handles
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use common::{
    create_test_server_resources, create_test_server_resources_with_llm, create_test_user,
    generate_test_token,
};
use helpers::agent_fixtures::{publish_catalogue_agent, publish_catalogue_agent_tagged};
use helpers::axum_test::AxumTestRequest;
use helpers::contremaitre_checkout::contremaitre_tree;
use helpers::recording_llm::RecordingProvider;
use pierre_agent_parser::{parse_frontmatter, AgentFrontmatter};
use pierre_core::models::agents::{Agent, AgentCategory, CreateAgentRequest};
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::agents::{AgentProposalResponse, ListAgentsResponse};
use pierre_routes_agents::build_agents_router;
use pierre_services::about_you::{persist_about_you, AboutYouAnswers};
use pierre_services::default_agent::{
    FUELLING_AGENT_HANDLE, MOBILITY_AGENT_HANDLE, RECOVERY_AGENT_HANDLE,
};
use pierre_services::team_rules::{TeamRule, COLD_START_LEAD_ORDER, RED_S_AGENT_HANDLE};

use axum::http::StatusCode;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use uuid::Uuid;

/// Build the agents router for a fresh user with no connected provider
/// (the cold-start case for personalization), and store one agent for it.
async fn setup_with_agent(title: &str) -> (axum::Router, String) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = generate_test_token(&resources, &user).await;
    seed_agent(
        &resources,
        user_id,
        json!({ "title": title, "system_prompt": "You are a fitness coach." }),
    )
    .await;
    let router = build_agents_router::<ServerContext>().with_state(resources);
    (router, format!("Bearer {token}"))
}

/// Store an agent for `user_id` in their tenant, through the repository every
/// creation surface writes with.
async fn seed_agent(resources: &Arc<ServerContext>, user_id: Uuid, body: serde_json::Value) {
    let repos = resources.agent.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .map(|t| t.id)
        .expect("test user owns a tenant");
    let request: CreateAgentRequest = serde_json::from_value(body).unwrap();
    repos
        .agents
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
}

/// With `personalize=true`, every agent is tagged with `match_score` and
/// `recommended`. For a fresh user (no provider, no activities) an
/// empty-prerequisite agent is a provider-free starter, so it is recommended
/// at the sport-agnostic base score.
#[tokio::test]
async fn personalize_true_tags_agents_with_score_and_recommended() {
    let (router, auth) = setup_with_agent("Personalize Coach").await;

    let response = AxumTestRequest::get("/api/agents?personalize=true")
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);

    let list: ListAgentsResponse = response.json();
    let agent = list
        .agents
        .iter()
        .find(|c| c.title == "Personalize Coach")
        .expect("created coach should be listed");

    assert_eq!(
        agent.recommended,
        Some(true),
        "empty-prereq coach is a cold-start starter"
    );
    let score = agent.match_score.expect("personalize sets match_score");
    // Sport-agnostic base score (COACH_REC_SPORT_AGNOSTIC_BASE_SCORE default).
    assert!(
        (score - 0.5).abs() < f32::EPSILON,
        "expected starter base score 0.5, got {score}"
    );
}

/// Without `personalize`, the recommendation fields are omitted entirely
/// (serde skips `None`), so the default agent list is unchanged.
#[tokio::test]
async fn personalize_absent_omits_recommendation_fields() {
    let (router, auth) = setup_with_agent("Plain Coach").await;

    let response = AxumTestRequest::get("/api/agents")
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);

    let list: ListAgentsResponse = response.json();
    let agent = list
        .agents
        .iter()
        .find(|c| c.title == "Plain Coach")
        .expect("created coach should be listed");

    assert!(agent.match_score.is_none(), "no score without personalize");
    assert!(agent.recommended.is_none(), "no flag without personalize");
}

/// An agent carrying the `coach-tool` tag is a coach-facing builder: it stays
/// hidden from a default (athlete) user's library and only appears once the
/// user switches to the Coach persona. Gates the library list for both the
/// `/api/agents` and `/api/agents/proposal` paths, which share the same
/// `user_sees_coach_tools` filter.
#[tokio::test]
async fn coach_tool_tagged_coach_hidden_from_athletes_shown_to_coaches() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = generate_test_token(&resources, &user).await;
    let auth = format!("Bearer {token}");
    let db = resources.agent.database.clone();

    // A coach-facing builder (carries the coach-tool tag).
    seed_agent(
        &resources,
        user_id,
        json!({
            "title": "Taper Builder",
            "system_prompt": "You are a taper builder.",
            "tags": ["coach-tool", "taper"],
        }),
    )
    .await;
    let router = build_agents_router::<ServerContext>().with_state(resources);

    // Athlete (default Casual persona) must not see the agent-facing builder.
    let list: ListAgentsResponse = AxumTestRequest::get("/api/agents")
        .header("authorization", &auth)
        .send(router.clone())
        .await
        .json();
    assert!(
        !list.agents.iter().any(|c| c.title == "Taper Builder"),
        "athlete must not see a coach-facing builder"
    );

    // Recording the coach role surfaces the builder.
    db.repositories()
        .users
        .set_coaches_others(user_id, true)
        .await
        .unwrap();
    let list: ListAgentsResponse = AxumTestRequest::get("/api/agents")
        .header("authorization", &auth)
        .send(router)
        .await
        .json();
    assert!(
        list.agents.iter().any(|c| c.title == "Taper Builder"),
        "a user who coaches others should see the coach-facing builder"
    );
}

// ============================================================================
// The proposal's team (carnet#1097)
// ============================================================================

/// A fresh athlete with no provider, and their tenant.
async fn athlete(resources: &Arc<ServerContext>) -> (Uuid, TenantId, String) {
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = generate_test_token(resources, &user).await;
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;
    (user_id, tenant, format!("Bearer {token}"))
}

/// Publish a catalogue agent and check it took `handle`, which the team rules
/// key on.
async fn catalogue_agent(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    title: &str,
    handle: &str,
) -> String {
    let repos = resources.agent.database.repositories();
    let id = publish_catalogue_agent(repos, user_id, tenant, title, "Test.").await;
    let published = repos
        .store_listings
        .get_published_agent(&id.to_string())
        .await
        .unwrap()
        .expect("published");
    assert_eq!(
        published.agent.handle.as_deref(),
        Some(handle),
        "fixture precondition"
    );
    id.to_string()
}

/// The three rule-table agents, published in reverse of the cold-start order
/// so the lead cannot be an accident of creation order.
async fn rule_agents(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> (String, String, String) {
    let mobility = catalogue_agent(
        resources,
        user_id,
        tenant,
        "Mobility Agent",
        MOBILITY_AGENT_HANDLE,
    )
    .await;
    let recovery = catalogue_agent(
        resources,
        user_id,
        tenant,
        "Recovery Agent",
        RECOVERY_AGENT_HANDLE,
    )
    .await;
    let fuelling = catalogue_agent(
        resources,
        user_id,
        tenant,
        "Fuelling Agent",
        FUELLING_AGENT_HANDLE,
    )
    .await;
    (fuelling, recovery, mobility)
}

async fn set_goal(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId, goal: &str) {
    let written = persist_about_you(
        resources.agent.database.repositories().memory.as_ref(),
        tenant,
        &user_id.to_string(),
        &AboutYouAnswers {
            goal: Some(goal.to_owned()),
            ..AboutYouAnswers::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(written, 1);
}

async fn proposal(resources: &Arc<ServerContext>, auth: &str) -> AgentProposalResponse {
    let router = build_agents_router::<ServerContext>().with_state(Arc::clone(resources));
    let response = AxumTestRequest::get("/api/agents/proposal")
        .header("authorization", auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    response.json()
}

/// An athlete the proposal knows nothing about is led by Fuelling — not by
/// whichever starter has the smallest id. With no signal — and no provider,
/// so no sleep source — nobody joins beside the lead.
#[tokio::test]
async fn a_cold_start_is_led_by_fuelling_and_earns_no_member() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant, auth) = athlete(&resources).await;
    let (fuelling, _, _) = rule_agents(&resources, user_id, tenant).await;

    let body = proposal(&resources, &auth).await;
    let team = body.team.expect("a team is proposed");
    assert_eq!(
        team.lead.agent.id, fuelling,
        "a cold start leads with Fuelling"
    );
    assert_eq!(body.agents[0].agent.id, fuelling, "the lead is agents[0]");
    assert!(
        team.members.is_empty(),
        "no signal earns a member, and without a provider there is no sleep source"
    );
    assert!(body.goal.is_none());
}

/// A goal puts Fuelling on the team, ticked, with its rule's reason; the lead
/// is then the next in the cold-start order, and never repeated as a member.
/// No provider means no Recovery member.
#[tokio::test]
async fn a_goal_puts_fuelling_on_the_team_beside_the_lead() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant, auth) = athlete(&resources).await;
    let (fuelling, recovery, _) = rule_agents(&resources, user_id, tenant).await;
    set_goal(&resources, user_id, tenant, "A gran fondo in June").await;

    let body = proposal(&resources, &auth).await;
    let team = body.team.expect("a team is proposed");
    assert_eq!(team.lead.agent.id, recovery);
    assert_eq!(team.members.len(), 1, "{:?}", team.members);
    let member = &team.members[0];
    assert_eq!(member.agent.id, fuelling);
    assert_eq!(member.rule, TeamRule::Goal);
    assert!(member.preselected);
    assert!(member.source.is_none());
    assert!(
        !member.reason.trim().is_empty(),
        "the rule's own reason is rendered"
    );
    assert!(
        team.members
            .iter()
            .all(|m| m.agent.id != team.lead.agent.id),
        "the lead is never a member"
    );
    assert_eq!(body.goal.as_deref(), Some("A gran fondo in June"));
}

/// The re-rank call writes the members' reasons and never makes a member the
/// lead, even when it lists the member first.
#[tokio::test]
async fn the_llm_writes_member_reasons_and_never_leads_with_a_member() {
    let llm = Arc::new(RecordingProvider::answering("[]"));
    let resources = create_test_server_resources_with_llm(llm.clone())
        .await
        .unwrap();
    let (user_id, tenant, auth) = athlete(&resources).await;
    let (fuelling, recovery, _) = rule_agents(&resources, user_id, tenant).await;
    set_goal(&resources, user_id, tenant, "A gran fondo in June").await;
    llm.answer_with(
        &json!([
            { "id": fuelling, "reason": "Plans what you eat on long days." },
            { "id": recovery, "reason": "Reads how you recover." },
        ])
        .to_string(),
    );

    let body = proposal(&resources, &auth).await;
    let team = body.team.expect("a team is proposed");
    assert_eq!(team.lead.agent.id, recovery);
    assert_eq!(team.lead.reason, "Reads how you recover.");
    assert_eq!(team.members.len(), 1);
    assert_eq!(team.members[0].agent.id, fuelling);
    assert_eq!(team.members[0].reason, "Plans what you eat on long days.");
    let asked = llm.requests_since(0);
    assert!(
        asked
            .iter()
            .any(|request| request.conversation.contains("Team members already chosen")),
        "one re-rank call lists the members"
    );
}

/// A coach-facing agent is never put on an athlete's team, whatever rule
/// would pick it.
#[tokio::test]
async fn a_coach_facing_agent_is_never_a_team_member() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant, auth) = athlete(&resources).await;
    let repos = resources.agent.database.repositories();
    let coach_tool = publish_catalogue_agent_tagged(
        repos,
        user_id,
        tenant,
        "Fuelling Agent",
        "Test.",
        AgentCategory::Nutrition,
        vec![Agent::COACH_TOOL_TAG.to_owned()],
    )
    .await
    .to_string();
    catalogue_agent(
        &resources,
        user_id,
        tenant,
        "Recovery Agent",
        RECOVERY_AGENT_HANDLE,
    )
    .await;
    set_goal(&resources, user_id, tenant, "A gran fondo in June").await;

    let body = proposal(&resources, &auth).await;
    let team = body.team.expect("a team is proposed");
    assert!(
        team.members.iter().all(|m| m.agent.id != coach_tool),
        "a coach tool never joins an athlete's team"
    );
    assert_ne!(team.lead.agent.id, coach_tool);
    assert!(!body.agents.iter().any(|a| a.agent.id == coach_tool));
}

/// Every agent in the contremaitre catalogue, by handle: the `en.md`
/// frontmatter of each `prompts/agents/<category>/<slug>/`.
fn catalogue_agents() -> HashMap<String, AgentFrontmatter> {
    // A catalogue scan: the agent catalogue is files in contremaitre that the
    // seeder reads at deploy time, never compiled into a build, so the files
    // are the data under test.
    let agents_dir = contremaitre_tree().join("prompts/agents");
    let mut agents = HashMap::new();
    for category in fs::read_dir(&agents_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", agents_dir.display()))
        .filter_map(Result::ok)
    {
        let Ok(slugs) = fs::read_dir(category.path()) else {
            continue;
        };
        for slug in slugs.filter_map(Result::ok) {
            let en = slug.path().join("en.md");
            let Ok(text) = fs::read_to_string(&en) else {
                continue;
            };
            let frontmatter =
                parse_frontmatter(&text).unwrap_or_else(|e| panic!("{} parses: {e}", en.display()));
            agents.insert(frontmatter.name.clone(), frontmatter);
        }
    }
    agents
}

/// The rule table and the cold start name catalogue agents by handle
/// (carnet#1097): a contremaitre rename must fail here, on the bump lane,
/// instead of silently dropping a member. Every cold-start lead is
/// provider-free, so an athlete with nothing connected can be given it; RED-S,
/// which never leads one, is catalogued under the handle the rules exclude.
#[test]
fn the_catalogue_holds_the_team_agents_and_each_needs_no_provider() {
    let agents = catalogue_agents();
    for handle in [
        FUELLING_AGENT_HANDLE,
        RECOVERY_AGENT_HANDLE,
        MOBILITY_AGENT_HANDLE,
        RED_S_AGENT_HANDLE,
    ] {
        assert!(
            agents.contains_key(handle),
            "the contremaitre catalogue holds {handle}"
        );
    }
    for handle in COLD_START_LEAD_ORDER {
        let agent = agents
            .get(handle)
            .unwrap_or_else(|| panic!("the contremaitre catalogue holds {handle}"));
        assert!(
            agent.prerequisites.providers.is_empty(),
            "{handle} needs no provider, so a cold start can be given it: {:?}",
            agent.prerequisites.providers
        );
    }
}
