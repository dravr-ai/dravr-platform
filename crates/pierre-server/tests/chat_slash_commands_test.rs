// ABOUTME: Integration tests for slash-command handling on the /api/chat/.../messages endpoint
// ABOUTME: Asserts web and mobile chat use the same dispatcher as the messaging channels
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use async_trait::async_trait;
use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_server_resources_with_chat_provider};
use futures_util::stream;
use helpers::axum_test::AxumTestRequest;
use helpers::coach_fixtures::{install_catalogue_coach, publish_catalogue_coach};
use pierre_chat_pipeline::stages::persistence::get_conversation_history;
use pierre_contremaitre::messaging_strings::{
    KEY_COACH_ASSIGN_FORBIDDEN, KEY_COACH_CREATE_CARD_TITLE, KEY_COACH_CREATE_DISCARDED,
    KEY_COACH_CREATE_EMPTY, KEY_COACH_CREATE_QUOTA, KEY_COACH_REMOVE_GROUP_THREAD,
    KEY_COACH_REMOVE_NOTHING, KEY_GUARDIAN_CONFIRM_NOT_FOUND,
};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
    StreamChunk, TokenUsage,
};
use pierre_core::models::coaches::{
    CoachCategory, CoachVisibility, CreateCoachRequest, CreateSystemCoachRequest, ListCoachesFilter,
};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{AddMessageParams, COMMAND_FINISH_REASON};
use pierre_core::models::{ConnectionType, TenantId};
use pierre_core::models::{OnboardingState, Tenant, User, UserStatus};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatMessageAction, ChatRoutes, ConversationListResponse, ConversationResponse,
    MessagesListResponse, ReplyBlockResponse, TurnResponse,
};

