// ABOUTME: A `@handle` mention hands one turn to an installed coach and nothing after it
// ABOUTME: Driven through the turn ladder on web and the Telegram webhook, asserting the prompt the model got
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `@handle` routes one turn — the `ultracode` shape.
//!
//! The assertion point is the system prompt the model actually received: a
//! `@recovery-coach` turn is assembled from the recovery coach's persona, the
//! next plain turn from the conversation's own coach, and the conversation row
//! never moves. Both the web ladder and a real Telegram webhook are driven,
//! because the two surfaces used to run separate ladders, and a mention
//! grammar that lived on one of them would have missed the other.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use common::create_test_server_resources_with_llm;
use futures_util::stream;
use helpers::coach_fixtures::{install_catalogue_coach, publish_catalogue_coach};
use pierre_chat_pipeline::stages::coach_mention::{mention_candidates, strip_mention};
use pierre_chat_pipeline::{
    CommandPersistence, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnRequest,
};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
    StreamChunk, TokenUsage,
};
use pierre_core::models::coaches::{CoachCategory, CoachHandle, CreateCoachRequest};
use pierre_core::models::{ConnectionType, ConversationTurnId, Tenant, TenantId, User, UserStatus};
use pierre_mcp_server::mcp::resources::ServerContext;
use serial_test::serial;
use tokio::task::spawn_blocking;
use tokio::time::sleep;
use uuid::Uuid;

/// Marker each persona's system prompt carries, so the assembled prompt says
/// which coach it was built from.
const RECOVERY_MARKER: &str = "RECOVERY_PERSONA_MARKER";
const TEMPO_MARKER: &str = "TEMPO_PERSONA_MARKER";
const STRENGTH_MARKER: &str = "STRENGTH_PERSONA_MARKER";

/// The message the athlete types, and what the model reads once the resolved
/// token is stripped.
const MENTION_TURN: &str = "@recovery-coach comment gérer ma récupération cette semaine ?";
const MENTION_TURN_FOR_MODEL: &str = "comment gérer ma récupération cette semaine ?";
const PLAIN_TURN: &str = "Et jeudi, quel tempo viser pour ma sortie longue ?";

type RequestLog = Arc<Mutex<Vec<Vec<ChatMessage>>>>;

/// Records every request the pipeline sends, so the test reads the prompt
/// the model was given rather than inferring it from the reply.
///
/// Mock is test-only: the LLM boundary is the assertion point itself.
struct CapturingLlm {
    requests: RequestLog,
}

impl CapturingLlm {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn log(&self) -> RequestLog {
        Arc::clone(&self.requests)
    }

    fn record(&self, request: &ChatRequest) {
        self.requests.lock().unwrap().push(request.messages.clone());
    }
}

