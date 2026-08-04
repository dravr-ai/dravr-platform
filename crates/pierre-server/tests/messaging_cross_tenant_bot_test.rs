// ABOUTME: Regression test — an admin-owned bot whose tenant differs from the user's own tenant must
// ABOUTME: still drive a full DM turn: session/conversation/messages land under the USER tenant, LLM runs.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

// This guards the precise bug the messaging-session user-tenant fix targets: a
// Telegram bot owned by tenant B (the webhook/channel tenant) serving a user
// whose active_tenant_id is their OWN tenant A. The session + conversation +
// messages must be stored under tenant A (so they align with the user's activity
// cache and the backfill push), while the channel link + config stay under
// tenant B. Before the fix, the conversation was created under tenant A but the
// live dispatch read it under tenant B → get_conversation missed → every turn
// failed with "Conversation not found" before the LLM ever ran. The same-tenant
// case (channel == user) masks this, so only a cross-tenant fixture catches it.
#[cfg(feature = "client-messaging")]
mod cross_tenant_bot_tests {
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
    use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
    use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRole};
    use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
    use pierre_database::backends::{
        CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::messaging::MessagingRoutes;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::spawn_blocking;
    use tokio::time::sleep;
    use uuid::Uuid;

    /// Deterministic LLM so the messaging pipeline runs end-to-end offline. The
    /// call counter is the regression signal: it only increments once the
    /// pipeline gets PAST `persist_user_message` (which reads the conversation
    /// under `conversation_tenant_id`). If the conversation were stored under the
    /// bot tenant, that read would miss and the LLM would never be invoked.
    struct MockLlmProvider {
        reply: String,
        model: String,
        calls: Arc<AtomicUsize>,
    }

    impl MockLlmProvider {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                reply: reply.into(),
                model: "mock-model".to_owned(),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn call_counter(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.calls)
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlmProvider {
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
        fn default_model(&self) -> &str {
            &self.model
        }
        fn available_models(&self) -> &[String] {
            &[]
        }
        async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ChatResponse {
                content: self.reply.clone(),
                model: self.model.clone(),
                usage: Some(TokenUsage {
                    prompt_tokens: 42,
                    completion_tokens: 11,
                    total_tokens: 53,
                }),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }
        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            let chunk = StreamChunk {
                delta: self.reply.clone(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            };
            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }
        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    /// Create an active user that owns a fresh tenant (their personal
    /// workspace). `tenants.create` adds the owner to `tenant_users`, so
    /// `list_for_user().first()` — the runtime's `active_tenant_id` — resolves to
    /// this tenant. Returns `(user_id, owned_tenant_id)`.
    async fn create_user_with_own_tenant(
        resources: &ServerContext,
        email: &str,
    ) -> (Uuid, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("CrossTenant123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Cross Tenant".to_owned()),
        );
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let user_id = user.id;
        resources.common.repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::new();
        let tenant = Tenant {
            id: tenant_id,
            name: format!("Own Tenant {email}"),
            slug: format!("own-tenant-{tenant_id}"),
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

    /// Poll the mock until the pipeline invokes it, or time out. A timeout is the
    /// failure mode of the bug: the conversation read missed and the turn died
    /// before the LLM stage.
    async fn wait_for_llm_call(calls: &Arc<AtomicUsize>) -> bool {
        for _ in 0..50 {
            if calls.load(Ordering::SeqCst) >= 1 {
                return true;
            }
            sleep(Duration::from_millis(200)).await;
        }
        false
    }

    #[tokio::test]
    #[serial]
    async fn cross_tenant_bot_dm_stores_under_user_tenant_and_reaches_llm() {
        // chat_conversations creation reads PIERRE_LLM_MODEL; the value is stored
        // on the row, never dispatched (the mock answers the actual call).
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = Arc::new(MockLlmProvider::new("Voici tes courses."));
        let call_counter = mock.call_counter();
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        // The user and THEIR own tenant (active_tenant_id resolves here).
        let (user_id, user_tenant) =
            create_user_with_own_tenant(&resources, "cross_tenant_user@example.com").await;
        // A synthetic provider so the onboarding gate lets the turn reach the LLM.
        resources
            .common
            .repos
            .provider_connections
            .register_connection(
                user_id,
                user_tenant,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();

        // A SEPARATE bot-owner + bot tenant. The user is NOT a member of it, so
        // it is purely the channel/webhook tenant — exactly the admin-owned-bot
        // shape that fragmented jf's data.
        let (_bot_owner, bot_tenant) =
            create_user_with_own_tenant(&resources, "bot_owner@example.com").await;
        assert_ne!(user_tenant, bot_tenant, "fixture must be cross-tenant");

        // Telegram channel config lives under the BOT tenant.
        let tg_secret = "cross_tenant_tg_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(tg_secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:CROSS_TENANT_BOT"),
            is_active: true,
        })
        .await
        .unwrap();

        // Channel link (sender -> user) also lives under the BOT tenant — that is
        // where get_channel_link must keep looking after the fix.
        let sender_id = "42";
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            user_id: &user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: sender_id,
            display_name: Some("Cross Tenant Sender"),
        })
        .await
        .unwrap();

        // Inbound Telegram DM: chat.id == from.id makes it a direct message, so
        // session_tenant resolves to the user's own tenant.
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", tg_secret)
            .json(&json!({
                "update_id": 9001,
                "message": {
                    "message_id": 1,
                    "from": { "id": 42, "first_name": "CrossTenant" },
                    "chat": { "id": 42, "type": "private" },
                    "text": "Donne moi mes courses en 2022"
                }
            }))
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        // The pipeline must reach the LLM — proof the conversation read under the
        // user tenant succeeded (the bug timed out here at "Conversation not found").
        assert!(
            wait_for_llm_call(&call_counter).await,
            "pipeline never reached the LLM — conversation read likely missed under the wrong tenant"
        );

        // Assert the whole DM unit lives under the USER tenant, not the bot tenant.
        let pool = resources
            .coach
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");
        let user_tenant_str = user_tenant.to_string();
        let bot_tenant_str = bot_tenant.to_string();
        let user_id_str = user_id.to_string();

        let session: Option<(String, String)> = sqlx::query_as(
            "SELECT tenant_id, pierre_conversation_id FROM messaging_sessions WHERE user_id = ?1",
        )
        .bind(&user_id_str)
        .fetch_optional(pool)
        .await
        .unwrap();
        let (session_tenant, conversation_id) =
            session.expect("a session must have been created for the linked user");
        assert_eq!(
            session_tenant, user_tenant_str,
            "session must live under the user's own tenant, not the bot tenant"
        );
        assert_ne!(session_tenant, bot_tenant_str);

        let conv_tenant: Option<(String,)> =
            sqlx::query_as("SELECT tenant_id FROM chat_conversations WHERE id = ?1")
                .bind(&conversation_id)
                .fetch_optional(pool)
                .await
                .unwrap();
        assert_eq!(
            conv_tenant.expect("conversation row must exist").0,
            user_tenant_str,
            "conversation must live under the user tenant so the pipeline can read it"
        );

        // The inbound message row must share the user tenant (no re-split).
        let inbound_under_bot: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messaging_messages mm \
             JOIN messaging_sessions s ON mm.session_id = s.id \
             WHERE s.user_id = ?1 AND mm.direction = 'inbound' AND mm.tenant_id = ?2",
        )
        .bind(&user_id_str)
        .bind(&bot_tenant_str)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            inbound_under_bot, 0,
            "inbound messages must not be written under the bot tenant"
        );

        let inbound_under_user: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messaging_messages mm \
             JOIN messaging_sessions s ON mm.session_id = s.id \
             WHERE s.user_id = ?1 AND mm.direction = 'inbound' AND mm.tenant_id = ?2",
        )
        .bind(&user_id_str)
        .bind(&user_tenant_str)
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(
            inbound_under_user >= 1,
            "the inbound message must be stored under the user tenant"
        );

        // The outbound (assistant) row must also land under the user tenant —
        // the other half of the DM unit. The fake bot token makes the real
        // Telegram send fail, so the reply persists via the retry path
        // (outbound_retry::enqueue_failed_outbound), which likewise uses
        // session_tenant_id for the message row. Poll because that write
        // happens asynchronously after the failed delivery.
        let mut outbound_tenant: Option<String> = None;
        for _ in 0..50 {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT mm.tenant_id FROM messaging_messages mm \
                 JOIN messaging_sessions s ON mm.session_id = s.id \
                 WHERE s.user_id = ?1 AND mm.direction = 'outbound' LIMIT 1",
            )
            .bind(&user_id_str)
            .fetch_optional(pool)
            .await
            .unwrap();
            if let Some((t,)) = row {
                outbound_tenant = Some(t);
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(
            outbound_tenant.expect("an outbound message row must be persisted"),
            user_tenant_str,
            "outbound message must be stored under the user tenant, not the bot tenant"
        );
    }

    /// The same cross-tenant topology, but for a GROUP chat and a slash command.
    ///
    /// A group session, its conversation and the `coaching_groups` row auto-bound
    /// to the chat all live under the BOT tenant, while the member's
    /// `active_tenant_id` is their own. `/group consent yes` is a privacy control:
    /// it must flip `peer_sharing_consent` on the group that owns the chat it was
    /// typed in. When the slash dispatcher looked the conversation up under the
    /// caller's tenant instead, the read missed and the handler fell through to
    /// `list_groups_for_user().first()` — an unfiltered, cross-tenant,
    /// `updated_at DESC` list — and published the athlete's training data to the
    /// members of whichever OTHER group they had touched most recently.
    #[tokio::test]
    #[serial]
    async fn cross_tenant_group_slash_consent_binds_to_the_chat_group() {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");

        let mock = Arc::new(MockLlmProvider::new("unused — slash commands skip the LLM"));
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        let db: &dyn MessagingRepository = &*resources.common.repos.messaging;

        let (user_id, user_tenant) =
            create_user_with_own_tenant(&resources, "group_slash_member@example.com").await;
        resources
            .common
            .repos
            .provider_connections
            .register_connection(
                user_id,
                user_tenant,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();

        let (bot_owner, bot_tenant) =
            create_user_with_own_tenant(&resources, "group_slash_bot@example.com").await;
        assert_ne!(user_tenant, bot_tenant, "fixture must be cross-tenant");

        // The channel-group bootstrap picks a system coach from the BOT tenant;
        // without one it declines to create the group and the chat stays
        // ungrouped.
        resources
            .common
            .repos
            .coaches
            .create_system_coach(
                bot_owner,
                bot_tenant,
                &CreateSystemCoachRequest {
                    title: "Group Slash Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap();

        // A second group in the member's OWN tenant, stamped in the future so it
        // heads `list_groups_for_user` (ORDER BY updated_at DESC). This is the
        // row consent must NOT land on.
        let decoy_coach = resources
            .common
            .repos
            .coaches
            .create_system_coach(
                user_id,
                user_tenant,
                &CreateSystemCoachRequest {
                    title: "Decoy Coach".to_owned(),
                    description: None,
                    system_prompt: "Test prompt".to_owned(),
                    category: CoachCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: CoachVisibility::Global,
                },
            )
            .await
            .unwrap();
        let decoy_group_id = Uuid::new_v4();
        let now = Utc::now();
        resources
            .common
            .repos
            .groups
            .create_group(
                user_tenant,
                &CoachingGroup {
                    id: decoy_group_id,
                    tenant_id: user_tenant.to_string(),
                    name: "Decoy group".to_owned(),
                    description: None,
                    coach_id: decoy_coach.id.to_string(),
                    owner_id: user_id,
                    coach_user_id: None,
                    peer_data_sharing: true,
                    max_members: 10,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now + chrono::Duration::seconds(600),
                },
            )
            .await
            .unwrap();
        resources
            .common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: decoy_group_id,
                user_id,
                tenant_id: user_tenant.to_string(),
                role: GroupRole::Owner,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();

        let tg_secret = "group_slash_tg_secret";
        db.upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            channel_type: "telegram",
            api_key: None,
            api_secret: None,
            webhook_secret: Some(tg_secret),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:GROUP_SLASH_BOT"),
            is_active: true,
        })
        .await
        .unwrap();

        let sender_id = "77";
        db.create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: bot_tenant,
            user_id: &user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: sender_id,
            display_name: Some("Group Slash Sender"),
        })
        .await
        .unwrap();

        // chat.id != from.id and chat.type = "supergroup" → not a DM, so the
        // session, conversation and coaching group land under the bot tenant.
        let router = MessagingRoutes::routes(Arc::clone(&resources));
        let resp = AxumTestRequest::post("/api/messaging/webhook/telegram")
            .header("content-type", "application/json")
            .header("x-telegram-bot-api-secret-token", tg_secret)
            .json(&json!({
                "update_id": 9101,
                "message": {
                    "message_id": 1,
                    "from": { "id": 77, "first_name": "GroupSlash" },
                    "chat": { "id": -1_001_234, "type": "supergroup", "title": "Club Run" },
                    "text": "/group consent yes"
                }
            }))
            .send(router)
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);

        let pool = resources
            .coach
            .database
            .sqlite_pool()
            .expect("test fixture runs against SQLite");

        // The chat's coaching group is created during session resolution; poll
        // until it exists, then until the consent write lands.
        let mut chat_group: Option<(String, String)> = None;
        for _ in 0..50 {
            chat_group = sqlx::query_as(
                "SELECT id, tenant_id FROM coaching_groups WHERE channel_chat_id = ?1",
            )
            .bind("-1001234")
            .fetch_optional(pool)
            .await
            .unwrap();
            if chat_group.is_some() {
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        let (chat_group_id, chat_group_tenant) =
            chat_group.expect("the supergroup chat must auto-create a coaching_groups row");
        assert_eq!(
            chat_group_tenant,
            bot_tenant.to_string(),
            "a shared room's coaching group belongs to the channel tenant"
        );

        let mut chat_consent = false;
        for _ in 0..50 {
            chat_consent = resources
                .common
                .repos
                .groups
                .list_members(&chat_group_id)
                .await
                .unwrap()
                .iter()
                .find(|m| m.user_id == user_id)
                .is_some_and(|m| m.peer_sharing_consent);
            if chat_consent {
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }
        assert!(
            chat_consent,
            "/group consent yes must flip consent on the group bound to the chat \
             it was typed in, even though the member's tenant is not the bot's"
        );

        let decoy_consent = resources
            .common
            .repos
            .groups
            .list_members(&decoy_group_id.to_string())
            .await
            .unwrap()
            .iter()
            .find(|m| m.user_id == user_id)
            .expect("decoy membership row")
            .peer_sharing_consent;
        assert!(
            !decoy_consent,
            "consent must not leak into the member's other, more recently updated group"
        );
    }
}