/// The controls a turn carries, or an empty slice when it carries none.
fn turn_actions(body: &TurnResponse) -> Vec<&ChatMessageAction> {
    body.assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlockResponse::Actions { actions, .. } => Some(actions.iter().collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The label above a turn's controls, when it carried one.
fn turn_actions_title(body: &TurnResponse) -> Option<&str> {
    body.assistant.blocks.iter().find_map(|block| match block {
        ReplyBlockResponse::Actions { title, .. } => title.as_deref(),
        _ => None,
    })
}
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::task::spawn_blocking;
use uuid::Uuid;

/// Telegram's ceiling on a button's callback data; every postback a card
/// carries must fit, or the channel rejects the whole card.
const TELEGRAM_CALLBACK_DATA_MAX: usize = 64;

/// The persona the mock model proposes, fenced the way real models fence
/// JSON they were asked to return bare.
const PROPOSAL_JSON: &str = "```json\n{\"title\":\"Coach Tempo\",\
\"description\":\"Tempo runs for the marathon build.\",\
\"system_prompt\":\"You are a tempo-run coach.\",\
\"category\":\"training\",\"tags\":[\"tempo\",\"marathon\"]}\n```";

/// Answers every completion with [`PROPOSAL_JSON`] and keeps what it was
/// asked, so a test reads the excerpt the model was given rather than
/// inferring it from the reply.
///
/// Mock is test-only: the model boundary is the assertion point itself.
struct ProposalLlm {
    requests: Mutex<Vec<Vec<ChatMessage>>>,
}

impl ProposalLlm {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// The user message of the only completion requested so far.
    fn only_user_prompt(&self) -> String {
        let requests = self.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "exactly one completion expected");
        requests[0]
            .iter()
            .find(|m| matches!(m.role, MessageRole::User))
            .map(|m| m.content.clone())
            .expect("the proposal request carries a user message")
    }
}

#[async_trait]
impl LlmProvider for ProposalLlm {
    fn name(&self) -> &'static str {
        "proposal_mock"
    }
    fn display_name(&self) -> &'static str {
        "Proposal mock LLM"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.requests.lock().unwrap().push(request.messages.clone());
        Ok(ChatResponse {
            content: PROPOSAL_JSON.to_owned(),
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(12, 6, 18)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        self.requests.lock().unwrap().push(request.messages.clone());
        let chunk = StreamChunk {
            delta: PROPOSAL_JSON.to_owned(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

async fn seed_user_tenant(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId, String) {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Chat Cmd Test".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Chat Cmd Tenant".to_owned(),
        slug: format!("chat-cmd-{tenant_id}"),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    // tenants.create() adds the owner to tenant_users automatically.
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    // Legacy tenant_id column mirrors tenant_users; keep them in sync
    // for the /api/chat endpoints that still read it.
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    // Register a real (non-synthetic) provider so the onboarding gate
    // (the provider gate, removed in Phase 5) doesn't 403
    // before the slash dispatcher even runs. Slash commands don't consume
    // provider data, but the gate fires upstream of dispatch and counts
    // only non-synthetic connections.
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

async fn seed_coach(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    description: &str,
) -> String {
    let request = CreateCoachRequest {
        title: title.to_owned(),
        description: Some(description.to_owned()),
        system_prompt: "You are a test coach.".to_owned(),
        category: CoachCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
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
    let coach = resources
        .common
        .repos
        .coaches
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    coach.id.to_string()
}

/// Put `user_id` in a coaching group with the given role, so `/help` resolves a
/// real membership the way the group handlers do.
async fn seed_group_membership(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    coach_id: &str,
    role: GroupRole,
) -> Uuid {
    let now = chrono::Utc::now();
    let group_id = Uuid::new_v4();
    let group = CoachingGroup {
        id: group_id,
        tenant_id: tenant_id.to_string(),
        name: "Help Filter Group".to_owned(),
        description: None,
        coach_id: coach_id.to_owned(),
        // owner_id must reference a real user (FK). It plays no part in the
        // filter either way: `caller_group_standing` reads the membership row's
        // `role`, which is what `role` below sets.
        owner_id: user_id,
        coach_user_id: None,
        peer_data_sharing: false,
        respond_mode: GroupRespondMode::default(),
        max_members: 20,
        is_active: true,
        channel_type: None,
        channel_chat_id: None,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .groups
        .create_group(tenant_id, &group)
        .await
        .unwrap();

    resources
        .common
        .repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();

    group_id
}

/// Send `/help` and return the rendered listing.
async fn fetch_help(router: axum::Router, auth: &str) -> String {
    let conv_id = create_conversation(router.clone(), auth).await;
    help_in_conversation(router, auth, &conv_id).await
}

/// Send `/help` on an existing conversation and return the rendered listing.
///
/// Separate from [`fetch_help`] so a test can bind the conversation to a
/// specific group first, which pins which group `/help` treats as ambient
/// instead of leaving it to `list_groups_for_user`'s `updated_at DESC`.
async fn help_in_conversation(router: axum::Router, auth: &str, conv_id: &str) -> String {
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", auth)
        .json(&json!({"content": "/help"}))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    body.assistant.message.content
}

/// Send `/help` from a fresh conversation bound to `group_id`, so the listing
/// answers for the caller's standing *in that group* — the thread names it,
/// the way a group room does. An unbound in-app thread names no group.
async fn help_in_group_thread(
    resources: &Arc<ServerContext>,
    router: axum::Router,
    auth: &str,
    tenant_id: TenantId,
    group_id: Uuid,
) -> String {
    let conv_id = create_conversation(router.clone(), auth).await;
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conv_id, Some(&group_id.to_string()), tenant_id)
        .await
        .unwrap();
    help_in_conversation(router, auth, &conv_id).await
}

async fn create_conversation(router: axum::Router, auth: &str) -> String {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", auth)
        .json(&json!({
            "title": "Cmd Test",
            "model": "gemini-1.5-flash"
        }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = resp.json();
    conv.id
}

/// Send one slash command on `conv_id` and return the turn.
async fn send_command(router: axum::Router, auth: &str, conv_id: &str, text: &str) -> TurnResponse {
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", auth)
        .json(&json!({"content": text}))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{text}");
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON),
        "{text} must be answered as a command"
    );
    body
}

/// Render a messaging string in the platform default locale, which is the
/// locale every user seeded here answers in.
fn rendered(resources: &Arc<ServerContext>, key: &str, args: &[&str]) -> String {
    resources
        .mcp
        .messaging_strings_registry
        .render(key, "fr", args)
}

/// Publish "Recovery Coach" under a fresh author and install it for the
/// athlete; returns the installed copy's id, which answers to `@recovery-coach`.
async fn install_recovery_coach(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> String {
    let (author_id, author_tenant, _author_auth) =
        seed_user_tenant(resources, &format!("author-{user_id}@test.com")).await;
    let origin = publish_catalogue_coach(
        &resources.common.repos,
        author_id,
        author_tenant,
        "Recovery Coach",
        "You are the recovery coach.",
    )
    .await;
    let installed =
        install_catalogue_coach(&resources.common.repos, origin, user_id, tenant_id).await;
    assert_eq!(installed.handle.as_deref(), Some("recovery-coach"));
    installed.id.to_string()
}

/// A fresh conversation bound to `group_id`, the way a group chat is.
async fn group_conversation(
    resources: &Arc<ServerContext>,
    router: axum::Router,
    auth: &str,
    tenant_id: TenantId,
    group_id: Uuid,
) -> String {
    let conv_id = create_conversation(router, auth).await;
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conv_id, Some(&group_id.to_string()), tenant_id)
        .await
        .unwrap();
    conv_id
}

/// The coach the conversation row is bound to.
async fn conversation_coach(
    resources: &Arc<ServerContext>,
    conv_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<String> {
    resources
        .common
        .repos
        .chat
        .get_conversation(conv_id, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .expect("the conversation exists")
        .coach_id
}

/// The athlete's selection pointer.
async fn selected_coach(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<String> {
    resources
        .common
        .repos
        .tenants
        .get_selected_coach(tenant_id, user_id)
        .await
        .unwrap()
}

/// The coach a group is pointed at.
async fn group_coach(
    resources: &Arc<ServerContext>,
    group_id: Uuid,
    tenant_id: TenantId,
) -> String {
    resources
        .common
        .repos
        .groups
        .get_group(&group_id.to_string(), tenant_id)
        .await
        .unwrap()
        .expect("the group exists")
        .coach_id
}

/// Write a coaching exchange into `conv_id`, so there is something to draft
/// a coach from.
async fn seed_coaching_exchange(
    resources: &Arc<ServerContext>,
    conv_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let user_id = user_id.to_string();
    for (role, content) in [
        (
            "user",
            "Je prépare un marathon, comment placer mes sorties tempo ?",
        ),
        (
            "assistant",
            "Une sortie tempo par semaine, le jeudi, à allure semi-marathon.",
        ),
    ] {
        resources
            .common
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id,
                conversation_id: conv_id,
                user_id: &user_id,
                role,
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
            })
            .await
            .unwrap();
    }
}

/// The athlete's coach count, read the way the quota reads it.
async fn coach_count(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) -> u32 {
    resources
        .common
        .repos
        .coaches
        .count(user_id, tenant_id)
        .await
        .unwrap()
}

#[tokio::test]
async fn coach_command_returns_card_with_actions_no_llm_call() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "web-chat@test.com").await;
    let coach = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Activity Analysis Coach",
        "Analyzes training data. What *not* to overlook.",
    )
    .await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/coach"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();

    // Command response marker is set so clients can skip LLM-specific UI.
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    // The card title labels the controls it belongs to rather than being
    // pre-folded onto the front of the body text.
    assert!(
        turn_actions_title(&body).is_some(),
        "expected a title on /coach's actions block"
    );
    // Actions populated with per-coach add buttons. A coach created here owns
    // no catalogue handle, so its button carries the id form.
    let actions = turn_actions(&body);
    assert!(!actions.is_empty(), "expected at least one coach action");
    assert_eq!(actions[0].action_type, "postback");
    assert_eq!(
        actions[0].value,
        format!("/coach add {coach}"),
        "a coach without a handle is added by id"
    );
    // Markdown emphasis stripped uniformly (same behaviour as messaging channels).
    assert!(
        !body.assistant.message.content.contains('*'),
        "command response must not contain literal markdown asterisks: {}",
        body.assistant.message.content
    );
    // No LLM tokens billed for a command turn.
    assert_eq!(body.telemetry.execution_time_ms, 0);
    assert_eq!(body.telemetry.model, "command");
}

/// `/coach` is the athlete's shelf, not the catalogue: the coaches they
/// created and the ones they installed, each with the `@handle` that adds it,
/// and no system coach they never installed. Every button fits Telegram's
/// callback-data ceiling.
#[tokio::test]
async fn coach_list_shows_installed_coaches_with_handles_and_no_uninstalled_system_coach() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "coach-list@test.com").await;
    let own = seed_coach(&resources, user_id, tenant_id, "My Own Coach", "Mine.").await;
    let installed = install_recovery_coach(&resources, user_id, tenant_id).await;
    // A system coach every tenant can see — visible to `/discover`, absent
    // from the shelf until it is installed.
    let uninstalled = resources
        .common
        .repos
        .coaches
        .create_system_coach(
            user_id,
            tenant_id,
            &CreateSystemCoachRequest {
                title: "Global Strength Coach".to_owned(),
                description: Some("Strength for everyone.".to_owned()),
                system_prompt: "You are the strength coach.".to_owned(),
                category: CoachCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: CoachVisibility::Global,
            },
        )
        .await
        .unwrap();

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    let body = send_command(router, &auth, &conv_id, "/coach").await;

    let text = &body.assistant.message.content;
    assert!(
        text.contains("@recovery-coach"),
        "the installed coach shows its handle:\n{text}"
    );
    assert!(
        text.contains("My Own Coach"),
        "the athlete's own coach is listed:\n{text}"
    );
    assert!(
        !text.contains("Global Strength Coach"),
        "a system coach never installed is not on the shelf:\n{text}"
    );
    let values: Vec<&str> = turn_actions(&body)
        .iter()
        .map(|a| a.value.as_str())
        .collect();
    assert!(
        values.contains(&"/coach add @recovery-coach"),
        "got {values:?}"
    );
    assert!(
        values.contains(&format!("/coach add {own}").as_str()),
        "got {values:?}"
    );
    assert!(
        !values
            .iter()
            .any(|v| v.contains(&uninstalled.id.to_string())),
        "no button for a coach that is not on the shelf: {values:?}"
    );
    assert!(
        !values.contains(&format!("/coach add {installed}").as_str()),
        "a coach with a handle is added by handle, never by id: {values:?}"
    );
    for value in &values {
        assert!(
            value.len() <= TELEGRAM_CALLBACK_DATA_MAX,
            "{value} exceeds Telegram's callback-data ceiling"
        );
    }
}

/// `/coach add <id>` — the form the list card sends for a coach without a
/// handle — moves the selection pointer and binds the conversation, and says
/// so without any "group" wording: an unbound thread is personal.
#[tokio::test]
async fn coach_add_by_id_in_chat_binds_the_conversation_and_the_selection() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "coach-add-id@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Polarized Training Coach",
        "Experts in training intensity distribution.",
    )
    .await;
    assert!(selected_coach(&resources, user_id, tenant_id)
        .await
        .is_none());

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    let body = send_command(router, &auth, &conv_id, &format!("/coach add {coach_id}")).await;

    let text = &body.assistant.message.content;
    assert!(
        text.contains("Polarized Training Coach"),
        "names the coach: {text}"
    );
    assert!(
        !text.to_lowercase().contains("group"),
        "a personal thread never mentions a group: {text}"
    );
    assert_eq!(
        selected_coach(&resources, user_id, tenant_id)
            .await
            .as_deref(),
        Some(coach_id.as_str())
    );
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id)
            .await
            .as_deref(),
        Some(coach_id.as_str()),
        "the conversation the command was typed in is bound"
    );
}

