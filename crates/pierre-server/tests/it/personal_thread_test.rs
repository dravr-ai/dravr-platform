// ABOUTME: Integration tests for the athlete's one personal thread — resolved on the server, divided by topics and /reset
// ABOUTME: Pins adoption and its divider, the cap-exempt forge, the race, topics, the rebind, the share refusal, and new chats beside it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#840, Personal Home step 2. The server names the athlete's one
//! thread, and Home, `/reset`, onboarding's Start and the coach's own posts
//! all land in it. A new chat once it exists is a conversation of its own,
//! and leaves it untouched (carnet#1473).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_chat_pipeline::stages::persistence::get_conversation_history;
use pierre_core::models::agents::{Agent, AgentCategory, CreateAgentRequest};
use pierre_core::models::{
    AddMessageParams, ConnectionType, Tenant, TenantId, User, UserStatus,
    AGENT_WELCOME_FINISH_REASON, DIVIDER_FINISH_REASON,
};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatRoutes, ConversationListResponse, ConversationResponse, MessageResponse,
    MessagesListResponse, OpenTopicResponse, PersonalThreadResponse, TurnResponse,
};
use pierre_services::personal_thread::{resolve_dm_thread, resolve_personal_thread, PersonalForge};
use pierre_test_support::db::create_concurrent_test_db;
use serde_json::json;
use tokio::task::spawn_blocking;
use tokio::time;
use uuid::Uuid;

/// An Active athlete with their own tenant, a provider that clears the
/// onboarding gate, and a bearer token for the chat routes.
async fn seed_athlete(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId, String) {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();
    let mut user = User::new(email.to_owned(), password_hash, Some("Personal".to_owned()));
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Personal Tenant".to_owned(),
        slug: format!("personal-{tenant_id}"),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    (user_id, tenant_id, format!("Bearer {token}"))
}

