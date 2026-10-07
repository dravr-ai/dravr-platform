// ABOUTME: An agent bound into a thread opens it with one welcome, as itself: title, role, starters
// ABOUTME: Drives the create route, /agent add, /reset, a room and a guided walk; pins the once-per-(agent, thread) rule

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#735.
//!
//! Seen 2026-10-02 on a local stack: an athlete picked « Fuelling Agent » from
//! the onboarding proposal and landed in an empty thread — a header, a
//! composer, nothing else. The agent's introduction (carnet#501) only opened
//! its first *reply*, so the athlete had to speak first, with no hint of what
//! to ask.
//!
//! These walk every way an agent is bound into a thread from the app — the
//! create route the proposal's « Démarrer » calls, `/agent add`, the fresh
//! thread `/reset` forges (carnet#750) — and assert
//! the one row the agent posts: in the athlete's language, with its starter
//! questions as controls, written once per agent per thread, and counted as
//! the agent's introduction.

mod common;
mod helpers;

use pierre_core::transport::TransportPolicy;
use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;
use tokio::task::spawn_blocking;
use uuid::Uuid;

use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_chat_pipeline::agent_welcome::{post_agent_welcome, WelcomeTarget};
use pierre_chat_pipeline::SurfaceId;
use pierre_contremaitre::messaging_strings::{
    KEY_AGENT_WELCOME_GREETING, KEY_AGENT_WELCOME_GREETING_NO_ROLE,
    KEY_AGENT_WELCOME_STARTERS_TITLE,
};
use pierre_core::models::agents::{
    AgentCategory, AgentVisibility, CreateAgentRequest, CreateSystemAgentRequest,
};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole, TranscriptSpeaker,
};
use pierre_core::models::{
    AddMessageParams, ConnectionType, GuidedFlow, OnboardingState, Tenant, TenantId, User,
    UserStatus, AGENT_WELCOME_FINISH_REASON, COMMAND_FINISH_REASON,
};
use pierre_database::seed_models::SeedAgentTranslation;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatRoutes, ConversationListResponse, ConversationResponse, MessageResponse,
    MessagesListResponse, TurnResponse,
};
use pierre_services::intake::{STATUS_COMPLETE, STEP_PARQ, STEP_PROFILE_TYPE};

const FUELLING_TITLE: &str = "Fuelling Agent";
const FUELLING_ROLE: &str =
    "Fuelling specialist for endurance athletes, before, during and after a session.";
const FUELLING_SAMPLES: [&str; 4] = [
    "What should I eat before a 6am run?",
    "How do I carb load for a marathon?",
    "How many gels during a marathon?",
    "What is the best hydration strategy for a hot race?",
];
const FUELLING_TITLE_FR: &str = "Agent Ravitaillement";
const FUELLING_ROLE_FR: &str =
    "Spécialiste du ravitaillement en endurance, avant, pendant et après la séance.";
const FUELLING_SAMPLES_FR: [&str; 4] = [
    "Que manger avant une course à 6 h du matin ?",
    "Comment faire ma charge glucidique pour un marathon ?",
    "Combien de gels pour un marathon ?",
    "Meilleure stratégie d'hydratation pour une course par chaleur ?",
];

struct Athlete {
    user_id: Uuid,
    tenant_id: TenantId,
    auth: String,
}

async fn setup() -> Arc<ServerContext> {
    create_test_server_resources().await.unwrap()
}