/// `/coach add @handle` binds the caller's installed copy; a handle nobody
/// installed — unknown, or a system coach still on the catalogue only — is
/// refused by name and binds nothing.
#[tokio::test]
async fn coach_add_by_handle_in_chat_binds_the_conversation() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "coach-add-handle@test.com").await;
    let installed_id = install_recovery_coach(&resources, user_id, tenant_id).await;
    let uninstalled_system = resources
        .common
        .repos
        .coaches
        .create_system_coach(
            user_id,
            tenant_id,
            &CreateSystemCoachRequest {
                title: "Global Strength Coach".to_owned(),
                description: None,
                system_prompt: "You are the strength coach.".to_owned(),
                category: CoachCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: CoachVisibility::Global,
            },
        )
        .await
        .unwrap();

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let body = send_command(router.clone(), &auth, &conv_id, "/coach add @nobody-here").await;
    assert!(
        body.assistant.message.content.contains("@nobody-here"),
        "the refusal names the handle: {}",
        body.assistant.message.content
    );
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id).await,
        None,
        "an unknown handle binds nothing"
    );

    // The id form is bounded by the shelf too: a system coach the athlete
    // never installed is refused even by id.
    let body = send_command(
        router.clone(),
        &auth,
        &conv_id,
        &format!("/coach add {}", uninstalled_system.id),
    )
    .await;
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id).await,
        None,
        "a coach off the shelf binds nothing: {}",
        body.assistant.message.content
    );

    let body = send_command(router, &auth, &conv_id, "/coach add @recovery-coach").await;
    assert!(
        body.assistant.message.content.contains("Recovery Coach"),
        "the confirmation names the coach: {}",
        body.assistant.message.content
    );
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id)
            .await
            .as_deref(),
        Some(installed_id.as_str()),
        "the conversation is bound to the installed copy"
    );
    assert_eq!(
        selected_coach(&resources, user_id, tenant_id)
            .await
            .as_deref(),
        Some(installed_id.as_str())
    );
}

