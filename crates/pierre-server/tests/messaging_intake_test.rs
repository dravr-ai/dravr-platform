// ABOUTME: A messaging athlete must be asked profile type and the PAR-Q+, verbatim and without the model
// ABOUTME: Drives the whole walk over real Telegram webhook turns and asserts what actually landed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

//! Web and mobile ask profile type and the PAR-Q+ on real wizard steps. A
//! messaging athlete reached neither: the pillar walk covers who they are, and
//! nothing covered whether it is safe for them to train.
//!
//! The instrument is the thing under test. A paraphrased PAR-Q+ with an
//! inferred yes/no is a different screen, so these tests pin that the model is
//! never consulted during the walk — `calls` stays at the one turn that opened
//! it — and that each answer lands where its web equivalent lands.

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod intake_tests {
    use crate::common::create_test_server_resources_with_llm;
    use crate::helpers::axum_test::AxumTestRequest;
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use chrono::Utc;
    use futures_util::stream;
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
        TokenUsage,
    };
    use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::factory::Database;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use pierre_services::intake::{parse_persona, parse_yes_no, PersonaAnswer};
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    const TG_SECRET: &str = "intake_tg_secret";

    /// Bcrypt cost the fixture hashes its athletes' passwords at.
    ///
    /// No athlete here authenticates by password — every turn arrives as a
    /// signed Telegram webhook against a channel link — so the hash only has to
    /// be a well-formed bcrypt digest. At the test profile's opt-level
    /// `bcrypt::DEFAULT_COST` (12) costs about a second per athlete, and every
    /// test in this file is `#[serial]`.
    const FIXTURE_BCRYPT_COST: u32 = 4;

    /// Poll cadence for the `wait_for_*` helpers below.
    ///
    /// The deadline each helper enforces is `POLL_TICKS * POLL_INTERVAL` —
    /// twelve seconds, unchanged. A shorter tick does not shorten that
    /// deadline; it shortens how long a helper keeps sleeping after the
    /// condition it waits on has already become true.
    const POLL_INTERVAL: Duration = Duration::from_millis(50);
    /// Ticks before a `wait_for_*` helper gives up — 12s at [`POLL_INTERVAL`].
    const POLL_TICKS: usize = 240;

    /// Deterministic offline LLM. The counter is the assertion that matters
    /// here: every intake turn that reached it would be a paraphrased question.
    struct MockLlm {
        calls: Arc<AtomicUsize>,
        /// How long the model "thinks" before answering.
        ///
        /// Zero for every test but the ordering one, which needs the served turn
        /// to take measurable time: whether the intake opens before or after the
        /// answer is only observable while the turn is still in flight.
        delay: Duration,
        /// Every message body handed to the model, across every request.
        ///
        /// The counter alone cannot pin the property under test: background
        /// memory extraction is an LLM call too, so a rising count says nothing
        /// about whether a PAR-Q question was paraphrased. What the transcript
        /// contains does.
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl MockLlm {
        fn new() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                seen: Arc::new(Mutex::new(Vec::new())),
                delay: Duration::ZERO,
            }
        }
        /// A model that takes `delay` to answer, so a test can sample the world
        /// while the turn is still running.
        fn slow(delay: Duration) -> Self {
            Self {
                delay,
                ..Self::new()
            }
        }
        fn counter(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }
        fn transcript(&self) -> Arc<Mutex<Vec<String>>> {
            Arc::clone(&self.seen)
        }
        fn record(&self, request: &ChatRequest) {
            let mut seen = self.seen.lock().unwrap();
            for m in &request.messages {
                seen.push(m.content.clone());
            }
        }
    }

    /// Every PAR-Q+ question, in the locale the fixture runs in. If any of these
    /// reaches the model, the instrument was paraphrased rather than asked.
    const PARQ_FRAGMENTS: [&str; 7] = [
        "problème cardiaque",
        "douleurs à la poitrine",
        "étourdissements",
        "maladie chronique",
        "médicaments prescrits",
        "articulaire",
        "supervision médicale",
    ];

    /// Assert the model never saw a PAR-Q question.
    fn assert_instrument_never_paraphrased(seen: &Arc<Mutex<Vec<String>>>) {
        let seen = seen.lock().unwrap();
        for body in seen.iter() {
            for fragment in PARQ_FRAGMENTS {
                assert!(
                    !body.contains(fragment),
                    "a PAR-Q question reached the model ({fragment:?}) — the platform must ask it verbatim"
                );
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlm {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn display_name(&self) -> &'static str {
            "Mock LLM (tests)"
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
            sleep(self.delay).await;
            self.record(request);
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ChatResponse {
                content: "Bien reçu.".to_owned(),
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage::new(10, 3, 13)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }
        async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
            sleep(self.delay).await;
            self.record(request);
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(stream::iter(vec![Ok(StreamChunk {
                delta: "Bien reçu.".to_owned(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            })])))
        }
        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    async fn create_user_with_own_tenant(
        resources: &ServerContext,
        email: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("Intake123!", FIXTURE_BCRYPT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Intake Athlete".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Intake Tenant {email}"),
            slug: format!("intake-tenant-{tenant_id}"),
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

    /// Register the bot config and link a channel sender to the athlete.
    async fn link_channel(
        resources: &ServerContext,
        tenant_id: TenantId,
        user_id: Uuid,
        sender: &str,
    ) {
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
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
            bot_token: Some("12345:INTAKE_BOT"),
            is_active: true,
        })
        .await
        .unwrap();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: sender,
            display_name: Some("Intake Sender"),
        })
        .await
        .unwrap();
    }

    /// Drive one inbound Telegram message through the real webhook.
    async fn send_turn(
        resources: &Arc<ServerContext>,
        update_id: u64,
        sender: u64,
        text: &str,
        group: bool,
    ) {
        let chat_type = if group { "supergroup" } else { "private" };
        let router = MessagingRoutes::routes(Arc::clone(resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", TG_SECRET)
            .json(&json!({
                "update_id": update_id,
                "message": {
                    "message_id": update_id,
                    "from": { "id": sender, "first_name": "Intake" },
                    "chat": { "id": sender, "type": chat_type },
                    "text": text
                }
            }))
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
    }

    /// Wait until at least `n` turns have reached the LLM.
    async fn wait_for_turns(calls: &Arc<AtomicUsize>, n: usize) -> bool {
        for _ in 0..POLL_TICKS {
            if calls.load(Ordering::SeqCst) >= n {
                return true;
            }
            sleep(POLL_INTERVAL).await;
        }
        false
    }

    /// Poll until the conversation's intake ledger holds `n` delivered questions.
    ///
    /// The opening question rides behind a served turn, which completes
    /// asynchronously, so the ledger is the only honest "the question went out"
    /// signal available to the caller.
    async fn wait_for_probes(resources: &ServerContext, user_id: Uuid, n: usize) -> bool {
        for _ in 0..POLL_TICKS {
            if probed_count(resources, user_id).await >= n {
                return true;
            }
            sleep(POLL_INTERVAL).await;
        }
        false
    }

    /// Count assistant rows on the athlete's messaging conversation.
    ///
    /// The coach's answer is persisted before `deliver_reply` sends it, so this
    /// is the observable "the turn was served" signal a test can order the
    /// intake against.
    /// Poll until `fragment` appears in the model transcript.
    ///
    /// The call COUNTER cannot stand in for this: background memory extraction
    /// is an LLM call too, so a rising count says nothing about WHICH message
    /// reached the model — the same reason `assert_instrument_never_paraphrased`
    /// reads the transcript rather than the counter.
    async fn wait_for_transcript(seen: &Arc<Mutex<Vec<String>>>, fragment: &str) -> bool {
        for _ in 0..POLL_TICKS {
            if seen
                .lock()
                .unwrap()
                .iter()
                .any(|body| body.contains(fragment))
            {
                return true;
            }
            sleep(POLL_INTERVAL).await;
        }
        false
    }

    /// Poll until `step` is recorded with `status`.
    ///
    /// The retirement now runs in the bottom-of-turn hook, AFTER the coach's
    /// reply is delivered, so a test that reads the steps the moment the model
    /// was called reads them before they are written.
    async fn wait_for_step(
        resources: &ServerContext,
        user_id: Uuid,
        step: &str,
        status: &str,
    ) -> bool {
        for _ in 0..POLL_TICKS {
            if onboarding_steps(resources, user_id)
                .await
                .contains(&(step.to_owned(), status.to_owned()))
            {
                return true;
            }
            sleep(POLL_INTERVAL).await;
        }
        false
    }

    async fn assistant_message_count(resources: &ServerContext, user_id: Uuid) -> usize {
        // `messaging_sessions.user_id` is a `uuid` column on PostgreSQL and
        // text on SQLite; the cast lets one bound text id serve both.
        const SQL: &str = "SELECT COUNT(*) FROM chat_messages m \
             JOIN messaging_sessions s ON s.pierre_conversation_id = m.conversation_id \
             WHERE CAST(s.user_id AS TEXT) = $1 AND m.role = 'assistant'";
        let user = user_id.to_string();
        let count: i64 = match resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_scalar(SQL)
                .bind(&user)
                .fetch_one(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_scalar(SQL)
                .bind(&user)
                .fetch_one(db.pool())
                .await
                .unwrap(),
        };
        usize::try_from(count).unwrap_or(0)
    }

    /// The raw `onboarding_state` JSON of the conversation the user's
    /// messaging session points at, on whichever backend the test database is.
    async fn onboarding_state(resources: &ServerContext, user_id: Uuid) -> Option<String> {
        const SQL: &str = "SELECT c.onboarding_state FROM chat_conversations c \
             JOIN messaging_sessions s ON s.pierre_conversation_id = c.id \
             WHERE CAST(s.user_id AS TEXT) = $1";
        let user = user_id.to_string();
        let row: Option<(Option<String>,)> = match resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_as(SQL)
                .bind(&user)
                .fetch_optional(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_as(SQL)
                .bind(&user)
                .fetch_optional(db.pool())
                .await
                .unwrap(),
        };
        row.and_then(|(state,)| state)
    }

    async fn probed_count(resources: &ServerContext, user_id: Uuid) -> usize {
        let Some(raw) = onboarding_state(resources, user_id).await else {
            return 0;
        };
        let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return 0;
        };
        // Only the intake's own ledger counts. The pillar walk shares this
        // column and records its delivered probes here too, so a walk that
        // asked one question would otherwise read as an intake in progress.
        if state.get("flow").and_then(serde_json::Value::as_str) != Some("intake") {
            return 0;
        }
        state
            .get("probed")
            .and_then(|p| p.as_array().map(Vec::len))
            .unwrap_or(0)
    }

    /// The guided flow the conversation currently carries, if any is active.
    async fn active_flow(resources: &ServerContext, user_id: Uuid) -> Option<String> {
        let raw = onboarding_state(resources, user_id).await?;
        let state = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
        if state.get("active").and_then(serde_json::Value::as_bool) != Some(true) {
            return None;
        }
        state
            .get("flow")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }

    /// Poll until the conversation carries `flow`.
    async fn wait_for_flow(resources: &ServerContext, user_id: Uuid, flow: &str) -> bool {
        for _ in 0..POLL_TICKS {
            if active_flow(resources, user_id).await.as_deref() == Some(flow) {
                return true;
            }
            sleep(POLL_INTERVAL).await;
        }
        false
    }

    /// `(step_id, status)` of every recorded onboarding step, ordered by step.
    async fn onboarding_steps(resources: &ServerContext, user_id: Uuid) -> Vec<(String, String)> {
        let mut steps: Vec<(String, String)> = resources
            .common
            .repos
            .user_onboarding
            .get_onboarding_steps(&user_id.to_string())
            .await
            .unwrap()
            .into_iter()
            .map(|step| (step.step_id, step.status))
            .collect();
        steps.sort();
        steps
    }

    async fn medical_facts(resources: &ServerContext, user_id: Uuid) -> Vec<String> {
        const SQL: &str = "SELECT object FROM user_facts WHERE user_id = $1 AND kind = 'medical'";
        let user = user_id.to_string();
        let rows: Vec<(String,)> = match resources.coach.database.as_ref() {
            Database::SQLite(db) => sqlx::query_as(SQL)
                .bind(&user)
                .fetch_all(db.pool())
                .await
                .unwrap(),
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(db) => sqlx::query_as(SQL)
                .bind(&user)
                .fetch_all(db.pool())
                .await
                .unwrap(),
        };
        rows.into_iter().map(|r| r.0).collect()
    }

    /// The persona persisted on the user row, as its wire string.
    async fn coaching_persona(resources: &ServerContext, user_id: Uuid) -> String {
        resources
            .common
            .repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .expect("the intake user exists")
            .coaching_persona
            .as_str()
            .to_owned()
    }

    /// The strict parse is the whole safety property: "2" is an answer to a
    /// medical question, "I run 2 times a week" is conversation. A loose parse
    /// would record a clinical "no" from someone who never answered.
    #[test]
    fn a_bare_answer_parses_and_a_sentence_does_not() {
        assert_eq!(parse_yes_no("1"), Some(true));
        assert_eq!(parse_yes_no("2"), Some(false));
        assert_eq!(parse_yes_no("yes"), Some(true));
        assert_eq!(parse_yes_no("Oui"), Some(true));
        assert_eq!(parse_yes_no("non"), Some(false));
        assert_eq!(parse_yes_no("Não"), Some(false));
        assert_eq!(parse_yes_no("nein"), Some(false));
        assert_eq!(parse_yes_no("sí"), Some(true));

        assert_eq!(parse_yes_no("yes but only when I sprint"), None);
        assert_eq!(parse_yes_no("I run 2 times a week"), None);
        assert_eq!(parse_yes_no("no idea honestly"), None);
        assert_eq!(parse_yes_no(""), None);

        assert_eq!(parse_persona("1"), Some(PersonaAnswer::Athlete));
        assert_eq!(parse_persona("2"), Some(PersonaAnswer::Coach));
        assert_eq!(parse_persona("coach"), Some(PersonaAnswer::Coach));
        assert_eq!(parse_persona("I coach a masters squad"), None);
    }

    /// The whole walk, over real webhook turns: one coached turn opens it, then
    /// eight answers land without the model ever seeing a question.
    /// The intake rides BEHIND the served turn, never in front of it.
    ///
    /// `maybe_send_intake_question` used to run at the top of
    /// `dispatch_and_respond`, before the pipeline had started, so the athlete
    /// got the form first and the answer to their actual question second —
    /// pinned below it in the thread, because the reply is delivered by editing
    /// a placeholder opened after the probe. Reported from production Telegram
    /// on 2026-08-28: "I never had a chance to answer the question."
    ///
    /// Three doc comments and the sibling test's comment all claimed the probe
    /// rides behind a served turn. Nothing asserted the ORDER — only that both
    /// eventually happened, which is equally true of the inverted order. That is
    /// how the inversion shipped green.
    ///
    /// Order is only observable WHILE the turn is in flight, so the model is
    /// made slow on purpose and the ledger is sampled mid-turn: the probe must
    /// not have been written yet. Sampling after the turn settles cannot tell
    /// the two orderings apart, and a test that cannot fail on the old code is
    /// not a regression test.
    #[tokio::test]
    #[serial]
    async fn the_intake_opens_behind_the_answer_not_in_front_of_it() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let think = Duration::from_secs(4);
        let mock = MockLlm::slow(think);
        let calls = mock.counter();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "intake_order@example.com").await;
        link_channel(&resources, tenant_id, user_id, "83").await;

        send_turn(
            &resources,
            8301,
            83,
            "j'attaque la tourbiere ce matin, c'est bon?",
            false,
        )
        .await;

        // Mid-turn: the model is still thinking, so nothing has been served yet
        // and the intake must not have spoken. Under the old ordering the probe
        // was already in the ledger microseconds after the webhook returned.
        sleep(think / 4).await;
        assert_eq!(
            probed_count(&resources, user_id).await,
            0,
            "the intake spoke before the coach answered — the athlete is handed a \
             form while the reply they asked for is still being written"
        );

        assert!(
            wait_for_turns(&calls, 1).await,
            "the athlete's question must reach the coach"
        );
        assert!(
            wait_for_probes(&resources, user_id, 1).await,
            "the intake must still open, once the turn has been served"
        );
        assert!(
            assistant_message_count(&resources, user_id).await >= 1,
            "the coach's answer must be durable before the intake opens"
        );
    }

    #[tokio::test]
    #[serial]
    async fn the_intake_walks_to_completion_without_the_model() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let mock = MockLlm::new();
        let calls = mock.counter();
        let seen = mock.transcript();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "intake_walk@example.com").await;
        link_channel(&resources, tenant_id, user_id, "81").await;

        // Turn 1 is ordinary coaching. The intake opens behind its reply.
        send_turn(&resources, 8001, 81, "Salut", false).await;
        assert!(
            wait_for_turns(&calls, 1).await,
            "the athlete's first message must still be answered by the coach"
        );
        assert!(
            wait_for_probes(&resources, user_id, 1).await,
            "the intake must open behind the served turn"
        );

        // Profile type, then the seven PAR-Q+ questions. Exactly one "yes".
        let answers = ["1", "2", "1", "2", "2", "2", "2", "2"];
        for (i, answer) in answers.iter().enumerate() {
            send_turn(&resources, 8002 + i as u64, 81, answer, false).await;
            // Each answer is replied to with the next question, so the ledger
            // grows by one — except the last, which completes and clears it.
            if i + 2 <= answers.len() {
                assert!(
                    wait_for_probes(&resources, user_id, i + 2).await,
                    "answer {} must be followed by the next question",
                    i + 1
                );
            }
        }

        // The walk never handed a question to the model.
        assert_instrument_never_paraphrased(&seen);

        let steps = onboarding_steps(&resources, user_id).await;
        assert!(
            steps.contains(&("parq".to_owned(), "complete".to_owned())),
            "PAR-Q must be recorded complete, got {steps:?}"
        );
        assert!(
            steps.contains(&("profile_type".to_owned(), "complete".to_owned())),
            "profile type must be recorded complete, got {steps:?}"
        );

        let flags = medical_facts(&resources, user_id).await;
        assert_eq!(
            flags.len(),
            1,
            "exactly the one 'yes' may raise a flag, got {flags:?}"
        );
        assert!(
            flags[0].contains("chest"),
            "the flag must carry the question it answers, got {:?}",
            flags[0]
        );

        assert_eq!(
            coaching_persona(&resources, user_id).await,
            "casual",
            "answering 'athlete' writes no persona — Casual IS the athlete default"
        );

        // The intake displaced the pillar walk at conversation creation. A
        // messaging channel holds ONE conversation per athlete, so if the walk
        // did not resume here it would never run at all — only a /reset would
        // ever start it.
        assert!(
            wait_for_flow(&resources, user_id, "pillars").await,
            "the pillar walk must take over once the intake retires, on the same conversation"
        );
    }

    /// "I coach others" is the one profile-type answer with a user-row write.
    #[tokio::test]
    #[serial]
    async fn choosing_coach_sets_the_persona() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let mock = MockLlm::new();
        let calls = mock.counter();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "intake_coach@example.com").await;
        link_channel(&resources, tenant_id, user_id, "82").await;

        send_turn(&resources, 8201, 82, "Bonjour", false).await;
        assert!(wait_for_turns(&calls, 1).await);
        assert!(wait_for_probes(&resources, user_id, 1).await);

        send_turn(&resources, 8202, 82, "2", false).await;
        assert!(
            wait_for_probes(&resources, user_id, 2).await,
            "the persona answer must be followed by the first PAR-Q question"
        );

        assert_eq!(
            coaching_persona(&resources, user_id).await,
            "coach",
            "'I coach others' must persist the coach persona, as the web step does"
        );
    }

    /// Someone who wants to talk gets one re-ask, then the coach takes over.
    /// The re-ask rides BEHIND the coach's answer, never in front of it.
    ///
    /// Production Telegram, 2026-08-28 17:34. With the persona question
    /// outstanding, the athlete asked "Demain j'attaque Alfred Kelly la Moc et
    /// son ascension. C'est bon basé sur ma semaine?" — a real training
    /// question. He got "Désolé — j'ai besoin du chiffre seul" and the form
    /// again. His question was swallowed: `HandledNotStored`, so it reached
    /// neither the transcript nor the model, and no answer was ever written.
    ///
    /// Ordering is only observable WHILE the turn is in flight, so the model is
    /// slow on purpose and the ledger is sampled mid-turn. On the old code
    /// `handle_unparsed` ran synchronously inside the webhook handler, so the
    /// ledger reached 2 before `send_turn` even returned — the sample at t=1s
    /// reads 2 and fails deterministically. After the fix the re-ask cannot fire
    /// before `deliver_reply`, i.e. no earlier than t=4s. Sampling after the
    /// turn settles cannot tell the two apart: both end at 2.
    #[tokio::test]
    #[serial]
    async fn the_re_ask_rides_behind_the_answer_not_in_front_of_it() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let think = Duration::from_secs(4);
        let mock = MockLlm::slow(think);
        let calls = mock.counter();
        let seen = mock.transcript();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "intake_reask@example.com").await;
        link_channel(&resources, tenant_id, user_id, "84").await;

        // Turn 1 arms the walk: the opener goes out behind an ordinary reply.
        send_turn(&resources, 8401, 84, "Salut", false).await;
        assert!(wait_for_turns(&calls, 1).await);
        assert!(wait_for_probes(&resources, user_id, 1).await);

        // Turn 2 is the incident message — a question, not an answer.
        send_turn(
            &resources,
            8402,
            84,
            "Demain j'attaque Alfred Kelly la Moc et son ascension. C'est bon basé sur ma semaine?",
            false,
        )
        .await;

        sleep(think / 4).await;
        assert_eq!(
            probed_count(&resources, user_id).await,
            1,
            "the re-ask went out before the coach answered — the athlete asked a question \
             and was handed the form instead"
        );

        // The question itself must reach the coach. On the old code it was
        // HandledNotStored and never reached the model at all, so this fragment
        // can only appear if the turn genuinely ran.
        assert!(
            wait_for_transcript(&seen, "Alfred Kelly la Moc").await,
            "the athlete's question was swallowed instead of answered"
        );
        assert!(
            wait_for_probes(&resources, user_id, 2).await,
            "and the re-ask must still go out, once the answer is served"
        );
        assert!(
            assistant_message_count(&resources, user_id).await >= 2,
            "the coach's answer must be durable before the re-ask"
        );
        assert_instrument_never_paraphrased(&seen);
    }

    /// A parsed answer still takes the turn — it earns the next question, not a
    /// coaching detour.
    ///
    /// The guard on the other side of the fix: only the UNPARSED case yields.
    /// Passes before and after, so it proves nothing about the change; it exists
    /// so a future "just let everything through" cannot pass unnoticed.
    #[tokio::test]
    #[serial]
    async fn a_parsed_answer_still_replaces_the_turn() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let mock = MockLlm::new();
        let calls = mock.counter();
        let seen = mock.transcript();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "intake_parsed@example.com").await;
        link_channel(&resources, tenant_id, user_id, "85").await;

        send_turn(&resources, 8501, 85, "Salut", false).await;
        assert!(wait_for_turns(&calls, 1).await);
        assert!(wait_for_probes(&resources, user_id, 1).await);
        let served_before = assistant_message_count(&resources, user_id).await;

        // "1" IS the answer, so it must advance the walk and produce no coaching.
        send_turn(&resources, 8502, 85, "1", false).await;
        assert!(
            wait_for_probes(&resources, user_id, 2).await,
            "a parsed answer must be replied to with the next question"
        );
        assert_eq!(
            assistant_message_count(&resources, user_id).await,
            served_before,
            "answering the form must not also spend a coaching turn"
        );
        assert_instrument_never_paraphrased(&seen);
    }

    #[tokio::test]
    #[serial]
    async fn two_unparsed_answers_stand_aside_for_the_coach() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let mock = MockLlm::new();
        let calls = mock.counter();
        let seen = mock.transcript();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "intake_aside@example.com").await;
        link_channel(&resources, tenant_id, user_id, "83").await;

        send_turn(&resources, 8301, 83, "Salut", false).await;
        assert!(wait_for_turns(&calls, 1).await);
        assert!(wait_for_probes(&resources, user_id, 1).await);

        // First non-answer: re-asked, so the ledger records a second delivery.
        send_turn(&resources, 8302, 83, "c'est quoi Dravr ?", false).await;
        assert!(
            wait_for_probes(&resources, user_id, 2).await,
            "an unparsed answer must be re-asked once"
        );
        assert!(
            wait_for_transcript(&seen, "c'est quoi Dravr").await,
            "the first non-answer must ALSO reach the coach — being mid-form is no reason \
             to refuse the athlete's question"
        );
        assert_instrument_never_paraphrased(&seen);

        // Second non-answer: the budget is spent, so the intake retires.
        //
        // `calls >= 3`, not 2. Every message now reaches the coach — that is the
        // fix — so turn 1 plus both non-answers is three. The old assertion said
        // 2, which the first non-answer alone already satisfies, making it
        // impossible to fail once the coach stopped being skipped. The claim is
        // tightened rather than relaxed: the count is paired with the third
        // message's own text, so it pins WHICH message reached the model instead
        // of only that some message did.
        send_turn(
            &resources,
            8303,
            83,
            "je veux juste discuter de mon plan",
            false,
        )
        .await;
        assert!(
            wait_for_transcript(&seen, "je veux juste discuter de mon plan").await,
            "the stood-aside message itself must reach the model. The call COUNTER is not \
             enough — background memory extraction raises it too, so it reaches three \
             without this turn having run."
        );

        assert!(
            wait_for_step(&resources, user_id, "parq", "skipped").await,
            "standing aside must record the PAR-Q as skipped so it does not reopen, got {:?}",
            onboarding_steps(&resources, user_id).await
        );
        assert!(
            wait_for_step(&resources, user_id, "profile_type", "skipped").await,
            "profile type was never answered, so it is skipped too, got {:?}",
            onboarding_steps(&resources, user_id).await
        );
    }

    /// An athlete already screened on the web is not screened again in chat.
    #[tokio::test]
    #[serial]
    async fn a_recorded_screen_is_not_repeated_in_chat() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let mock = MockLlm::new();
        let calls = mock.counter();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "intake_done@example.com").await;
        link_channel(&resources, tenant_id, user_id, "84").await;

        // Exactly what the web wizard writes when it finishes both steps.
        for step in ["profile_type", "parq"] {
            resources
                .common
                .repos
                .user_onboarding
                .set_onboarding_step(
                    &user_id.to_string(),
                    step,
                    "complete",
                    None,
                    Some(&tenant_id.to_string()),
                )
                .await
                .unwrap();
        }

        send_turn(&resources, 8401, 84, "Salut", false).await;
        assert!(wait_for_turns(&calls, 1).await);
        // Give the post-turn hooks the same window the other tests rely on.
        sleep(Duration::from_millis(1200)).await;

        assert_eq!(
            probed_count(&resources, user_id).await,
            0,
            "an athlete who answered on the web must not be asked again in chat"
        );
    }

    /// Medical questions never go into a shared room.
    #[tokio::test]
    #[serial]
    async fn a_group_conversation_is_never_screened() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let mock = MockLlm::new();
        let calls = mock.counter();
        let resources = create_test_server_resources_with_llm(Arc::new(mock))
            .await
            .unwrap();

        let (user_id, tenant_id) =
            create_user_with_own_tenant(&resources, "intake_group@example.com").await;
        link_channel(&resources, tenant_id, user_id, "85").await;

        send_turn(&resources, 8501, 85, "@bot salut", true).await;
        assert!(wait_for_turns(&calls, 1).await);
        sleep(Duration::from_millis(1200)).await;

        assert_eq!(
            probed_count(&resources, user_id).await,
            0,
            "an intake question in a group would publish a medical screen to the room"
        );
    }
}
