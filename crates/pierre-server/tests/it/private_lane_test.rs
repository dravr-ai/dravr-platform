// ABOUTME: A coach's private lane in a one-athlete room (carnet#1249): work-thread turns stay out of the room, quotes are verified
// ABOUTME: Also the room's solo athlete, the line a publish leaves in the room and its bound chat, and that nothing else reaches the chat

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Design H, built as Design B: a coach's "Only Dravr" turns go to their
//! thread about the room's athlete (carnet#1070), which the room never reads.
//! These tests pin the server's half of that promise end to end — through
//! the chat, group and subject-plan routes, a recording model and a recording
//! group-chat poster — and the three things the room does get: the athlete
//! named to the coach as the room's one athlete, a quote the coach may carry,
//! and one line when the coach publishes.

#![cfg(all(feature = "client-messaging", feature = "client-groups"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::Router;
use chrono::{Datelike, Days, NaiveDate, Utc};
use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use futures_util::stream;
use helpers::axum_test::AxumTestRequest;
use pierre_contremaitre::messaging_strings::{
    KEY_PLAN_PUBLISHED_ROOM_POST, KEY_PLAN_PUBLISHED_ROOM_POST_MORE,
    KEY_PLAN_PUBLISHED_ROOM_POST_NEW,
};
use pierre_core::errors::AppError;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    NewGroupTranscriptEntry, RoomEntryBody, RoomTranscriptEntry, TranscriptSpeaker,
};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::RepositoryRegistry;
use pierre_llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_routes_groups::GroupRoutes;
use pierre_services::coach_plan_room_post::{
    post_plan_published, PlanChange, PlanRoomPost, RoomPostOutcome,
};
use pierre_services::group_chat_poster::GroupChatPoster;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plan_drafts::PublishTrainingPlanTool;
use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

const MODEL: &str = "private-lane-model";
const ANSWER: &str = "Noted — I'll look at the week with you.";

const COACH_NAME: &str = "Casey Coach";
const SUBJECT_NAME: &str = "Mira Subject";
const OTHER_NAME: &str = "Oscar Other";
const HIDDEN_NAME: &str = "Hana Hidden";

/// What the coach tells the agent privately; must reach nobody else.
const SECRET: &str = "private-lane-secret: she is not ready for the A race";
/// What the athlete says in the room before and after the private turn.
const ROOM_HELLO: &str = "Bonjour le groupe, grosse semaine pour moi";
const ROOM_LATER: &str = "Et pour samedi, on garde la sortie longue ?";
/// The athlete's room words the coach quotes.
const ATHLETE_WORDS: &str = "My knee hurt on Sunday's long run";
/// A member who does not share with the coach.
const HIDDEN_WORDS: &str = "hidden member words";
const COACH_ASK: &str = "Move her long run off Sunday";
const COACH_LATER: &str = "And what about next week?";
const COACH_WORKOUT: &str = "coach-drafted tempo";
const CHAT_ID: &str = "-100777";

/// Records every request's messages, joined, and answers in prose. Test-only:
/// the assertion point is what each prompt carried, not what the model said.
struct RecordingModel {
    calls: Mutex<Vec<String>>,
    models: Vec<String>,
}

impl RecordingModel {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            models: vec![MODEL.to_owned()],
        }
    }

    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Every message of every call after `from`, joined.
    fn since(&self, from: usize) -> String {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .skip(from)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait]