async fn athlete(resources: &Arc<ServerContext>, email: &str) -> Athlete {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Welcome Test".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    let user_id = user.id;
    let repos = &resources.common.repos;
    repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Welcome Tenant".to_owned(),
            slug: format!("welcome-{tenant_id}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    // The chat routes' provider gate counts only real connections.
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    Athlete {
        user_id,
        tenant_id,
        auth: format!("Bearer {token}"),
    }
}

/// A catalogue agent as the seeder writes it: canonical English title, role
/// and samples, and a French overlay of all three.
async fn catalogue_agent(resources: &Arc<ServerContext>, owner: &Athlete) -> String {
    let repos = &resources.common.repos;
    let agent = repos
        .agents
        .create_system_agent(
            owner.user_id,
            owner.tenant_id,
            &CreateSystemAgentRequest {
                title: FUELLING_TITLE.to_owned(),
                description: Some(FUELLING_ROLE.to_owned()),
                system_prompt: "You are a fuelling specialist.".to_owned(),
                category: AgentCategory::Nutrition,
                tags: vec![],
                sample_prompts: FUELLING_SAMPLES.iter().map(|s| (*s).to_owned()).collect(),
                visibility: AgentVisibility::Tenant,
            },
        )
        .await
        .unwrap();
    let id = agent.id.to_string();
    repos
        .seeder
        .seed_upsert_agent_translation(&SeedAgentTranslation {
            agent_id: id.clone(),
            locale: "fr".to_owned(),
            title: Some(FUELLING_TITLE_FR.to_owned()),
            description: Some(FUELLING_ROLE_FR.to_owned()),
            purpose: None,
            instructions: None,
            source_sha: None,
            tags: None,
            sample_prompts: Some(
                FUELLING_SAMPLES_FR
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            ),
        })
        .await
        .unwrap();
    id
}

/// An agent the athlete authored — the kind `/agent add <id>` resolves.
async fn own_agent(
    resources: &Arc<ServerContext>,
    owner: &Athlete,
    title: &str,
    description: Option<&str>,
    samples: &[&str],
) -> String {
    resources
        .common
        .repos
        .agents
        .create(
            owner.user_id,
            owner.tenant_id,
            &CreateAgentRequest {
                title: title.to_owned(),
                description: description.map(ToOwned::to_owned),
                system_prompt: "You are a test agent.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: samples.iter().map(|s| (*s).to_owned()).collect(),
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

async fn create_conversation(
    resources: &Arc<ServerContext>,
    who: &Athlete,
    agent_id: Option<&str>,
) -> String {
    let body = agent_id.map_or_else(|| json!({}), |id| json!({ "agent_id": id }));
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &who.auth)
        .json(&body)
        .send(ChatRoutes::routes(Arc::clone(resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    resp.json::<ConversationResponse>().id
}

async fn messages(
    resources: &Arc<ServerContext>,
    who: &Athlete,
    conv: &str,
) -> Vec<MessageResponse> {
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{conv}/messages"))
        .header("authorization", &who.auth)
        .send(ChatRoutes::routes(Arc::clone(resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json::<MessagesListResponse>().messages
}

fn welcomes(rows: &[MessageResponse]) -> Vec<&MessageResponse> {
    rows.iter()
        .filter(|m| m.finish_reason.as_deref() == Some(AGENT_WELCOME_FINISH_REASON))
        .collect()
}

async fn send_command(
    resources: &Arc<ServerContext>,
    who: &Athlete,
    conv: &str,
    text: &str,
) -> TurnResponse {
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv}/messages"))
        .header("authorization", &who.auth)
        .json(&json!({ "content": text }))
        .send(ChatRoutes::routes(Arc::clone(resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{text}");
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON),
        "{text} is answered as a command"
    );
    body
}

async fn introduced(
    resources: &Arc<ServerContext>,
    thread: &str,
    agent: &str,
    tenant: TenantId,
) -> bool {
    resources
        .common
        .repos
        .chat
        .has_agent_introduction(thread, agent, tenant)
        .await
        .unwrap()
}

fn render(resources: &Arc<ServerContext>, key: &str, locale: &str, args: &[&str]) -> String {
    resources
        .mcp
        .messaging_strings_registry
        .render(key, locale, args)
}

/// The labels of a row's postback controls, asserting each sends the opaque
/// postback of its slot, which the turn resolves back into the label
/// (carnet#828).
fn starters(row: &MessageResponse) -> Vec<String> {
    let actions = row
        .actions
        .as_ref()
        .expect("the welcome carries its starters");
    actions
        .actions
        .iter()
        .enumerate()
        .map(|(slot, action)| {
            assert_eq!(action.action_type, "postback", "a starter is a postback");
            assert_eq!(
                action.value,
                format!("ex:{slot}:{slot}"),
                "a starter sends its postback, never its words"
            );
            action.label.clone()
        })
        .collect()
}

fn first_three(samples: &[&str]) -> Vec<String> {
    samples.iter().take(3).map(|s| (*s).to_owned()).collect()
}

/// The onboarding proposal's « Démarrer »: a French athlete creating a thread
/// with a catalogue agent finds exactly one row in it — the agent's welcome,
/// in French, with its first three French examples as starters — already
/// read, and counted as the agent's introduction.
#[tokio::test]
async fn creating_an_agent_thread_posts_one_localized_welcome() {
    let resources = setup().await;
    let alice = athlete(&resources, "welcome-create@test.com").await;
    let agent = catalogue_agent(&resources, &alice).await;

    let conv = create_conversation(&resources, &alice, Some(&agent)).await;

    let rows = messages(&resources, &alice, &conv).await;
    assert_eq!(
        rows.len(),
        1,
        "the thread holds the welcome and nothing else"
    );
    let welcome = &rows[0];
    assert_eq!(welcome.role, "assistant");
    assert_eq!(
        welcome.finish_reason.as_deref(),
        Some(AGENT_WELCOME_FINISH_REASON)
    );
    assert_eq!(
        welcome.content,
        render(
            &resources,
            KEY_AGENT_WELCOME_GREETING,
            "fr",
            &[FUELLING_TITLE_FR, FUELLING_ROLE_FR]
        )
    );
    assert!(
        welcome.content.contains(FUELLING_TITLE_FR),
        "{}",
        welcome.content
    );
    assert_eq!(
        welcome.actions.as_ref().and_then(|a| a.title.as_deref()),
        Some(render(&resources, KEY_AGENT_WELCOME_STARTERS_TITLE, "fr", &[]).as_str())
    );
    assert_eq!(starters(welcome), first_three(&FUELLING_SAMPLES_FR));
    assert!(
        introduced(&resources, &conv, &agent, alice.tenant_id).await,
        "the welcome is the agent's introduction"
    );

    let list = AxumTestRequest::get("/api/chat/conversations")
        .header("authorization", &alice.auth)
        .send(ChatRoutes::routes(Arc::clone(&resources)))
        .await
        .json::<ConversationListResponse>();
    let row = list.conversations.iter().find(|c| c.id == conv).unwrap();
    assert_eq!(row.unread_count, 0, "the athlete is in front of the thread");
}

/// A locale the catalogue has no agent copy for keeps the agent's English
/// title, role and starters, under a greeting in the athlete's language. An
/// agent with no role and no examples still greets, without either.
#[tokio::test]
async fn missing_copy_falls_back_to_the_canonical_agent() {
    let resources = setup().await;
    let bea = athlete(&resources, "welcome-es@test.com").await;
    resources
        .common
        .repos
        .users
        .update_locale(bea.user_id, "es")
        .await
        .unwrap();
    let agent = catalogue_agent(&resources, &bea).await;

    let conv = create_conversation(&resources, &bea, Some(&agent)).await;
    let rows = messages(&resources, &bea, &conv).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].content,
        render(
            &resources,
            KEY_AGENT_WELCOME_GREETING,
            "es",
            &[FUELLING_TITLE, FUELLING_ROLE]
        )
    );
    assert_eq!(starters(&rows[0]), first_three(&FUELLING_SAMPLES));

    let bare = own_agent(&resources, &bea, "Bare Agent", None, &[]).await;
    let conv = create_conversation(&resources, &bea, Some(&bare)).await;
    let rows = messages(&resources, &bea, &conv).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].content,
        render(
            &resources,
            KEY_AGENT_WELCOME_GREETING_NO_ROLE,
            "es",
            &["Bare Agent"]
        )
    );
    assert!(rows[0].actions.is_none(), "no examples, no starters");
}

/// No agent, no welcome — and an agent the athlete cannot see is no agent:
/// the thread is still created, empty, and no introduction is recorded.
#[tokio::test]
async fn a_thread_without_a_visible_agent_stays_empty() {
    let resources = setup().await;
    let cleo = athlete(&resources, "welcome-none@test.com").await;
    let stranger = athlete(&resources, "welcome-stranger@test.com").await;
    let theirs = own_agent(
        &resources,
        &stranger,
        "Private Agent",
        Some("Theirs."),
        &["Q?"],
    )
    .await;

    let plain = create_conversation(&resources, &cleo, None).await;
    assert!(messages(&resources, &cleo, &plain).await.is_empty());

    let foreign = create_conversation(&resources, &cleo, Some(&theirs)).await;
    assert!(messages(&resources, &cleo, &foreign).await.is_empty());
    assert!(!introduced(&resources, &foreign, &theirs, cleo.tenant_id).await);
}

/// `/agent add` in an empty thread: the command's two rows, then the agent's
/// welcome — which also rides the turn, so the client draws it and its
/// starters without re-reading the thread.
#[tokio::test]
async fn agent_add_welcomes_after_the_command_rows() {
    let resources = setup().await;
    let dan = athlete(&resources, "welcome-add@test.com").await;
    let agent = own_agent(
        &resources,
        &dan,
        "Tempo Agent",
        Some("Tempo runs for the marathon build."),
        &[
            "Plan my tempo week",
            "Is my tempo pace right",
            "How long should a tempo be",
            "Fourth",
        ],
    )
    .await;
    let conv = create_conversation(&resources, &dan, None).await;

    let turn = send_command(&resources, &dan, &conv, &format!("/agent add {agent}")).await;

    let carried = turn
        .welcome_message
        .as_ref()
        .expect("the turn carries the welcome");
    assert_eq!(
        starters(carried),
        vec![
            "Plan my tempo week",
            "Is my tempo pace right",
            "How long should a tempo be"
        ]
    );
    let rows = messages(&resources, &dan, &conv).await;
    let reasons: Vec<Option<&str>> = rows.iter().map(|m| m.finish_reason.as_deref()).collect();
    assert_eq!(
        reasons,
        vec![
            Some(COMMAND_FINISH_REASON),
            Some(COMMAND_FINISH_REASON),
            Some(AGENT_WELCOME_FINISH_REASON)
        ],
        "the welcome follows the command's own rows"
    );
    assert_eq!(rows[2].id, carried.id, "the turn carries the persisted row");
    assert_eq!(
        rows[2].content,
        render(
            &resources,
            KEY_AGENT_WELCOME_GREETING,
            "fr",
            &["Tempo Agent", "Tempo runs for the marathon build."]
        )
    );
    assert!(introduced(&resources, &conv, &agent, dan.tenant_id).await);
}

/// Binding the agent a thread already met posts nothing, however it comes
/// back — re-added while bound, or after `/agent remove`.
#[tokio::test]
async fn the_same_agent_never_welcomes_a_thread_twice() {
    let resources = setup().await;
    let eve = athlete(&resources, "welcome-rebind@test.com").await;
    let agent = own_agent(&resources, &eve, "Rebind Agent", Some("Role."), &["Q one"]).await;
    let conv = create_conversation(&resources, &eve, Some(&agent)).await;
    assert_eq!(welcomes(&messages(&resources, &eve, &conv).await).len(), 1);

    let add = format!("/agent add {agent}");
    assert!(send_command(&resources, &eve, &conv, &add)
        .await
        .welcome_message
        .is_none());
    send_command(&resources, &eve, &conv, "/agent remove").await;
    assert!(send_command(&resources, &eve, &conv, &add)
        .await
        .welcome_message
        .is_none());

    assert_eq!(welcomes(&messages(&resources, &eve, &conv).await).len(), 1);
}

/// A second agent bound into the same thread posts its own welcome, once;
/// switching back and forth posts nothing more.
#[tokio::test]
async fn each_agent_welcomes_a_shared_thread_once() {
    let resources = setup().await;
    let finn = athlete(&resources, "welcome-second@test.com").await;
    let first = own_agent(&resources, &finn, "First Agent", Some("One."), &["Q one"]).await;
    let second = own_agent(&resources, &finn, "Second Agent", Some("Two."), &["Q two"]).await;
    let conv = create_conversation(&resources, &finn, Some(&first)).await;

    let to_second = format!("/agent add {second}");
    let to_first = format!("/agent add {first}");
    assert!(send_command(&resources, &finn, &conv, &to_second)
        .await
        .welcome_message
        .is_some());
    assert!(send_command(&resources, &finn, &conv, &to_first)
        .await
        .welcome_message
        .is_none());
    assert!(send_command(&resources, &finn, &conv, &to_second)
        .await
        .welcome_message
        .is_none());

    let rows = messages(&resources, &finn, &conv).await;
    let titles: Vec<bool> = welcomes(&rows)
        .iter()
        .map(|w| w.content.contains("First Agent"))
        .collect();
    assert_eq!(titles, vec![true, false], "one welcome each, in bind order");
    assert!(introduced(&resources, &conv, &second, finn.tenant_id).await);
}

/// A guided walk owns its thread's first word: binding an agent mid-walk
/// posts no welcome and records no introduction, so the agent's first reply
/// still introduces it.
#[tokio::test]
async fn no_welcome_while_a_guided_walk_owns_the_thread() {
    let resources = setup().await;
    let gus = athlete(&resources, "welcome-walk@test.com").await;
    let agent = own_agent(&resources, &gus, "Walk Agent", Some("Role."), &["Q"]).await;
    let conv = create_conversation(&resources, &gus, None).await;
    let state = OnboardingState::start_now_column(GuidedFlow::Pillars);
    resources
        .common
        .repos
        .chat
        .set_conversation_onboarding_state(&conv, Some(&state), gus.tenant_id)
        .await
        .unwrap();

    let turn = send_command(&resources, &gus, &conv, &format!("/agent add {agent}")).await;

    assert!(turn.welcome_message.is_none());
    assert!(welcomes(&messages(&resources, &gus, &conv).await).is_empty());
    assert!(!introduced(&resources, &conv, &agent, gus.tenant_id).await);
}

/// A room keeps one conversation row per member, and the room is the thread:
/// the agent's welcome lands once — in the row of the member whose action
/// bound it, and in the room transcript as the agent's — and a second
/// member's bind finds the room already welcomed.
#[tokio::test]
async fn a_room_is_welcomed_once_per_agent() {
    let resources = setup().await;
    let repos = &resources.common.repos;
    let hana = athlete(&resources, "welcome-room-a@test.com").await;
    let ivo = athlete(&resources, "welcome-room-b@test.com").await;
    let tenant = hana.tenant_id;
    let agent = repos
        .agents
        .create_system_agent(
            hana.user_id,
            tenant,
            &CreateSystemAgentRequest {
                title: "Room Agent".to_owned(),
                description: Some("Coaches the whole room.".to_owned()),
                system_prompt: "You coach a room.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec!["How did the room train".to_owned()],
                visibility: AgentVisibility::Tenant,
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    let group_id = room(&resources, &agent, tenant, &[hana.user_id, ivo.user_id]).await;
    let group = group_id.to_string();

    let mut posted = Vec::new();
    for member in [&hana, &ivo] {
        let row = repos
            .chat
            .create_conversation(
                &member.user_id.to_string(),
                tenant,
                "Room",
                "test-model",
                Some(&agent),
                Some(&group),
            )
            .await
            .unwrap();
        let user = member.user_id.to_string();
        let welcome = post_agent_welcome(
            repos,
            &resources.mcp.messaging_strings_registry,
            WelcomeTarget {
                conversation: &row,
                user_id: &user,
                conversation_tenant_id: tenant,
                agent_id: &agent,
                agent_tenant_id: tenant,
                locale: "en",
                surface: SurfaceId::Web,
            },
        )
        .await
        .unwrap();
        posted.push(welcome.map(|w| w.message.id));
    }

    let first = posted[0].clone().expect("the first bind welcomes the room");
    assert!(posted[1].is_none(), "the room has already met the agent");
    assert!(introduced(&resources, &group, &agent, tenant).await);
    let transcript = repos
        .groups
        .list_transcript_visible_to(&group, hana.user_id, 10)
        .await
        .unwrap();
    let entry = transcript
        .iter()
        .find(|e| e.source_message_id.as_deref() == Some(first.as_str()))
        .expect("the welcome reaches the room transcript");
    assert_eq!(entry.speaker, TranscriptSpeaker::Coach);
}

async fn room(
    resources: &Arc<ServerContext>,
    agent_id: &str,
    tenant: TenantId,
    members: &[Uuid],
) -> Uuid {
    let now = chrono::Utc::now();
    let group_id = Uuid::new_v4();
    let repos = &resources.common.repos;
    repos
        .groups
        .create_group(
            tenant,
            &CoachingGroup {
                id: group_id,
                tenant_id: tenant.to_string(),
                name: "Welcome Room".to_owned(),
                description: None,
                agent_id: agent_id.to_owned(),
                owner_id: members[0],
                coach_user_id: None,
                peer_data_sharing: true,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    for (i, user_id) in members.iter().enumerate() {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id: *user_id,
                tenant_id: tenant.to_string(),
                role: if i == 0 {
                    GroupRole::Owner
                } else {
                    GroupRole::Member
                },
                peer_sharing_consent: true,
                coach_sharing_consent: true,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }
    group_id
}

/// The ledger row is what makes the welcome once per agent per thread: two
/// writes — one after the other, or racing — leave one row and one ledger
/// entry, on whichever backend the suite runs.
#[tokio::test]
async fn the_welcome_write_is_once_per_thread_even_when_racing() {
    let resources = setup().await;
    let jo = athlete(&resources, "welcome-race@test.com").await;
    let chat = &resources.common.repos.chat;
    let user = jo.user_id.to_string();
    let conv = chat
        .create_conversation(&user, jo.tenant_id, "Race", "test-model", None, None)
        .await
        .unwrap()
        .id;
    let agent = Uuid::new_v4().to_string();
    let params = AddMessageParams {
        tenant_id: jo.tenant_id,
        conversation_id: &conv,
        user_id: &user,
        role: "assistant",
        content: "Hi!",
        token_count: None,
        finish_reason: Some(AGENT_WELCOME_FINISH_REASON),
        prompt_tokens: None,
        model: None,
        content_blocks: None,
        transport_policy: TransportPolicy::AnyTransport,
    };

    let (a, b) = tokio::join!(
        chat.add_agent_welcome(&params, &conv, &agent),
        chat.add_agent_welcome(&params, &conv, &agent),
    );
    let written = [a.unwrap(), b.unwrap()];
    assert_eq!(
        written.iter().filter(|w| w.is_some()).count(),
        1,
        "exactly one of two racing writes lands"
    );
    assert!(chat
        .add_agent_welcome(&params, &conv, &agent)
        .await
        .unwrap()
        .is_none());

    let rows = chat.get_messages(&conv, &user, jo.tenant_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_agent_welcome());
    assert!(introduced(&resources, &conv, &agent, jo.tenant_id).await);
}

/// `/reset` on an agent thread (carnet#750): the athlete lands on a fresh
/// thread bound to the same agent, and the agent opens it the way it opens
/// any new thread — one welcome, in the athlete's language, with its
/// starters. The welcome stays off the turn: the client posted to the
/// archived thread and reads the fresh one whole, so a welcome riding the
/// turn would be drawn under the thread the athlete just left.
#[tokio::test]
async fn reset_reopens_the_fresh_thread_with_the_agent_welcome() {
    let resources = setup().await;
    let kai = athlete(&resources, "welcome-reset@test.com").await;
    let agent = own_agent(
        &resources,
        &kai,
        "Reset Agent",
        Some("Keeps the marathon build on track."),
        &[
            "Plan my week",
            "Check my long run",
            "Taper advice",
            "Fourth",
        ],
    )
    .await;
    let old = create_conversation(&resources, &kai, Some(&agent)).await;
    assert_eq!(welcomes(&messages(&resources, &kai, &old).await).len(), 1);

    let turn = send_command(&resources, &kai, &old, "/reset").await;

    let fresh = turn
        .rotated_to_conversation_id
        .clone()
        .expect("/reset moves the athlete to a fresh thread");
    assert_ne!(fresh, old);
    assert!(
        turn.welcome_message.is_none(),
        "the welcome belongs to the fresh thread, not the turn posted to the old one"
    );
    let rows = messages(&resources, &kai, &fresh).await;
    let posted = welcomes(&rows);
    assert_eq!(posted.len(), 1, "the agent opens the fresh thread once");
    assert_eq!(
        posted[0].content,
        render(
            &resources,
            KEY_AGENT_WELCOME_GREETING,
            "fr",
            &["Reset Agent", "Keeps the marathon build on track."]
        )
    );
    assert_eq!(
        starters(posted[0]),
        vec!["Plan my week", "Check my long run", "Taper advice"]
    );
    assert!(introduced(&resources, &fresh, &agent, kai.tenant_id).await);
    assert_eq!(
        welcomes(&messages(&resources, &kai, &old).await).len(),
        1,
        "the archived thread keeps its own welcome and gains none"
    );
}

/// A thread with no agent resets onto a thread with no agent: nothing to
/// welcome, in either.
#[tokio::test]
async fn reset_of_a_thread_without_an_agent_posts_no_welcome() {
    let resources = setup().await;
    let lea = athlete(&resources, "welcome-reset-none@test.com").await;
    let old = create_conversation(&resources, &lea, None).await;

    let turn = send_command(&resources, &lea, &old, "/reset").await;

    let fresh = turn
        .rotated_to_conversation_id
        .clone()
        .expect("/reset moves the athlete to a fresh thread");
    assert!(turn.welcome_message.is_none());
    assert!(welcomes(&messages(&resources, &lea, &fresh).await).is_empty());
    assert!(welcomes(&messages(&resources, &lea, &old).await).is_empty());
}

/// The fresh thread a 1:1 reset forges can open with the pillar walk — an
/// athlete past the intake who has told us nothing yet. The walk owns that
/// thread's first word, exactly as on any other bind, so the agent posts no
/// welcome and its first reply still introduces it.
#[tokio::test]
async fn reset_into_a_guided_walk_posts_no_welcome() {
    let resources = setup().await;
    let max = athlete(&resources, "welcome-reset-walk@test.com").await;
    let user = max.user_id.to_string();
    for step in [STEP_PROFILE_TYPE, STEP_PARQ] {
        resources
            .common
            .repos
            .user_onboarding
            .set_onboarding_step(&user, step, STATUS_COMPLETE, None, None)
            .await
            .unwrap();
    }
    let agent = own_agent(&resources, &max, "Walk Reset Agent", Some("Role."), &["Q"]).await;
    let old = create_conversation(&resources, &max, Some(&agent)).await;

    let turn = send_command(&resources, &max, &old, "/reset").await;

    let fresh = turn
        .rotated_to_conversation_id
        .clone()
        .expect("/reset moves the athlete to a fresh thread");
    let thread = resources
        .common
        .repos
        .chat
        .get_conversation(&fresh, &user, max.tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        OnboardingState::from_column(thread.onboarding_state.as_deref()).map(|s| s.flow),
        Some(GuidedFlow::Pillars),
        "the fresh thread opens with the pillar walk, or this test proves nothing"
    );
    assert_eq!(thread.agent_id.as_deref(), Some(agent.as_str()));
    assert!(turn.welcome_message.is_none());
    assert!(welcomes(&messages(&resources, &max, &fresh).await).is_empty());
    assert!(!introduced(&resources, &fresh, &agent, max.tenant_id).await);
}
