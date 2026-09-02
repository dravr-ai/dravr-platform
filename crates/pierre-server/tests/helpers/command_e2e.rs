// ABOUTME: Reusable e2e fixture for slash commands and guided walks — real webhook → ingress → pipeline → mock LLM
// ABOUTME: Builds linked members with own tenants, binds supergroups, posts DM/room turns, reads ledger and DB state
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The canonical fixture for driving a slash command or a guided-walk turn
//! through the REAL messaging path: a signed Telegram webhook into
//! `MessagingRoutes`, through ingress (ambient gate, slash dispatch, session
//! resolution) and the chat pipeline, against a deterministic routing mock at
//! the LLM seam. Nothing here shortcuts the wire — the handler-level fixtures
//! (`room_guided_walk_test`) prove component contracts; this proves the path.
//!
//! Wired the way PRODUCTION wires a provider: `chat_provider` set,
//! `llm_provider` None (`create_test_server_resources_with_chat_provider`) —
//! background fact extraction reads only `chat_provider` and silently skips
//! without it, which is exactly the inert-feature trap the `with_llm` helper
//! documents.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::Utc;
use futures_util::stream;
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk, TokenUsage,
};
use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{ConnectionType, OnboardingState, Tenant, TenantId, User, UserStatus};
use pierre_database::backends::factory::Database;
use pierre_database::backends::{
    CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use pierre_memory::{FactSource, UserFact};
use pierre_messaging::channels::telegram::transport::TelegramTransport;
use serde_json::{json, Value};
use tokio::task::spawn_blocking;
use tokio::time::sleep;
use uuid::Uuid;

use super::axum_test::AxumTestRequest;

/// Numeric bot id encoded in the fixture bot token's prefix.
pub const BOT_ID: i64 = 54_321;
/// Fixture bot token — canot derives the bot id from the prefix before `:`.
pub const BOT_TOKEN: &str = "54321:COMMAND_E2E_BOT";
/// Bot username seeded into canot's process-wide identity cache, so mention
/// detection never issues a live `getMe` from the suite.
pub const BOT_USERNAME: &str = "dravr_command_e2e_bot";
/// Webhook secret the fixture channel config registers and every post signs.
pub const TG_SECRET: &str = "command_e2e_tg_secret";

/// System-prompt marker of a background fact-extraction request
/// (`PROVENANCE_ADDENDUM`, `pierre-services/src/memory_extraction.rs`).
const EXTRACTION_MARKER: &str = "## Provenance (required)";
/// System-prompt marker of a playbook advice-capture request
/// (`ADVICE_EXTRACTION_PROMPT`, `pierre-services/src/advice_capture.rs`).
const ADVICE_MARKER: &str = "You analyze a fitness coaching exchange";
/// System-prompt marker of the Layer-5 claim judge
/// (`CLAIM_JUDGE_SYSTEM_PROMPT`, `pierre-evals/src/judge.rs`).
const JUDGE_MARKER: &str = "You are the final-stage fact checker";

/// Deterministic routing mock for the ONE shared `chat_provider` seam.
///
/// That seam serves four consumers — the coaching turn, background fact
/// extraction, playbook advice capture, and the Layer-5 claim judge — so a
/// mock that answers them all with one reply corrupts every count and every
/// queue. Requests are classified by the background prompts' own markers;
/// only unmatched requests are coaching turns.
///
/// Mock is test-only (CLAUDE.md): the LLM boundary is the assertion point
/// itself here; the realistic counterpart is the live chat-eval lane.
pub struct RouterLlm {
    model: String,
    /// Coaching-turn replies, popped in order; the last one repeats. Keep
    /// them under 40 characters and free of advice cues ("commence",
    /// "essaie", "cette semaine"…) so `looks_like_recommendation` never
    /// routes a turn reply into advice capture mid-test.
    turn_replies: Mutex<Vec<String>>,
    /// Content-routed extraction replies: the first rule whose needle occurs
    /// in the request text wins; unmatched extractions answer `[]`. Routing
    /// by content rather than order is what makes the queue immune to the
    /// background workers' scheduling.
    extraction_rules: Mutex<Vec<(String, String)>>,
    pub turn_calls: Arc<AtomicUsize>,
    pub extraction_calls: Arc<AtomicUsize>,
    pub background_calls: Arc<AtomicUsize>,
    pub seen_turn_requests: Arc<Mutex<Vec<String>>>,
}

impl RouterLlm {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            model: "mock-model".to_owned(),
            turn_replies: Mutex::new(vec!["OK.".to_owned()]),
            extraction_rules: Mutex::new(Vec::new()),
            turn_calls: Arc::new(AtomicUsize::new(0)),
            extraction_calls: Arc::new(AtomicUsize::new(0)),
            background_calls: Arc::new(AtomicUsize::new(0)),
            seen_turn_requests: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Replace the coaching-turn reply script (popped in order, last repeats).
    pub fn set_turn_replies(&self, replies: &[&str]) {
        *self.turn_replies.lock().unwrap() = replies.iter().map(|r| (*r).to_owned()).collect();
    }

    /// When an extraction request's text contains `needle`, answer with
    /// `json_array` (the bare `RawFact` array `parse_raw_facts` expects).
    pub fn extraction_rule(&self, needle: &str, json_array: &str) {
        self.extraction_rules
            .lock()
            .unwrap()
            .push((needle.to_owned(), json_array.to_owned()));
    }

    fn reply_for(&self, request: &ChatRequest) -> String {
        let text = serde_json::to_string(&request.messages).unwrap_or_default();
        if text.contains(EXTRACTION_MARKER) {
            self.extraction_calls.fetch_add(1, Ordering::SeqCst);
            let rules = self.extraction_rules.lock().unwrap();
            return rules
                .iter()
                .find(|(needle, _)| text.contains(needle.as_str()))
                .map_or_else(|| "[]".to_owned(), |(_, reply)| reply.clone());
        }
        if text.contains(ADVICE_MARKER) {
            self.background_calls.fetch_add(1, Ordering::SeqCst);
            return "[]".to_owned();
        }
        if text.contains(JUDGE_MARKER) {
            self.background_calls.fetch_add(1, Ordering::SeqCst);
            return r#"{"verdict":"unverifiable","confidence":0.2,"rationale":"mock judge"}"#
                .to_owned();
        }
        self.turn_calls.fetch_add(1, Ordering::SeqCst);
        self.seen_turn_requests.lock().unwrap().push(text);
        let mut replies = self.turn_replies.lock().unwrap();
        if replies.len() > 1 {
            replies.remove(0)
        } else {
            replies.first().cloned().unwrap_or_else(|| "OK.".to_owned())
        }
    }
}

#[async_trait]
impl LlmProvider for RouterLlm {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn display_name(&self) -> &'static str {
        "Routing mock LLM (command e2e)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::STREAMING
            | LlmCapabilities::FUNCTION_CALLING
            | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &str {
        &self.model
    }
    fn available_models(&self) -> &[String] {
        &[]
    }
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: self.reply_for(request),
            model: self.model.clone(),
            usage: Some(TokenUsage::new(42, 11, 53)),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: self.reply_for(request),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// One linked human: an Active user owning their own tenant, channel-linked
/// under the bot tenant.
#[derive(Clone)]
pub struct Member {
    pub user_id: Uuid,
    pub home_tenant: TenantId,
    pub channel_user_id: String,
}

/// The base fixture: server with production LLM wiring, a bot tenant with a
/// Telegram channel config, and the seeded bot-identity cache.
pub struct CommandE2e {
    pub resources: Arc<ServerContext>,
    pub llm: Arc<RouterLlm>,
    pub bot_tenant: TenantId,
    update_id: AtomicI64,
    sender_seq: AtomicI64,
}

/// `CommandE2e` plus one coaching group bound to a Telegram supergroup.
pub struct RoomE2e {
    pub base: Arc<CommandE2e>,
    pub group_id: Uuid,
    pub chat_id: i64,
}

/// The webhook ACK the ingress returns.
pub struct WebhookOutcome {
    pub status: StatusCode,
    pub body: Value,
}

impl WebhookOutcome {
    /// The ingress recognition proxy: a slash command handled synchronously
    /// stores no pipeline message.
    pub fn messages_stored(&self) -> i64 {
        self.body
            .get("messages_stored")
            .and_then(Value::as_i64)
            .unwrap_or(-1)
    }
}

/// The repo-root `commands/` catalog directory, from the crate manifest.
pub fn commands_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("commands"))
        .expect("repo root resolves from CARGO_MANIFEST_DIR")
}