/// In a group thread `/coach add` is a group setting: an owner points the
/// group at the coach — every member gets it — and the thread rebinds, while
/// the owner's personal selection is left alone.
#[tokio::test]
async fn coach_add_in_a_group_thread_by_an_owner_sets_the_group_coach_and_binds() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "coach-add-owner@test.com").await;
    let first_coach = seed_coach(&resources, user_id, tenant_id, "First Coach", "First.").await;
    let group_id = seed_group_membership(
        &resources,
        user_id,
        tenant_id,
        &first_coach,
        GroupRole::Owner,
    )
    .await;
    let installed_id = install_recovery_coach(&resources, user_id, tenant_id).await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = group_conversation(&resources, router.clone(), &auth, tenant_id, group_id).await;
    let body = send_command(router, &auth, &conv_id, "/coach add @recovery-coach").await;

    let text = &body.assistant.message.content;
    assert!(text.contains("Recovery Coach"), "names the coach: {text}");
    assert!(
        text.contains("Help Filter Group"),
        "names the group: {text}"
    );
    assert_eq!(
        group_coach(&resources, group_id, tenant_id).await,
        installed_id,
        "the group's coach changed"
    );
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id)
            .await
            .as_deref(),
        Some(installed_id.as_str()),
        "the group thread is bound"
    );
    assert_eq!(
        selected_coach(&resources, user_id, tenant_id).await,
        None,
        "a group setting does not move the owner's personal selection"
    );
}

/// A plain member may not change the group's coach: refused in their locale,
/// and nothing moves.
#[tokio::test]
async fn coach_add_in_a_group_thread_by_a_member_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "coach-add-member@test.com").await;
    let first_coach = seed_coach(&resources, user_id, tenant_id, "First Coach", "First.").await;
    let group_id = seed_group_membership(
        &resources,
        user_id,
        tenant_id,
        &first_coach,
        GroupRole::Member,
    )
    .await;
    install_recovery_coach(&resources, user_id, tenant_id).await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = group_conversation(&resources, router.clone(), &auth, tenant_id, group_id).await;
    let body = send_command(router, &auth, &conv_id, "/coach add @recovery-coach").await;

    assert_eq!(
        body.assistant.message.content,
        rendered(&resources, KEY_COACH_ASSIGN_FORBIDDEN, &[])
    );
    assert_eq!(
        group_coach(&resources, group_id, tenant_id).await,
        first_coach,
        "the group's coach is untouched"
    );
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id).await,
        None,
        "nothing was bound"
    );
}

/// `/coach remove` detaches the conversation's coach and clears the
/// selection a messaging thread would otherwise re-apply on the next
/// message; a thread with no coach says so; a group thread is refused
/// because its coach is the group's.
#[tokio::test]
async fn coach_remove_in_chat_detaches_the_coach() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "coach-remove@test.com").await;
    let coach_id = seed_coach(&resources, user_id, tenant_id, "Removable Coach", "Bye.").await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    send_command(
        router.clone(),
        &auth,
        &conv_id,
        &format!("/coach add {coach_id}"),
    )
    .await;
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id)
            .await
            .as_deref(),
        Some(coach_id.as_str())
    );

    let body = send_command(router.clone(), &auth, &conv_id, "/coach remove").await;
    assert!(
        body.assistant.message.content.contains("Removable Coach"),
        "the confirmation names the coach: {}",
        body.assistant.message.content
    );
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id).await,
        None,
        "the conversation is detached"
    );
    assert_eq!(
        selected_coach(&resources, user_id, tenant_id).await,
        None,
        "the selection pointer is cleared too"
    );

    let body = send_command(router.clone(), &auth, &conv_id, "/coach remove").await;
    assert_eq!(
        body.assistant.message.content,
        rendered(&resources, KEY_COACH_REMOVE_NOTHING, &[])
    );

    let group_id =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;
    let group_conv =
        group_conversation(&resources, router.clone(), &auth, tenant_id, group_id).await;
    let body = send_command(router, &auth, &group_conv, "/coach remove").await;
    assert_eq!(
        body.assistant.message.content,
        rendered(&resources, KEY_COACH_REMOVE_GROUP_THREAD, &[])
    );
    assert_eq!(
        group_coach(&resources, group_id, tenant_id).await,
        coach_id,
        "a group thread's coach is the group's and stays"
    );
}

