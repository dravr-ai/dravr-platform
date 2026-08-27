// ABOUTME: Full-pipeline e2e for capability-failure recovery — a fabricated "can't access your data"
// ABOUTME: reply is either re-asked away with verified data or replaced by the reconnect re-challenge

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Regression guard for the live 2026-07-24 / 2026-08-11 Telegram incidents:
//! the coach answered « Je ne suis pas capable d'accéder à tes données
//! d'activité en ce moment (problème de connexion de mon côté) » on turns
//! with **zero tool calls** while every scrape in the surrounding weeks was
//! green. The fabricated apology reached the user, was persisted, and
//! replayed — teaching the model its data access was broken.
//!
//! The capability-recovery stage adjudicates such a claim with one real
//! read-only `get_activities` fetch:
//!
//! - fetch succeeds → the claim is disproven; one re-ask with the fetched
//!   data replaces the apology (`fabricated_claim_with_working_provider…`).
//! - fetch needs re-auth → the claim was right but useless; the auth-recovery
//!   stage replaces it with the localized reconnect link so the athlete gets
//!   an actionable re-challenge (`fabricated_claim_with_dead_provider…`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "client-messaging")]
mod capability_recovery {
    use crate::common::create_test_server_resources_with_llm;
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::sciotte_mock::{seed_sciotte_session, spawn_mock_scraper};
    use async_trait::async_trait;
    use axum::http::StatusCode;
    use chrono::Utc;
    use embacle::types::ToolCallRequest;
    use futures_util::stream;
    use hmac::{Hmac, Mac};
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
        TokenUsage,
    };
    use pierre_core::models::ConnectionType;
    use pierre_core::models::{Tenant, TenantId, User, UserStatus};
    use pierre_core::permissions::UserRole;
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::json;
    use serial_test::serial;
    use sha2::Sha256;
    use std::env;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// The verbatim first sentence of the 2026-08-11 live incident.
    const FABRICATED_CLAIM: &str =
        "Je ne suis pas capable d'accéder à tes données d'activité en ce moment (problème de \
         connexion de mon côté) — je ne veux pas inventer des chiffres.";

    /// The clean coaching reply the re-ask must surface instead.
    const CLEAN_REPLY: &str =
        "Parfait — basé sur tes 5 dernières sorties: 45 min de vélo facile ce soir, bol de riz \
         au tofu après.";

    /// A substring of the stage's re-ask instruction; seeing it in the request
    /// proves the verification data actually reached the wire.
    const REASK_MARKER: &str = "fetched successfully on your behalf";

    /// A re-ask reply that passes the capability-claim check and still carries
    /// raw tool-call scaffolding — the shape a re-challenged provider actually
    /// produced on 2026-08-18, when the athlete received this verbatim.
    const REASK_WITH_SCAFFOLDING: &str = concat!(
        "<tool_call>\n",
        r#"{"name": "get_activities", "arguments": {"after": 1784563200, "limit": 100}}"#,
        "\n</tool_call>",
    );

    /// Deterministic provider reproducing the incident shape: the main turn
    /// fabricates the access-failure apology; the recovery re-ask (recognised
    /// by the instruction appended after the verified tool result) answers
    /// cleanly, as the live model does once real data is in hand.
    struct ClaimThenCleanMockProvider;

    /// Fabricates a capability failure, then answers the re-ask with raw
    /// `<tool_call>` scaffolding instead of prose.
    struct ClaimThenScaffoldingMockProvider;

    #[async_trait]
    impl LlmProvider for ClaimThenScaffoldingMockProvider {
        fn name(&self) -> &'static str {
            "claim_then_scaffolding_mock"
        }
        fn display_name(&self) -> &'static str {
            "Claim-then-scaffolding Mock LLM (capability-recovery e2e)"
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
            let is_reask = request
                .messages
                .iter()
                .any(|m| m.content.contains(REASK_MARKER));
            let content = if is_reask {
                REASK_WITH_SCAFFOLDING
            } else {
                FABRICATED_CLAIM
            };
            Ok(ChatResponse {
                content: content.to_owned(),
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage::new(30, 40, 70)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            Err(AppError::internal("streaming not used by this test"))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    #[async_trait]
    impl LlmProvider for ClaimThenCleanMockProvider {
        fn name(&self) -> &'static str {
            "claim_then_clean_mock"
        }
        fn display_name(&self) -> &'static str {
            "Claim-then-clean Mock LLM (capability-recovery e2e)"
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
            let is_reask = request
                .messages
                .iter()
                .any(|m| m.content.contains(REASK_MARKER));
            let content = if is_reask {
                CLEAN_REPLY
            } else {
                FABRICATED_CLAIM
            };
            Ok(ChatResponse {
                content: content.to_owned(),
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage::new(30, 40, 70)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            let chunk = StreamChunk {
                delta: "streaming not used".to_owned(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            };
            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    /// Compute the Slack webhook signature (`v0=<hex-hmac-sha256>` over
    /// `v0:{ts}:{body}`).
    fn compute_slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// Seed an active admin user + tenant with a provider connection of the
    /// caller's choice — `synthetic` serves activities in the test env (the
    /// verified path); `sciotte` with no stored session is auth-dead (the
    /// re-challenge path).
    async fn create_user_with_connection(
        resources: &Arc<ServerContext>,
        email: &str,
        provider: &str,
        connection_type: &ConnectionType,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Cap Admin".to_owned()),
        );
        user.is_admin = true;
        user.role = UserRole::Admin;
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Cap Tenant {email}"),
            slug: format!("cap-tenant-{tenant_id}"),
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
            .register_connection(user_id, tenant_id, provider, connection_type, None)
            .await
            .unwrap();

        (user_id, tenant_id)
    }

    /// Wire a Slack channel + link for the user, post one inbound message, and
    /// return once the webhook was accepted.
    async fn drive_slack_turn(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
        user_id: Uuid,
        slack_sender_id: &str,
        signing_secret: &str,
        text: &str,
    ) {
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: "slack",
            api_key: Some("xoxb-capability-recovery"),
            api_secret: None,
            webhook_secret: Some(signing_secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: None,
            is_active: true,
        })
        .await
        .unwrap();
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "slack",
            channel_user_id: slack_sender_id,
            display_name: Some("Cap Sender"),
        })
        .await
        .unwrap();

        let body = json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "user": slack_sender_id,
                "text": text,
                "channel": "C_CAPABILITY_RECOVERY",
                "ts": "1700000005.000001"
            }
        })
        .to_string();
        let timestamp = Utc::now().timestamp().to_string();
        let sig = compute_slack_sig(signing_secret, &timestamp, &body);

        let router = MessagingRoutes::routes(Arc::clone(resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/slack")
            .header("content-type", "application/json")
            .header("x-slack-request-timestamp", &timestamp)
            .header("x-slack-signature", &sig)
            .text(&body)
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
    }

    /// Poll `chat_messages` until the pipeline persists an `assistant` row for
    /// the tenant, then return its (already post-processed) content.
    /// The `finish_reason` persisted on the tenant's latest assistant row.
    /// `build_llm_messages` drops a row stamped
    /// `capability_claim_unverified` from every later prompt, so this is the
    /// pin that a moment-in-time failure cannot replay.
    async fn persisted_finish_reason(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
    ) -> Option<String> {
        let pool = resources.coach.database.sqlite_pool().unwrap();
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT m.finish_reason \
             FROM chat_messages m \
             JOIN chat_conversations c ON m.conversation_id = c.id \
             WHERE c.tenant_id = ?1 AND m.role = 'assistant' \
             ORDER BY m.created_at DESC LIMIT 1",
        )
        .bind(tenant_id.to_string())
        .fetch_optional(pool)
        .await
        .unwrap();
        row.and_then(|(reason,)| reason)
    }

    async fn wait_for_persisted_assistant_reply(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
    ) -> Option<String> {
        let pool = resources
            .coach
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");
        let tenant_str = tenant_id.to_string();

        for _ in 0..150 {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT m.content \
                 FROM chat_messages m \
                 JOIN chat_conversations c ON m.conversation_id = c.id \
                 WHERE c.tenant_id = ?1 AND m.role = 'assistant' \
                 ORDER BY m.created_at DESC LIMIT 1",
            )
            .bind(&tenant_str)
            .fetch_optional(pool)
            .await
            .unwrap();
            if let Some((content,)) = row {
                return Some(content);
            }
            sleep(Duration::from_millis(200)).await;
        }
        None
    }

    /// The reply as it went **out**, not as it was stored.
    ///
    /// The distinction is the whole point of the test below: stage 19 strips the
    /// durable copy, so a persisted-only assertion passes while the athlete is
    /// receiving raw scaffolding. `messaging_messages` records what was actually
    /// delivered.
    async fn wait_for_outbound_body(
        resources: &Arc<ServerContext>,
        tenant_id: TenantId,
    ) -> Option<String> {
        let pool = resources
            .coach
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");
        let tenant_str = tenant_id.to_string();

        for _ in 0..150 {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT content_body FROM messaging_messages \
                 WHERE tenant_id = ?1 AND direction = 'outbound' \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(&tenant_str)
            .fetch_optional(pool)
            .await
            .unwrap();
            if let Some((body,)) = row {
                return Some(body);
            }
            sleep(Duration::from_millis(200)).await;
        }
        None
    }

    #[tokio::test]
    #[serial]
    async fn fabricated_claim_with_working_provider_is_reasked_away() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");
        let scraper_url = spawn_mock_scraper().await;
        env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
        // The remote client is both-or-neither: a URL with no audience disables
        // it, because unsigned requests are refused by the scraper rather than served.
        env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");

        let mock = Arc::new(ClaimThenCleanMockProvider);
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let (user_id, tenant_id) = create_user_with_connection(
            &resources,
            "capability-verified@example.com",
            "sciotte",
            &ConnectionType::Manual,
        )
        .await;
        seed_sciotte_session(&resources, user_id, tenant_id).await;

        drive_slack_turn(
            &resources,
            tenant_id,
            user_id,
            "U_CAP_VERIFIED",
            "capability_verified_secret",
            "Propose-moi une sortie basée sur mes activités récentes",
        )
        .await;

        let reply = wait_for_persisted_assistant_reply(&resources, tenant_id)
            .await
            .expect("pipeline did not persist an assistant chat_messages row within 30s");

        assert!(
            reply.contains("45 min"),
            "the re-asked coaching reply must be the delivered/durable one, got: {reply:?}"
        );
        assert!(
            !reply.contains("problème de connexion"),
            "the fabricated access-failure claim must not survive a working provider, \
             got: {reply:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn fabricated_claim_with_dead_provider_becomes_a_reconnect_challenge() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");

        let mock = Arc::new(ClaimThenCleanMockProvider);
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        // A sciotte connection with no stored scrape session: the verification
        // fetch fails auth-shaped, which must route to the reconnect link.
        let (user_id, tenant_id) = create_user_with_connection(
            &resources,
            "capability-dead@example.com",
            "sciotte",
            &ConnectionType::Manual,
        )
        .await;

        drive_slack_turn(
            &resources,
            tenant_id,
            user_id,
            "U_CAP_DEAD",
            "capability_dead_secret",
            "Propose-moi une sortie basée sur mon activité d'hier",
        )
        .await;

        let reply = wait_for_persisted_assistant_reply(&resources, tenant_id)
            .await
            .expect("pipeline did not persist an assistant chat_messages row within 30s");

        // The localized re-challenge (FR default locale): imperative reconnect
        // copy plus an actionable link — not the dead-end apology.
        assert!(
            reply.contains("Reconnecte"),
            "the reply must be the localized reconnect re-challenge, got: {reply:?}"
        );
        assert!(
            reply.contains("/r/"),
            "the re-challenge must carry the shortened login link (test env base_url is \
             relative; production prefixes the host), got: {reply:?}"
        );
        assert!(
            !reply.contains("problème de connexion de mon côté"),
            "the fabricated apology must not be the durable reply, got: {reply:?}"
        );

        // The reconnect message is true of this moment only — connection state
        // is re-derived every turn — so it is stamped out of later prompts.
        // Replaying it is how a 07-24 apology produced an identical 08-11 one.
        assert_eq!(
            persisted_finish_reason(&resources, tenant_id)
                .await
                .as_deref(),
            Some("capability_claim_unverified"),
            "a moment-in-time capability reply must be stamped so it never replays"
        );
    }

    /// Deterministic provider that never says it is broken — it just answers a
    /// data question with no data behind it. This is the failure the lexical
    /// detector structurally cannot see, and the reason the trigger stopped
    /// depending on the model's choice of words.
    struct UngroundedThenGroundedMockProvider;

    /// Generic filler with zero capability-failure vocabulary in any locale.
    const UNGROUNDED_REPLY: &str =
        "Pour aujourd'hui je partirais sur quelque chose de tranquille, environ une heure, \
         à l'aise. Écoute tes jambes et ajuste au feeling.";

    #[async_trait]
    impl LlmProvider for UngroundedThenGroundedMockProvider {
        fn name(&self) -> &'static str {
            "ungrounded_then_grounded_mock"
        }
        fn display_name(&self) -> &'static str {
            "Ungrounded-then-grounded Mock LLM (structural-trigger e2e)"
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
            let is_reask = request
                .messages
                .iter()
                .any(|m| m.content.contains(REASK_MARKER));
            let content = if is_reask {
                CLEAN_REPLY
            } else {
                UNGROUNDED_REPLY
            };
            Ok(ChatResponse {
                content: content.to_owned(),
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage::new(30, 40, 70)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            let chunk = StreamChunk {
                delta: "streaming not used".to_owned(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            };
            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    #[tokio::test]
    #[serial]
    async fn an_ungrounded_data_answer_is_regrounded_without_any_refusal_wording() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");
        let scraper_url = spawn_mock_scraper().await;
        env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
        // The remote client is both-or-neither: a URL with no audience disables
        // it, because unsigned requests are refused by the scraper rather than served.
        env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");

        let mock = Arc::new(UngroundedThenGroundedMockProvider);
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let (user_id, tenant_id) = create_user_with_connection(
            &resources,
            "capability-ungrounded@example.com",
            "sciotte",
            &ConnectionType::Manual,
        )
        .await;
        seed_sciotte_session(&resources, user_id, tenant_id).await;

        drive_slack_turn(
            &resources,
            tenant_id,
            user_id,
            "U_CAP_UNGROUNDED",
            "capability_ungrounded_secret",
            "Propose moi une sortie basée sur mon activité d'hier",
        )
        .await;

        let reply = wait_for_persisted_assistant_reply(&resources, tenant_id)
            .await
            .expect("pipeline did not persist an assistant chat_messages row within 30s");

        assert!(
            reply.contains("45 min"),
            "a data ask answered with no tool call and no injected block must be \
             re-asked against real activities, got: {reply:?}"
        );
        assert!(
            !reply.contains("Écoute tes jambes"),
            "the ungrounded filler must not survive as the durable reply, got: {reply:?}"
        );
        // Nothing here was stamped: the athlete got a grounded answer, so there
        // is no moment-in-time failure to keep out of later prompts.
        assert_ne!(
            persisted_finish_reason(&resources, tenant_id)
                .await
                .as_deref(),
            Some("capability_claim_unverified"),
            "a successfully re-grounded turn is ordinary history"
        );
    }

    /// The nine-character fragment delivered to a live Telegram group on
    /// 2026-08-22 in place of a training-comparison answer.
    const DEGENERATE_REPLY: &str = "by Dravr.";

    /// Calls a real tool, then "answers" the synthesis turn with the dangling
    /// fragment; the recovery re-ask (recognised by its instruction marker)
    /// answers cleanly, as a re-challenged model does with data in hand.
    struct ToolThenFragmentMockProvider;

    #[async_trait]
    impl LlmProvider for ToolThenFragmentMockProvider {
        fn name(&self) -> &'static str {
            "tool_then_fragment_mock"
        }
        fn display_name(&self) -> &'static str {
            "Tool-then-fragment Mock LLM (degenerate-reply e2e)"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
        }
        fn default_model(&self) -> &'static str {
            "mock-model"
        }
        fn available_models(&self) -> &[String] {
            &[]
        }

        async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
            let is_reask = request
                .messages
                .iter()
                .any(|m| m.content.contains(REASK_MARKER));
            let has_tool_results = request
                .messages
                .iter()
                .any(|m| m.content.contains("[Tool Result for "));
            let (content, tool_calls) = if is_reask {
                (CLEAN_REPLY.to_owned(), None)
            } else if has_tool_results {
                (DEGENERATE_REPLY.to_owned(), None)
            } else {
                (
                    String::new(),
                    Some(vec![ToolCallRequest {
                        id: "call-1".to_owned(),
                        function_name: "get_activities".to_owned(),
                        arguments: json!({ "limit": 5 }),
                    }]),
                )
            };
            Ok(ChatResponse {
                content,
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage::new(30, 40, 70)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls,
            })
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            Err(AppError::internal("streaming not used by this test"))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    /// Tools ran, real data came back, and the model still delivered a
    /// dangling fragment — the exact complement of the ungrounded case above
    /// (there the model had no data and answered anyway; here it had data and
    /// failed to answer at all). The `DegenerateReply` trigger must run the
    /// verification fetch and re-ask, so the athlete receives the grounded
    /// answer instead of «by Dravr.».
    #[tokio::test]
    #[serial]
    async fn a_degenerate_fragment_after_tool_calls_is_reasked_into_an_answer() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");
        let scraper_url = spawn_mock_scraper().await;
        env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
        // The remote client is both-or-neither: a URL with no audience disables
        // it, because unsigned requests are refused by the scraper rather than served.
        env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");

        let mock = Arc::new(ToolThenFragmentMockProvider);
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let (user_id, tenant_id) = create_user_with_connection(
            &resources,
            "capability-degenerate@example.com",
            "sciotte",
            &ConnectionType::Manual,
        )
        .await;
        seed_sciotte_session(&resources, user_id, tenant_id).await;

        drive_slack_turn(
            &resources,
            tenant_id,
            user_id,
            "U_CAP_DEGEN",
            "capability_degenerate_secret",
            "Compare mes heures d'entraînement de cette semaine avec la précédente",
        )
        .await;

        let reply = wait_for_persisted_assistant_reply(&resources, tenant_id)
            .await
            .expect("pipeline did not persist an assistant chat_messages row within 30s");

        assert!(
            reply.contains("45 min"),
            "a degenerate fragment on a turn with tool calls must be re-asked \
             into the grounded answer, got: {reply:?}"
        );
        assert!(
            !reply.contains(DEGENERATE_REPLY),
            "the dangling fragment must not survive as the durable reply, got: {reply:?}"
        );
    }
    /// The re-ask reply must be stripped before it goes on the wire.
    ///
    /// `apply_reask_outcome` assigns the re-ask's content downstream of every
    /// other strip in the turn: the tool loop cleans its own output, and stage
    /// 19 cleans only the durable copy. A re-ask reply assigned raw is washed by
    /// neither, and on 2026-08-18 a Telegram athlete received the model's
    /// `<tool_call>` scaffolding verbatim while `chat_messages` recorded an
    /// empty reply for the same turn.
    ///
    /// This asserts on the **outbound** row rather than the persisted one on
    /// purpose. Stage 19 strips what is stored, so a persisted-only assertion —
    /// which is what the test above does, and all this file had — passes with
    /// the bug fully present. Two values for one reply is why the database
    /// looked clean exactly when the delivery was worst.
    #[tokio::test]
    #[serial]
    async fn a_reask_reply_is_stripped_before_it_is_delivered() {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");
        let scraper_url = spawn_mock_scraper().await;
        env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
        // The remote client is both-or-neither: a URL with no audience disables
        // it, because unsigned requests are refused by the scraper rather than served.
        env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");

        let mock = Arc::new(ClaimThenScaffoldingMockProvider);
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let (user_id, tenant_id) = create_user_with_connection(
            &resources,
            "reask-scaffolding@example.com",
            "sciotte",
            &ConnectionType::Manual,
        )
        .await;
        seed_sciotte_session(&resources, user_id, tenant_id).await;

        drive_slack_turn(
            &resources,
            tenant_id,
            user_id,
            "U_REASK_SCAFFOLD",
            "reask_scaffolding_secret",
            "Donne moi ma progression du dernier mois",
        )
        .await;

        let delivered = wait_for_outbound_body(&resources, tenant_id)
            .await
            .expect("pipeline did not record an outbound messaging_messages row within 30s");

        assert!(
            !delivered.contains("<tool_call>"),
            "raw tool-call scaffolding was delivered to the athlete: {delivered:?}"
        );
        assert!(
            !delivered.contains("get_activities"),
            "the tool-call payload leaked into the delivered reply: {delivered:?}"
        );
    }
}