async fn create_user_with_own_tenant(resources: &ServerContext, email: &str) -> (Uuid, TenantId) {
    let password_hash =
        spawn_blocking(|| bcrypt::hash("CommandE2e123!", bcrypt::DEFAULT_COST).unwrap())
            .await
            .unwrap();
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Command E2e".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Own Tenant {email}"),
        slug: format!("cmd-e2e-{tenant_id}"),
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
    (user_id, tenant_id)
}

impl CommandE2e {
    /// Wrap a pre-built server into the fixture: register the bot tenant,
    /// its Telegram channel config, and seed canot's bot-identity cache for
    /// mention detection.
    ///
    /// `resources` MUST come from
    /// `common::create_test_server_resources_with_chat_provider(llm)` — the
    /// production wiring (`chat_provider` set, `llm_provider` None). The
    /// `with_llm` factory starves background extraction silently, the
    /// documented inert-feature trap. The harness cannot build the server
    /// itself: `helpers/` compiles into every test binary, and `common` only
    /// exists in the binaries that declare it.
    pub async fn start(resources: Arc<ServerContext>, llm: Arc<RouterLlm>) -> Arc<Self> {
        let _seed =
            TelegramTransport::with_bot_identity("unused-secret".to_owned(), BOT_ID, BOT_USERNAME);

        let (_bot_owner, bot_tenant) = create_user_with_own_tenant(
            &resources,
            &format!("cmd_e2e_bot_{}@example.com", Uuid::new_v4()),
        )
        .await;

        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(TG_SECRET),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some(BOT_TOKEN),
            is_active: true,
        })
        .await
        .unwrap();

        Arc::new(Self {
            resources,
            llm,
            bot_tenant,
            update_id: AtomicI64::new(1_000),
            sender_seq: AtomicI64::new(7_000),
        })
    }

    /// A fresh linked member: own tenant, telegram link under the bot tenant,
    /// and (optionally) a synthetic provider connection so load snapshots and
    /// prefetches have a data source to name.
    pub async fn linked_member(&self, with_provider: bool) -> Member {
        let seq = self.sender_seq.fetch_add(1, Ordering::SeqCst);
        let email = format!("cmd_e2e_member_{seq}_{}@example.com", Uuid::new_v4());
        let (user_id, home_tenant) = create_user_with_own_tenant(&self.resources, &email).await;
        if with_provider {
            self.resources
                .common
                .repos
                .provider_connections
                .register_connection(
                    user_id,
                    home_tenant,
                    "synthetic",
                    &ConnectionType::Synthetic,
                    None,
                )
                .await
                .unwrap();
        }
        let channel_user_id = seq.to_string();
        let db: &dyn MessagingRepository = &*self.resources.common.repos.messaging;
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: self.bot_tenant,
            user_id: &user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: &channel_user_id,
            display_name: Some("Command E2e Member"),
        })
        .await
        .unwrap();
        Member {
            user_id,
            home_tenant,
            channel_user_id,
        }
    }

    async fn post_update(&self, message: Value) -> WebhookOutcome {
        let update_id = self.update_id.fetch_add(1, Ordering::SeqCst);
        let router = MessagingRoutes::routes(Arc::clone(&self.resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", TG_SECRET)
            .json(&json!({ "update_id": update_id, "message": message }))
            .send(router)
            .await;
        let status = resp.status_code();
        let body: Value = resp.json();
        assert_eq!(status, StatusCode::OK, "webhook rejected: {body}");
        WebhookOutcome { status, body }
    }

    /// A signed Telegram DM turn from `m` (private chat, chat id == sender).
    pub async fn send_dm(&self, m: &Member, text: &str) -> WebhookOutcome {
        let message_id = self.update_id.fetch_add(1, Ordering::SeqCst);
        let sender: i64 = m.channel_user_id.parse().unwrap();
        self.post_update(json!({
            "message_id": message_id,
            "from": { "id": sender, "first_name": "Member" },
            "chat": { "id": sender, "type": "private" },
            "text": text
        }))
        .await
    }

    /// The member's messaging session id under `tenant` for `chat_id`
    /// (DM: chat id == sender, tenant == their own).
    pub async fn session_id(&self, m: &Member, tenant: TenantId, chat_id: &str) -> Option<String> {
        self.resources
            .common
            .repos
            .messaging
            .get_session_by_channel_identity(tenant, "telegram", &m.channel_user_id, Some(chat_id))
            .await
            .ok()
            .flatten()
            .and_then(|s| s["id"].as_str().map(str::to_owned))
    }

    /// The conversation id the member's session under `tenant`/`chat_id`
    /// points at, when that session exists.
    pub async fn conversation_id(
        &self,
        m: &Member,
        tenant: TenantId,
        chat_id: &str,
    ) -> Option<String> {
        let session = self
            .resources
            .common
            .repos
            .messaging
            .get_session_by_channel_identity(tenant, "telegram", &m.channel_user_id, Some(chat_id))
            .await
            .ok()
            .flatten()?;
        session["pierre_conversation_id"]
            .as_str()
            .map(str::to_owned)
    }

    /// The guided-flow state on the conversation the member's session under
    /// `tenant`/`chat_id` points at. `None` when no session, no conversation,
    /// or no active walk.
    pub async fn onboarding_state(
        &self,
        m: &Member,
        tenant: TenantId,
        chat_id: &str,
    ) -> Option<OnboardingState> {
        let conversation_id = self.conversation_id(m, tenant, chat_id).await?;
        let conv = self
            .resources
            .common
            .repos
            .chat
            .get_conversation(&conversation_id, &m.user_id.to_string(), tenant)
            .await
            .ok()
            .flatten()?;
        OnboardingState::from_column(conv.onboarding_state.as_deref())
    }

    /// Facts for `m` by source under `tenant`, right now.
    pub async fn facts_now(
        &self,
        tenant: TenantId,
        m: &Member,
        source: FactSource,
    ) -> Vec<UserFact> {
        self.resources
            .common
            .repos
            .memory
            .list_user_facts_by_source(tenant, &m.user_id.to_string(), source, 100)
            .await
            .unwrap_or_default()
    }

    /// Poll (≤30s) until at least `at_least` facts of `source` exist for `m`
    /// under `tenant`; panics with the count on timeout.
    pub async fn wait_facts(
        &self,
        tenant: TenantId,
        m: &Member,
        source: FactSource,
        at_least: usize,
    ) -> Vec<UserFact> {
        for _ in 0..300 {
            let facts = self.facts_now(tenant, m, source).await;
            if facts.len() >= at_least {
                return facts;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let facts = self.facts_now(tenant, m, source).await;
        panic!(
            "expected >= {at_least} {source:?} facts under {tenant}, found {}",
            facts.len()
        );
    }

    /// Poll (≤30s) until at least `want` non-empty outbound ledger rows exist
    /// for `session_id`; the delivery task persists them asynchronously after
    /// the channel send fails on test credentials.
    pub async fn wait_outbound_for_session(&self, session_id: &str, want: i64) -> i64 {
        for _ in 0..300 {
            let n = self.outbound_count_for_session(session_id).await;
            if n >= want {
                return n;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let n = self.outbound_count_for_session(session_id).await;
        panic!("expected >= {want} outbound ledger rows for session {session_id}, found {n}");
    }

    /// Non-empty outbound ledger rows for `session_id`, right now.
    pub async fn outbound_count_for_session(&self, session_id: &str) -> i64 {
        const SQL: &str = "SELECT COUNT(*) FROM messaging_messages \
             WHERE direction = 'outbound' AND session_id = $1 \
               AND content_body IS NOT NULL AND content_body != ''";
        match self.resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_scalar(SQL)
                .bind(session_id)
                .fetch_one(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                .bind(session_id)
                .fetch_one(db.pool())
                .await
                .unwrap(),
        }
    }

    /// Non-empty outbound ledger bodies for `session_id`, oldest first — the
    /// record of what the athlete was told, read back verbatim.
    pub async fn outbound_bodies_for_session(&self, session_id: &str) -> Vec<String> {
        const SQL: &str = "SELECT content_body FROM messaging_messages \
             WHERE direction = 'outbound' AND session_id = $1 \
               AND content_body IS NOT NULL AND content_body != '' \
             ORDER BY created_at ASC";
        match self.resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_scalar(SQL)
                .bind(session_id)
                .fetch_all(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                .bind(session_id)
                .fetch_all(db.pool())
                .await
                .unwrap(),
        }
    }

    /// Poll (≤30s) until `want` outbound ledger rows carry `needle`.
    pub async fn wait_outbound_containing(&self, needle: &str, want: i64) -> i64 {
        for _ in 0..300 {
            let n = self.outbound_count_containing(needle).await;
            if n >= want {
                return n;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let n = self.outbound_count_containing(needle).await;
        panic!("expected >= {want} outbound rows carrying {needle:?}, found {n}");
    }

    /// Outbound ledger rows carrying `needle`, right now.
    pub async fn outbound_count_containing(&self, needle: &str) -> i64 {
        const SQL: &str = "SELECT COUNT(*) FROM messaging_messages \
             WHERE direction = 'outbound' AND content_body LIKE '%' || $1 || '%'";
        match self.resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_scalar(SQL)
                .bind(needle)
                .fetch_one(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                .bind(needle)
                .fetch_one(db.pool())
                .await
                .unwrap(),
        }
    }

    /// Inbound ledger rows whose body is exactly `body` (ambient capture).
    pub async fn count_inbound_with_body(&self, body: &str) -> i64 {
        const SQL: &str = "SELECT COUNT(*) FROM messaging_messages \
             WHERE direction = 'inbound' AND content_body = $1";
        match self.resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_scalar(SQL)
                .bind(body)
                .fetch_one(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                .bind(body)
                .fetch_one(db.pool())
                .await
                .unwrap(),
        }
    }

    /// Poll (≤10s) until the coaching-turn call count reaches `at_least`.
    pub async fn wait_llm_turns(&self, at_least: usize) -> bool {
        for _ in 0..50 {
            if self.llm.turn_calls.load(Ordering::SeqCst) >= at_least {
                return true;
            }
            sleep(Duration::from_millis(200)).await;
        }
        false
    }
}

impl RoomE2e {
    /// Bind a fresh supergroup: a system coach + a `CoachingGroup` pre-bound
    /// to `chat_id` under the bot tenant in `mode`, with `owner` enrolled.
    pub async fn bind_room(
        base: Arc<CommandE2e>,
        chat_id: i64,
        mode: GroupRespondMode,
        owner: &Member,
    ) -> Self {
        let coach = base
            .resources
            .common
            .repos
            .coaches
            .create_system_coach(
                owner.user_id,
                base.bot_tenant,
                &CreateSystemCoachRequest {
                    title: "Command E2e Coach".to_owned(),
                    description: None,
                    system_prompt: "You are a concise test coach.".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap();

        let group_id = Uuid::new_v4();
        let now = Utc::now();
        let group = CoachingGroup {
            id: group_id,
            tenant_id: base.bot_tenant.to_string(),
            name: format!("Command E2e Group {chat_id}"),
            description: None,
            coach_id: coach.id.to_string(),
            owner_id: owner.user_id,
            coach_user_id: None,
            peer_data_sharing: true,
            respond_mode: mode,
            max_members: 10,
            is_active: true,
            channel_type: Some("telegram".to_owned()),
            channel_chat_id: Some(chat_id.to_string()),
            created_at: now,
            updated_at: now,
        };
        base.resources
            .common
            .repos
            .groups
            .create_group(base.bot_tenant, &group)
            .await
            .unwrap();

        let room = Self {
            base,
            group_id,
            chat_id,
        };
        room.add_member(owner, GroupRole::Owner).await;
        room
    }

    pub async fn add_member(&self, m: &Member, role: GroupRole) {
        let now = Utc::now();
        self.base
            .resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: self.group_id,
                user_id: m.user_id,
                tenant_id: self.base.bot_tenant.to_string(),
                role,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    fn group_message(&self, m: &Member, message_id: i64, text: &str) -> Value {
        let sender: i64 = m.channel_user_id.parse().unwrap();
        json!({
            "message_id": message_id,
            "from": { "id": sender, "first_name": "Member" },
            "chat": {
                "id": self.chat_id,
                "type": "supergroup",
                "title": "Command E2e Group"
            },
            "text": text
        })
    }

    /// An UNADDRESSED room message — plain supergroup text, no mention.
    pub async fn send_room(&self, m: &Member, text: &str) -> WebhookOutcome {
        let message_id = self.base.update_id.fetch_add(1, Ordering::SeqCst);
        self.base
            .post_update(self.group_message(m, message_id, text))
            .await
    }

    /// An ADDRESSED room message: `@bot_username text`.
    pub async fn send_room_addressed(&self, m: &Member, text: &str) -> WebhookOutcome {
        self.send_room(m, &format!("@{BOT_USERNAME} {text}")).await
    }

    /// A slash command in the room. Slash commands bypass the ambient gate by
    /// design, so no mention is needed.
    pub async fn send_room_slash(&self, m: &Member, command: &str) -> WebhookOutcome {
        self.send_room(m, command).await
    }

    /// The member's ROOM session id (bot tenant, this chat).
    pub async fn session_id(&self, m: &Member) -> Option<String> {
        self.base
            .session_id(m, self.base.bot_tenant, &self.chat_id.to_string())
            .await
    }

    /// The guided-flow state on the member's own room conversation.
    pub async fn onboarding_state(&self, m: &Member) -> Option<OnboardingState> {
        self.base
            .onboarding_state(m, self.base.bot_tenant, &self.chat_id.to_string())
            .await
    }

    /// The member's own room conversation id, when their session exists.
    pub async fn conversation_id(&self, m: &Member) -> Option<String> {
        self.base
            .conversation_id(m, self.base.bot_tenant, &self.chat_id.to_string())
            .await
    }

    /// `chat_messages` rows in `conversation_id` whose content carries
    /// `needle` — the room-transcript persistence contract.
    pub async fn chat_rows_carrying(&self, conversation_id: &str, needle: &str) -> i64 {
        const SQL: &str = "SELECT COUNT(*) FROM chat_messages \
             WHERE conversation_id = $1 AND content LIKE '%' || $2 || '%'";
        match self.base.resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_scalar(SQL)
                .bind(conversation_id)
                .bind(needle)
                .fetch_one(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                .bind(conversation_id)
                .bind(needle)
                .fetch_one(db.pool())
                .await
                .unwrap(),
        }
    }
}