/// `/coach create` drafts a persona from the conversation's coaching turns —
/// never from the command lines — parks it behind a confirm/deny pair that
/// fits Telegram's buttons, and creates nothing until the athlete confirms.
/// Confirming creates the coach with its catalogue handle, binds the thread,
/// and spends the single-use token: a second confirm is refused.
#[tokio::test]
async fn coach_create_drafts_then_confirm_creates_and_binds_once() {
    let llm = Arc::new(ProposalLlm::new());
    let resources =
        create_test_server_resources_with_chat_provider(Arc::clone(&llm) as Arc<dyn LlmProvider>)
            .await
            .unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "coach-create@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    // A command turn sits in the transcript before the exchange: the draft
    // must be built from the coaching turns alone.
    send_command(router.clone(), &auth, &conv_id, "/status").await;
    seed_coaching_exchange(&resources, &conv_id, user_id, tenant_id).await;

    let body = send_command(router.clone(), &auth, &conv_id, "/coach create").await;

    assert_eq!(
        turn_actions_title(&body),
        Some(rendered(&resources, KEY_COACH_CREATE_CARD_TITLE, &[]).as_str())
    );
    assert!(
        body.assistant.message.content.contains("Coach Tempo"),
        "the card shows the proposed title: {}",
        body.assistant.message.content
    );
    let actions = turn_actions(&body);
    assert_eq!(actions.len(), 2, "create and discard");
    let confirm = actions[0].value.clone();
    let deny = actions[1].value.clone();
    assert!(confirm.starts_with("/coach create confirm "), "{confirm}");
    assert!(deny.starts_with("/deny "), "{deny}");
    let token = confirm
        .trim_start_matches("/coach create confirm ")
        .to_owned();
    assert_eq!(
        deny,
        format!("/deny {token}"),
        "both buttons carry the same token"
    );
    for value in [&confirm, &deny] {
        assert!(
            value.len() <= TELEGRAM_CALLBACK_DATA_MAX,
            "{value} exceeds Telegram's callback-data ceiling"
        );
    }
    let prompt = llm.only_user_prompt();
    assert!(
        prompt.contains("sorties tempo"),
        "the excerpt reached the model: {prompt}"
    );
    assert!(
        !prompt.contains("/status") && !prompt.contains("/coach create"),
        "command turns never reach the proposal prompt: {prompt}"
    );
    assert_eq!(
        coach_count(&resources, user_id, tenant_id).await,
        0,
        "drafting creates nothing"
    );

    let body = send_command(router.clone(), &auth, &conv_id, &confirm).await;
    let text = &body.assistant.message.content;
    assert!(
        text.contains("Coach Tempo"),
        "names the created coach: {text}"
    );
    assert!(
        text.contains("@coach-tempo"),
        "teaches the coach's handle: {text}"
    );
    assert_eq!(coach_count(&resources, user_id, tenant_id).await, 1);
    let created = resources
        .common
        .repos
        .coaches
        .list(user_id, tenant_id, &ListCoachesFilter::with_defaults())
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.coach)
        .find(|coach| coach.title == "Coach Tempo")
        .expect("the coach was created");
    assert_eq!(created.handle.as_deref(), Some("coach-tempo"));
    assert_eq!(created.category, CoachCategory::Training);
    assert_eq!(created.system_prompt, "You are a tempo-run coach.");
    assert_eq!(
        created.tags,
        vec!["tempo".to_owned(), "marathon".to_owned()]
    );
    let created_id = created.id.to_string();
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id)
            .await
            .as_deref(),
        Some(created_id.as_str()),
        "the new coach answers in this thread"
    );
    assert_eq!(
        selected_coach(&resources, user_id, tenant_id)
            .await
            .as_deref(),
        Some(created_id.as_str())
    );

    // The token was single-use.
    let body = send_command(router.clone(), &auth, &conv_id, &confirm).await;
    assert_eq!(
        body.assistant.message.content,
        rendered(&resources, KEY_GUARDIAN_CONFIRM_NOT_FOUND, &[])
    );
    assert_eq!(coach_count(&resources, user_id, tenant_id).await, 1);

    // The created coach is on the shelf under its handle, so `/coach add`
    // reaches it from any other thread.
    let other = create_conversation(router.clone(), &auth).await;
    send_command(router, &auth, &other, "/coach add @coach-tempo").await;
    assert_eq!(
        conversation_coach(&resources, &other, user_id, tenant_id)
            .await
            .as_deref(),
        Some(created_id.as_str())
    );
    assert_eq!(llm.calls(), 1, "one draft, one model call");
}

/// An empty conversation is refused before any model is asked.
#[tokio::test]
async fn coach_create_on_an_empty_conversation_is_refused_without_a_model_call() {
    let llm = Arc::new(ProposalLlm::new());
    let resources =
        create_test_server_resources_with_chat_provider(Arc::clone(&llm) as Arc<dyn LlmProvider>)
            .await
            .unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "coach-create-empty@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let body = send_command(router, &auth, &conv_id, "/coach create").await;

    assert_eq!(
        body.assistant.message.content,
        rendered(&resources, KEY_COACH_CREATE_EMPTY, &[])
    );
    assert!(turn_actions(&body).is_empty(), "nothing to confirm");
    assert_eq!(llm.calls(), 0, "no model call for an empty conversation");
    assert_eq!(coach_count(&resources, user_id, tenant_id).await, 0);
}

/// `/deny <token>` drops a draft: nothing is created, and the token is spent.
#[tokio::test]
async fn deny_discards_a_coach_draft() {
    let llm = Arc::new(ProposalLlm::new());
    let resources =
        create_test_server_resources_with_chat_provider(Arc::clone(&llm) as Arc<dyn LlmProvider>)
            .await
            .unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "coach-create-deny@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    seed_coaching_exchange(&resources, &conv_id, user_id, tenant_id).await;

    let body = send_command(router.clone(), &auth, &conv_id, "/coach create").await;
    let actions = turn_actions(&body);
    let confirm = actions[0].value.clone();
    let deny = actions[1].value.clone();

    let body = send_command(router.clone(), &auth, &conv_id, &deny).await;
    assert_eq!(
        body.assistant.message.content,
        rendered(&resources, KEY_COACH_CREATE_DISCARDED, &[])
    );
    let body = send_command(router, &auth, &conv_id, &confirm).await;
    assert_eq!(
        body.assistant.message.content,
        rendered(&resources, KEY_GUARDIAN_CONFIRM_NOT_FOUND, &[]),
        "a discarded draft cannot be confirmed"
    );
    assert_eq!(coach_count(&resources, user_id, tenant_id).await, 0);
}