/// An agent the athlete owns, so it is one they can see and be welcomed by.
async fn seed_agent(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
) -> String {
    let request = CreateAgentRequest {
        title: title.to_owned(),
        description: Some("Threshold work, one week at a time.".to_owned()),
        system_prompt: "You are a test coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec!["Plan my tempo week".to_owned()],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    let agent = resources
        .common
        .repos
        .agents
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    agent.id.to_string()
}

/// A plain thread created the way the app's REST create makes one.
async fn create_thread(router: axum::Router, auth: &str, title: &str) -> String {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", auth)
        .json(&json!({ "title": title }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let conversation: ConversationResponse = resp.json();
    conversation.id
}

/// What `GET /api/chat/personal` finds; `None` before the athlete has one.
async fn get_personal(router: axum::Router, auth: &str) -> Option<ConversationResponse> {
    let resp = AxumTestRequest::get("/api/chat/personal")
        .header("authorization", auth)
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: PersonalThreadResponse = resp.json();
    body.conversation
}

async fn open_topic(
    router: axum::Router,
    auth: &str,
    body: serde_json::Value,
) -> OpenTopicResponse {
    let resp = AxumTestRequest::post("/api/chat/personal/topics")
        .header("authorization", auth)
        .json(&body)
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    resp.json()
}

async fn messages(router: axum::Router, auth: &str, conversation_id: &str) -> Vec<MessageResponse> {
    let resp = AxumTestRequest::get(&format!(
        "/api/chat/conversations/{conversation_id}/messages"
    ))
    .header("authorization", auth)
    .send(router)
    .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: MessagesListResponse = resp.json();
    body.messages
}

/// Write one turn of the athlete's straight into a thread, as history.
async fn say(
    resources: &Arc<ServerContext>,
    conversation_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    role: &str,
    content: &str,
) {
    resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id,
            user_id: &user_id.to_string(),
            role,
            content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap();
}

fn dividers(rows: &[MessageResponse]) -> usize {
    rows.iter()
        .filter(|row| row.finish_reason.as_deref() == Some(DIVIDER_FINISH_REASON))
        .count()
}

/// The words the model reads of `thread` on its next turn.
async fn prompt_history(
    resources: &Arc<ServerContext>,
    thread: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<String> {
    get_conversation_history(
        resources.common.repos.chat.as_ref(),
        thread,
        &user_id.to_string(),
        tenant_id,
        160,
        &[],
    )
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.content)
    .collect()
}

#[tokio::test]
async fn the_newest_own_thread_is_adopted_and_a_shared_one_is_passed_over() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "adopt@test.com").await;
    let (friend_id, _, _) = seed_athlete(&resources, "friend@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let older = create_thread(router.clone(), &auth, "Older").await;
    say(
        &resources,
        &older,
        user_id,
        tenant_id,
        "user",
        "an old question",
    )
    .await;
    let newer = create_thread(router.clone(), &auth, "Newer").await;
    say(
        &resources,
        &newer,
        user_id,
        tenant_id,
        "user",
        "a newer question",
    )
    .await;
    // Newest of all, but somebody else reads it too: it is not theirs alone.
    let shared = create_thread(router.clone(), &auth, "Shared").await;
    resources
        .common
        .repos
        .chat
        .add_participant(
            &shared,
            tenant_id,
            &friend_id.to_string(),
            &user_id.to_string(),
        )
        .await
        .unwrap();
    say(
        &resources,
        &shared,
        user_id,
        tenant_id,
        "user",
        "the newest question",
    )
    .await;

    let personal = get_personal(router.clone(), &auth).await.unwrap();
    assert_eq!(personal.id, newer, "the newest thread that is theirs alone");
    assert!(personal.personal);

    // Asked again, the designation holds: activity elsewhere moves nothing.
    say(
        &resources,
        &older,
        user_id,
        tenant_id,
        "user",
        "back to the old one",
    )
    .await;
    assert_eq!(get_personal(router.clone(), &auth).await.unwrap().id, newer);

    let resp = AxumTestRequest::get("/api/chat/conversations")
        .header("authorization", &auth)
        .send(router)
        .await;
    let list: ConversationListResponse = resp.json();
    let flagged: Vec<&str> = list
        .conversations
        .iter()
        .filter(|row| row.personal)
        .map(|row| row.id.as_str())
        .collect();
    assert_eq!(flagged, [newer.as_str()], "one row carries the flag");
}

#[tokio::test]
async fn a_thread_the_phone_opened_is_the_athletes_own_too() {
    // A `/reset` typed on the phone stamps the thread it forges `mobile`; it
    // is as much the athlete's own in-app thread as one the web opened.
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "phone@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let web = create_thread(router.clone(), &auth, "Web").await;
    say(&resources, &web, user_id, tenant_id, "user", "from the web").await;
    let phone = create_thread(router.clone(), &auth, "Phone").await;
    assert!(resources
        .common
        .repos
        .chat
        .set_conversation_channel(&phone, &user_id.to_string(), tenant_id, "mobile")
        .await
        .unwrap());
    say(
        &resources,
        &phone,
        user_id,
        tenant_id,
        "user",
        "from the phone",
    )
    .await;

    assert_eq!(get_personal(router, &auth).await.unwrap().id, phone);
}

#[tokio::test]
async fn looking_forges_nothing_and_a_first_chat_forges_one_thread_outside_the_cap() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "forge@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    assert!(
        get_personal(router.clone(), &auth).await.is_none(),
        "Home opens on its empty pane; a look forges nothing"
    );

    let opened = open_topic(router.clone(), &auth, json!({})).await;
    assert!(opened.conversation.personal);
    assert!(
        opened.divider.is_none(),
        "a fresh thread is its own first topic"
    );
    assert_eq!(
        get_personal(router, &auth).await.unwrap().id,
        opened.conversation.id,
        "the same thread from then on"
    );
    assert_eq!(
        resources
            .common
            .repos
            .chat
            .count_conversations(&user_id.to_string(), tenant_id)
            .await
            .unwrap(),
        0,
        "the personal thread holds no slot of the cap the athlete manages"
    );
}

/// Rounds of the race below, each a first request for the athlete again.
const RACE_ROUNDS: u32 = 20;

