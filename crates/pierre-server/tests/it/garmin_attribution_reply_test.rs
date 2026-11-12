// ABOUTME: carnet#521 — a coach reply derived from Garmin device-sourced data carries the platform's Garmin attribution line
// ABOUTME: Through the public chat pipeline entry: a Garmin-sourced turn gets the line, a turn with none does not

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin's API brand guidelines require an AI insight derived in part from
//! Garmin device-sourced data to say so. The platform knows whether a turn
//! handed the model such data — every activity a model reads passes the
//! AI-read filter, which notes its attribution — so the line is appended by
//! the pipeline, never left to the model.
//!
//! These tests drive `pierre_chat_pipeline::run`, the entry every surface
//! calls, with a model that reads the athlete's activities once and answers:
//! the delivered and persisted reply carries the localized line when a read
//! activity was Garmin-recorded, nothing when none was, and nothing on the
//! platform's own reconnect text. Another pins the turn's provenance
//! (carnet#769), the route a tool read on the Copilot loopback takes.
//!
//! A coach's aside about one athlete carries that athlete's roster card alone
//! and owes only what her data owes, never the coach's own or another
//! member's; an aside about the room owes every card it carried (carnet#1464),
//! and the line appears once, the platform's: a copy the model writes is dropped, and the
//! model never reads the line back from an earlier reply.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use embacle::types::ToolCallRequest;
use futures_util::stream;
use pierre_chat_pipeline::{
    InputSource, PipelineHooks, QuotaState, SurfaceId, SurfaceProfile, SurfaceRequest, TurnInput,
    TurnOrigin,
};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_REPLY_GARMIN_ATTRIBUTION,
};
use pierre_core::errors::AppError;
use pierre_core::models::{
    ActivityBuilder, AddMessageParams, ConnectionType, ConversationSubject, ConversationTurnId,
    SportType, TenantId,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::NewSubjectConversation;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_mcp_server::context::ServerContext;
use pierre_providers::ai_scope::{self, Provenance};
use pierre_test_support::delegation::{add_group_member, create_coached_group, create_group_agent};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

use crate::common::{
    create_test_server_resources_with_chat_provider, create_test_user, create_test_user_with_plan,
};

/// The agent's answer over the athlete's week.
const AGENT_ANSWER: &str =
    "Ta sortie longue de 200 km domine ta semaine. On garde le tempo pour jeudi.";

/// A model that makes one tool call, when it has one, then answers, keeping
/// every message it was handed.
struct ScriptedModel {
    call: Mutex<Option<(String, serde_json::Value)>>,
    answer: String,
    read: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl ScriptedModel {
    fn new(call: Option<(&str, serde_json::Value)>, answer: &str) -> Self {
        Self {
            call: Mutex::new(call.map(|(tool, args)| (tool.to_owned(), args))),
            answer: answer.to_owned(),
            read: Mutex::new(Vec::new()),
            models: vec!["attribution-model".to_owned()],
        }
    }

    /// Reads the athlete's activities once, then answers `answer`.
    fn activities_then(answer: &str) -> Self {
        Self::new(
            Some(("get_activities", json!({ "limit": 10, "mode": "summary" }))),
            answer,
        )
    }

    /// Every message the model was handed, across its calls.
    fn read(&self) -> Vec<String> {
        self.read.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for ScriptedModel {
    fn name(&self) -> &'static str {
        "scripted_model"
    }
    fn display_name(&self) -> &'static str {
        "Scripted mock (Garmin attribution pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "attribution-model"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.read
            .lock()
            .unwrap()
            .extend(request.messages.iter().map(|m| m.content.clone()));
        let tool_calls = self.call.lock().unwrap().take().map(|(tool, arguments)| {
            vec![ToolCallRequest {
                id: "call-1".to_owned(),
                function_name: tool,
                arguments,
            }]
        });
        let content = if tool_calls.is_some() {
            String::new()
        } else {
            self.answer.clone()
        };
        Ok(ChatResponse {
            content,
            model: "attribution-model".to_owned(),
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

/// An athlete whose Strava connection holds a long ride in the durable cache,
/// relayed from Garmin (`source: garmin`), recorded on a named `device`, or
/// by nothing named.
async fn athlete_with_a_ride(
    resources: &Arc<ServerContext>,
    source: Option<&str>,
    device: Option<&str>,
    sibling_serves: bool,
) -> (Uuid, TenantId) {
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    seed_ride(resources, (user_id, tenant), source, device, sibling_serves).await;
    (user_id, tenant)
}

/// `who`'s Strava connection holding one long ride, as [`athlete_with_a_ride`]
/// describes.
async fn seed_ride(
    resources: &Arc<ServerContext>,
    (user_id, tenant): (Uuid, TenantId),
    source: Option<&str>,
    device: Option<&str>,
    sibling_serves: bool,
) {
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        "strava-ride-1".to_owned(),
        "Sortie longue".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(2),
        7_200,
        "strava".to_owned(),
    )
    .distance_meters(200_000.0)
    .source_opt(source.map(str::to_owned))
    .device_name_opt(device.map(str::to_owned))
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &[ride])
        .await
        .unwrap();
    // A second connection with no session, elected last: the turn's live read
    // fails on it and the Strava cache serves the window, as it does for an
    // athlete whose other provider lapsed. The model still answers. Without
    // it, the lapsed Strava link blanks the turn into the reconnect message.
    if !sibling_serves {
        return;
    }
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();
}

fn web_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

/// The reply `pierre_chat_pipeline::run` delivers for one turn.
async fn delivered_reply(
    source: Option<&str>,
    device: Option<&str>,
    sibling_serves: bool,
) -> String {
    let model = Arc::new(ScriptedModel::activities_then(AGENT_ANSWER));
    let resources = create_test_server_resources_with_chat_provider(model)
        .await
        .unwrap();
    let (user_id, tenant) = athlete_with_a_ride(&resources, source, device, sibling_serves).await;
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "garmin attribution pin",
            "attribution-model",
            None,
            None,
        )
        .await
        .unwrap();
    served_reply(
        &resources,
        (user_id, tenant),
        &conversation.id,
        "Comment se passe ma semaine ?",
    )
    .await
}

/// The reply `pierre_chat_pipeline::run` delivers for `sender`'s `message` in
/// `conversation`, checked to be what the conversation keeps.
async fn served_reply(
    resources: &Arc<ServerContext>,
    (user_id, tenant): (Uuid, TenantId),
    conversation: &str,
    message: &str,
) -> String {
    let input = TurnInput {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        conversation_id: conversation.to_owned(),
        user_id: user_id.to_string(),
        conversation_tenant_id: tenant,
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
    let ctx = resources.chat_pipeline_context();
    // Tracked the way `turn_service::execute`, the only caller of `run`,
    // tracks every turn (carnet#769), over the web app its profile names
    // (carnet#724).
    let envelope = ai_scope::serve_over(
        Transport::WebApp,
        ai_scope::tracking(
            Provenance::new(),
            pierre_chat_pipeline::run(&ctx, input, &web_profile(), &PipelineHooks::none()),
        ),
    )
    .await
    .expect("the turn is served");

    // What the athlete receives is what the conversation keeps.
    let stored = resources
        .common
        .repos
        .chat
        .get_messages(conversation, &user_id.to_string(), tenant)
        .await
        .unwrap();
    let kept = stored
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .expect("the reply is persisted");
    assert_eq!(kept.content, envelope.assistant.message.content);
    envelope.assistant.message.content
}

fn garmin_line(locale: &str) -> String {
    MessagingStringsRegistry::new().get(KEY_REPLY_GARMIN_ATTRIBUTION, locale)
}

#[tokio::test]
async fn a_reply_derived_from_garmin_data_carries_the_attribution_line() {
    let reply = delivered_reply(Some("garmin"), None, true).await;
    let line = garmin_line("fr");
    assert!(!line.is_empty(), "the string exists");
    assert!(
        reply.contains("200 km"),
        "the agent's answer stays: {reply}"
    );
    assert!(
        reply.trim_end().ends_with(&line),
        "the reply ends with the attribution, in its language: {reply}"
    );
    assert_eq!(reply.matches(&line).count(), 1, "{reply}");
}

#[tokio::test]
async fn a_reply_with_no_garmin_data_carries_no_attribution_line() {
    let reply = delivered_reply(None, None, true).await;
    assert!(reply.contains("200 km"), "{reply}");
    for locale in ["fr", "en"] {
        assert!(
            !reply.contains(&garmin_line(locale)),
            "no Garmin data, no Garmin line: {reply}"
        );
    }
}

/// A Garmin watch's ride Strava relayed names Strava as its `source`, and is
/// Garmin device-sourced data all the same: its device says so (carnet#521).
#[tokio::test]
async fn a_reply_derived_from_a_relayed_garmin_recording_carries_the_attribution_line() {
    let reply = delivered_reply(None, Some("Garmin Edge 840"), true).await;
    let line = garmin_line("fr");
    assert!(reply.contains("200 km"), "{reply}");
    assert!(reply.trim_end().ends_with(&line), "{reply}");
    assert_eq!(reply.matches(&line).count(), 1, "{reply}");
}

/// Another maker's device owes Garmin nothing.
#[tokio::test]
async fn a_reply_derived_from_another_devices_recording_carries_no_attribution_line() {
    let reply = delivered_reply(None, Some("Wahoo ELEMNT BOLT"), true).await;
    assert!(reply.contains("200 km"), "{reply}");
    for locale in ["fr", "en"] {
        assert!(!reply.contains(&garmin_line(locale)), "{reply}");
    }
}

/// The platform's own reconnect message answers nothing from the athlete's
/// data, so it owes no attribution even though the turn read a Garmin ride.
#[tokio::test]
async fn the_platforms_reconnect_message_carries_no_attribution_line() {
    let reply = delivered_reply(Some("garmin"), None, false).await;
    assert!(!reply.contains("200 km"), "the turn blanked: {reply}");
    for locale in ["fr", "en"] {
        assert!(!reply.contains(&garmin_line(locale)), "{reply}");
    }
}

/// A tool read on the Copilot loopback runs on a task the turn never sees: an
/// executor built inside the turn carries the turn's provenance there, so what
/// it handed the model reaches the turn's reply (carnet#769's accumulator).
#[tokio::test]
async fn a_loopback_tool_read_attributes_the_turn_that_built_its_executor() {
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedModel::activities_then(AGENT_ANSWER));
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, tenant) = athlete_with_a_ride(&resources, Some("garmin"), None, true).await;
    let turn = Provenance::new();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = ai_scope::tracking(turn.clone(), async {
        UniversalExecutor::new(Arc::clone(&runtime))
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp)
    })
    .await;
    let response = tokio::spawn(async move {
        executor
            .execute_tool(UniversalRequest {
                tool_name: "get_activities".to_owned(),
                parameters: json!({ "limit": 10, "mode": "summary" }),
                user_id: user_id.to_string(),
                protocol: "mcp".to_owned(),
                tenant_id: Some(tenant.to_string()),
            })
            .await
    })
    .await
    .unwrap()
    .expect("the window is served");
    assert!(response.success, "{:?}", response.error);
    assert_eq!(turn.attributions_owed(|_| true), BTreeSet::from(["Garmin"]));
}

// ── carnet#1464: whose data the reply is about, and the line once ───────────

/// Devices a ride can be recorded on.
const GARMIN_EDGE: &str = "Garmin Edge 840";
const WAHOO_BOLT: &str = "Wahoo ELEMNT BOLT";

/// The coached athlete the aside is about.
const ATHLETE_NAME: &str = "Mira Attribution";

/// The coach's answer about the athlete.
const COACH_ANSWER: &str = "Mira roule entre 125 et 145 bpm : garde ses sorties en endurance.";

/// A person in their own tenant, named `name`, whose Strava connection holds
/// one long ride recorded on `device`.
async fn rider(resources: &Arc<ServerContext>, name: &str, device: &str) -> (Uuid, TenantId) {
    let email = format!("rider-{}@garmin-attribution.test", Uuid::new_v4());
    let (id, _user, tenant) =
        create_test_user_with_plan(&resources.agent.database, &email, "professional")
            .await
            .unwrap();
    let who = (id, tenant);
    seed_ride(resources, who, None, Some(device), false).await;
    resources
        .common
        .repos
        .users
        .update_display_name(who.0, name)
        .await
        .unwrap();
    who
}

/// The other member of the coached group, whose card the aside about the
/// athlete leaves out.
const OTHER_NAME: &str = "Olga Other";

/// What the coach's aside asks.
#[derive(Clone, Copy)]
enum Aside {
    /// Names the athlete; the model reads her activities before answering.
    ReadsTheAthlete,
    /// Names the athlete; the model answers from the room's context alone.
    NamesTheAthlete,
    /// Names no one; the model answers from the room's context alone.
    NamesNoOne,
}

/// What a coach's aside turn delivered, and every message its model read.
struct AsideTurn {
    reply: String,
    model_read: Vec<String>,
}

impl AsideTurn {
    /// Whether any message the model read carried `name`.
    fn model_saw(&self, name: &str) -> bool {
        self.model_read.iter().any(|message| message.contains(name))
    }
}

/// The coach's `aside` in the room of a group they coach, in which another
/// member rides on `other` and the athlete on `athlete`. The coach rides on
/// `coach_device`.
async fn coach_aside(coach_device: &str, other: &str, athlete: &str, aside: Aside) -> AsideTurn {
    let call = matches!(aside, Aside::ReadsTheAthlete).then(|| {
        (
            "get_group_member_activities",
            json!({ "member": ATHLETE_NAME }),
        )
    });
    let message = match aside {
        Aside::ReadsTheAthlete | Aside::NamesTheAthlete => "Pour toi seulement : comment va Mira ?",
        Aside::NamesNoOne => "Pour toi seulement : comment va le groupe ?",
    };
    let model = Arc::new(ScriptedModel::new(call, COACH_ANSWER));
    let provider: Arc<dyn LlmProvider> = model.clone();
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let coach = rider(&resources, "Cole Coach", coach_device).await;
    let other = rider(&resources, OTHER_NAME, other).await;
    let athlete = rider(&resources, ATHLETE_NAME, athlete).await;
    let repos = &resources.common.repos;
    let agent = create_group_agent(repos, coach.0, coach.1).await.unwrap();
    let group = create_coached_group(repos, coach.0, coach.1, &agent, "Squad", coach.0)
        .await
        .unwrap();
    for member in [other, athlete] {
        add_group_member(repos, group, member.0, member.1)
            .await
            .unwrap();
    }
    let room = repos
        .chat
        .create_conversation(
            &coach.0.to_string(),
            coach.1,
            "room",
            "attribution-model",
            None,
            Some(&group.to_string()),
        )
        .await
        .unwrap();
    let reply = served_reply(&resources, coach, &room.id, message).await;
    AsideTurn {
        reply,
        model_read: model.read(),
    }
}

fn carries_no_line(reply: &str) {
    for locale in ["fr", "en"] {
        assert!(
            !reply.contains(&garmin_line(locale)),
            "no Garmin line is owed: {reply}"
        );
    }
}

/// The Garmin line's sentence, whatever its apostrophe.
fn line_copies(reply: &str) -> usize {
    reply
        .replace('\u{2019}', "'")
        .matches(&straight(&garmin_line("fr")))
        .count()
}

fn straight(line: &str) -> String {
    line.replace('\u{2019}', "'")
}

/// The 2026-11-09 aside: an athlete with no Garmin device, a coach and
/// another member who record on one, and a reply about the athlete alone.
/// Whether or not the model reads her, the room context carries her card
/// alone, so nothing the other member's card holds reaches the model.
#[tokio::test]
async fn a_coachs_aside_about_an_athlete_with_no_garmin_device_carries_no_line() {
    for aside in [Aside::ReadsTheAthlete, Aside::NamesTheAthlete] {
        let turn = coach_aside(GARMIN_EDGE, GARMIN_EDGE, WAHOO_BOLT, aside).await;
        assert!(
            turn.reply.contains("125 et 145"),
            "the coach's answer: {}",
            turn.reply
        );
        carries_no_line(&turn.reply);
        assert!(
            turn.model_saw(ATHLETE_NAME),
            "the athlete's card is in the prompt"
        );
        assert!(
            !turn.model_saw(OTHER_NAME),
            "another member's card stays out of an aside about one athlete"
        );
    }
}

#[tokio::test]
async fn a_coachs_aside_about_an_athlete_on_a_garmin_device_carries_the_line_once() {
    for aside in [Aside::ReadsTheAthlete, Aside::NamesTheAthlete] {
        let turn = coach_aside(WAHOO_BOLT, WAHOO_BOLT, GARMIN_EDGE, aside).await;
        let line = garmin_line("fr");
        assert!(turn.reply.trim_end().ends_with(&line), "{}", turn.reply);
        assert_eq!(line_copies(&turn.reply), 1, "{}", turn.reply);
    }
}

/// An aside that names no one is about the room: every member's card is in
/// the prompt, so another member's Garmin data is owed, while the coach's own
/// card is never read on their seat.
#[tokio::test]
async fn a_coachs_aside_about_the_room_owes_a_members_garmin_and_never_the_coachs() {
    let turn = coach_aside(GARMIN_EDGE, WAHOO_BOLT, WAHOO_BOLT, Aside::NamesNoOne).await;
    assert!(turn.reply.contains("125 et 145"), "{}", turn.reply);
    carries_no_line(&turn.reply);
    assert!(
        turn.model_saw(OTHER_NAME) && turn.model_saw(ATHLETE_NAME),
        "every member's card is in the prompt"
    );

    let turn = coach_aside(WAHOO_BOLT, GARMIN_EDGE, WAHOO_BOLT, Aside::NamesNoOne).await;
    assert!(
        turn.reply.trim_end().ends_with(&garmin_line("fr")),
        "{}",
        turn.reply
    );
    assert_eq!(line_copies(&turn.reply), 1, "{}", turn.reply);
}

/// The coach's message in their thread about the athlete, who rides on
/// `athlete`; the coach rides on `coach_device`. The model answers from the
/// thread's context alone, reading no one.
async fn coach_thread_reply(coach_device: &str, athlete: &str) -> AsideTurn {
    let model = Arc::new(ScriptedModel::new(None, COACH_ANSWER));
    let provider: Arc<dyn LlmProvider> = model.clone();
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let coach = rider(&resources, "Cole Coach", coach_device).await;
    let athlete = rider(&resources, ATHLETE_NAME, athlete).await;
    let repos = &resources.common.repos;
    let agent = create_group_agent(repos, coach.0, coach.1).await.unwrap();
    let group = create_coached_group(repos, coach.0, coach.1, &agent, "Squad", coach.0)
        .await
        .unwrap();
    add_group_member(repos, group, athlete.0, athlete.1)
        .await
        .unwrap();
    let thread = repos
        .chat
        .open_subject_conversation(&NewSubjectConversation {
            user_id: &coach.0.to_string(),
            tenant_id: coach.1,
            title: ATHLETE_NAME,
            model: "attribution-model",
            agent_id: &agent,
            subject: ConversationSubject {
                athlete_user_id: athlete.0,
                group_id: group,
            },
        })
        .await
        .unwrap()
        .conversation;
    let reply = served_reply(&resources, coach, &thread.id, "Comment va-t-elle ?").await;
    AsideTurn {
        reply,
        model_read: model.read(),
    }
}

/// A coach's thread about one athlete carries her roster line, so a reply
/// that reads no one still answers from her data: her Garmin recording is
/// owed, and the coach's own never is.
#[tokio::test]
async fn a_coachs_thread_about_an_athlete_owes_her_garmin_without_a_tool_read() {
    let turn = coach_thread_reply(WAHOO_BOLT, GARMIN_EDGE).await;
    assert!(
        turn.model_saw(ATHLETE_NAME),
        "the thread's prompt carries the athlete it is about"
    );
    assert!(
        turn.reply.trim_end().ends_with(&garmin_line("fr")),
        "her roster line reached the model, so her Garmin data is owed: {}",
        turn.reply
    );
    assert_eq!(
        line_copies(&turn.reply),
        1,
        "the line appears once: {}",
        turn.reply
    );

    let turn = coach_thread_reply(GARMIN_EDGE, WAHOO_BOLT).await;
    carries_no_line(&turn.reply);
}

/// The athlete's own turn in which the model writes the line itself, with a
/// straight apostrophe, after its answer.
async fn reply_with_a_copied_line(source: Option<&str>) -> String {
    let answer = format!("{AGENT_ANSWER}\n\n{}", straight(&garmin_line("fr")));
    let model = Arc::new(ScriptedModel::activities_then(&answer));
    let resources = create_test_server_resources_with_chat_provider(model)
        .await
        .unwrap();
    let who = athlete_with_a_ride(&resources, source, None, true).await;
    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &who.0.to_string(),
            who.1,
            "copied line",
            "attribution-model",
            None,
            None,
        )
        .await
        .unwrap();
    served_reply(
        &resources,
        who,
        &conversation.id,
        "Comment se passe ma semaine ?",
    )
    .await
}