#[async_trait]
impl LlmProvider for CapturingLlm {
    fn name(&self) -> &'static str {
        "capturing_mock"
    }
    fn display_name(&self) -> &'static str {
        "Capturing mock LLM (mention pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING
            | LlmCapabilities::FUNCTION_CALLING
            | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.record(request);
        Ok(ChatResponse {
            content: "Bien reçu, on regarde ça ensemble.".to_owned(),
            model: "mock-model".to_owned(),
            usage: Some(TokenUsage::new(12, 6, 18)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        self.record(request);
        let chunk = StreamChunk {
            delta: "Bien reçu, on regarde ça ensemble.".to_owned(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// The coaching request for one turn: the captured request carrying a user
/// message that is exactly `user_text`. Background extraction calls quote the
/// transcript inside a larger prompt, so they never match.
fn coaching_request(log: &RequestLog, user_text: &str) -> Option<Vec<ChatMessage>> {
    log.lock()
        .unwrap()
        .iter()
        .find(|messages| {
            messages
                .iter()
                .any(|m| matches!(m.role, MessageRole::User) && m.content == user_text)
        })
        .cloned()
}

/// Poll for the coaching request of a turn served on a background task.
async fn wait_for_coaching_request(log: &RequestLog, user_text: &str) -> Vec<ChatMessage> {
    for _ in 0..150 {
        if let Some(request) = coaching_request(log, user_text) {
            return request;
        }
        sleep(Duration::from_millis(200)).await;
    }
    panic!("no coaching request carrying {user_text:?} reached the model within 30s");
}

/// Every system message of a request, joined — the assembled prompt.
fn system_prompt(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .filter(|m| matches!(m.role, MessageRole::System))
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Create an active athlete owning their own tenant, with a synthetic
/// provider connection so the turn clears the onboarding gate.
async fn create_athlete(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId) {
    let password_hash =
        spawn_blocking(|| bcrypt::hash("Mention123!", bcrypt::DEFAULT_COST).unwrap())
            .await
            .unwrap();
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Mention Athlete".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant {email}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
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
        .register_connection(
            user_id,
            tenant_id,
            "synthetic",
            &ConnectionType::Synthetic,
            None,
        )
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// A coach the athlete authored themselves — the kind a conversation is bound
/// to — carrying `system_prompt`.
async fn create_custom_coach(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    system_prompt: &str,
) -> String {
    let request = CreateCoachRequest {
        title: title.to_owned(),
        description: Some(format!("{title} for the mention pin")),
        system_prompt: system_prompt.to_owned(),
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
    resources
        .common
        .repos
        .coaches
        .create(user_id, tenant_id, &request)
        .await
        .unwrap()
        .id
        .to_string()
}

/// The coach the conversation row is bound to, read back from the store.
async fn conversation_coach(
    resources: &Arc<ServerContext>,
    conversation_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<String> {
    resources
        .common
        .repos
        .chat
        .get_conversation(conversation_id, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .expect("the conversation exists")
        .coach_id
}

/// The athlete's persisted messages, oldest first.
async fn persisted_user_rows(
    resources: &Arc<ServerContext>,
    conversation_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<String> {
    resources
        .common
        .repos
        .chat
        .get_recent_messages(conversation_id, &user_id.to_string(), tenant_id, 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.role == "user")
        .map(|m| m.content)
        .collect()
}

/// Run one web turn through the ladder every surface uses.
async fn web_turn(
    resources: &Arc<ServerContext>,
    conversation_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    content: &str,
) {
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    });
    let ctx = resources.chat_pipeline_context();
    let served = pierre_chat_pipeline::execute(
        &ctx,
        TurnRequest {
            conversation_id: conversation_id.to_owned(),
            user_id,
            conversation_tenant_id: tenant_id,
            tool_tenant_id: tenant_id,
            content: content.to_owned(),
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
    .expect("the turn must be admitted and served");
    assert!(
        matches!(served, ServedTurn::Pipeline(_)),
        "a plain-prose turn must run the pipeline"
    );
}

/// The grammar is `CoachHandle::parse`, opened by a `@` at a token boundary.
#[test]
fn mention_grammar_is_the_handle_grammar() {
    let handles = |text: &str| {
        mention_candidates(text)
            .iter()
            .map(|h| h.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(handles(MENTION_TURN), ["recovery-coach"]);
    assert_eq!(
        handles("Hey @recovery-coach, and (@strength_v2)!"),
        ["recovery-coach", "strength_v2"],
        "punctuation ends a token; a bracket opens one"
    );
    assert_eq!(
        handles("@recovery-coach @recovery-coach"),
        ["recovery-coach"],
        "no repeats"
    );
    assert!(
        handles("write to jf@dravr.ai").is_empty(),
        "an address is not a mention"
    );
    assert!(
        handles("@Recovery-Coach").is_empty(),
        "the grammar is CoachHandle::parse: lowercase only"
    );
    assert!(handles("no mention here").is_empty());
}

/// Only the resolved token goes, and the gap it leaves is closed.
#[test]
fn stripping_removes_only_the_resolved_token_and_closes_the_gap() {
    let recovery = CoachHandle::parse("recovery-coach").unwrap();
    assert_eq!(
        strip_mention(MENTION_TURN, &recovery),
        MENTION_TURN_FOR_MODEL
    );
    assert_eq!(
        strip_mention("Hey @recovery-coach, how?", &recovery),
        "Hey, how?"
    );
    assert_eq!(strip_mention("ask @recovery-coach", &recovery), "ask");
    assert_eq!(
        strip_mention("@recovery-coach and @tempo-coach", &recovery),
        "and @tempo-coach",
        "another coach's token stays"
    );
    assert_eq!(
        strip_mention("@recovery-coach", &recovery),
        "@recovery-coach",
        "a bare summon is kept, an empty turn would be dropped"
    );
    assert_eq!(strip_mention("plain text", &recovery), "plain text");
}

/// The `ultracode` shape, end to end on the web ladder: the mentioned coach's
/// persona assembles the mentioned turn, the stripped text is what the model
/// reads while the raw text is what persists, the conversation row is never
/// written, and the next plain turn is the conversation's own coach again.
#[tokio::test]
#[serial]
async fn a_mention_routes_that_turn_only_and_the_next_turn_reverts() {
    env::set_var("PIERRE_LLM_MODEL", "mock-model");
    let mock = CapturingLlm::new();
    let log = mock.log();
    let resources = create_test_server_resources_with_llm(Arc::new(mock))
        .await
        .unwrap();
    let repos = &resources.common.repos;

    let (author_id, author_tenant) = create_athlete(&resources, "mention-author@example.com").await;
    let (athlete_id, athlete_tenant) =
        create_athlete(&resources, "mention-athlete@example.com").await;

    let origin = publish_catalogue_coach(
        repos,
        author_id,
        author_tenant,
        "Recovery Coach",
        &format!("{RECOVERY_MARKER}: you are the recovery coach."),
    )
    .await;
    let installed = install_catalogue_coach(repos, origin, athlete_id, athlete_tenant).await;
    assert_eq!(installed.handle.as_deref(), Some("recovery-coach"));

    let tempo_id = create_custom_coach(
        &resources,
        athlete_id,
        athlete_tenant,
        "Tempo Coach",
        &format!("{TEMPO_MARKER}: you are the tempo coach."),
    )
    .await;
    let conversation = repos
        .chat
        .create_conversation(
            &athlete_id.to_string(),
            athlete_tenant,
            "mention pin",
            "mock-model",
            Some(&tempo_id),
            None,
        )
        .await
        .unwrap();

    // Turn 1 — addressed to the installed recovery coach.
    web_turn(
        &resources,
        &conversation.id,
        athlete_id,
        athlete_tenant,
        MENTION_TURN,
    )
    .await;
    let request = coaching_request(&log, MENTION_TURN_FOR_MODEL)
        .expect("the mentioned turn reaches the model with the token stripped");
    let prompt = system_prompt(&request);
    assert!(
        prompt.contains(RECOVERY_MARKER),
        "the mentioned coach's persona must assemble the turn: {prompt}"
    );
    assert!(
        !prompt.contains(TEMPO_MARKER),
        "the conversation's own coach must not leak into the mentioned turn: {prompt}"
    );
    assert!(
        prompt.contains("Answer as the recovery-coach coach"),
        "the voice anchor names the mentioned coach: {prompt}"
    );
    assert!(
        coaching_request(&log, MENTION_TURN).is_none(),
        "the raw token must not reach the model"
    );
    assert_eq!(
        persisted_user_rows(&resources, &conversation.id, athlete_id, athlete_tenant).await,
        vec![MENTION_TURN.to_owned()],
        "the persisted row keeps the athlete's raw text"
    );
    assert_eq!(
        conversation_coach(&resources, &conversation.id, athlete_id, athlete_tenant)
            .await
            .as_deref(),
        Some(tempo_id.as_str()),
        "a mention never writes the conversation's coach_id"
    );

    // Turn 2 — plain: the conversation's own coach is back.
    web_turn(
        &resources,
        &conversation.id,
        athlete_id,
        athlete_tenant,
        PLAIN_TURN,
    )
    .await;
    let request = coaching_request(&log, PLAIN_TURN).expect("the plain turn reaches the model");
    let prompt = system_prompt(&request);
    assert!(
        prompt.contains(TEMPO_MARKER),
        "the next turn reverts to the conversation's own coach: {prompt}"
    );
    assert!(
        !prompt.contains(RECOVERY_MARKER),
        "the mention must not outlive its turn: {prompt}"
    );
    assert_eq!(
        conversation_coach(&resources, &conversation.id, athlete_id, athlete_tenant)
            .await
            .as_deref(),
        Some(tempo_id.as_str())
    );
}

/// A mention resolves only against installed coaches: a catalogue coach the
/// athlete never installed does not route, and neither does a handle nobody
/// owns. The turn proceeds with its text untouched.
#[tokio::test]
#[serial]
async fn a_mention_of_a_coach_that_is_not_installed_does_not_route() {
    env::set_var("PIERRE_LLM_MODEL", "mock-model");
    let mock = CapturingLlm::new();
    let log = mock.log();
    let resources = create_test_server_resources_with_llm(Arc::new(mock))
        .await
        .unwrap();
    let repos = &resources.common.repos;

    let (author_id, author_tenant) =
        create_athlete(&resources, "uninstalled-author@example.com").await;
    let (athlete_id, athlete_tenant) =
        create_athlete(&resources, "uninstalled-athlete@example.com").await;

    // Published, so `strength-coach` exists in the catalogue — but the athlete
    // never installed it.
    publish_catalogue_coach(
        repos,
        author_id,
        author_tenant,
        "Strength Coach",
        &format!("{STRENGTH_MARKER}: you are the strength coach."),
    )
    .await;

    let tempo_id = create_custom_coach(
        &resources,
        athlete_id,
        athlete_tenant,
        "Tempo Coach",
        &format!("{TEMPO_MARKER}: you are the tempo coach."),
    )
    .await;
    let conversation = repos
        .chat
        .create_conversation(
            &athlete_id.to_string(),
            athlete_tenant,
            "uninstalled pin",
            "mock-model",
            Some(&tempo_id),
            None,
        )
        .await
        .unwrap();

    for text in [
        "@strength-coach quel plan de force pour la saison ?",
        "@nobody-here que penses-tu de ma semaine ?",
    ] {
        web_turn(
            &resources,
            &conversation.id,
            athlete_id,
            athlete_tenant,
            text,
        )
        .await;
        let request = coaching_request(&log, text)
            .unwrap_or_else(|| panic!("an unresolved mention reaches the model untouched: {text}"));
        let prompt = system_prompt(&request);
        assert!(
            prompt.contains(TEMPO_MARKER),
            "the conversation's own coach answers {text:?}: {prompt}"
        );
        assert!(
            !prompt.contains(STRENGTH_MARKER),
            "a coach the athlete never installed must not answer {text:?}: {prompt}"
        );
        assert_eq!(
            conversation_coach(&resources, &conversation.id, athlete_id, athlete_tenant)
                .await
                .as_deref(),
            Some(tempo_id.as_str())
        );
    }
}

#[cfg(feature = "client-messaging")]
mod messaging {
    use super::*;
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::json;

    const TG_SECRET: &str = "mention_pin_secret";
    const CHAT_ID: i64 = 771_001;

    /// The conversation id the messaging session points at, once it exists.
    async fn session_conversation_id(resources: &ServerContext, user_id: Uuid) -> Option<String> {
        let pool = resources
            .coach
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT pierre_conversation_id FROM messaging_sessions WHERE user_id = ?1",
        )
        .bind(user_id.to_string())
        .fetch_optional(pool)
        .await
        .unwrap();
        row.and_then(|r| r.0)
    }

    /// Post one private-chat Telegram message through the real webhook.
    async fn send_telegram(resources: &Arc<ServerContext>, msg_id: i64, text: &str) {
        let router = MessagingRoutes::routes(Arc::clone(resources));
        let response = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", TG_SECRET)
            .json(&json!({
                "update_id": 9_100 + msg_id,
                "message": {
                    "message_id": msg_id,
                    "date": Utc::now().timestamp(),
                    "chat": { "id": CHAT_ID, "type": "private" },
                    "from": { "id": CHAT_ID, "is_bot": false, "first_name": "Mention" },
                    "text": text
                }
            }))
            .send(router)
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "webhooks always ack"
        );
    }

    /// The same two turns through Telegram: the mention grammar lives on the
    /// ladder, not on a surface, so a messaging channel routes exactly like
    /// web with no client change.
    #[tokio::test]
    #[serial]
    async fn a_mention_from_telegram_routes_exactly_like_web() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");
        let mock = CapturingLlm::new();
        let log = mock.log();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();
        let repos = &resources.common.repos;
        let db: &dyn MessagingRepository = &*repos.messaging;

        let (author_id, author_tenant) =
            create_athlete(&resources, "tg-mention-author@example.com").await;
        let (athlete_id, tenant_id) =
            create_athlete(&resources, "tg-mention-athlete@example.com").await;
        // A returning athlete: a fresh messaging conversation opens the platform
        // intake (profile type, then the PAR-Q) and reads the *next* inbound
        // message as its answer, which would swallow the plain turn below. An
        // athlete who already answered on web is left alone, and that is the
        // conversation a mention lands in.
        for step in ["profile_type", "parq"] {
            repos
                .user_onboarding
                .set_onboarding_step(
                    &athlete_id.to_string(),
                    step,
                    "complete",
                    None,
                    Some(&tenant_id.to_string()),
                )
                .await
                .unwrap();
        }

        let origin = publish_catalogue_coach(
            repos,
            author_id,
            author_tenant,
            "Recovery Coach",
            &format!("{RECOVERY_MARKER}: you are the recovery coach."),
        )
        .await;
        let installed = install_catalogue_coach(repos, origin, athlete_id, tenant_id).await;
        assert_eq!(installed.handle.as_deref(), Some("recovery-coach"));

        // The conversation's own coach, selected the way `/coach add` selects
        // it, so the messaging session binds it when it opens.
        let tempo_id = create_custom_coach(
            &resources,
            athlete_id,
            tenant_id,
            "Tempo Coach",
            &format!("{TEMPO_MARKER}: you are the tempo coach."),
        )
        .await;
        repos
            .coaches
            .activate_coach(&tempo_id, athlete_id, tenant_id)
            .await
            .unwrap()
            .expect("activation resolves the coach");

        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(TG_SECRET),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:MENTION_BOT"),
            is_active: true,
        })
        .await
        .unwrap();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &athlete_id.to_string(),
            channel_type: "telegram",
            channel_user_id: &CHAT_ID.to_string(),
            display_name: Some("Mention Athlete"),
        })
        .await
        .unwrap();

        // Turn 1 — the mention.
        send_telegram(&resources, 1, MENTION_TURN).await;
        let request = wait_for_coaching_request(&log, MENTION_TURN_FOR_MODEL).await;
        let prompt = system_prompt(&request);
        assert!(
            prompt.contains(RECOVERY_MARKER),
            "the mentioned coach's persona must assemble the Telegram turn: {prompt}"
        );
        assert!(
            !prompt.contains(TEMPO_MARKER),
            "the conversation's own coach must not leak into the mentioned turn: {prompt}"
        );
        let conversation_id = session_conversation_id(&resources, athlete_id)
            .await
            .expect("turn 1 opened the session's conversation");
        assert_eq!(
            conversation_coach(&resources, &conversation_id, athlete_id, tenant_id)
                .await
                .as_deref(),
            Some(tempo_id.as_str()),
            "the session's conversation stays bound to the selected coach"
        );

        // Turn 2 — plain: the conversation's own coach again, same row.
        send_telegram(&resources, 2, PLAIN_TURN).await;
        let request = wait_for_coaching_request(&log, PLAIN_TURN).await;
        let prompt = system_prompt(&request);
        assert!(
            prompt.contains(TEMPO_MARKER),
            "the next Telegram turn reverts to the conversation's own coach: {prompt}"
        );
        assert!(
            !prompt.contains(RECOVERY_MARKER),
            "the mention must not outlive its turn: {prompt}"
        );
        assert_eq!(
            session_conversation_id(&resources, athlete_id)
                .await
                .as_deref(),
            Some(conversation_id.as_str()),
            "both turns ride the same conversation row"
        );
        assert_eq!(
            conversation_coach(&resources, &conversation_id, athlete_id, tenant_id)
                .await
                .as_deref(),
            Some(tempo_id.as_str())
        );
    }
}