/// The confirm step enforces the same per-user coach cap as `POST /api/coaches`:
/// at the cap, the draft is refused with the numbers and nothing is created.
#[tokio::test]
async fn coach_create_confirm_is_refused_at_the_coach_quota() {
    let llm = Arc::new(ProposalLlm::new());
    let resources =
        create_test_server_resources_with_chat_provider(Arc::clone(&llm) as Arc<dyn LlmProvider>)
            .await
            .unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "coach-create-quota@test.com").await;
    // The compiled-in cap is three coaches per athlete.
    for n in 1..=3 {
        seed_coach(
            &resources,
            user_id,
            tenant_id,
            &format!("Coach {n}"),
            "Full.",
        )
        .await;
    }
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    seed_coaching_exchange(&resources, &conv_id, user_id, tenant_id).await;

    let body = send_command(router.clone(), &auth, &conv_id, "/coach create").await;
    let confirm = turn_actions(&body)[0].value.clone();
    let body = send_command(router, &auth, &conv_id, &confirm).await;

    assert_eq!(
        body.assistant.message.content,
        rendered(&resources, KEY_COACH_CREATE_QUOTA, &["3", "3"])
    );
    assert_eq!(coach_count(&resources, user_id, tenant_id).await, 3);
    assert_eq!(
        conversation_coach(&resources, &conv_id, user_id, tenant_id).await,
        None
    );
}

#[tokio::test]
async fn unknown_slash_command_short_circuits_before_llm() {
    let resources = create_test_server_resources().await.unwrap();
    let (_uid, _tid, auth) = seed_user_tenant(&resources, "unknown-cmd@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/notarealcommand"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    // No card, no actions — unknown commands render as plain localized text.
    assert!(turn_actions_title(&body).is_none());
    assert!(turn_actions(&body).is_empty());
    assert_eq!(body.telemetry.model, "command");
}

#[tokio::test]
async fn client_platform_header_shapes_channel_type_without_breaking() {
    // Web and mobile currently share the same dispatch semantics, but
    // the X-Client-Platform header still rides through the pipeline for
    // analytics. Smoke-test that mobile flag is accepted (no 400).
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "mobile-cmd@test.com").await;
    let _coach = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Recovery Coach",
        "Rest-day specialist.",
    )
    .await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .header("x-client-platform", "mobile")
        .json(&json!({"content": "/coach"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
}

/// A command turn is transcript, like Telegram keeps a bot's answer: both rows
/// land, stamped `command`, under the ids the response reports — so a reload
/// shows the same exchange the athlete just saw — and neither row ever reaches
/// the prompt of the next coaching turn.
#[tokio::test]
async fn slash_command_turn_is_persisted_to_history_and_kept_out_of_the_prompt() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "persisted-cmd@test.com").await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/status"}))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    assert_eq!(
        body.user_message.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON),
        "the athlete's own line is stamped too"
    );

    // Both rows landed, under the ids the turn reported.
    let history = resources
        .common
        .repos
        .chat
        .get_messages(&conv_id, &user_id.to_string(), tenant_id)
        .await
        .unwrap();
    assert_eq!(history.len(), 2, "the command line and its answer");
    assert_eq!(history[0].id, body.user_message.id);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].content, "/status");
    assert_eq!(
        history[0].finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    assert_eq!(history[1].id, body.assistant.message.id);
    assert_eq!(history[1].role, "assistant");
    assert_eq!(history[1].content, body.assistant.message.content);
    assert_eq!(
        history[1].finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );
    assert_eq!(
        body.conversation_updated_at,
        resources
            .common
            .repos
            .chat
            .get_conversation(&conv_id, &user_id.to_string(), tenant_id)
            .await
            .unwrap()
            .unwrap()
            .updated_at,
        "the turn reports the conversation as the persisted rows left it"
    );

    // The read path carries the stamp, so a client tells the rows apart on
    // reload the way it does on the live turn.
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let listing: MessagesListResponse = resp.json();
    assert_eq!(listing.messages.len(), 2);
    assert!(listing
        .messages
        .iter()
        .all(|m| m.finish_reason.as_deref() == Some(COMMAND_FINISH_REASON)));

    // The transcript holds the turn; the prompt never does. The history loader
    // every coaching turn reads through is blind to both rows, while an
    // ordinary user row loaded the same way comes through.
    let prompt_history = get_conversation_history(
        resources.common.repos.chat.as_ref(),
        &conv_id,
        &user_id.to_string(),
        tenant_id,
        50,
    )
    .await
    .unwrap();
    assert!(
        prompt_history.is_empty(),
        "command rows must not reach the prompt, found {} row(s)",
        prompt_history.len()
    );

    // The caller just read their own answer: no unread badge on the row, and
    // the reply is the row's preview.
    let resp = AxumTestRequest::get("/api/chat/conversations")
        .header("authorization", &auth)
        .send(router)
        .await;
    let list: ConversationListResponse = resp.json();
    let row = list.conversations.iter().find(|c| c.id == conv_id).unwrap();
    assert_eq!(row.unread_count, 0);
    assert_eq!(row.message_count, 2);
    // The preview is the reply's head on one line: whitespace runs collapsed.
    let preview = &row.last_message.as_ref().unwrap().preview;
    let one_line = body
        .assistant
        .message
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        one_line.starts_with(preview.as_str()),
        "the row preview is the head of the command reply: {preview}"
    );
}

