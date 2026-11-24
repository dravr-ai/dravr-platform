// ABOUTME: carnet#1718 — a reply whose tools show numbers a science change moved carries that change's note, once per athlete
// ABOUTME: Through the public chat pipeline entry: told on the first topic turn, never again, never off topic or to a newer account

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! When a science change moves numbers an athlete has already seen, the agent
//! says so once, the next time that topic comes up: the platform appends the
//! change's note, from the catalogue, to the reply of the first turn whose
//! tools show those numbers, and claims it so no later turn tells it again.
//!
//! These tests drive `pierre_chat_pipeline::run`, the entry every surface
//! calls, with a model that reads the athlete's training load when asked
//! about it. The note is the pinned catalogue's text, in the reply's
//! language.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::{Elapsed, Timestamp};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use pierre_chat_pipeline::{
    InputSource, PipelineHooks, QuotaState, SurfaceId, SurfaceProfile, SurfaceRequest, TurnInput,
    TurnOrigin,
};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_REPLY_GARMIN_ATTRIBUTION, KEY_SCIENCE_CHANGE_TRAINING_LOAD,
};
use pierre_core::errors::AppError;
use pierre_core::models::{
    Activity, ActivityBuilder, ConversationTurnId, PostPurpose, SportType, Tenant, TenantId, User,
    UserPhysiologicalProfile, UserStatus,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
};
use pierre_mcp_server::context::ServerContext;
use pierre_providers::ai_scope::{self, Provenance};
use serde_json::json;
use uuid::Uuid;

use crate::common::{
    create_test_server_resources_with_chat_provider, init_server_config, init_test_http_clients,
};
use crate::helpers::garmin_consent::give_garmin_ai_consent;

const MODEL: &str = "science-note-model";

/// What the athlete asks when the model reads their training load.
const LOAD_ASK: &str = "How is my training load trending these last few weeks?";

/// What the athlete asks when no tool shows their load.
const SMALL_TALK: &str = "Thanks for yesterday, that session advice was really useful.";

/// The agent's answer, whatever it read.
const ANSWER: &str = "Your training load is steady this week. We keep the same volume \
    through Sunday and add one longer run next week.";

/// How the catalogue's English note on the training-load change opens.
const LOAD_NOTE_OPENING: &str = "A note on your numbers: on 21 November";

/// The upload provider, read from the durable cache with no connection.
const UPLOAD: &str = "upload";

/// A Garmin watch, whose recordings owe Garmin's attribution (carnet#521).
const GARMIN_WATCH: &str = "Garmin Forerunner 965";

/// A model that reads the athlete's training load once per turn when asked
/// about it, then answers.
struct LoadReader {
    models: Vec<String>,
}

impl LoadReader {
    fn new() -> Self {
        Self {
            models: vec![MODEL.to_owned()],
        }
    }
}

/// Whether the turn the request belongs to asks about training load and has
/// not read it yet: the last athlete message asks, and no tool result follows it.
fn asks_for_load_unread(request: &ChatRequest) -> bool {
    let read_this_turn = request
        .messages
        .iter()
        .rev()
        .take_while(|message| !matches!(message.role, MessageRole::User))
        .any(|message| matches!(message.role, MessageRole::Tool));
    let asked = request
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, MessageRole::User))
        .is_some_and(|message| message.content.contains(LOAD_ASK));
    asked && !read_this_turn
}