/// How much later than the round before the last racer arrives. Across the
/// rounds it lands everywhere in the first ones' forge, including the window
/// between a forged thread's insert and its designation, where it finds that
/// thread and adopts it: the forging request then keeps the thread it was
/// refused, rather than removing it as a losing forge from under the racer
/// that adopted it (carnet#1557).
const RACE_STAGGER_STEP: Duration = Duration::from_millis(1);

#[tokio::test]
async fn first_requests_racing_each_other_settle_on_one_thread() {
    // Connections that overlap in the engine, as a deployed pool's do: the
    // default one-connection SQLite pool queues the racers one statement at a
    // time and never reaches the window above.
    let database = create_concurrent_test_db().await.unwrap();
    let (user_id, _, tenant_id) = common::create_test_user_with_plan(
        &database,
        &format!("race-{}@test.com", Uuid::new_v4()),
        "professional",
    )
    .await
    .unwrap();
    let user = user_id.to_string();
    let repos = database.repositories();
    let llm_chain = common::placeholder_llm_chain();

    let forge = PersonalForge {
        title_fallback: "Fallback",
        guided_flow: false,
        llm_chain: &llm_chain,
    };
    for round in 0..RACE_ROUNDS {
        let raced: [_; 3] = tokio::join!(
            resolve_personal_thread(repos, &user, tenant_id, forge),
            resolve_personal_thread(repos, &user, tenant_id, forge),
            async {
                time::sleep(RACE_STAGGER_STEP * round).await;
                resolve_personal_thread(repos, &user, tenant_id, forge).await
            },
        )
        .into();
        let threads = raced
            .map(|thread| thread.unwrap_or_else(|e| panic!("round {round}: a racer failed: {e}")));
        let ids = threads.each_ref().map(|thread| &thread.conversation.id);
        assert!(
            ids.iter().all(|id| *id == ids[0]),
            "round {round}: every racer is given the one personal thread: {ids:?}"
        );
        assert!(
            threads.iter().filter(|thread| thread.forged).count() <= 1,
            "round {round}: only the request whose forge became the thread may treat it as its own"
        );
        let listed = repos
            .chat
            .list_conversations(&user, tenant_id, 50, 0, TransportPolicy::FirstPartyOnly)
            .await
            .unwrap()
            .items;
        assert_eq!(
            listed.len(),
            1,
            "round {round}: the losing forges were removed"
        );
        assert_eq!(
            listed[0].id, *ids[0],
            "round {round}: the thread left is the one every racer was given"
        );
        // The athlete is back to no thread at all, so the next round is a
        // first request again.
        assert!(
            repos
                .chat
                .delete_conversation(ids[0], &user, tenant_id)
                .await
                .unwrap(),
            "round {round}: the athlete owns their thread"
        );
    }
}

#[tokio::test]
async fn a_topic_opens_with_a_rule_and_never_a_second_on_an_empty_one() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "plus@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let thread = create_thread(router.clone(), &auth, "Mine").await;
    // Adopted while empty, so the turns below are the topic's own.
    assert_eq!(
        get_personal(router.clone(), &auth).await.unwrap().id,
        thread
    );
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "user",
        "How was my week?",
    )
    .await;
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "assistant",
        "Solid: 62 km.",
    )
    .await;

    let opened = open_topic(router.clone(), &auth, json!({})).await;
    assert_eq!(
        opened.conversation.id, thread,
        "the topic opens in the athlete's thread"
    );
    assert!(opened.divider.is_some());
    let count_before = resources
        .common
        .repos
        .chat
        .count_conversations(&user_id.to_string(), tenant_id)
        .await
        .unwrap();

    // Nothing said since: a second topic draws no second rule.
    let again = open_topic(router.clone(), &auth, json!({})).await;
    assert!(again.divider.is_none());
    let rows = messages(router, &auth, &thread).await;
    assert_eq!(dividers(&rows), 1);
    assert_eq!(
        resources
            .common
            .repos
            .chat
            .count_conversations(&user_id.to_string(), tenant_id)
            .await
            .unwrap(),
        count_before,
        "no thread was created"
    );

    // The model reads the topic afresh: nothing before the divider.
    let history = prompt_history(&resources, &thread, user_id, tenant_id).await;
    assert!(
        history.iter().all(|row| row != "How was my week?"),
        "{history:?}"
    );
}

