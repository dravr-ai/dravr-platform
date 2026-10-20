// ABOUTME: A coach post the platform makes on its own is stored only when grounded, stays unread, never reaches a room
// ABOUTME: Pins PlatformPost against a real pipeline turn and the origin-aware persistence beneath it (carnet#831)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#831.
//!
//! The coach speaks without the athlete starting the turn: after a backfill,
//! and first of all once their data lands. Each such post is a pipeline turn
//! the platform composed, so it is verified like any turn — and it is only
//! worth posting when it says something about the athlete's own data. These
//! tests drive the production [`PipelinePlatformPost`] through a real turn
//! with a scripted model, so the rule is pinned where it runs: a grounded
//! reply is stored once, as the agent's alone and unread; an ungrounded reply
//! or a refusal stores nothing at all.
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;
use pierre_chat_pipeline::stages::persistence::persist_assistant_response;
use pierre_chat_pipeline::{
    InputSource, PipelineHooks, QuotaState, ReplyBlock, SurfaceId, SurfaceProfile, SurfaceRequest,
    TurnInput, TurnOrigin,
};
use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_SCOPE_REFUSAL};
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::{
    AddMessageParams, ConnectionType, ConversationTurnId, DataSource, DeviceType, MessageRecord,
    StoredRecoveryMetrics, TenantId,
};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::platform_post::{
    PipelinePlatformPost, PlatformPost, PlatformPostRequest, PostSurface,
};
use pierre_providers::ai_scope::{self, Provenance};
use pierre_test_support::delegation::{create_coachless_group, create_group_agent};
use tokio::time::sleep;
use uuid::Uuid;

use helpers::notify_capture::{capture_notify, named, only};

/// The instruction every post here answers. Never stored.
const INSTRUCTION: &str = "Tell the athlete what their recovery says about this week.";

/// What the model answers once it has read the athlete's recovery.
const GROUNDED_ANSWER: &str = "Your recovery held at 71% this morning, so the week can build.";

/// A model that answers `answer`, after reading the athlete's recovery once
/// when `reads_recovery` is set. It declares no function calling, so the turn
/// takes the text tool loop, which reads the `<tool_call>` block out of its
/// first reply.
struct ScriptedModel {
    reads_recovery: bool,
    answer: String,
    calls: AtomicUsize,
}

impl ScriptedModel {
    fn reads_recovery_then(answer: &str) -> Self {
        Self {
            reads_recovery: true,
            answer: answer.to_owned(),
            calls: AtomicUsize::new(0),
        }
    }