#[tokio::test]
async fn a_line_the_model_copies_gives_way_to_the_platforms_once() {
    let owed = reply_with_a_copied_line(Some("garmin")).await;
    assert!(owed.contains("200 km"), "{owed}");
    assert!(owed.trim_end().ends_with(&garmin_line("fr")), "{owed}");
    assert_eq!(line_copies(&owed), 1, "the platform's line, once: {owed}");

    let unowed = reply_with_a_copied_line(None).await;
    assert!(unowed.contains("200 km"), "{unowed}");
    assert_eq!(
        line_copies(&unowed),
        0,
        "a turn that owes nothing carries no line, the model's included: {unowed}"
    );
}

/// The model never reads the line back from an earlier reply, so it never
/// learns to write it.
#[tokio::test]
async fn the_model_never_reads_an_earlier_replys_attribution() {
    let model = Arc::new(ScriptedModel::activities_then(AGENT_ANSWER));
    let provider: Arc<dyn LlmProvider> = model.clone();
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let who = athlete_with_a_ride(&resources, Some("garmin"), None, true).await;
    let user = who.0.to_string();
    let chat = &resources.common.repos.chat;
    let conversation = chat
        .create_conversation(
            &user,
            who.1,
            "earlier reply",
            "attribution-model",
            None,
            None,
        )
        .await
        .unwrap();
    let earlier = format!("Ta semaine est solide.\n\n{}", garmin_line("fr"));
    for (role, content) in [("user", "Et ma semaine ?"), ("assistant", earlier.as_str())] {
        chat.add_message(&AddMessageParams {
            tenant_id: who.1,
            conversation_id: &conversation.id,
            user_id: &user,
            role,
            content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: Some("attribution-model"),
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await
        .unwrap();
    }

    let reply = served_reply(&resources, who, &conversation.id, "Et aujourd'hui ?").await;
    assert_eq!(line_copies(&reply), 1, "this reply owes its own: {reply}");
    let read = model.read();
    assert!(
        read.iter()
            .any(|message| message.contains("Ta semaine est solide.")),
        "the earlier reply is replayed: {read:?}"
    );
    assert!(
        read.iter().all(|message| line_copies(message) == 0),
        "the earlier reply replays without its attribution line"
    );
}
