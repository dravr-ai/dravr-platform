// ABOUTME: The activity list a turn ships renders in the turn's resolved locale, not the profile's
// ABOUTME: An English turn on a French profile once got English prose over a list dated "jeu (hier)"
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! On 2026-10-23 a turn resolved English from the athlete's message while the
//! activity list beside the reply, rendered from the profile's stored French,
//! read "2026-10-22 jeu (hier)". The turn service scopes the resolved locale
//! around the turn; the executor carries it into every tool body.
//!
//! The first test runs a whole turn, so it fails if the turn service stops
//! scoping the locale; the second pins the executor's half on its own.

use crate::common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use common::{
    create_test_server_resources, create_test_server_resources_with_llm, create_test_user,
};
use futures_util::stream;
use pierre_chat_pipeline::{
    CommandPersistence, InputSource, PipelineHooks, ReplyBlock, ServedTurn, SurfaceProfile,
    TurnOrigin, TurnRequest,
};
use pierre_core::civil_time::format_civil_day;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
    TokenUsage,
};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::ConversationTurnId;
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::messaging_ingress::surface::messaging_surface_request;
use pierre_tool_runtime::context::TURN_LOCALE;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use serde_json::json;
use uuid::Uuid;

/// A French-profile athlete with one cached Strava ride, served with no
/// network (the fixture `activities_payload_projection_test` uses).
async fn french_athlete_with_a_ride(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    resources
        .common
        .repos
        .users
        .update_locale(user_id, "fr")
        .await
        .expect("stored locale");
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        "strava-ride-locale".to_owned(),
        "Sortie longue".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(30_000.0)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &[ride])
        .await
        .unwrap();
    // A Garmin connection with no session is elected primary, so the window
    // is served from the Strava cache with no network.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    (user_id, tenant)
}

/// The rendered `activity_list` for a turn conducted in `turn_locale`. The
/// executor is built inside the scope, as the turn's own executors are.
async fn activity_list_in(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    turn_locale: Option<&str>,
) -> String {
    let resources = Arc::clone(resources);
    TURN_LOCALE
        .scope(turn_locale.map(ToOwned::to_owned), async move {
            let response = UniversalToolExecutor::new(resources)
                .with_scopes(OAuthScope::self_grant())
                .with_transport(Transport::WebApp)
                .execute_tool(UniversalRequest {
                    tool_name: "get_activities".to_owned(),
                    parameters: json!({ "limit": 5 }),
                    user_id: user_id.to_string(),
                    protocol: "mcp".to_owned(),
                    tenant_id: Some(tenant.to_string()),
                })
                .await
                .expect("get_activities dispatches");
            assert!(response.success, "get_activities ran: {:?}", response.error);
            response.result.expect("a payload")["activity_list"]
                .as_str()
                .expect("an activity_list")
                .to_owned()
        })
        .await
}

/// Heads the ordinary turn directive: how the coaching call is told apart
/// from the background passes a turn spawns.
const COACHING_CALL_MARKER: &str = "# This turn";

const GET_ACTIVITIES_CALL: &str = "<tool_call>\n{\"name\": \"get_activities\", \"arguments\": {\"limit\": 5, \"mode\": \"summary\"}}\n</tool_call>";

/// Asks for `get_activities` on the coaching call, then answers in prose.
struct ListingProvider {
    tool_call: Mutex<Option<&'static str>>,
}

#[async_trait]
impl LlmProvider for ListingProvider {
    fn name(&self) -> &'static str {
        "listing_mock"
    }
    fn display_name(&self) -> &'static str {
        "Listing Mock LLM (turn locale)"
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
        let coaching = request
            .messages
            .iter()
            .any(|m| m.role == MessageRole::System && m.content.contains(COACHING_CALL_MARKER));
        let tool_call = if coaching {
            self.tool_call.lock().unwrap().take()
        } else {
            None
        };
        Ok(ChatResponse {
            content: tool_call.map_or_else(
                || "You rode yesterday, an easy hour.".to_owned(),
                ToOwned::to_owned,
            ),
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

/// The 2026-10-23 turn, end to end: a long English question on Telegram from
/// a French profile. Telegram folds the list into the reply's prose, so the
/// athlete reads both in one message, and both must be English.
#[tokio::test]
async fn an_english_turn_folds_an_english_list_into_its_reply() {
    let provider = Arc::new(ListingProvider {
        tool_call: Mutex::new(Some(GET_ACTIVITIES_CALL)),
    });
    let resources = create_test_server_resources_with_llm(provider)
        .await
        .expect("resources");
    let (user_id, tenant) = french_athlete_with_a_ride(&resources).await;
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "Rides",
            "mock-model",
            None,
            None,
        )
        .await
        .expect("conversation")
        .id;
    let telegram = SurfaceProfile::resolve(&messaging_surface_request(
        ChannelType::Telegram,
        "fr".to_owned(),
        None,
    ));

    let served = pierre_chat_pipeline::execute(
        &resources.chat_pipeline_context(),
        TurnRequest {
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            conversation_id: conversation,
            user_id,
            conversation_tenant_id: tenant,
            tool_tenant_id: tenant,
            content: "Can you show me my most recent rides and how they went?".to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quote: None,
            channel_type: "telegram",
            transport: Transport::Messaging,
            is_direct_message: true,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            hooks: PipelineHooks::none(),
        },
        &telegram,
    )
    .await
    .expect("the turn is served");
    let ServedTurn::Pipeline(envelope) = served else {
        panic!("a coaching question is answered by the pipeline, not a command");
    };
    assert!(
        envelope.telemetry.activity_list_captured,
        "premise: the turn's get_activities round produced a list — tools called: {:?}",
        envelope.telemetry.tools_called
    );
    let prose = envelope
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::Prose { text } => Some(text.as_str()),
            _ => None,
        })
        .expect("the reply is delivered as prose");
    let today = Utc::now().date_naive();
    assert!(
        prose.contains(&format!("[Today] {}", format_civil_day(today, "en"))),
        "bug: the English turn's folded list must be English, not the profile's French: {prose}"
    );
}

#[tokio::test]
async fn an_english_turn_on_a_french_profile_lists_in_english() {
    let resources = create_test_server_resources().await.expect("resources");
    let (user_id, tenant) = french_athlete_with_a_ride(&resources).await;
    let today = Utc::now().date_naive();

    let english = activity_list_in(&resources, user_id, tenant, Some("en")).await;
    assert!(
        english.contains(&format!("[Today] {}", format_civil_day(today, "en"))),
        "bug: an English turn must list in English, not the profile's French: {english}"
    );

    // Outside a turn (MCP-direct, A2A) the profile's stored locale still rules.
    let stored = activity_list_in(&resources, user_id, tenant, None).await;
    assert!(
        stored.contains(&format!("[Today] {}", format_civil_day(today, "fr"))),
        "bug: a call outside any turn must list in the stored locale: {stored}"
    );
}