    fn answers_blind(answer: &str) -> Self {
        Self {
            reads_recovery: false,
            answer: answer.to_owned(),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedModel {
    fn name(&self) -> &'static str {
        "scripted_platform_post_mock"
    }
    fn display_name(&self) -> &'static str {
        "Scripted Mock LLM (platform post)"
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
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let content = if first && self.reads_recovery {
            r#"Let me look. <tool_call>{"name":"get_recovery_metrics","arguments":{}}</tool_call>"#
                .to_owned()
        } else {
            self.answer.clone()
        };
        Ok(ChatResponse {
            content,
            model: "mock-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let response = self.complete(request).await?;
        Ok(Box::pin(stream::iter(vec![Ok(StreamChunk {
            delta: response.content,
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        })])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

struct Fixture {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: String,
}

/// An athlete with one empty in-app thread, on a server whose model is
/// `model`. With `whoop_morning`, WHOOP is connected and this morning's
/// recovery is stored, which is what a grounded post reads.
async fn fixture(model: ScriptedModel, whoop_morning: bool) -> Fixture {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources_with_chat_provider(Arc::new(model))
        .await
        .unwrap();
    let email = format!("platform-post-{}@example.com", Uuid::new_v4());
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "professional")
            .await
            .unwrap();
    if whoop_morning {
        seed_whoop_morning(&resources, user_id, tenant_id).await;
    }
    let conversation_id = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant_id,
            "Accueil",
            "mock-model",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    Fixture {
        resources,
        user_id,
        tenant_id,
        conversation_id,
    }
}

/// WHOOP connected, its terms accepted (the consent to hand its records to a
/// model, carnet#726), and this morning's recovery stored.
async fn seed_whoop_morning(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) {
    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(user_id, tenant_id, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .users
        .record_provider_terms(
            user_id,
            "whoop",
            provider_terms_version("whoop").expect("WHOOP carries a notice"),
        )
        .await
        .unwrap();
    let data_source_id = repos
        .data_sources
        .upsert_data_source(
            &tenant_id,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: "whoop".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let morning = StoredRecoveryMetrics {
        id: format!("whoop-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id,
        date: Utc::now().date_naive(),
        recovery_score: Some(71),
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: Some(62.0),
        resting_heart_rate: None,
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: "whoop".to_owned(),
        recorded_at: Utc::now(),
    };
    repos
        .recovery
        .upsert_recovery_metrics(&tenant_id, &morning)
        .await
        .unwrap();
}

/// Post `INSTRUCTION` into the fixture's in-app thread through the production
/// pipeline post.
async fn post(fx: &Fixture) -> Option<MessageRecord> {
    PipelinePlatformPost::new(Arc::downgrade(&fx.resources))
        .post(PlatformPostRequest {
            user_id: fx.user_id,
            tenant_id: fx.tenant_id,
            conversation_id: &fx.conversation_id,
            surface: PostSurface::InApp,
            locale: "en",
            prompt: INSTRUCTION,
        })
        .await
        .map(|posted| posted.assistant.message)
}

async fn rows(fx: &Fixture) -> Vec<MessageRecord> {
    fx.resources
        .common
        .repos
        .chat
        .get_messages(&fx.conversation_id, &fx.user_id.to_string(), fx.tenant_id)
        .await
        .unwrap()
}

/// How many rows of the thread are unread for its athlete, as the
/// conversation list counts them.
async fn unread(fx: &Fixture) -> i64 {
    fx.resources
        .common
        .repos
        .chat
        .list_conversations(
            &fx.user_id.to_string(),
            fx.tenant_id,
            50,
            0,
            TransportPolicy::FirstPartyOnly,
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|row| row.id == fx.conversation_id)
        .expect("the athlete's thread is listed")
        .unread_count
}

/// The whole point: a reply grounded in the athlete's data is stored, once,
/// as the agent's — never beside a question the athlete did not ask — and it
/// waits for them as unread.
#[tokio::test]
async fn a_grounded_post_is_stored_once_unread_and_reported_as_a_platform_answer() {
    let fx = fixture(ScriptedModel::reads_recovery_then(GROUNDED_ANSWER), true).await;
    let (events, _guard) = capture_notify();

    let posted = post(&fx)
        .await
        .expect("a reply grounded in the athlete's recovery is posted");

    let rows = rows(&fx).await;
    assert!(
        rows.iter().all(|row| row.role != "user"),
        "the platform's instruction is never written as the athlete's message: {rows:?}"
    );
    let assistant: Vec<_> = rows.iter().filter(|row| row.role == "assistant").collect();
    assert_eq!(assistant.len(), 1, "exactly the post: {rows:?}");
    assert_eq!(assistant[0].id, posted.id, "the stored row is the post");
    // The tool round that read the recovery is kept for the athlete's
    // follow-ups, written once the post was admitted and ahead of it.
    let evidence = rows
        .iter()
        .position(|row| row.role == "tool_call")
        .unwrap_or_else(|| panic!("the recovery read is kept: {rows:?}"));
    let reply = rows
        .iter()
        .position(|row| row.id == posted.id)
        .expect("the post");
    assert!(evidence < reply, "the evidence precedes the post: {rows:?}");
    assert!(
        assistant[0].content.contains("71%"),
        "the post speaks from the athlete's recovery: {}",
        assistant[0].content
    );
    assert_eq!(
        unread(&fx).await,
        1,
        "nobody was in front of the post when it landed, so it stays unread"
    );

    let delivered = only(&events, "chat.answer_delivered");
    assert_eq!(delivered.field("origin"), "platform");
    assert_eq!(delivered.field("grounded"), "true");
}

/// A reply with nothing of the athlete's in front of it is not worth speaking
/// unprompted: nothing is stored, and no answer is reported delivered. With no
/// provider connected the platform's own activity read is refused, which puts
/// no data in front of the model either.
#[tokio::test]
async fn an_ungrounded_reply_is_never_posted() {
    let fx = fixture(
        ScriptedModel::answers_blind("Big week ahead — keep it up!"),
        false,
    )
    .await;
    let (events, _guard) = capture_notify();

    assert!(post(&fx).await.is_none(), "an ungrounded reply is withheld");

    assert!(
        rows(&fx).await.is_empty(),
        "a withheld post writes nothing to the thread"
    );
    assert_eq!(unread(&fx).await, 0);
    assert!(
        named(&events, "chat.answer_delivered").is_empty(),
        "nothing reached the athlete, so no answer was delivered"
    );
    assert_eq!(
        only(&events, "chat.question_asked").field("origin"),
        "platform",
        "the turn itself ran, as the platform's"
    );
}

/// The model had the athlete's recovery in front of it and declined anyway:
/// a refusal sent unprompted reads as the coach volunteering that it cannot
/// help, so it is withheld like an ungrounded reply — the recovery read that
/// led up to it included, which would otherwise sit in the thread alone.
#[tokio::test]
async fn a_refusal_over_the_athletes_data_is_never_posted() {
    let refusal = MessagingStringsRegistry::new().get(KEY_SCOPE_REFUSAL, "en");
    assert!(
        !refusal.trim().is_empty(),
        "the catalogue carries the English refusal this test stands on"
    );
    let fx = fixture(ScriptedModel::reads_recovery_then(&refusal), true).await;

    assert!(post(&fx).await.is_none(), "a refusal is withheld");
    assert!(
        rows(&fx).await.is_empty(),
        "a withheld post writes nothing to the thread"
    );
}

/// The same rounds on a turn the athlete sent are written as they land, not
/// held: their thread keeps the evidence even if the turn dies before its
/// reply. Each is stamped with what it read — the athlete's WHOOP recovery,
/// which no external transport may carry — as the post's own round is.
#[tokio::test]
async fn an_athletes_tool_round_is_written_as_it_lands_stamped_with_what_it_read() {
    let fx = fixture(ScriptedModel::reads_recovery_then(GROUNDED_ANSWER), true).await;
    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        conversation_id: fx.conversation_id.clone(),
        user_id: fx.user_id.to_string(),
        conversation_tenant_id: fx.tenant_id,
        tool_tenant_id: fx.tenant_id,
        is_direct_message: true,
        content: INSTRUCTION.to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
        steps_after: None,
    };
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    });
    let ctx = fx.resources.chat_pipeline_context();
    // Tracked the way the turn service tracks every turn, over the web app
    // its profile names.
    ai_scope::serve_over(
        Transport::WebApp,
        ai_scope::tracking(
            Provenance::new(),
            pierre_chat_pipeline::run(&ctx, input, &profile, &PipelineHooks::none()),
        ),
    )
    .await
    .expect("the turn is served");

    // An athlete's round is written from its own task; wait for it to land.
    let mut round = None;
    for _ in 0..100 {
        round = rows(&fx)
            .await
            .into_iter()
            .find(|row| row.role == "tool_call");
        if round.is_some() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    let round = round.expect("the athlete's tool round is written");
    assert_eq!(round.content, "Let me look.");
    assert_eq!(
        round.transport_policy,
        TransportPolicy::FirstPartyOnly,
        "the round read WHOOP recovery, which stays on Dravr's own surfaces"
    );
}

/// Count the shared room transcript's entries for `group_id`. No repository
/// read returns a room's entries regardless of who may see them, and that is
/// the question here: did the post reach the room at all.
async fn room_entries(db: &Database, group_id: Uuid) -> i64 {
    const SQL: &str =
        "SELECT COUNT(*) FROM group_transcript_entries WHERE CAST(group_id AS TEXT) = $1";
    match db.backend() {
        DatabaseBackend::SQLite(inner) => sqlx::query_scalar(SQL)
            .bind(group_id.to_string())
            .fetch_one(inner.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(inner) => sqlx::query_scalar(SQL)
            .bind(group_id.to_string())
            .fetch_one(inner.pool())
            .await
            .unwrap(),
    }
}

/// Beneath the post: an assistant row the platform posted leaves the
/// athlete's read marker where it was and never fans out to a room, even on a
/// group-bound thread; a reply to the athlete does both, as before.
#[tokio::test]
async fn a_platform_reply_stays_unread_and_out_of_the_room() {
    let fx = fixture(ScriptedModel::answers_blind("unused"), false).await;
    let repos = &fx.resources.common.repos;
    let agent = create_group_agent(repos, fx.user_id, fx.tenant_id)
        .await
        .unwrap();
    let group = create_coachless_group(repos, fx.user_id, fx.tenant_id, &agent, "Club")
        .await
        .unwrap();
    repos
        .chat
        .set_conversation_group_id(&fx.conversation_id, Some(&group.to_string()), fx.tenant_id)
        .await
        .unwrap();
    let user = fx.user_id.to_string();
    let reply = |content: &'static str| AddMessageParams {
        tenant_id: fx.tenant_id,
        conversation_id: &fx.conversation_id,
        user_id: &user,
        role: "assistant",
        content,
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
        content_blocks: None,
        transport_policy: TransportPolicy::AnyTransport,
    };

    persist_assistant_response(
        repos.chat.as_ref(),
        repos.groups.as_ref(),
        &reply("The coach speaks first."),
        fx.tenant_id,
        TurnOrigin::Platform,
    )
    .await
    .unwrap();
    assert_eq!(unread(&fx).await, 1, "the platform's post stays unread");
    assert_eq!(
        room_entries(&fx.resources.agent.database, group).await,
        0,
        "one athlete's own coaching never reaches the room"
    );

    persist_assistant_response(
        repos.chat.as_ref(),
        repos.groups.as_ref(),
        &reply("A reply the athlete is reading."),
        fx.tenant_id,
        TurnOrigin::Athlete,
    )
    .await
    .unwrap();
    assert_eq!(
        unread(&fx).await,
        0,
        "a reply to the athlete is read where it lands, and carries the marker past both"
    );
    assert_eq!(
        room_entries(&fx.resources.agent.database, group).await,
        1,
        "a reply in a group thread is the room's"
    );
}

/// A stored post ends on what the athlete could do next: at most three of the
/// catalogue's use cases, ranked for them and the thread agent's domain,
/// offered as next steps (carnet#830) — drawn as buttons on the live reply
/// and stored on the row, so a reload draws the same ones.
#[tokio::test]
async fn a_stored_post_ends_on_at_most_three_ranked_use_cases() {
    let fx = fixture(ScriptedModel::reads_recovery_then(GROUNDED_ANSWER), true).await;
    let repos = &fx.resources.common.repos;
    let agent = repos
        .agents
        .create(
            fx.user_id,
            fx.tenant_id,
            &CreateAgentRequest {
                title: "Training Agent".to_owned(),
                description: None,
                system_prompt: "You are a test agent.".to_owned(),
                category: AgentCategory::Training,
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
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    let user = fx.user_id.to_string();
    let thread = repos
        .chat
        .create_conversation(
            &user,
            fx.tenant_id,
            "Accueil",
            "mock-model",
            Some(&agent),
            None,
        )
        .await
        .unwrap()
        .id;

    let posted = PipelinePlatformPost::new(Arc::downgrade(&fx.resources))
        .post(PlatformPostRequest {
            user_id: fx.user_id,
            tenant_id: fx.tenant_id,
            conversation_id: &thread,
            surface: PostSurface::InApp,
            locale: "en",
            prompt: INSTRUCTION,
        })
        .await
        .expect("a reply grounded in the athlete's recovery is posted");

    let live: Vec<String> = posted
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::Actions { actions, .. } => {
                Some(actions.iter().map(|action| action.value.clone()).collect())
            }
            _ => None,
        })
        .expect("the post ends on its next steps");
    assert!(
        (1..=3).contains(&live.len()),
        "one to three steps: {live:?}"
    );
    for (slot, value) in live.iter().enumerate() {
        assert!(
            value.starts_with(&format!("nx:{slot}:")),
            "each step posts its catalogue entry from its own slot: {value}"
        );
    }

    let stored = repos
        .chat
        .get_messages(&thread, &user, fx.tenant_id)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.id == posted.assistant.message.id)
        .expect("the post is stored");
    let blocks: Vec<serde_json::Value> =
        serde_json::from_str(stored.content_blocks.as_deref().expect("its stored blocks")).unwrap();
    let stored_values: Vec<String> = blocks
        .iter()
        .filter(|entry| entry["type"] == "actions")
        .flat_map(|entry| {
            entry["actions"]
                .as_array()
                .expect("an actions entry lists its actions")
                .iter()
                .map(|action| action["value"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(stored_values, live, "a reload draws the same steps");
}