/// A card's controls survive the reload: `/coach` is answered with buttons,
/// and reading the thread back returns the same buttons on the persisted row.
#[tokio::test]
async fn coach_command_persists_its_actions_for_reload() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "persisted-card@test.com").await;
    let _coach = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Persisted Card Coach",
        "Keeps its buttons.",
    )
    .await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/coach"}))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();
    let live_title = turn_actions_title(&body).map(ToOwned::to_owned);
    let live_actions: Vec<(String, String, String)> = turn_actions(&body)
        .into_iter()
        .map(|a| (a.label.clone(), a.action_type.clone(), a.value.clone()))
        .collect();
    assert!(!live_actions.is_empty(), "the picker carries buttons");

    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let listing: MessagesListResponse = resp.json();
    let reply = listing
        .messages
        .iter()
        .find(|m| m.role == "assistant")
        .expect("the persisted reply");
    assert_eq!(reply.id, body.assistant.message.id);
    let stored = reply
        .actions
        .as_ref()
        .expect("the reply keeps its controls");
    assert_eq!(stored.title, live_title);
    let stored_actions: Vec<(String, String, String)> = stored
        .actions
        .iter()
        .map(|a| (a.label.clone(), a.action_type.clone(), a.value.clone()))
        .collect();
    assert_eq!(stored_actions, live_actions);
    assert!(
        reply.scene_blocks.is_none(),
        "controls are not visuals: nothing for photograveur to resolve"
    );
}

/// An owner typing alone is offered nothing that acts on "the group": a solo
/// thread names no group, and the messaging DM's ambient fallback — the first
/// group the athlete belongs to — is off in the app. The commands that read
/// the athlete's own groups stay.
#[tokio::test]
async fn owner_in_a_solo_thread_is_not_offered_group_management() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-solo-owner@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Solo Thread Coach",
        "Coaches a group.",
    )
    .await;
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = fetch_help(router, &auth).await;

    for hidden in [
        "/group invite",
        "/group coach",
        "/group respond",
        "/group consent",
    ] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` acts on the thread's group, and a solo thread has none:\n{text}"
        );
    }
    for shown in [
        "/group status",
        "/group members",
        "/group leave",
        "/coach assign",
    ] {
        assert!(
            text.contains(shown),
            "`{shown}` reads the athlete's own groups, so it stays listed:\n{text}"
        );
    }
}

#[tokio::test]
async fn pillars_command_activates_onboarding_mode_no_llm_call() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "context-cmd@test.com").await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/pillars"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: TurnResponse = resp.json();

    // Deterministic command dispatch — no LLM turn, marked so clients skip
    // streaming UI. The greeting opens the guided walk on the North Star.
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON),
        "/pillars must dispatch as a command"
    );
    assert!(
        body.assistant.message.content.contains("North Star"),
        "expected the onboarding greeting, got: {}",
        body.assistant.message.content
    );

    // The handler flips the conversation into onboarding mode: subsequent
    // (non-command) turns run the guided pillar walk until coverage completes.
    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conv_id, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .expect("conversation exists");
    assert!(
        OnboardingState::from_column(conv.onboarding_state.as_deref()).is_some(),
        "/pillars must activate onboarding_state on the conversation"
    );
}

/// An athlete who belongs to no group is shown no group commands.
///
/// Every one of them resolves a group before doing anything, so for this user
/// they can only answer "you are not a member of a group" — listing them
/// teaches nothing and reads as a broken bot.
#[tokio::test]
async fn help_hides_group_commands_from_an_athlete_with_no_group() {
    let resources = create_test_server_resources().await.unwrap();
    let (_uid, _tid, auth) = seed_user_tenant(&resources, "help-nogroup@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = fetch_help(router, &auth).await;

    for hidden in [
        "/group consent",
        "/group invite",
        "/group members",
        "/group respond",
        "/group status",
        "/group leave",
        "/group coach",
        "/coach assign",
    ] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` needs a group this athlete does not have:\n{text}"
        );
    }
    // The domain survives with exactly the commands that need no group: the
    // list (possibly empty) and the two ways into one — creating a group and
    // joining one by invite code.
    let group_commands: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("/group"))
        .map(|rest| rest.split(" — ").next().unwrap_or(rest).trim())
        .map(|args| args.split_whitespace().next().unwrap_or(""))
        .collect();
    assert_eq!(
        group_commands,
        vec!["", "create", "join"],
        "only `/group`, `/group create` and `/group join` survive for a groupless athlete:\n{text}"
    );
    // Everything that does not need a group is untouched.
    for shown in ["/status", "/plan", "/coach ", "/pillars", "/timezone"] {
        assert!(
            text.contains(shown),
            "`{shown}` must still be listed:\n{text}"
        );
    }
}