#[async_trait]
impl LlmProvider for LoadReader {
    fn name(&self) -> &'static str {
        "load_reader"
    }
    fn display_name(&self) -> &'static str {
        "Scripted mock (science note pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        MODEL
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let tool_calls = asks_for_load_unread(request).then(|| {
            vec![ToolCallRequest {
                id: format!("call-{}", Uuid::new_v4()),
                function_name: "analyze_training_load".to_owned(),
                arguments: json!({ "provider": UPLOAD }),
            }]
        });
        let content = if tool_calls.is_some() {
            String::new()
        } else {
            ANSWER.to_owned()
        };
        Ok(ChatResponse {
            replay_blocks: None,
            content,
            model: MODEL.to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let response = self.complete(request).await?;
        let chunk = StreamChunk {
            delta: response.content,
            is_final: true,
            finish_reason: response.finish_reason,
        };
        Ok(Box::pin(stream::once(async move { Ok(chunk) })))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// A server whose chat model is [`LoadReader`].
async fn server() -> Arc<ServerContext> {
    init_server_config();
    init_test_http_clients();
    create_test_server_resources_with_chat_provider(Arc::new(LoadReader::new()))
        .await
        .unwrap()
}

/// The pinned catalogue's English note on the training-load change.
fn load_note() -> String {
    let note = MessagingStringsRegistry::new().get(KEY_SCIENCE_CHANGE_TRAINING_LOAD, "en");
    assert!(
        note.starts_with(LOAD_NOTE_OPENING),
        "the pinned catalogue carries the English note: {note}"
    );
    note
}

/// A fortnight of daily uploaded runs, recorded on `device` when one is named.
fn fortnight_of_runs(device: Option<&str>) -> Vec<Activity> {
    (1..=14)
        .map(|day| {
            ActivityBuilder::new(
                format!("science-note-run-{day}"),
                format!("Run {day}"),
                SportType::Run,
                Timestamp::now() - Elapsed::days(day),
                3_600,
                UPLOAD.to_owned(),
            )
            .distance_meters(10_000.0)
            .average_heart_rate(150)
            .device_name_opt(device.map(str::to_owned))
            .build()
        })
        .collect()
}

/// An athlete whose account was created at `created_at`, in a tenant they
/// own, with a fortnight of uploaded runs and the thresholds they are scored
/// against, and their own conversation.
async fn athlete(
    resources: &Arc<ServerContext>,
    created_at: Timestamp,
) -> ((Uuid, TenantId), String) {
    let repos = &resources.common.repos;
    let mut user = User::new(
        format!("science-note-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some("Science Note Athlete".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.created_at = created_at;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant,
            name: "science note tenant".to_owned(),
            slug: format!("science-note-{tenant}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        })
        .await
        .unwrap();
    repos.users.update_tenant_id(user_id, tenant).await.unwrap();

    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, UPLOAD, &fortnight_of_runs(None))
        .await
        .unwrap();
    let mut profile = UserPhysiologicalProfile::new(user_id, SportType::Run);
    profile.threshold_hr = Some(165);
    profile.max_hr = Some(190);
    profile.resting_hr = Some(50);
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant, user_id, &profile)
        .await
        .unwrap();

    let conversation = repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "science note pin",
            MODEL,
            None,
            None,
        )
        .await
        .unwrap();
    ((user_id, tenant), conversation.id)
}

/// An account created before the training-load change of
/// 2026-11-21T22:03:59Z, so it saw the numbers the change moved.
fn before_the_change() -> Timestamp {
    Timestamp::parse_from_rfc3339("2026-11-01T08:00:00Z").unwrap()
}

/// A conversation with the athlete that a bot's tenant owns, as the owner of
/// a messaging channel's webhook holds one: its tenant is not the athlete's.
async fn bot_conversation(resources: &Arc<ServerContext>, user_id: Uuid) -> (TenantId, String) {
    let repos = &resources.common.repos;
    let owner = User::new(
        format!("science-note-bot-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some("Science Note Bot Owner".to_owned()),
    );
    repos.users.create(&owner).await.unwrap();
    let bot_tenant = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: bot_tenant,
            name: "science note bot tenant".to_owned(),
            slug: format!("science-note-bot-{bot_tenant}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner.id,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        })
        .await
        .unwrap();
    let conversation = repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            bot_tenant,
            "science note bot pin",
            MODEL,
            None,
            None,
        )
        .await
        .unwrap();
    (bot_tenant, conversation.id)
}

/// The reply `pierre_chat_pipeline::run` delivers for the athlete's
/// `message` in their own conversation, checked to be what it keeps.
async fn served_reply(
    resources: &Arc<ServerContext>,
    who: (Uuid, TenantId),
    conversation: &str,
    message: &str,
) -> String {
    served_reply_in(resources, who, who.1, conversation, message).await
}

/// The reply `pierre_chat_pipeline::run` delivers for the athlete's
/// `message` in `conversation`, which `conversation_tenant` owns, checked to
/// be what the conversation keeps. The athlete's tools run in their own
/// tenant.
async fn served_reply_in(
    resources: &Arc<ServerContext>,
    (user_id, tenant): (Uuid, TenantId),
    conversation_tenant: TenantId,
    conversation: &str,
    message: &str,
) -> String {
    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        conversation_id: conversation.to_owned(),
        user_id: user_id.to_string(),
        conversation_tenant_id: conversation_tenant,
        tool_tenant_id: tenant,
        is_direct_message: true,
        content: message.to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quote: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
        steps_after: None,
        grant: OAuthScope::self_grant(),
    };
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "en".to_owned(),
        transport: None,
        prose_contract: None,
    });
    let ctx = resources.chat_pipeline_context();
    // Tracked the way `turn_service::execute`, the only caller of `run`,
    // tracks every turn.
    let envelope = ai_scope::serve_over(
        Transport::WebApp,
        ai_scope::tracking(
            Provenance::new(),
            pierre_chat_pipeline::run(&ctx, input, &profile, &PipelineHooks::none()),
        ),
    )
    .await
    .expect("the turn is served");

    let stored = resources
        .common
        .repos
        .chat
        .get_messages(conversation, &user_id.to_string(), conversation_tenant)
        .await
        .unwrap();
    let kept = stored
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .expect("the reply is persisted");
    assert_eq!(
        kept.content, envelope.assistant.message.content,
        "what the athlete receives is what the conversation keeps"
    );
    envelope.assistant.message.content
}

