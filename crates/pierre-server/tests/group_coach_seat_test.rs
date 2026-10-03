// ABOUTME: Integration tests for the coach's seat in a group turn: the coach's own training is never the group's
// ABOUTME: Coach mode is a role in one group — a coach-only account joined as an athlete elsewhere stays the subject
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "tools-groups")]
mod group_coach_seat_tests {
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
    use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
    use pierre_core::models::groups::{
        CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    };
    use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::chat::ChatRoutes;
    use pierre_services::intake::{STATUS_NOT_APPLICABLE, STEP_PARQ};
    use serde_json::{json, Value};
    use serial_test::serial;
    use std::env;
    use std::iter::once;
    use std::sync::{Arc, Mutex};
    use tokio::task::spawn_blocking;
    use uuid::Uuid;

    /// A line only the coach-mode directive carries.
    const COACH_SEAT_MARKER: &str = "this group's coach, not an athlete in it";
    /// The empty-group half of that directive.
    const PENDING_MARKER: &str = "the invite is still pending";
    /// The heading of the sender's own connected-provider block, rendered for
    /// every subject (connected, none, or unknown) and never for the coach.
    const OWN_PROVIDERS_MARKER: &str = "Connected Fitness Data Providers";

    /// The declared-tool marker for the caller's own activities: withheld on
    /// the coach's seat (carnet#742), declared for every subject.
    const OWN_ACTIVITIES_DECLARED: &str = "TOOL_DECL[get_activities]";
    /// The declared-tool marker for a named athlete's activities.
    const ATHLETE_ACTIVITIES_DECLARED: &str = "TOOL_DECL[get_group_member_activities]";

    /// Deterministic LLM that captures each request's serialized messages and
    /// the names of the tools declared to it, as `TOOL_DECL[name]` markers.
    /// The assertion point is the assembled request, not model output.
    struct CapturingLlm {
        model: String,
        seen_requests: Arc<Mutex<Vec<String>>>,
    }