#[tokio::test]
async fn a_topic_with_a_specialist_rebinds_the_thread_and_its_agent_opens_it() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "specialist@test.com").await;
    let agent_id = seed_agent(&resources, user_id, tenant_id, "Tempo Coach").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let thread = create_thread(router.clone(), &auth, "Mine").await;
    assert_eq!(
        get_personal(router.clone(), &auth).await.unwrap().id,
        thread
    );
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "user",
        "How was my week?",
    )
    .await;

    let opened = open_topic(router.clone(), &auth, json!({ "agent_id": agent_id })).await;
    assert_eq!(
        opened.conversation.agent_id.as_deref(),
        Some(agent_id.as_str())
    );
    let divider = opened.divider.expect("the topic opened with a rule");

    let rows = messages(router, &auth, &thread).await;
    let after: Vec<&MessageResponse> = rows
        .iter()
        .skip_while(|row| row.id != divider.id)
        .skip(1)
        .collect();
    assert!(
        after
            .iter()
            .any(|row| row.finish_reason.as_deref() == Some(AGENT_WELCOME_FINISH_REASON)),
        "the specialist welcomes the topic it was bound to: {rows:?}"
    );
}

#[tokio::test]
async fn the_threads_own_agent_opens_each_topic_once() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "each-topic@test.com").await;
    let agent_id = seed_agent(&resources, user_id, tenant_id, "Tempo Coach").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    // Bound to the agent from the start, so its welcome opened the thread.
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &auth)
        .json(&json!({ "title": "Mine", "agent_id": agent_id }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let created: ConversationResponse = resp.json();
    let thread = created.id;
    // Adopted on its welcome alone, which is no turn: no rule yet.
    assert_eq!(
        get_personal(router.clone(), &auth).await.unwrap().id,
        thread
    );
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "user",
        "How was my week?",
    )
    .await;

    // The introduction ledger is keyed by the topic, so the same agent opens
    // the new one as it opened the thread — and only once, however often a
    // topic is opened before anyone speaks.
    let divider = open_topic(router.clone(), &auth, json!({}))
        .await
        .divider
        .expect("the topic opened with a rule");
    assert!(open_topic(router.clone(), &auth, json!({}))
        .await
        .divider
        .is_none());

    let rows = messages(router, &auth, &thread).await;
    let divider_at = rows.iter().position(|row| row.id == divider.id).unwrap();
    let welcomes: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.finish_reason.as_deref() == Some(AGENT_WELCOME_FINISH_REASON))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(welcomes.len(), 2, "one welcome per topic: {rows:?}");
    assert!(
        welcomes[0] < divider_at && divider_at < welcomes[1],
        "{rows:?}"
    );
}