impl LlmProvider for RecordingModel {
    fn name(&self) -> &'static str {
        "private_lane_mock"
    }
    fn display_name(&self) -> &'static str {
        "Private-lane mock (records prompts)"
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
        let messages = request
            .messages
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.calls.lock().unwrap().push(messages);
        Ok(ChatResponse {
            content: ANSWER.to_owned(),
            model: MODEL.to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
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

/// Records every chat post instead of reaching a channel, and accepts each
/// as one part. Test-only: the real poster's escape, split and send path has
/// its own suite (`group_digest_room_test`).
#[derive(Default)]
struct RecordingPoster {
    posts: Mutex<Vec<(TenantId, ChannelType, String, String)>>,
}

impl RecordingPoster {
    fn posts(&self) -> Vec<(TenantId, ChannelType, String, String)> {
        self.posts.lock().unwrap().clone()
    }
}

#[async_trait]
impl GroupChatPoster for RecordingPoster {
    async fn post(
        &self,
        tenant_id: TenantId,
        channel_type: ChannelType,
        chat_id: &str,
        text: &str,
    ) -> usize {
        self.posts.lock().unwrap().push((
            tenant_id,
            channel_type,
            chat_id.to_owned(),
            text.to_owned(),
        ));
        1
    }
}

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@private-lane.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.users.update_locale(id, "en").await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    let token = generate_test_token(res, &user).await;
    Person {
        id,
        tenant,
        auth: format!("Bearer {token}"),
    }
}

/// The Monday `weeks` weeks after this one's, in UTC.
fn monday(weeks: u64) -> NaiveDate {
    let today = Utc::now().date_naive();
    let this = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    this + Days::new(7 * weeks)
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn week(start: NaiveDate, workout: &str) -> Value {
    let days: Vec<Value> = (0..7)
        .map(|offset| {
            json!({
                "date": ymd(start + Days::new(offset)),
                "sport": "run",
                "workout": workout,
                "duration_min": 40,
                "intensity": "Z2",
            })
        })
        .collect();
    json!({ "week_start": ymd(start), "focus": workout, "days": days })
}

fn outline(race: &str) -> Value {
    json!({
        "goal_race": { "name": race, "date": "2027-05-16", "discipline": "trail", "priority": "A" },
        "strategy": "a season",
    })
}

/// How the room post names a day to an English reader: `Mon Oct 27`.
fn day_label(date: NaiveDate) -> String {
    date.format("%a %b %-d").to_string()
}

async fn create_group(
    repos: &RepositoryRegistry,
    owner: &Person,
    agent_id: &str,
    channel: Option<(&str, &str)>,
) -> CoachingGroup {
    let now = Utc::now();
    repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: "Tempo Squad".to_owned(),
                description: None,
                agent_id: agent_id.to_owned(),
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 20,
                is_active: true,
                channel_type: channel.map(|(c, _)| c.to_owned()),
                channel_chat_id: channel.map(|(_, id)| id.to_owned()),
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
}

async fn attach_coach(repos: &RepositoryRegistry, group: &CoachingGroup, coach: &Person) {
    let tenant = TenantId::parse_str(&group.tenant_id).unwrap();
    assert!(
        repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(coach.id), tenant)
            .await
            .unwrap(),
        "premise: the coach is attached to the group"
    );
}

async fn join(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    who: &Person,
    role: GroupRole,
    shares_with_coach: bool,
) {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: who.id,
            tenant_id: who.tenant.to_string(),
            role,
            peer_sharing_consent: false,
            coach_sharing_consent: shares_with_coach,
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// Seed the room-post strings the contremaitre bump brings, in English.
fn seed_room_post_strings(res: &ServerContext) {
    for (key, text) in [
        (
            KEY_PLAN_PUBLISHED_ROOM_POST,
            "{0} published an update to your plan: {1}.",
        ),
        (
            KEY_PLAN_PUBLISHED_ROOM_POST_NEW,
            "{0} published your new plan.",
        ),
        (KEY_PLAN_PUBLISHED_ROOM_POST_MORE, "+{0} more"),
    ] {
        res.mcp
            .messaging_strings_registry
            .update(key, "en", text.to_owned(), "sha".to_owned());
    }
}

/// A group its owner owns and the coach coaches, the subject its one
/// consenting athlete, and the coach's thread about the subject open.
struct Fixture {
    res: Arc<ServerContext>,
    model: Arc<RecordingModel>,
    poster: Arc<RecordingPoster>,
    owner: Person,
    coach: Person,
    subject: Person,
    group: CoachingGroup,
    thread: String,
}

impl Fixture {
    /// The room bound to `channel` (`(channel_type, chat_id)`) when given.
    async fn new(channel: Option<(&str, &str)>) -> Self {
        let model = Arc::new(RecordingModel::new());
        let base = create_test_server_resources_with_chat_provider(model.clone())
            .await
            .unwrap();
        // The context's own poster, swapped for one that records: every post
        // into a group's chat goes out through this seam.
        let poster = Arc::new(RecordingPoster::default());
        let shared: Arc<dyn GroupChatPoster> = poster.clone();
        let mut context = (*base).clone();
        context.mcp.group_chat_poster = shared;
        let res = Arc::new(context);
        seed_room_post_strings(&res);

        let owner = person(&res, "owner", "Olive Owner").await;
        let coach = person(&res, "coach", COACH_NAME).await;
        let subject = person(&res, "subject", SUBJECT_NAME).await;
        let repos = &res.common.repos;
        // A connected source, so the athlete's own room turns reach the model
        // rather than an onboarding answer.
        repos
            .provider_connections
            .register_connection(
                subject.id,
                subject.tenant,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();
        let agent_id = create_group_agent(repos, owner.id, owner.tenant)
            .await
            .unwrap();
        let group = create_group(repos, &owner, &agent_id, channel).await;
        attach_coach(repos, &group, &coach).await;
        join(repos, &group, &subject, GroupRole::Member, true).await;
        let thread = open_thread(&res, &coach, &subject, &group).await;
        Self {
            res,
            model,
            poster,
            owner,
            coach,
            subject,
            group,
            thread,
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    fn chat(&self) -> Router {
        ChatRoutes::routes(Arc::clone(&self.res))
    }

    async fn request(
        &self,
        request: AxumTestRequest,
        auth: &str,
        router: Router,
    ) -> (StatusCode, Value) {
        let resp = request.header("authorization", auth).send(router).await;
        let status = resp.status_code();
        let body = if status == StatusCode::NO_CONTENT {
            Value::Null
        } else {
            resp.json()
        };
        (status, body)
    }

    /// `who` sends `body` into `conversation`.
    async fn send(&self, conversation: &str, who: &Person, body: Value) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::post(&format!("/api/chat/conversations/{conversation}/messages"))
                .json(&body),
            &who.auth,
            self.chat(),
        )
        .await
    }

    async fn messages(&self, conversation: &str, who: &Person) -> Value {
        let (status, body) = self
            .request(
                AxumTestRequest::get(&format!("/api/chat/conversations/{conversation}/messages")),
                &who.auth,
                self.chat(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn transcript(&self, who: &Person) -> Value {
        let (status, body) = self
            .request(
                AxumTestRequest::get(&format!("/api/chat/groups/{}/transcript", self.group.id)),
                &who.auth,
                self.chat(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// Every entry of the room, newest first, as the coach reads it: the
    /// athlete's own entries readable while she shares with the coach, every
    /// other entry in its place.
    async fn room(&self) -> Vec<RoomTranscriptEntry> {
        self.repos()
            .groups
            .list_room_transcript_for(&self.group.id.to_string(), self.coach.id, None, 500)
            .await
            .unwrap()
    }

    /// The athlete's room conversation, as they open it from the group.
    async fn athletes_room(&self) -> String {
        let (status, body) = self
            .request(
                AxumTestRequest::post("/api/chat/conversations")
                    .json(&json!({ "title": "Squad room", "group_id": self.group.id })),
                &self.subject.auth,
                self.chat(),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_str().unwrap().to_owned()
    }

    /// `who` saves through the plan tool, inside `conversation` when given.
    async fn saves(&self, who: &Person, conversation: Option<&str>, args: Value) -> Value {
        call(&self.res, who, &SaveTrainingPlanTool, conversation, args).await
    }

    async fn athlete_season(&self) {
        let saved = self
            .saves(
                &self.subject,
                None,
                json!({
                    "outline": outline("Athlete Race"),
                    "weeks": [week(monday(0), "athlete run"), week(monday(1), "athlete run")],
                }),
            )
            .await;
        assert!(saved.get("error").is_none(), "{saved}");
    }

    /// The coach drafts this week over the athlete's season.
    async fn coach_drafts_this_week(&self) {
        let saved = self
            .saves(
                &self.coach,
                Some(&self.thread),
                json!({ "weeks": [week(monday(0), COACH_WORKOUT)] }),
            )
            .await;
        assert_eq!(saved["draft"], true, "{saved}");
    }

    async fn publish(&self) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::post(&format!(
                "/api/chat/conversations/{}/subject-plan/publish",
                self.thread
            ))
            .json(&json!({ "overwrite_athlete_changes": false })),
            &self.coach.auth,
            self.chat(),
        )
        .await
    }
}

async fn open_thread(
    res: &Arc<ServerContext>,
    coach: &Person,
    athlete: &Person,
    group: &CoachingGroup,
) -> String {
    let resp = AxumTestRequest::post(&format!("/api/chat/athletes/{}/thread", athlete.id))
        .header("authorization", &coach.auth)
        .json(&json!({ "group_id": group.id }))
        .send(ChatRoutes::routes(Arc::clone(res)))
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::OK,
        "premise: the coach opens their thread about the athlete"
    );
    resp.json::<Value>()["id"].as_str().unwrap().to_owned()
}

async fn call<T: McpTool<dyn ToolRuntime>>(
    res: &Arc<ServerContext>,
    who: &Person,
    tool: &T,
    conversation: Option<&str>,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(res);
    let ctx = ToolContext::new()
        .with_user(who.id.to_string())
        .with_tenant(who.tenant.to_string())
        .with_auth_method("jwt_bearer");
    CONVERSATION_ID
        .scope(conversation.map(ToOwned::to_owned), async {
            tool.execute(&runtime, &ctx, args)
                .await
                .structured_content
                .expect("tool result carries structured content")
        })
        .await
}

/// Append `content` to `group`'s room as `author`'s, spoken by `speaker`,
/// as the group's chat writes it — under the group's tenant, stamped for any
/// transport — and return the entry's id.
async fn room_entry(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    author: &Person,
    speaker: TranscriptSpeaker,
    content: &str,
) -> Uuid {
    let written = Written {
        tenant: &group.tenant_id,
        policy: TransportPolicy::AnyTransport,
    };
    room_entry_written(repos, group, author, (speaker, content), written).await
}

/// The tenant a room entry is written under and the stamp it carries.
struct Written<'tenant> {
    tenant: &'tenant str,
    policy: TransportPolicy,
}

/// [`room_entry`], written under `written.tenant` and stamped
/// `written.policy`.
async fn room_entry_written(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    author: &Person,
    (speaker, content): (TranscriptSpeaker, &str),
    written: Written<'_>,
) -> Uuid {
    let group_id = group.id.to_string();
    repos
        .groups
        .append_transcript_entry(&NewGroupTranscriptEntry {
            group_id: &group_id,
            tenant_id: written.tenant,
            author_user_id: author.id,
            speaker,
            content,
            source_conversation_id: None,
            source_message_id: None,
            transport_policy: written.policy,
        })
        .await
        .unwrap();
    repos
        .groups
        .list_room_transcript_for(&group_id, author.id, None, 500)
        .await
        .unwrap()
        .into_iter()
        .find(|entry| match &entry.body {
            RoomEntryBody::Shared(shared) => shared.content == content,
            RoomEntryBody::Withheld => false,
        })
        .map(|entry| entry.id)
        .expect("the entry its author just wrote")
}

/// The words of every readable room entry.
fn contents(entries: &[RoomTranscriptEntry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| match &entry.body {
            RoomEntryBody::Shared(shared) => Some(shared.content.clone()),
            RoomEntryBody::Withheld => None,
        })
        .collect()
}

/// The `details.reason` of a refused route call.
fn reason(body: &Value) -> &str {
    body["details"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal naming its reason, got {body}"))
}

// ── The private turn ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_private_turn_stays_out_of_the_room_and_every_athlete_prompt() {
    let fx = Fixture::new(None).await;
    let room = fx.athletes_room().await;
    let (status, body) = fx
        .send(&room, &fx.subject, json!({ "content": ROOM_HELLO }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let before = fx.room().await;
    assert!(
        contents(&before).iter().any(|c| c == ROOM_HELLO),
        "premise: the athlete's room turn reaches the room"
    );

    let (status, body) = fx
        .send(&fx.thread, &fx.coach, json!({ "content": SECRET }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let after = fx.room().await;
    assert_eq!(
        after.len(),
        before.len(),
        "a work-thread turn writes no room entry"
    );
    assert!(
        !contents(&after).iter().any(|c| c.contains(SECRET)),
        "the private words are nowhere in the room"
    );
    for reader in [&fx.subject, &fx.coach] {
        let read = fx.transcript(reader).await;
        assert!(
            !read.to_string().contains(SECRET),
            "no room read carries the private words: {read}"
        );
    }

    // The athlete's next room turn: their prompt carries the room's own
    // history and not one word of the coach's private turn.
    let from = fx.model.calls();
    let (status, body) = fx
        .send(&room, &fx.subject, json!({ "content": ROOM_LATER }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let athlete_prompts = fx.model.since(from);
    assert!(
        athlete_prompts.contains(ROOM_LATER) && athlete_prompts.contains(ROOM_HELLO),
        "premise: the athlete's turn reached the model with the room's history"
    );
    assert!(
        !athlete_prompts.contains(SECRET),
        "the private words reached an athlete's prompt"
    );
    let history = fx.messages(&room, &fx.subject).await;
    assert!(
        !history.to_string().contains(SECRET),
        "the room conversation's history holds no private row: {history}"
    );
}

// ── The verified quote ───────────────────────────────────────────────────

#[tokio::test]
async fn a_quoted_room_message_reaches_the_model_as_the_athletes_words() {
    let fx = Fixture::new(None).await;
    let entry = room_entry(
        fx.repos(),
        &fx.group,
        &fx.subject,
        TranscriptSpeaker::Member,
        ATHLETE_WORDS,
    )
    .await;
    let room_before = fx.room().await.len();

    let from = fx.model.calls();
    let (status, body) = fx
        .send(
            &fx.thread,
            &fx.coach,
            json!({ "content": COACH_ASK, "quote_entry_id": entry }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let quote = &body["user_message"]["quote"];
    assert_eq!(quote["entry_id"], entry.to_string(), "{body}");
    assert_eq!(
        quote["author_user_id"],
        fx.subject.id.to_string(),
        "the quote names its author"
    );
    assert_eq!(
        quote["author_display_name"], SUBJECT_NAME,
        "by her roster name"
    );
    assert_eq!(quote["content"], ATHLETE_WORDS, "in her own words");
    assert!(
        quote["created_at"]
            .as_str()
            .is_some_and(|at| !at.is_empty()),
        "{quote}"
    );
    assert_eq!(
        body["user_message"]["content"], COACH_ASK,
        "the coach's words stay their own"
    );

    let prompt = fx.model.since(from);
    assert!(
        prompt.contains(&format!(
            "[Quoting {SUBJECT_NAME}'s message in the group room, "
        )) && prompt.contains(&format!("> {ATHLETE_WORDS}\n\n{COACH_ASK}")),
        "the athlete's words lead the coach's, framed as hers: {prompt}"
    );

    // A reload shows the quote on the coach's row; the room gained nothing.
    let history = fx.messages(&fx.thread, &fx.coach).await;
    let row = history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user" && m["content"] == COACH_ASK)
        .expect("the coach's quoting row");
    assert_eq!(row["quote"]["content"], ATHLETE_WORDS, "{row}");
    assert_eq!(
        fx.room().await.len(),
        room_before,
        "a quote writes no room entry"
    );

    // Once the athlete stops sharing, the quote is withheld with its row.
    assert!(
        fx.repos()
            .groups
            .update_coach_sharing_consent(&fx.group.id.to_string(), fx.subject.id, false)
            .await
            .unwrap(),
        "premise: the athlete stops sharing with the coach"
    );
    let from = fx.model.calls();
    let (status, body) = fx
        .send(&fx.thread, &fx.coach, json!({ "content": COACH_LATER }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let refused = fx.model.since(from);
    assert!(refused.contains(COACH_LATER), "{refused}");
    assert!(
        !refused.contains(ATHLETE_WORDS),
        "a withdrawn athlete's quoted words reached the model: {refused}"
    );
}

/// The coach's quoting row as `fx.thread` reads back to the coach.
async fn quoting_row(fx: &Fixture) -> Value {
    let history = fx.messages(&fx.thread, &fx.coach).await;
    history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user" && m["content"] == COACH_ASK)
        .cloned()
        .unwrap_or_else(|| panic!("the coach's quoting row: {history}"))
}

/// A thread read shows the quote only while the athlete's words are the
/// coach's to read, as the room does: withheld once she stops sharing or
/// leaves, shown again when she shares again.
#[tokio::test]
async fn a_thread_read_withholds_the_quote_the_room_withholds() {
    let fx = Fixture::new(None).await;
    let repos = fx.repos();
    let group_id = fx.group.id.to_string();
    let entry = room_entry(
        repos,
        &fx.group,
        &fx.subject,
        TranscriptSpeaker::Member,
        ATHLETE_WORDS,
    )
    .await;
    let (status, body) = fx
        .send(
            &fx.thread,
            &fx.coach,
            json!({ "content": COACH_ASK, "quote_entry_id": entry }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        quoting_row(&fx).await["quote"]["content"],
        ATHLETE_WORDS,
        "premise: the quote shows while she shares"
    );

    assert!(
        repos
            .groups
            .update_coach_sharing_consent(&group_id, fx.subject.id, false)
            .await
            .unwrap(),
        "premise: the athlete stops sharing with the coach"
    );
    let revoked = quoting_row(&fx).await;
    assert!(
        revoked.get("quote").is_none(),
        "a revoked athlete's words left the thread read: {revoked}"
    );
    assert_eq!(revoked["content"], COACH_ASK, "the coach's own words stay");

    assert!(
        repos
            .groups
            .update_coach_sharing_consent(&group_id, fx.subject.id, true)
            .await
            .unwrap(),
        "premise: she shares again"
    );
    assert_eq!(
        quoting_row(&fx).await["quote"]["content"],
        ATHLETE_WORDS,
        "the row kept the quote; only the read withheld it"
    );

    assert!(
        repos
            .groups
            .remove_member(&group_id, fx.subject.id)
            .await
            .unwrap(),
        "premise: the athlete leaves the group"
    );
    let left = quoting_row(&fx).await;
    assert!(
        left.get("quote").is_none(),
        "a departed athlete's words left the thread read: {left}"
    );
}

/// The coach's quoting row goes only where the entry it quotes may: a
/// first-party-only entry makes the row first-party-only.
#[tokio::test]
async fn a_quoting_row_carries_the_quoted_entrys_stamp() {
    let fx = Fixture::new(None).await;
    let entry = room_entry_written(
        fx.repos(),
        &fx.group,
        &fx.subject,
        (TranscriptSpeaker::Member, ATHLETE_WORDS),
        Written {
            tenant: &fx.group.tenant_id,
            policy: TransportPolicy::FirstPartyOnly,
        },
    )
    .await;
    let (status, body) = fx
        .send(
            &fx.thread,
            &fx.coach,
            json!({ "content": COACH_ASK, "quote_entry_id": entry }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "the app may carry it: {body}");

    let page = fx
        .repos()
        .chat
        .get_message_page(
            &fx.thread,
            &fx.coach.id.to_string(),
            fx.coach.tenant,
            None,
            100,
        )
        .await
        .unwrap();
    let row = page
        .messages
        .iter()
        .find(|m| m.role == "user" && m.content == COACH_ASK)
        .expect("the coach's quoting row");
    assert_eq!(
        row.transport_policy,
        TransportPolicy::FirstPartyOnly,
        "the row is stamped as strictly as the entry it quotes"
    );
}

/// The coach's quoting row, and the reply drawn from it, name the athlete
/// whose words they carry (carnet#1174): they replay only while the coach may
/// still read her, and go with her account. A question that quotes nothing
/// names no one.
#[tokio::test]
async fn a_quoting_row_and_its_reply_name_the_quoted_athlete() {
    let fx = Fixture::new(None).await;
    let (status, body) = fx
        .send(&fx.thread, &fx.coach, json!({ "content": COACH_LATER }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entry = room_entry(
        fx.repos(),
        &fx.group,
        &fx.subject,
        TranscriptSpeaker::Member,
        ATHLETE_WORDS,
    )
    .await;
    let (status, body) = fx
        .send(
            &fx.thread,
            &fx.coach,
            json!({ "content": COACH_ASK, "quote_entry_id": entry }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows = fx
        .repos()
        .chat
        .get_messages(&fx.thread, &fx.coach.id.to_string(), fx.coach.tenant)
        .await
        .unwrap();
    // Each question with the member it and its reply are about.
    let about = |question: &str| {
        let at = rows
            .iter()
            .position(|m| m.role == "user" && m.content == question)
            .unwrap_or_else(|| panic!("the coach's question {question:?}: {rows:?}"));
        let reply = rows[at..]
            .iter()
            .find(|m| m.role == "assistant")
            .unwrap_or_else(|| panic!("the reply to {question:?}: {rows:?}"));
        (rows[at].about_user_id, reply.about_user_id)
    };
    assert_eq!(
        about(COACH_ASK),
        (Some(fx.subject.id), Some(fx.subject.id)),
        "the quoting row and the reply drawn from it name the quoted athlete"
    );
    assert_eq!(
        about(COACH_LATER).0,
        None,
        "a question that quotes nothing carries no one's words"
    );
}

/// The athlete's room words are written under the group's tenant (its bound
/// chat) or under her own (her in-app room conversation); an entry of the
/// group written under any other tenant is no entry to quote.
#[tokio::test]
async fn a_quote_reads_the_room_under_its_own_tenants_only() {
    let fx = Fixture::new(None).await;
    let room = fx.athletes_room().await;
    let (status, body) = fx
        .send(&room, &fx.subject, json!({ "content": ROOM_HELLO }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_ne!(
        fx.subject.tenant.to_string(),
        fx.group.tenant_id,
        "premise: she joined from her own tenant"
    );
    let in_app = fx
        .room()
        .await
        .into_iter()
        .find(|entry| match &entry.body {
            RoomEntryBody::Shared(shared) => shared.content == ROOM_HELLO,
            RoomEntryBody::Withheld => false,
        })
        .expect("her in-app room turn in the room")
        .id;
    let (status, body) = fx
        .send(
            &fx.thread,
            &fx.coach,
            json!({ "content": COACH_ASK, "quote_entry_id": in_app }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "her words from the app, under her tenant: {body}"
    );

    let stranger = person(&fx.res, "stranger", "Sam Stranger").await;
    let stranger_tenant = stranger.tenant.to_string();
    let foreign = room_entry_written(
        fx.repos(),
        &fx.group,
        &fx.subject,
        (TranscriptSpeaker::Member, "words under a stranger's tenant"),
        Written {
            tenant: &stranger_tenant,
            policy: TransportPolicy::AnyTransport,
        },
    )
    .await;
    let (status, body) = fx
        .send(
            &fx.thread,
            &fx.coach,
            json!({ "content": COACH_LATER, "quote_entry_id": foreign }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(reason(&body), "quote_not_found", "{body}");
}

#[tokio::test]
async fn a_quote_the_coach_may_not_carry_is_refused_by_its_reason() {
    let fx = Fixture::new(None).await;
    let repos = fx.repos();
    let other = person(&fx.res, "other", OTHER_NAME).await;
    let hidden = person(&fx.res, "hidden", HIDDEN_NAME).await;
    join(repos, &fx.group, &other, GroupRole::Member, true).await;
    join(repos, &fx.group, &hidden, GroupRole::Member, false).await;
    let athlete_words = room_entry(
        repos,
        &fx.group,
        &fx.subject,
        TranscriptSpeaker::Member,
        ATHLETE_WORDS,
    )
    .await;
    let other_words = room_entry(
        repos,
        &fx.group,
        &other,
        TranscriptSpeaker::Member,
        "other words",
    )
    .await;
    let hidden_words = room_entry(
        repos,
        &fx.group,
        &hidden,
        TranscriptSpeaker::Member,
        HIDDEN_WORDS,
    )
    .await;
    let agent_reply = room_entry(
        repos,
        &fx.group,
        &fx.subject,
        TranscriptSpeaker::Coach,
        "agent reply to her",
    )
    .await;

    // The same athlete's words in another group the coach coaches her in.
    let second = create_group(repos, &fx.owner, &fx.group.agent_id, None).await;
    attach_coach(repos, &second, &fx.coach).await;
    join(repos, &second, &fx.subject, GroupRole::Member, true).await;
    let elsewhere = room_entry(
        repos,
        &second,
        &fx.subject,
        TranscriptSpeaker::Member,
        "words elsewhere",
    )
    .await;

    let (status, body) = fx
        .request(
            AxumTestRequest::post("/api/chat/conversations").json(&json!({ "title": "Mine" })),
            &fx.coach.auth,
            fx.chat(),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let own_thread = body["id"].as_str().unwrap().to_owned();

    let rows_before = fx.messages(&fx.thread, &fx.coach).await["messages"]
        .as_array()
        .unwrap()
        .len();
    let thread = fx.thread.as_str();
    let cases: [(&str, String, &str, StatusCode, &str); 7] = [
        (
            thread,
            elsewhere.to_string(),
            COACH_ASK,
            StatusCode::NOT_FOUND,
            "quote_not_found",
        ),
        (
            thread,
            "not-a-uuid".to_owned(),
            COACH_ASK,
            StatusCode::NOT_FOUND,
            "quote_not_found",
        ),
        (
            thread,
            other_words.to_string(),
            COACH_ASK,
            StatusCode::BAD_REQUEST,
            "quote_not_athlete",
        ),
        (
            thread,
            agent_reply.to_string(),
            COACH_ASK,
            StatusCode::BAD_REQUEST,
            "quote_not_athlete",
        ),
        (
            thread,
            hidden_words.to_string(),
            COACH_ASK,
            StatusCode::FORBIDDEN,
            "quote_withheld",
        ),
        (
            own_thread.as_str(),
            athlete_words.to_string(),
            COACH_ASK,
            StatusCode::BAD_REQUEST,
            "quote_not_allowed",
        ),
        (
            thread,
            athlete_words.to_string(),
            "/plan",
            StatusCode::BAD_REQUEST,
            "quote_not_allowed",
        ),
    ];
    for (conversation, entry, content, status, why) in cases {
        let (got, body) = fx
            .send(
                conversation,
                &fx.coach,
                json!({ "content": content, "quote_entry_id": entry }),
            )
            .await;
        assert_eq!(got, status, "{why}: {body}");
        assert_eq!(reason(&body), why, "{body}");
        assert!(
            !body.to_string().contains(HIDDEN_WORDS),
            "a refusal never carries the words it refused"
        );
    }
    let rows_after = fx.messages(&fx.thread, &fx.coach).await["messages"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(rows_after, rows_before, "a refused quote writes no row");

    // The athlete stops sharing with the coach: her own words are no longer
    // the coach's to quote.
    assert!(
        repos
            .groups
            .update_coach_sharing_consent(&fx.group.id.to_string(), fx.subject.id, false)
            .await
            .unwrap(),
        "premise: the athlete stops sharing with the coach"
    );
    let (status, body) = fx
        .send(
            &fx.thread,
            &fx.coach,
            json!({ "content": COACH_ASK, "quote_entry_id": athlete_words }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(reason(&body), "coach_sharing_revoked", "{body}");
}

// ── The room's shape ─────────────────────────────────────────────────────

async fn group_as(fx: &Fixture, group: &CoachingGroup, who: &Person) -> Value {
    let (status, body) = fx
        .request(
            AxumTestRequest::get(&format!("/api/groups/{}", group.id)),
            &who.auth,
            GroupRoutes::routes(Arc::clone(&fx.res)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn the_room_names_its_one_athlete_to_its_coach_alone() {
    let fx = Fixture::new(None).await;
    let repos = fx.repos();

    let solo = group_as(&fx, &fx.group, &fx.coach).await;
    assert_eq!(
        solo["solo_athlete"],
        json!({ "user_id": fx.subject.id.to_string(), "display_name": SUBJECT_NAME }),
        "{solo}"
    );
    let as_athlete = group_as(&fx, &fx.group, &fx.subject).await;
    assert!(
        as_athlete.get("solo_athlete").is_none(),
        "never named to the athlete: {as_athlete}"
    );

    // A second athlete joins: the room is no longer one coach and one athlete.
    let other = person(&fx.res, "other", OTHER_NAME).await;
    join(repos, &fx.group, &other, GroupRole::Member, true).await;
    let pair = group_as(&fx, &fx.group, &fx.coach).await;
    assert!(pair.get("solo_athlete").is_none(), "{pair}");

    // An owner who is not the group's coach is told nothing, and a coach who
    // also owns the group is not counted as its athlete.
    let coach_agent = create_group_agent(repos, fx.coach.id, fx.coach.tenant)
        .await
        .unwrap();
    let owned = create_group(repos, &fx.coach, &coach_agent, None).await;
    join(repos, &owned, &fx.coach, GroupRole::Owner, true).await;
    join(repos, &owned, &fx.subject, GroupRole::Member, true).await;
    let uncoached = group_as(&fx, &owned, &fx.coach).await;
    assert!(
        uncoached.get("solo_athlete").is_none(),
        "an owner is not the room's coach: {uncoached}"
    );
    attach_coach(repos, &owned, &fx.coach).await;
    let owner_coach = group_as(&fx, &owned, &fx.coach).await;
    assert_eq!(
        owner_coach["solo_athlete"]["user_id"],
        fx.subject.id.to_string(),
        "the coach's own membership is not an athlete: {owner_coach}"
    );
}

// ── The publish line ─────────────────────────────────────────────────────

#[tokio::test]
async fn a_publish_says_the_changed_days_in_the_room_once() {
    let fx = Fixture::new(None).await;
    fx.athlete_season().await;
    fx.coach_drafts_this_week().await;
    let before = fx.room().await.len();

    let (status, body) = fx.publish().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["posted_in_room"], true, "{body}");

    let room = fx.room().await;
    assert_eq!(room.len(), before + 1, "exactly one room entry");
    let entry = &room[0];
    assert_eq!(entry.speaker, TranscriptSpeaker::Coach, "the agent's voice");
    let days: Vec<String> = (0..4)
        .map(|d| day_label(monday(0) + Days::new(d)))
        .collect();
    let expected = format!(
        "{COACH_NAME} published an update to your plan: {}, +3 more.",
        days.join(", ")
    );
    match &entry.body {
        RoomEntryBody::Shared(shared) => {
            assert_eq!(
                shared.author_user_id, fx.subject.id,
                "attributed to the athlete"
            );
            assert_eq!(
                shared.content, expected,
                "the line names the coach and the changed days"
            );
            assert!(
                shared.source_conversation_id.is_none(),
                "from no conversation"
            );
        }
        RoomEntryBody::Withheld => panic!("the coach reads the line while the athlete shares"),
    }

    // As the athlete's room read returns it.
    let read = fx.transcript(&fx.subject).await;
    let line = read["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["content"] == expected.as_str())
        .unwrap_or_else(|| panic!("the line in the athlete's room: {read}"));
    assert_eq!(line["speaker"], "coach", "the agent's voice");
    assert_eq!(line["own"], true, "the athlete's own entry");
    assert_eq!(line["withheld"], false, "readable by the athlete");
    assert!(line["message_id"].is_null(), "{line}");
    assert!(
        fx.poster.posts().is_empty(),
        "a room bound to no chat posts nowhere"
    );
}

#[tokio::test]
async fn a_first_plan_published_from_the_tool_is_said_new() {
    let fx = Fixture::new(None).await;
    let saved = fx
        .saves(
            &fx.coach,
            Some(&fx.thread),
            json!({ "outline": outline("Coach Race"), "weeks": [week(monday(0), COACH_WORKOUT)] }),
        )
        .await;
    assert_eq!(saved["draft"], true, "{saved}");
    let before = fx.room().await.len();

    let published = call(
        &fx.res,
        &fx.coach,
        &PublishTrainingPlanTool,
        Some(&fx.thread),
        json!({ "action": "publish" }),
    )
    .await;
    assert_eq!(published["status"], "published", "{published}");
    assert!(
        published["message"]
            .as_str()
            .is_some_and(|m| m.contains("was posted in your group with")),
        "the tool tells the coach the room was told: {published}"
    );
    let room = fx.room().await;
    assert_eq!(room.len(), before + 1, "exactly one room entry");
    assert_eq!(
        contents(&room[..1]),
        vec![format!("{COACH_NAME} published your new plan.")],
        "a plan the athlete never had is said new"
    );
}

#[tokio::test]
async fn no_line_is_posted_once_the_athlete_has_left() {
    let fx = Fixture::new(Some(("telegram", CHAT_ID))).await;
    let repos = fx.repos();
    let registry = &fx.res.mcp.messaging_strings_registry;
    let post = PlanRoomPost {
        group_id: fx.group.id,
        coach: fx.coach.id,
        athlete: fx.subject.id,
        change: &PlanChange::New,
        transport_policy: TransportPolicy::AnyTransport,
    };
    let before = fx.room().await.len();

    let posted = post_plan_published(repos, registry, Some(fx.poster.as_ref()), &post).await;
    assert!(
        posted.in_room && posted.in_chat,
        "premise: a live member's room is told"
    );
    assert_eq!(
        fx.room().await.len(),
        before + 1,
        "premise: one entry while she is a member"
    );

    assert!(
        repos
            .groups
            .remove_member(&fx.group.id.to_string(), fx.subject.id)
            .await
            .unwrap(),
        "premise: the athlete leaves"
    );
    let left = post_plan_published(repos, registry, Some(fx.poster.as_ref()), &post).await;
    assert!(!left.in_room && !left.in_chat, "{left:?}");
    assert_eq!(fx.room().await.len(), before + 1, "no entry once she left");
    assert_eq!(fx.poster.posts().len(), 1, "no chat post once she left");
}

/// A second athlete makes the room a team's: the line names the athlete's
/// plan, so it is said nowhere, and the publish answer says so.
#[tokio::test]
async fn a_publish_says_nothing_in_a_room_with_a_second_athlete() {
    let fx = Fixture::new(None).await;
    let other = person(&fx.res, "other", OTHER_NAME).await;
    join(fx.repos(), &fx.group, &other, GroupRole::Member, true).await;
    fx.athlete_season().await;
    fx.coach_drafts_this_week().await;
    let before = fx.room().await.len();

    let (status, body) = fx.publish().await;
    assert_eq!(status, StatusCode::OK, "the publish itself lands: {body}");
    assert_eq!(body["posted_in_room"], false, "{body}");
    assert_eq!(
        fx.room().await.len(),
        before,
        "no room entry in a two-athlete room"
    );
}

/// A Telegram team group the coach coaches the athlete in hears nothing of
/// her plan — not in its room, not in its chat — while the same room held
/// by the coach and her alone is told in both.
#[tokio::test]
async fn a_chat_bound_team_hears_nothing_of_a_publish() {
    let fx = Fixture::new(Some(("telegram", CHAT_ID))).await;
    let repos = fx.repos();
    let registry = &fx.res.mcp.messaging_strings_registry;
    let post = PlanRoomPost {
        group_id: fx.group.id,
        coach: fx.coach.id,
        athlete: fx.subject.id,
        change: &PlanChange::New,
        transport_policy: TransportPolicy::AnyTransport,
    };
    let solo = post_plan_published(repos, registry, Some(fx.poster.as_ref()), &post).await;
    assert!(
        solo.in_room && solo.in_chat,
        "premise: the coach's and the athlete's room is told, its chat too"
    );
    let room_before = fx.room().await.len();
    let posts_before = fx.poster.posts().len();

    for n in 0..3 {
        let teammate = person(&fx.res, &format!("teammate-{n}"), &format!("Teammate {n}")).await;
        join(repos, &fx.group, &teammate, GroupRole::Member, true).await;
    }
    let team = post_plan_published(repos, registry, Some(fx.poster.as_ref()), &post).await;
    assert_eq!(team, RoomPostOutcome::default(), "nothing posted: {team:?}");
    assert_eq!(
        fx.room().await.len(),
        room_before,
        "no entry in the team's room"
    );
    assert_eq!(
        fx.poster.posts().len(),
        posts_before,
        "nothing reaches the team's chat"
    );

    // The same through the publish route.
    fx.athlete_season().await;
    fx.coach_drafts_this_week().await;
    let (status, body) = fx.publish().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["posted_in_room"], false, "{body}");
    assert_eq!(fx.room().await.len(), room_before, "{body}");
    assert_eq!(fx.poster.posts().len(), posts_before, "{body}");
}

/// The coach's display name is whatever the coach typed. In the agent's line
/// it is one bounded line that a Markdown client draws as text — no link, no
/// URL to linkify — and the chat gets the same folded name, which the poster
/// escapes for its channel.
#[tokio::test]
async fn the_publish_line_names_the_coach_as_text() {
    let fx = Fixture::new(Some(("telegram", CHAT_ID))).await;
    let repos = fx.repos();
    repos
        .users
        .update_display_name(
            fx.coach.id,
            "[Confirm your plan](https://evil.example)\nSecond line",
        )
        .await
        .unwrap();
    let post = PlanRoomPost {
        group_id: fx.group.id,
        coach: fx.coach.id,
        athlete: fx.subject.id,
        change: &PlanChange::New,
        transport_policy: TransportPolicy::AnyTransport,
    };

    let posted = post_plan_published(
        repos,
        &fx.res.mcp.messaging_strings_registry,
        Some(fx.poster.as_ref()),
        &post,
    )
    .await;
    assert!(posted.in_room && posted.in_chat, "{posted:?}");

    assert_eq!(
        contents(&fx.room().await[..1]),
        vec![
            r"\[Confirm your plan\] \(https\:\/\/evil\.example\) Second line published your new plan."
                .to_owned()
        ],
        "the room's line holds the name as one escaped line"
    );
    let posts = fx.poster.posts();
    assert_eq!(posts.len(), 1, "one chat post");
    assert_eq!(
        posts[0].3,
        "[Confirm your plan] (https://evil.example) Second line published your new plan.",
        "the chat's line holds the folded name, left to the poster to escape"
    );
}

/// Reopening the thread from another group the coach coaches the athlete in
/// rebinds it there, so the publish is said in that group's room alone.
#[tokio::test]
async fn a_rebound_thread_says_the_publish_where_it_was_last_opened() {
    let fx = Fixture::new(None).await;
    let repos = fx.repos();
    let second = create_group(repos, &fx.owner, &fx.group.agent_id, None).await;
    attach_coach(repos, &second, &fx.coach).await;
    join(repos, &second, &fx.subject, GroupRole::Member, true).await;
    let reopened = open_thread(&fx.res, &fx.coach, &fx.subject, &second).await;
    assert_eq!(
        reopened, fx.thread,
        "premise: one thread per coach and athlete"
    );
    fx.athlete_season().await;
    fx.coach_drafts_this_week().await;
    let first_before = fx.room().await.len();

    let (status, body) = fx.publish().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        fx.room().await.len(),
        first_before,
        "the group the thread was rebound away from hears nothing"
    );
    let second_room = repos
        .groups
        .list_room_transcript_for(&second.id.to_string(), fx.coach.id, None, 500)
        .await
        .unwrap();
    assert_eq!(
        second_room.len(),
        1,
        "the group the thread was last opened from gets the line"
    );
}

// ── No forwarding (Phil's D4) ────────────────────────────────────────────

#[tokio::test]
async fn only_the_publish_line_reaches_a_chat_bound_room() {
    let fx = Fixture::new(Some(("telegram", CHAT_ID))).await;
    fx.athlete_season().await;
    let entry = room_entry(
        fx.repos(),
        &fx.group,
        &fx.subject,
        TranscriptSpeaker::Member,
        ATHLETE_WORDS,
    )
    .await;
    let before = fx.room().await.len();

    for body in [
        json!({ "content": SECRET }),
        json!({ "content": COACH_ASK, "quote_entry_id": entry }),
    ] {
        let (status, reply) = fx.send(&fx.thread, &fx.coach, body).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
    }
    assert!(
        fx.poster.posts().is_empty(),
        "a work-thread turn sends nothing to the room's chat"
    );
    assert_eq!(fx.room().await.len(), before, "nor to its transcript");
    assert!(
        fx.repos()
            .messaging
            .get_session_by_pierre_conversation_id(fx.coach.tenant, &fx.thread)
            .await
            .unwrap()
            .is_none(),
        "the work thread is bound to no messaging session"
    );

    fx.coach_drafts_this_week().await;
    let (status, body) = fx.publish().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let posts = fx.poster.posts();
    assert_eq!(posts.len(), 1, "the publish line, once");
    let (tenant, channel, chat_id, text) = &posts[0];
    assert_eq!(
        tenant.to_string(),
        fx.group.tenant_id,
        "under the group's tenant"
    );
    assert_eq!(
        *channel,
        ChannelType::Telegram,
        "posted on the group's channel"
    );
    assert_eq!(chat_id, CHAT_ID, "into the group's chat");
    assert!(
        text.starts_with(&format!("{COACH_NAME} published an update to your plan: ")),
        "{text}"
    );
    assert_eq!(
        contents(&fx.room().await[..1]),
        vec![text.clone()],
        "the chat reads what the room reads"
    );
    for private in [SECRET, COACH_ASK, ATHLETE_WORDS, ANSWER] {
        assert!(!text.contains(private), "{private:?} reached the chat");
    }
}