/// A plain member sees the member-level group commands and none of the
/// admin-only ones — the four whose handlers check `can_modify_settings` /
/// `can_manage_members` before acting.
#[tokio::test]
async fn help_hides_admin_only_commands_from_a_plain_group_member() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-member@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Group Coach",
        "Coaches a group.",
    )
    .await;
    let group_id =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = help_in_group_thread(&resources, router, &auth, tenant_id, group_id).await;

    for shown in [
        "/group consent",
        "/group members",
        "/group status",
        "/group leave",
    ] {
        assert!(
            text.contains(shown),
            "a member can run `{shown}`, so it must be listed:\n{text}"
        );
    }
    for hidden in [
        "/group invite",
        "/group coach",
        "/group respond",
        "/coach assign",
    ] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` is owner/admin only and must not be listed for a member:\n{text}"
        );
    }
}

/// The group owner sees the admin-only commands the member does not.
#[tokio::test]
async fn help_shows_admin_only_commands_to_a_group_owner() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-owner@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Owned Group Coach",
        "Coaches a group.",
    )
    .await;
    let group_id =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = help_in_group_thread(&resources, router, &auth, tenant_id, group_id).await;

    for shown in [
        "/group invite",
        "/group coach coach-name",
        "/group respond mentions|all",
        "/coach assign coach-id group-id",
        "/group consent yes|no",
    ] {
        assert!(
            text.contains(shown),
            "an owner can run `{shown}`, so it must be listed:\n{text}"
        );
    }
}

#[tokio::test]
async fn help_shows_argument_options_localized_headings_and_stable_order() {
    // /help is the only discovery surface for command arguments: a reader who
    // cannot see `yes|no` has no way to learn that /group consent takes one.
    // Pins all three properties of the rendered list — argument signatures,
    // localized domain headings, and deterministic ordering.
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-args@test.com").await;
    // Owner standing so every command is listed — the signatures, not the
    // role filter, are what this test pins.
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Args Group Coach",
        "Coaches a group.",
    )
    .await;
    let group_id =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;
    let router = ChatRoutes::routes(Arc::clone(&resources));

    let text = help_in_group_thread(&resources, router, &auth, tenant_id, group_id).await;

    // Every command whose handler reads an argument advertises what it takes.
    for signature in [
        "/group consent yes|no",
        "/group respond mentions|all",
        "/group coach coach-name",
        "/plan [week|today]",
        // Named, not a `pillar` placeholder: the athlete must be able to type
        // one of these without knowing the internal DB slugs.
        "/pillars [full|training|fuelling|sleep|mental|community|substances]",
        "/timezone area/city",
        "/coach add @handle",
        "/coach create [confirm token]",
        "/coach assign coach-id group-id",
        "/confirm action-id",
        "/deny action-id",
    ] {
        assert!(
            text.contains(signature),
            "/help must show `{signature}`, got:\n{text}"
        );
    }

    // Argument-free commands stay bare — no invented placeholder.
    assert!(
        text.contains("/status — ") && text.contains("/logout — "),
        "argument-free commands must render without a signature:\n{text}"
    );

    // Domain headings are localized (fr is the default locale). A raw domain
    // slug in the output means the heading has no messaging string.
    assert!(
        text.contains("Entraînement:"),
        "training domain heading must be localized:\n{text}"
    );
    assert!(
        !text.contains("training:"),
        "raw domain slug leaked into /help:\n{text}"
    );

    // Commands are sorted within their domain — the registry stores them in a
    // HashMap, so an unsorted render reshuffles on every process start.
    // Compare the command halves, not whole lines: the em-dash separator sorts
    // after letters, so a whole-line sort is not the order commands are in.
    let group_block: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with("/group"))
        .map(|l| l.trim().split(" — ").next().unwrap_or(l))
        .collect();
    assert!(
        group_block.len() >= 7,
        "expected the full /group block, got {group_block:?}"
    );
    let mut sorted = group_block.clone();
    sorted.sort_unstable();
    assert_eq!(
        group_block, sorted,
        "/help must list commands in a stable sorted order"
    );
}

/// `/coach assign` names its own group in the arguments, so an owner of *any*
/// group can run it — even from a room where they are only a plain member.
///
/// `/group invite`, `/group coach` and `/group respond` resolve the
/// conversation's group and check the caller's role there, so the ambient role
/// decides them. `CoachAssignHandler` does not: it reads `get_member` on the
/// group id the caller typed. Deciding it on the ambient role hid a command
/// that works.
#[tokio::test]
async fn help_shows_coach_assign_to_an_owner_who_is_a_member_of_the_ambient_group() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "help-two-groups@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Two Group Coach",
        "Coaches two groups.",
    )
    .await;

    // Owner of one group, plain member of another.
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;
    let member_group =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;

    // Bind the conversation to the group where the caller is only a member, so
    // the ambient role is Member no matter how the group list happens to sort.
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conv_id, Some(&member_group.to_string()), tenant_id)
        .await
        .unwrap();

    let text = help_in_conversation(router, &auth, &conv_id).await;

    assert!(
        text.contains("/coach assign coach-id group-id"),
        "an owner of another group can run `/coach assign`, so it must be listed:\n{text}"
    );
    // The ambient-group commands are still filtered on the ambient role — this
    // is what proves the conversation resolved to the member group, and that
    // the fix did not simply widen the filter for everything.
    for hidden in ["/group invite", "/group coach", "/group respond"] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` acts on the ambient group, where this caller is only a member:\n{text}"
        );
    }
}

/// `/group status`, `/group members` and `/group leave` read
/// `list_groups_for_user().first()`, never the conversation's group — so
/// belonging to *any* group is enough to run them.
///
/// Deciding them on the conversation's group hid all three from someone
/// sitting in a room bound to a group they are not a member of, even though
/// the commands would have answered about their own group. This is the same
/// defect class as `/coach assign`, in three more commands, and it is why
/// `/help` asks each handler instead of applying one shared rule to all of
/// them.
#[tokio::test]
async fn help_shows_own_group_commands_when_the_room_belongs_to_another_group() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "help-foreign-room@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Foreign Room Coach",
        "Coaches a group.",
    )
    .await;

    // The caller's own group, which `/group status` would answer about.
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;

    // A second group the caller does NOT belong to, bound to the conversation.
    let (other_id, _, _) = seed_user_tenant(&resources, "help-room-owner@test.com").await;
    let foreign_group =
        seed_group_membership(&resources, other_id, tenant_id, &coach_id, GroupRole::Owner).await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conv_id, Some(&foreign_group.to_string()), tenant_id)
        .await
        .unwrap();

    let text = help_in_conversation(router, &auth, &conv_id).await;

    for shown in ["/group status", "/group members", "/group leave"] {
        assert!(
            text.contains(shown),
            "`{shown}` reads the caller's own group, so it must be listed even \
             though this room belongs to a group they are not in:\n{text}"
        );
    }
    // The commands that really do act on this room's group stay hidden — the
    // caller is not a member of it, so they would refuse.
    for hidden in [
        "/group invite",
        "/group coach",
        "/group respond",
        "/group consent",
    ] {
        assert!(
            !text.contains(hidden),
            "`{hidden}` acts on this room's group, which the caller is not in:\n{text}"
        );
    }
}
