// ABOUTME: The unified conversation list over REST — kind facts, preview, unread badge, paging, read marker
// ABOUTME: One list for every thread a participant is in, whatever surface opened it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::StatusCode;
use futures_util::stream;
use serde_json::json;
use uuid::Uuid;

use common::{
    create_test_server_resources, create_test_server_resources_with_llm,
    create_test_user_with_plan, generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::coach_fixtures::{install_catalogue_coach, publish_catalogue_coach};
use pierre_chat_pipeline::stages::persistence::get_conversation_history;
use pierre_chat_pipeline::stages::prompt_builder::build_llm_messages;
use pierre_chat_pipeline::{
    CommandPersistence, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnRequest,
};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{AddMessageParams, ConversationTurnId, TenantId, COMMAND_FINISH_REASON};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatRoutes, ConversationListResponse, ConversationResponse, ConversationSummaryResponse,
    MessagesListResponse, TurnResponse, MAX_LIST_LIMIT,
};

/// One reply, real token counts, no tools — enough to run a coaching turn
/// through the real pipeline and watch what it does to the list.
struct ReplyingMockProvider;

const MOCK_REPLY: &str = "Ta semaine est bien dosée: garde le volume et dors davantage.";

#[async_trait]
impl LlmProvider for ReplyingMockProvider {
    fn name(&self) -> &'static str {
        "replying_mock"
    }
    fn display_name(&self) -> &'static str {
        "Replying Mock LLM (list pin)"
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

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: MOCK_REPLY.to_owned(),
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(25, 15, 40)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: String::new(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

struct Fixture {
    resources: Arc<ServerContext>,
    router: axum::Router,
    tenant_id: TenantId,
    owner_id: Uuid,
    owner_auth: String,
    /// Same tenant as the owner; gets added to threads.
    member_id: Uuid,
    member_auth: String,
    /// Same tenant as the owner; never added.
    stranger_auth: String,
}

/// An owner, a member and a stranger in one tenant, each with a token scoped
/// to it, so every refusal under test is a membership decision.
async fn setup_on(resources: Arc<ServerContext>) -> Fixture {
    let repos = resources.coach.database.repositories();
    let (owner_id, owner, _) = create_test_user_with_plan(
        &resources.coach.database,
        "list-owner@test.com",
        "professional",
    )
    .await
    .unwrap();
    let (member_id, member, _) = create_test_user_with_plan(
        &resources.coach.database,
        "list-member@test.com",
        "professional",
    )
    .await
    .unwrap();
    let (stranger_id, stranger, _) = create_test_user_with_plan(
        &resources.coach.database,
        "list-stranger@test.com",
        "professional",
    )
    .await
    .unwrap();

    let tenant_id = repos
        .tenants
        .list_for_user(owner_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    for id in [member_id, stranger_id] {
        repos.users.update_tenant_id(id, tenant_id).await.unwrap();
    }
    let tenant_token = |user| {
        format!(
            "Bearer {}",
            resources
                .auth
                .auth_manager
                .generate_token_with_tenant(
                    user,
                    &resources.auth.jwks_manager,
                    Some(tenant_id.to_string()),
                )
                .unwrap()
        )
    };
    let member_auth = tenant_token(&member);
    let stranger_auth = tenant_token(&stranger);
    let owner_auth = format!("Bearer {}", generate_test_token(&resources, &owner).await);
    let router = ChatRoutes::routes(Arc::clone(&resources));

    Fixture {
        resources,
        router,
        tenant_id,
        owner_id,
        owner_auth,
        member_id,
        member_auth,
        stranger_auth,
    }
}

async fn setup() -> Fixture {
    setup_on(create_test_server_resources().await.unwrap()).await
}

async fn create_conversation(fx: &Fixture, body: serde_json::Value) -> ConversationResponse {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.owner_auth)
        .json(&body)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    resp.json()
}

/// Append one row straight through the repository — a row the list must
/// count and preview but that moved nobody's read marker.
async fn add_row(
    fx: &Fixture,
    conversation_id: &str,
    user_id: Uuid,
    role: &str,
    content: &str,
) -> String {
    fx.resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: fx.tenant_id,
            conversation_id,
            user_id: &user_id.to_string(),
            role,
            content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap()
        .id
}

async fn list(fx: &Fixture, auth: &str, query: &str) -> ConversationListResponse {
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations{query}"))
        .header("authorization", auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json()
}

async fn row(fx: &Fixture, auth: &str, conversation_id: &str) -> ConversationSummaryResponse {
    list(fx, auth, "")
        .await
        .conversations
        .into_iter()
        .find(|c| c.id == conversation_id)
        .expect("the conversation is listed")
}

async fn add_member(fx: &Fixture, conversation_id: &str) {
    let resp = AxumTestRequest::post(&format!(
        "/api/chat/conversations/{conversation_id}/participants"
    ))
    .header("authorization", &fx.owner_auth)
    .json(&json!({ "user_id": fx.member_id.to_string() }))
    .send(fx.router.clone())
    .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
}

async fn mark_read(
    fx: &Fixture,
    auth: &str,
    conversation_id: &str,
    body: Option<serde_json::Value>,
) -> StatusCode {
    let request = AxumTestRequest::post(&format!("/api/chat/conversations/{conversation_id}/read"))
        .header("authorization", auth);
    let request = match body {
        Some(body) => request.json(&body),
        None => request,
    };
    request.send(fx.router.clone()).await.status_code()
}

async fn mark_unread(fx: &Fixture, auth: &str, conversation_id: &str) -> StatusCode {
    AxumTestRequest::delete(&format!("/api/chat/conversations/{conversation_id}/read"))
        .header("authorization", auth)
        .send(fx.router.clone())
        .await
        .status_code()
}

/// A long markdown reply carrying a chart marker, the shape a coach's
/// persisted row has after the visual was lifted out of it.
const LONG_REPLY: &str =
    "Ta charge grimpe depuis trois semaines.\n\n⟦viz:0⟧\n\nC'est pourquoi on coupe jeudi:   \
     une sortie facile de quarante minutes, puis deux jours complets de repos avant la longue \
     sortie du dimanche, que l'on garde à l'allure conversationnelle du début à la fin.";

#[tokio::test]
async fn every_row_carries_its_kind_facts_preview_and_counts() {
    let fx = setup().await;
    let repos = &fx.resources.common.repos;

    // A catalogue coach, installed for the owner: the row shows its @handle.
    let origin = publish_catalogue_coach(
        repos,
        fx.owner_id,
        fx.tenant_id,
        "Recovery Coach",
        "You are the recovery coach.",
    )
    .await;
    let installed = install_catalogue_coach(repos, origin, fx.owner_id, fx.tenant_id).await;

    // A coaching group the owner belongs to: the row shows its name.
    let now = chrono::Utc::now();
    let group_id = Uuid::new_v4();
    repos
        .groups
        .create_group(
            fx.tenant_id,
            &CoachingGroup {
                id: group_id,
                tenant_id: fx.tenant_id.to_string(),
                name: "Marathon Squad".to_owned(),
                description: None,
                coach_id: installed.id.to_string(),
                owner_id: fx.owner_id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
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
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id: fx.owner_id,
            tenant_id: fx.tenant_id.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();

    // Three threads of three kinds, oldest first.
    let telegram = create_conversation(&fx, json!({ "title": "Messaging: telegram" })).await;
    assert!(repos
        .chat
        .set_conversation_channel(
            &telegram.id,
            &fx.owner_id.to_string(),
            fx.tenant_id,
            "telegram"
        )
        .await
        .unwrap());
    let grouped = create_conversation(
        &fx,
        json!({ "title": "Squad talk", "group_id": group_id.to_string() }),
    )
    .await;
    let coached = create_conversation(
        &fx,
        json!({ "title": "Recovery", "coach_id": installed.id.to_string() }),
    )
    .await;
    add_row(
        &fx,
        &coached.id,
        fx.owner_id,
        "user",
        "Comment va ma charge?",
    )
    .await;
    add_row(
        &fx,
        &coached.id,
        fx.owner_id,
        "tool_result",
        "<tool_result>…</tool_result>",
    )
    .await;
    add_row(&fx, &coached.id, fx.owner_id, "assistant", LONG_REPLY).await;

    let page = list(&fx, &fx.owner_auth, "").await;
    assert_eq!(page.total, 3);
    let ids: Vec<&str> = page.conversations.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            coached.id.as_str(),
            grouped.id.as_str(),
            telegram.id.as_str()
        ],
        "newest activity first"
    );

    let coach_row = &page.conversations[0];
    assert_eq!(
        coach_row.coach_id.as_deref(),
        Some(installed.id.to_string().as_str())
    );
    assert_eq!(coach_row.coach_handle.as_deref(), Some("recovery-coach"));
    assert_eq!(coach_row.coach_title.as_deref(), Some("Recovery Coach"));
    assert_eq!(coach_row.message_count, 2, "the tool row is not a turn");
    assert_eq!(
        coach_row.unread_count, 2,
        "rows written behind the marker are unread"
    );
    assert_eq!(coach_row.channel_type.as_deref(), Some("web"));
    let preview = coach_row.last_message.as_ref().expect("the newest row");
    assert_eq!(preview.role, "assistant");
    assert!(
        !preview.preview.contains('⟦'),
        "markers are stripped: {}",
        preview.preview
    );
    assert!(
        !preview.preview.contains('\n'),
        "one line: {}",
        preview.preview
    );
    assert!(
        !preview.preview.contains("  "),
        "whitespace collapsed: {}",
        preview.preview
    );
    assert!(
        preview.preview.starts_with(
            "Ta charge grimpe depuis trois semaines. C'est pourquoi on coupe jeudi: une sortie"
        ),
        "{}",
        preview.preview
    );
    assert_eq!(
        preview.preview.chars().count(),
        120,
        "cut at 120 characters"
    );
    assert!(!preview.created_at.is_empty());

    let group_row = &page.conversations[1];
    assert_eq!(
        group_row.group_id.as_deref(),
        Some(group_id.to_string().as_str())
    );
    assert_eq!(group_row.group_name.as_deref(), Some("Marathon Squad"));
    assert!(group_row.coach_handle.is_none());
    assert!(group_row.last_message.is_none());
    assert_eq!(group_row.unread_count, 0);

    let telegram_row = &page.conversations[2];
    assert_eq!(telegram_row.channel_type.as_deref(), Some("telegram"));
    assert!(telegram_row.group_id.is_none());
}

#[tokio::test]
async fn paging_is_clamped_and_the_total_is_the_participants_count() {
    let fx = setup().await;
    for i in 1..=3 {
        create_conversation(&fx, json!({ "title": format!("Thread {i}") })).await;
    }
    // A thread the owner was added to counts in their total like their own.
    let shared = fx
        .resources
        .common
        .repos
        .chat
        .create_conversation(
            &fx.member_id.to_string(),
            fx.tenant_id,
            "Member's",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    fx.resources
        .common
        .repos
        .chat
        .add_participant(
            &shared.id,
            fx.tenant_id,
            &fx.owner_id.to_string(),
            &fx.member_id.to_string(),
        )
        .await
        .unwrap();

    let page = list(&fx, &fx.owner_auth, "?limit=2").await;
    assert_eq!(page.conversations.len(), 2);
    assert_eq!(page.total, 4);
    assert_eq!(
        page.conversations[0].id, shared.id,
        "the newest thread leads"
    );

    let page = list(&fx, &fx.owner_auth, "?limit=0&offset=-3").await;
    assert_eq!(
        page.conversations.len(),
        1,
        "limit clamps up to one, offset up to zero"
    );
    assert_eq!(page.total, 4);

    let page = list(
        &fx,
        &fx.owner_auth,
        &format!("?limit={}", MAX_LIST_LIMIT * 10),
    )
    .await;
    assert_eq!(
        page.conversations.len(),
        4,
        "an oversized limit is capped, not refused"
    );

    let page = list(&fx, &fx.owner_auth, "?limit=3&offset=3").await;
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(page.total, 4);

    // The member's total is their own; the stranger's is nothing.
    assert_eq!(list(&fx, &fx.member_auth, "").await.total, 1);
    let empty = list(&fx, &fx.stranger_auth, "").await;
    assert_eq!(empty.total, 0);
    assert!(empty.conversations.is_empty());
}

#[tokio::test]
async fn reading_the_thread_and_the_read_routes_move_the_unread_badge() {
    let fx = setup().await;
    let conv = create_conversation(&fx, json!({ "title": "Badge" })).await;
    let first = add_row(&fx, &conv.id, fx.owner_id, "user", "one").await;
    add_row(&fx, &conv.id, fx.owner_id, "assistant", "two").await;
    assert_eq!(row(&fx, &fx.owner_auth, &conv.id).await.unread_count, 2);

    // Opening the thread reads it.
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{}/messages", conv.id))
        .header("authorization", &fx.owner_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    assert_eq!(resp.json::<MessagesListResponse>().messages.len(), 2);
    assert_eq!(row(&fx, &fx.owner_auth, &conv.id).await.unread_count, 0);

    // A member added later has read nothing; what they post is new to the owner.
    add_member(&fx, &conv.id).await;
    assert_eq!(row(&fx, &fx.member_auth, &conv.id).await.unread_count, 2);
    add_row(&fx, &conv.id, fx.member_id, "user", "three from the member").await;
    let owner_row = row(&fx, &fx.owner_auth, &conv.id).await;
    assert_eq!(owner_row.unread_count, 1);
    assert_eq!(
        owner_row.last_message.as_ref().unwrap().preview,
        "three from the member"
    );
    assert_eq!(owner_row.last_message.as_ref().unwrap().role, "user");

    // POST …/read with no body: the whole thread.
    assert_eq!(
        mark_read(&fx, &fx.owner_auth, &conv.id, None).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(row(&fx, &fx.owner_auth, &conv.id).await.unread_count, 0);
    // The owner's marker is the owner's alone — and a row written straight
    // through the repository moves nobody's, so the member's own post counts
    // for them too until they open the thread.
    assert_eq!(row(&fx, &fx.member_auth, &conv.id).await.unread_count, 3);

    // DELETE …/read: mark unread — every turn counts again.
    assert_eq!(
        mark_unread(&fx, &fx.owner_auth, &conv.id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(row(&fx, &fx.owner_auth, &conv.id).await.unread_count, 3);

    // POST …/read up to a named row: the rows after it stay unread.
    assert_eq!(
        mark_read(
            &fx,
            &fx.owner_auth,
            &conv.id,
            Some(json!({ "up_to_message_id": first }))
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(row(&fx, &fx.owner_auth, &conv.id).await.unread_count, 2);

    // A row that is not in this thread, and a stranger, are both 404.
    assert_eq!(
        mark_read(
            &fx,
            &fx.owner_auth,
            &conv.id,
            Some(json!({ "up_to_message_id": "not-a-row" }))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        mark_read(&fx, &fx.stranger_auth, &conv.id, None).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        mark_unread(&fx, &fx.stranger_auth, &conv.id).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        row(&fx, &fx.owner_auth, &conv.id).await.unread_count,
        2,
        "nothing above moved it"
    );
}

#[tokio::test]
async fn a_command_turn_is_read_by_its_author_and_new_to_the_other_participant() {
    let fx = setup().await;
    let conv = create_conversation(&fx, json!({ "title": "Commands" })).await;
    add_member(&fx, &conv.id).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{}/messages", conv.id))
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "content": "/status" }))
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let turn: TurnResponse = resp.json();
    assert_eq!(
        turn.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON)
    );

    let owner_row = row(&fx, &fx.owner_auth, &conv.id).await;
    assert_eq!(
        owner_row.message_count, 2,
        "the command line and its answer are turns"
    );
    assert_eq!(owner_row.unread_count, 0, "the author just read the answer");
    let member_row = row(&fx, &fx.member_auth, &conv.id).await;
    assert_eq!(member_row.unread_count, 2, "the other participant has not");
    assert_eq!(member_row.last_message.as_ref().unwrap().role, "assistant");

    // The command rows are transcript, not prompt: the loader every coaching
    // turn reads through skips them, and a real row loaded beside them replays
    // alone.
    add_row(&fx, &conv.id, fx.owner_id, "user", "et pour samedi?").await;
    let history = get_conversation_history(
        fx.resources.common.repos.chat.as_ref(),
        &conv.id,
        &fx.owner_id.to_string(),
        fx.tenant_id,
        50,
    )
    .await
    .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, "et pour samedi?");
    let every_row = fx
        .resources
        .common
        .repos
        .chat
        .get_messages(&conv.id, &fx.owner_id.to_string(), fx.tenant_id)
        .await
        .unwrap();
    assert_eq!(every_row.len(), 3, "the transcript keeps all three");
    let (messages, _) = build_llm_messages(None, &every_row);
    assert_eq!(messages.len(), 1);
    assert!(!messages[0].content.contains("/status"));
}

#[tokio::test]
async fn a_coaching_turn_reads_the_thread_for_its_author_only() {
    let fx = setup_on(
        create_test_server_resources_with_llm(Arc::new(ReplyingMockProvider))
            .await
            .unwrap(),
    )
    .await;
    let conv = create_conversation(&fx, json!({ "title": "Turn", "model": "mock-model" })).await;
    add_member(&fx, &conv.id).await;

    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    });
    let ctx = fx.resources.chat_pipeline_context();
    let served = pierre_chat_pipeline::execute(
        &ctx,
        TurnRequest {
            conversation_id: conv.id.clone(),
            user_id: fx.owner_id,
            conversation_tenant_id: fx.tenant_id,
            tool_tenant_id: fx.tenant_id,
            content: "Comment se présente ma semaine d'entraînement?".to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            channel_type: "web",
            is_direct_message: true,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            hooks: PipelineHooks::none(),
        },
        &profile,
    )
    .await
    .expect("the turn is served");
    let ServedTurn::Pipeline(envelope) = served else {
        panic!("a plain-prose turn runs the pipeline");
    };
    let reply = envelope.assistant.message.content.clone();
    assert!(
        !reply.trim().is_empty(),
        "the turn carries the coach's reply"
    );

    let owner_row = row(&fx, &fx.owner_auth, &conv.id).await;
    assert_eq!(owner_row.message_count, 2);
    assert_eq!(owner_row.unread_count, 0, "the author's own turn is read");
    let newest = owner_row.last_message.as_ref().unwrap();
    assert_eq!(newest.role, "assistant");
    // The preview is the persisted reply's head, on one line.
    let one_line: String = reply.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        one_line.starts_with(newest.preview.as_str()),
        "preview {:?} is the head of {one_line:?}",
        newest.preview
    );

    let member_row = row(&fx, &fx.member_auth, &conv.id).await;
    assert_eq!(
        member_row.unread_count, 2,
        "both rows are new to the other participant"
    );
}