#[tokio::test]
async fn reset_in_the_personal_thread_opens_a_topic_in_place() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "reset@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let thread = create_thread(router.clone(), &auth, "Mine").await;
    let personal = get_personal(router.clone(), &auth).await.unwrap();
    assert_eq!(personal.id, thread);
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "user",
        "How was my week?",
    )
    .await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{thread}/messages"))
        .header("authorization", &auth)
        .json(&json!({ "content": "/reset" }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let turn: TurnResponse = resp.json();
    assert!(turn.rotated_to_conversation_id.is_none(), "nobody moves");
    let divider_id = turn
        .opened_topic_divider_id
        .expect("the turn names the topic it opened");

    let record = resources
        .common
        .repos
        .chat
        .get_conversation(&thread, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        record.archived_at.is_none(),
        "the personal thread is never archived"
    );
    let rows = messages(router.clone(), &auth, &thread).await;
    assert_eq!(dividers(&rows), 1);
    assert!(rows.iter().any(|row| row.id == divider_id));
    assert_eq!(
        get_personal(router, &auth).await.unwrap().id,
        thread,
        "still the athlete's thread"
    );
}

#[tokio::test]
async fn reset_anywhere_else_still_rotates() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "rotate@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let personal = open_topic(router.clone(), &auth, json!({}))
        .await
        .conversation;
    let other = create_thread(router.clone(), &auth, "Another").await;
    assert_ne!(other, personal.id);
    say(&resources, &other, user_id, tenant_id, "user", "a question").await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{other}/messages"))
        .header("authorization", &auth)
        .json(&json!({ "content": "/reset" }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let turn: TurnResponse = resp.json();
    assert!(turn.rotated_to_conversation_id.is_some());
    assert!(turn.opened_topic_divider_id.is_none());
}

#[tokio::test]
async fn the_personal_thread_cannot_be_shared() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, _, auth) = seed_athlete(&resources, "share@test.com").await;
    let (friend_id, _, _) = seed_athlete(&resources, "share-friend@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let personal = open_topic(router.clone(), &auth, json!({}))
        .await
        .conversation;

    let resp = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{}/participants",
        personal.id
    ))
    .header("authorization", &auth)
    .json(&json!({ "user_id": friend_id.to_string() }))
    .send(router)
    .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_topic_after_a_reset_draws_no_second_rule_over_the_reset_alone() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "reset-then-plus@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let thread = create_thread(router.clone(), &auth, "Mine").await;
    // Home has opened it while empty, so it is the athlete's personal thread
    // and the rule below is the one `/reset` draws.
    assert_eq!(
        get_personal(router.clone(), &auth).await.unwrap().id,
        thread
    );
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "user",
        "How was my week?",
    )
    .await;
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "assistant",
        "Solid: 62 km.",
    )
    .await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{thread}/messages"))
        .header("authorization", &auth)
        .json(&json!({ "content": "/reset" }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);

    // The topic holds only the `/reset` exchange, which the model never reads:
    // it is still the fresh start its rule marked.
    let again = open_topic(router.clone(), &auth, json!({})).await;
    assert!(again.divider.is_none());
    assert_eq!(dividers(&messages(router, &auth, &thread).await), 1);
}

/// An agent written for coaches, owned by the athlete so they can read it.
async fn seed_coach_facing_agent(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> String {
    let request: CreateAgentRequest = serde_json::from_value(json!({
        "title": "Roster Builder",
        "system_prompt": "Plan the coach's roster.",
        "category": "training",
        "tags": [Agent::COACH_TOOL_TAG]
    }))
    .unwrap();
    resources
        .common
        .repos
        .agents
        .create(user_id, tenant_id, &request)
        .await
        .unwrap()
        .id
        .to_string()
}

/// carnet#910: a topic with a coach-facing agent is refused to an athlete,
/// as `/agent add` refuses it, and the thread keeps the agent it had.
#[tokio::test]
async fn a_topic_never_rebinds_an_athletes_thread_to_a_coach_facing_agent() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "athlete-910@test.com").await;
    let own = seed_agent(&resources, user_id, tenant_id, "Tempo Coach").await;
    let coach_agent = seed_coach_facing_agent(&resources, user_id, tenant_id).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    open_topic(router.clone(), &auth, json!({ "agent_id": own })).await;

    let resp = AxumTestRequest::post("/api/chat/personal/topics")
        .header("authorization", &auth)
        .json(&json!({ "agent_id": coach_agent }))
        .send(router.clone())
        .await;

    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    let thread = get_personal(router, &auth)
        .await
        .expect("the thread exists");
    assert_eq!(thread.agent_id.as_deref(), Some(own.as_str()));
}

/// A coach may bind the same agent: it is written for them.
#[tokio::test]
async fn a_topic_rebinds_a_coachs_thread_to_a_coach_facing_agent() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "coach-910@test.com").await;
    resources
        .common
        .repos
        .users
        .set_coaches_others(user_id, true)
        .await
        .unwrap();
    let coach_agent = seed_coach_facing_agent(&resources, user_id, tenant_id).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let opened = open_topic(router, &auth, json!({ "agent_id": coach_agent })).await;

    assert_eq!(
        opened.conversation.agent_id.as_deref(),
        Some(coach_agent.as_str())
    );
}