    impl CapturingLlm {
        fn new() -> Self {
            Self {
                model: "mock-model".to_owned(),
                seen_requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn record(&self, request: &ChatRequest) {
            let mut serialized = serde_json::to_string(&request.messages).unwrap_or_default();
            for tool in request.tools.iter().flatten() {
                serialized.push_str("\nTOOL_DECL[");
                serialized.push_str(&tool.name);
                serialized.push(']');
            }
            self.seen_requests.lock().unwrap().push(serialized);
        }
    }

    #[async_trait]
    impl LlmProvider for CapturingLlm {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn display_name(&self) -> &'static str {
            "Capturing mock LLM (tests)"
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
            self.record(request);
            Ok(ChatResponse {
                content: "Noted.".to_owned(),
                model: self.model.clone(),
                usage: Some(TokenUsage::new(42, 11, 53)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }
        async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
            self.record(request);
            let chunk = StreamChunk {
                delta: "Noted.".to_owned(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            };
            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }
        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    /// An active user owning a fresh tenant, with a provider connection so the
    /// onboarding gate never fires before the turn.
    async fn create_user(resources: &ServerContext, email: &str) -> (User, TenantId) {
        let password_hash =
            spawn_blocking(|| bcrypt::hash("CoachSeat123!", bcrypt::DEFAULT_COST).unwrap())
                .await
                .unwrap();
        let mut user = User::new(email.to_owned(), password_hash, Some(email.to_owned()));
        user.user_status = UserStatus::Active;
        user.approved_by = Some(user.id);
        user.approved_at = Some(Utc::now());
        let repos = &resources.common.repos;
        repos.users.create(&user).await.unwrap();

        let tenant_id = TenantId::generate();
        repos
            .tenants
            .create(&Tenant {
                id: tenant_id,
                name: format!("Own Tenant {email}"),
                slug: format!("coach-seat-{tenant_id}"),
                domain: None,
                plan: "professional".to_owned(),
                owner_user_id: user.id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        repos
            .users
            .update_tenant_id(user.id, tenant_id)
            .await
            .unwrap();
        repos
            .provider_connections
            .register_connection(
                user.id,
                tenant_id,
                "synthetic",
                &ConnectionType::Synthetic,
                None,
            )
            .await
            .unwrap();
        let user = repos.users.get_global(user.id).await.unwrap().unwrap();
        (user, tenant_id)
    }

    /// Record the coach-only answer onboarding stores: PAR-Q not applicable.
    async fn mark_coach_only(resources: &ServerContext, user_id: Uuid) {
        resources
            .common
            .repos
            .user_onboarding
            .set_onboarding_step(
                &user_id.to_string(),
                STEP_PARQ,
                STATUS_NOT_APPLICABLE,
                None,
                None,
            )
            .await
            .unwrap();
    }

    /// A group owned by `owner` in `tenant`, with `members` joined beside the
    /// owner and `coach` as its recorded human coach.
    async fn create_group(
        resources: &ServerContext,
        tenant: TenantId,
        owner: Uuid,
        coach: Option<Uuid>,
        members: &[Uuid],
    ) -> Uuid {
        let repos = &resources.common.repos;
        let agent = repos
            .agents
            .create_system_agent(
                owner,
                tenant,
                &CreateSystemAgentRequest {
                    title: "Seat Coach".to_owned(),
                    description: None,
                    system_prompt: "You are a concise test coach.".to_owned(),
                    category: AgentCategory::Training,
                    tags: vec![],
                    sample_prompts: vec![],
                    visibility: AgentVisibility::Global,
                },
            )
            .await
            .unwrap();
        let group_id = Uuid::new_v4();
        let now = Utc::now();
        repos
            .groups
            .create_group(
                tenant,
                &CoachingGroup {
                    id: group_id,
                    tenant_id: tenant.to_string(),
                    name: "Seat Test Group".to_owned(),
                    description: None,
                    agent_id: agent.id.to_string(),
                    owner_id: owner,
                    coach_user_id: coach,
                    peer_data_sharing: true,
                    respond_mode: GroupRespondMode::Mentions,
                    digest_mode: GroupDigestMode::Off,
                    max_members: 10,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        let roster =
            once((owner, GroupRole::Owner)).chain(members.iter().map(|m| (*m, GroupRole::Member)));
        for (user_id, role) in roster {
            repos
                .groups
                .add_member(&GroupMember {
                    id: Uuid::new_v4(),
                    group_id,
                    user_id,
                    tenant_id: tenant.to_string(),
                    role,
                    peer_sharing_consent: true,
                    consent_given_at: now,
                    joined_at: now,
                    left_at: None,
                    display_name: None,
                })
                .await
                .unwrap();
        }
        group_id
    }

    /// Send one web turn as `sender` in a conversation scoped to `group_id`,
    /// and return every prompt the model was handed.
    async fn group_turn_prompts(
        resources: &Arc<ServerContext>,
        requests: &Arc<Mutex<Vec<String>>>,
        sender: &User,
        group_id: Uuid,
    ) -> String {
        let auth = format!(
            "Bearer {}",
            resources
                .auth
                .auth_manager
                .generate_token(sender, &resources.auth.jwks_manager)
                .unwrap()
        );
        let router = ChatRoutes::routes(Arc::clone(resources));
        let conv = AxumTestRequest::post("/api/chat/conversations")
            .header("authorization", &auth)
            .json(&json!({ "title": "Group thread", "group_id": group_id.to_string() }))
            .send(router.clone())
            .await;
        assert_eq!(conv.status_code(), StatusCode::CREATED);
        let conv_id = conv.json::<Value>()["id"].as_str().unwrap().to_owned();

        let sent = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
            .header("authorization", &auth)
            .json(&json!({ "content": "How is the group doing this week?" }))
            .send(router)
            .await;
        assert_eq!(sent.status_code(), StatusCode::OK);
        let captured = requests.lock().unwrap().join("\n");
        assert!(!captured.is_empty(), "the turn must reach the model");
        captured
    }

    async fn harness() -> (Arc<ServerContext>, Arc<Mutex<Vec<String>>>) {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let mock = Arc::new(CapturingLlm::new());
        let requests = Arc::clone(&mock.seen_requests);
        let resources = create_test_server_resources_with_llm(mock).await.unwrap();
        (resources, requests)
    }

    /// The carnet#741 shape: a coach-only account owns the group onboarding
    /// made, nobody has redeemed the invite, and the coach has no roster
    /// grant yet — so no `coach_user_id` was recorded either.
    #[tokio::test]
    #[serial]
    async fn a_coach_only_owner_of_an_empty_group_is_talked_to_as_its_coach() {
        let (resources, requests) = harness().await;
        let (coach, tenant) = create_user(&resources, "seat-coach-only@example.com").await;
        mark_coach_only(&resources, coach.id).await;
        let group_id = create_group(&resources, tenant, coach.id, None, &[]).await;

        let prompts = group_turn_prompts(&resources, &requests, &coach, group_id).await;

        assert!(
            prompts.contains(COACH_SEAT_MARKER),
            "the coach's turn must say who the agent is talking to"
        );
        assert!(
            prompts.contains(PENDING_MARKER),
            "with no athlete joined, the agent must be told the invite is pending"
        );
        assert!(
            !prompts.contains(OWN_ACTIVITIES_DECLARED),
            "the coach's own activities tool must not be declared on the seat"
        );
        assert!(
            prompts.contains(ATHLETE_ACTIVITIES_DECLARED),
            "the tool that reads a named athlete must stay declared"
        );
        assert!(
            !prompts.contains(OWN_PROVIDERS_MARKER),
            "the coach's own connected providers must not reach the group turn"
        );
    }

    /// A seat is a role in one group: the same coach-only account, joined as an
    /// athlete in someone else's group, is that turn's subject as usual.
    #[tokio::test]
    #[serial]
    async fn a_coach_only_account_joined_as_an_athlete_stays_the_subject() {
        let (resources, requests) = harness().await;
        let (owner, tenant) = create_user(&resources, "seat-owner-athlete@example.com").await;
        let (invited, _) = create_user(&resources, "seat-invited-coach@example.com").await;
        mark_coach_only(&resources, invited.id).await;
        let group_id = create_group(&resources, tenant, owner.id, None, &[invited.id]).await;

        let prompts = group_turn_prompts(&resources, &requests, &invited, group_id).await;

        assert!(
            !prompts.contains(COACH_SEAT_MARKER),
            "an athlete member must never be put in the coach's seat"
        );
        assert!(
            prompts.contains(OWN_ACTIVITIES_DECLARED),
            "an athlete keeps the tool that reads their own activities"
        );
        assert!(
            prompts.contains(OWN_PROVIDERS_MARKER),
            "an athlete member's own data must reach their turn"
        );
    }

    /// Friends' groups have an owner too: an athlete who created one is still
    /// asking about their own training.
    #[tokio::test]
    #[serial]
    async fn an_athlete_who_owns_a_peer_group_stays_the_subject() {
        let (resources, requests) = harness().await;
        let (owner, tenant) = create_user(&resources, "seat-peer-owner@example.com").await;
        let (friend, _) = create_user(&resources, "seat-peer-friend@example.com").await;
        let group_id = create_group(&resources, tenant, owner.id, None, &[friend.id]).await;

        let prompts = group_turn_prompts(&resources, &requests, &owner, group_id).await;

        assert!(
            !prompts.contains(COACH_SEAT_MARKER),
            "owning a peer group does not make an athlete its coach"
        );
        assert!(
            prompts.contains(OWN_PROVIDERS_MARKER),
            "a peer-group owner's own data must reach their turn"
        );
    }

    /// The recorded human coach holds the seat whatever their onboarding said,
    /// and the directive counts the athletes who joined.
    #[tokio::test]
    #[serial]
    async fn the_recorded_coach_is_told_how_many_athletes_joined() {
        let (resources, requests) = harness().await;
        let (coach, tenant) = create_user(&resources, "seat-recorded-coach@example.com").await;
        let (athlete, _) = create_user(&resources, "seat-joined-athlete@example.com").await;
        let group_id =
            create_group(&resources, tenant, coach.id, Some(coach.id), &[athlete.id]).await;

        let prompts = group_turn_prompts(&resources, &requests, &coach, group_id).await;

        assert!(prompts.contains(COACH_SEAT_MARKER));
        assert!(
            prompts.contains("Athletes in this group: 1."),
            "the coach's seat must count the joined athlete"
        );
        assert!(!prompts.contains(PENDING_MARKER));
        assert!(
            !prompts.contains(OWN_PROVIDERS_MARKER),
            "a coach who trains still keeps their own data out of their group"
        );
        assert!(
            !prompts.contains(OWN_ACTIVITIES_DECLARED),
            "a coach who also trains does not get their own activities tool in the group"
        );
    }
}