/// Whether a science note was ever marked told to the athlete.
async fn told_at(resources: &Arc<ServerContext>, (user_id, tenant): (Uuid, TenantId)) -> bool {
    resources
        .common
        .repos
        .platform_post_claims
        .platform_post_posted_at(tenant, user_id, PostPurpose::ScienceNote.as_str())
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn an_athlete_who_saw_the_old_load_numbers_is_told_once() {
    let resources = server().await;
    let (who, conversation) = athlete(&resources, before_the_change()).await;

    let first = served_reply(&resources, who, &conversation, LOAD_ASK).await;
    assert!(first.contains(ANSWER), "the agent's answer stays: {first}");
    assert!(
        first.contains(LOAD_NOTE_OPENING),
        "the first turn on the topic carries the note: {first}"
    );
    assert_eq!(
        first,
        format!("{ANSWER}\n\n{}", load_note()),
        "the catalogue's note follows the answer after a blank line, verbatim"
    );
    assert!(told_at(&resources, who).await, "the note is marked told");

    let second = served_reply(&resources, who, &conversation, LOAD_ASK).await;
    assert!(
        second.contains(ANSWER),
        "the agent's answer stays: {second}"
    );
    assert!(
        !second.contains(LOAD_NOTE_OPENING),
        "the note is told once, ever: {second}"
    );
}

#[tokio::test]
async fn an_account_created_after_the_change_is_told_nothing() {
    let resources = server().await;
    let (who, conversation) = athlete(&resources, Timestamp::now()).await;

    let reply = served_reply(&resources, who, &conversation, LOAD_ASK).await;
    assert!(reply.contains(ANSWER), "the agent's answer stays: {reply}");
    assert!(
        !reply.contains(LOAD_NOTE_OPENING),
        "an account newer than the change never saw the old numbers: {reply}"
    );
    assert!(!told_at(&resources, who).await, "and nothing is claimed");
}

#[tokio::test]
async fn a_turn_without_a_topic_tool_is_told_nothing_and_spends_no_note() {
    let resources = server().await;
    let (who, conversation) = athlete(&resources, before_the_change()).await;

    let small_talk = served_reply(&resources, who, &conversation, SMALL_TALK).await;
    assert!(
        small_talk.contains(ANSWER),
        "the agent's answer stays: {small_talk}"
    );
    assert!(
        !small_talk.contains(LOAD_NOTE_OPENING),
        "no tool showed the load: {small_talk}"
    );
    assert!(!told_at(&resources, who).await, "nothing is claimed");

    let on_topic = served_reply(&resources, who, &conversation, LOAD_ASK).await;
    assert!(
        on_topic.trim_end().ends_with(&load_note()),
        "the first turn on the topic still tells it: {on_topic}"
    );
}

/// A messaging channel's conversation belongs to the bot's tenant while the
/// athlete's tools run in their own. The note is claimed in the athlete's own
/// tenant, the one every surface claims it in, so it is told once across all
/// of them.
#[tokio::test]
async fn a_conversation_the_bot_owns_claims_the_note_in_the_athletes_own_tenant() {
    let resources = server().await;
    let (who, own_conversation) = athlete(&resources, before_the_change()).await;
    let (bot_tenant, conversation) = bot_conversation(&resources, who.0).await;

    let reply = served_reply_in(&resources, who, bot_tenant, &conversation, LOAD_ASK).await;
    assert_eq!(
        reply,
        format!("{ANSWER}\n\n{}", load_note()),
        "the first turn on the topic carries the note, in the bot's conversation too"
    );
    assert!(
        told_at(&resources, who).await,
        "the note is claimed in the athlete's own tenant"
    );
    assert!(
        !told_at(&resources, (who.0, bot_tenant)).await,
        "the note is never claimed in the bot's tenant"
    );

    let own = served_reply(&resources, who, &own_conversation, LOAD_ASK).await;
    assert!(
        !own.contains(LOAD_NOTE_OPENING),
        "a note told in one conversation is not told again in another: {own}"
    );
}

/// A reply drawn from a Garmin watch's recordings ends on Garmin's
/// attribution line; the note goes ahead of it, so that line stays last.
#[tokio::test]
async fn the_note_goes_ahead_of_the_garmin_attribution() {
    let resources = server().await;
    let ((user_id, tenant), conversation) = athlete(&resources, before_the_change()).await;
    give_garmin_ai_consent(&resources.common.repos, user_id, tenant).await;
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            user_id,
            &tenant,
            UPLOAD,
            &fortnight_of_runs(Some(GARMIN_WATCH)),
        )
        .await
        .unwrap();
    let garmin_line = MessagingStringsRegistry::new().get(KEY_REPLY_GARMIN_ATTRIBUTION, "en");
    assert!(
        !garmin_line.trim().is_empty(),
        "test bug: the pinned catalogue carries the English Garmin line"
    );

    let reply = served_reply(&resources, (user_id, tenant), &conversation, LOAD_ASK).await;
    assert_eq!(
        reply,
        format!("{ANSWER}\n\n{}\n\n{garmin_line}", load_note()),
        "the answer, then the note, then the Garmin line last"
    );
}