/// carnet#1473: an ordinary chat adopted as the personal thread opens on a
/// rule, so the coach does not answer an old exchange as current context.
#[tokio::test]
async fn adopting_a_thread_with_turns_opens_it_on_one_rule() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "adopt-rule@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let thread = create_thread(router.clone(), &auth, "September").await;
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "user",
        "Taper for the September race?",
    )
    .await;
    say(
        &resources,
        &thread,
        user_id,
        tenant_id,
        "assistant",
        "Cut volume by a third.",
    )
    .await;

    assert_eq!(
        get_personal(router.clone(), &auth).await.unwrap().id,
        thread
    );
    let rows = messages(router.clone(), &auth, &thread).await;
    assert_eq!(dividers(&rows), 1, "{rows:?}");
    assert!(
        rows.iter()
            .any(|row| row.content == "Taper for the September race?"),
        "the old exchange stays readable above the rule"
    );
    let history = prompt_history(&resources, &thread, user_id, tenant_id).await;
    assert!(
        history
            .iter()
            .all(|row| row != "Taper for the September race?" && row != "Cut volume by a third."),
        "{history:?}"
    );

    // Asked again, on Home and by a writer, the designation holds and no
    // second rule is drawn.
    assert_eq!(
        get_personal(router.clone(), &auth).await.unwrap().id,
        thread
    );
    let forge = PersonalForge {
        title_fallback: "Fallback",
        guided_flow: false,
        llm_chain: &resources.common.config.llm_chain,
    };
    let resolved = resolve_personal_thread(
        &resources.common.repos,
        &user_id.to_string(),
        tenant_id,
        forge,
    )
    .await
    .unwrap();
    assert_eq!(resolved.conversation.id, thread);
    assert!(!resolved.forged);
    assert_eq!(dividers(&messages(router, &auth, &thread).await), 1);
}

#[tokio::test]
async fn adopting_an_empty_thread_draws_no_rule() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, _, auth) = seed_athlete(&resources, "adopt-empty@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let thread = create_thread(router.clone(), &auth, "Blank").await;

    assert_eq!(
        get_personal(router.clone(), &auth).await.unwrap().id,
        thread
    );
    assert_eq!(dividers(&messages(router, &auth, &thread).await), 0);
}

/// Only the call that designates a thread is told it did: a second
/// designation of the same thread — a racing first request — answers false,
/// so the adoption's divider is written once.
#[tokio::test]
async fn designating_the_personal_thread_again_is_not_a_second_adoption() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "designate-twice@test.com").await;
    let user = user_id.to_string();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let thread = create_thread(router, &auth, "Once").await;
    let chat = &resources.common.repos.chat;

    assert!(chat
        .designate_personal_conversation(&thread, &user, tenant_id)
        .await
        .unwrap());
    assert!(!chat
        .designate_personal_conversation(&thread, &user, tenant_id)
        .await
        .unwrap());
}

/// The DM's own thread, adopted with its Telegram turns, carries on as the
/// personal thread: it is the conversation the athlete is having right now, so
/// no rule cuts the coach off from it.
#[tokio::test]
async fn a_dm_adopting_its_own_thread_keeps_the_conversation_going() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "dm-own@test.com").await;
    let user = user_id.to_string();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let dm = create_thread(router.clone(), &auth, "On Telegram").await;
    assert!(resources
        .common
        .repos
        .chat
        .set_conversation_channel(&dm, &user, tenant_id, "telegram")
        .await
        .unwrap());
    say(
        &resources,
        &dm,
        user_id,
        tenant_id,
        "user",
        "Sore calf after the long run",
    )
    .await;
    say(
        &resources,
        &dm,
        user_id,
        tenant_id,
        "assistant",
        "Easy days until it settles.",
    )
    .await;

    let forge = PersonalForge {
        title_fallback: "Fallback",
        guided_flow: false,
        llm_chain: &resources.common.config.llm_chain,
    };
    let thread = resolve_dm_thread(&resources.common.repos, &user, tenant_id, Some(&dm), forge)
        .await
        .unwrap();
    assert_eq!(thread.conversation.id, dm, "the Telegram history is kept");
    assert!(
        thread.conversation.personal,
        "the DM's own thread becomes the personal thread"
    );
    assert_eq!(
        dividers(&messages(router, &auth, &dm).await),
        0,
        "the DM's own thread carries on with no rule across it"
    );
    let history = prompt_history(&resources, &dm, user_id, tenant_id).await;
    assert!(
        history
            .iter()
            .any(|row| row == "Sore calf after the long run"),
        "the coach still reads the exchange in progress: {history:?}"
    );
}

/// A DM that finds a newer in-app chat than its own thread adopts that chat,
/// and that chat opens on a rule: the DM never wrote in it, so its rows are
/// not the athlete's current topic.
#[tokio::test]
async fn a_dm_adopting_an_in_app_chat_opens_it_on_a_rule() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "dm-rule@test.com").await;
    let user = user_id.to_string();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let dm = create_thread(router.clone(), &auth, "On Telegram").await;
    assert!(resources
        .common
        .repos
        .chat
        .set_conversation_channel(&dm, &user, tenant_id, "telegram")
        .await
        .unwrap());
    let in_app = create_thread(router.clone(), &auth, "Half marathon").await;
    say(
        &resources,
        &in_app,
        user_id,
        tenant_id,
        "user",
        "A half in eight weeks",
    )
    .await;
    say(
        &resources,
        &in_app,
        user_id,
        tenant_id,
        "assistant",
        "Then a tempo run each week.",
    )
    .await;

    let forge = PersonalForge {
        title_fallback: "Fallback",
        guided_flow: false,
        llm_chain: &resources.common.config.llm_chain,
    };
    let thread = resolve_dm_thread(&resources.common.repos, &user, tenant_id, Some(&dm), forge)
        .await
        .unwrap();
    assert_eq!(thread.conversation.id, in_app, "the newer in-app chat wins");
    assert!(
        thread.conversation.personal,
        "the adopted in-app chat becomes the personal thread"
    );
    assert_eq!(
        dividers(&messages(router, &auth, &in_app).await),
        1,
        "an in-app chat the DM adopts opens on exactly one rule"
    );
    let history = prompt_history(&resources, &in_app, user_id, tenant_id).await;
    assert!(
        history.iter().all(|row| row != "A half in eight weeks"),
        "{history:?}"
    );
}

/// carnet#1473: once the personal thread exists, a new chat is its own
/// conversation with the agent picked for it; the personal thread keeps its
/// agent and draws no rule.
#[tokio::test]
async fn a_new_chat_is_its_own_conversation_and_leaves_the_personal_thread_alone() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_athlete(&resources, "new-chat@test.com").await;
    let tempo = seed_agent(&resources, user_id, tenant_id, "Tempo Coach").await;
    let climbing = seed_agent(&resources, user_id, tenant_id, "Climbing Coach").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let personal = create_thread(router.clone(), &auth, "Mine").await;
    let before = get_personal(router.clone(), &auth).await.unwrap();
    assert_eq!(before.id, personal);
    say(
        &resources,
        &personal,
        user_id,
        tenant_id,
        "user",
        "How was my week?",
    )
    .await;
    let rows_before = messages(router.clone(), &auth, &personal).await;

    let mut opened = Vec::new();
    for agent_id in [&tempo, &climbing] {
        let resp = AxumTestRequest::post("/api/chat/conversations")
            .header("authorization", &auth)
            .json(&json!({ "agent_id": agent_id }))
            .send(router.clone())
            .await;
        assert_eq!(resp.status_code(), StatusCode::CREATED);
        let created: ConversationResponse = resp.json();
        assert_eq!(created.agent_id.as_deref(), Some(agent_id.as_str()));
        assert!(!created.personal);
        opened.push(created.id);
    }
    assert_ne!(opened[0], opened[1]);
    assert!(opened.iter().all(|id| id != &personal), "{opened:?}");

    let after = get_personal(router.clone(), &auth).await.unwrap();
    assert_eq!(after.id, personal, "still the athlete's personal thread");
    assert_eq!(after.agent_id, before.agent_id, "its agent was not rebound");
    let rows_after = messages(router, &auth, &personal).await;
    assert_eq!(dividers(&rows_after), 0, "no rule was drawn in it");
    assert_eq!(
        rows_after.iter().map(|row| &row.id).collect::<Vec<_>>(),
        rows_before.iter().map(|row| &row.id).collect::<Vec<_>>(),
        "nothing was written into it"
    );
}
