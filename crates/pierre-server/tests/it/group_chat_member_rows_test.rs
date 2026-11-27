// ABOUTME: A coach's own group chat replays a member's tool results, replies and summaries only while that member still shares
// ABOUTME: Each row names the member it read; withdrawal and leaving take it out of the prompt, account deletion out of the database

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1174. A human coach asks the agent about the athletes of a group
//! they coach, in their own copy of the group's chat. Each tool round that
//! read an athlete, the reply drawn from it and a summary of those name that
//! athlete (`about_user_id`), so:
//!
//! - once the athlete stops sharing with the coach, or leaves the group, the
//!   coach's next turn replays none of it, while another, still-sharing
//!   athlete's rows stay;
//! - the history load filters in its statement, so its `LIMIT` counts only
//!   the rows the turn may replay;
//! - a compaction block covers rows carrying reads and is read by theirs;
//! - the athlete's account deletion takes every such row with it.
//!
//! The pipeline tests drive real turns through `pierre_chat_pipeline::run`
//! against a model that calls `get_group_member_plan_status` and records
//! what it was sent; they read what the model saw, never the source.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::permissions::scopes::OAuthScope;
use std::collections::{BTreeSet, HashMap};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use futures_util::stream;
use pierre_chat_pipeline::recorders::ChatRepoToolMessageRecorder;
use pierre_chat_pipeline::TurnOrigin;
use pierre_core::config::CompactionConfig;
use pierre_core::errors::AppError;
use pierre_core::models::{AddMessageParams, CarriedRead, MessageRecord, TenantId};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::{InsertCompactionBlockParams, MemberReplay};
use pierre_database::RepositoryRegistry;
use pierre_llm::{
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
    StreamChunk, TokenUsage,
};
use pierre_services::conversation_compaction::{
    CompactionContext, CompactionOutcome, ConversationCompactor,
};
use pierre_test_support::account::delete_account;
use pierre_tool_runtime::tool_loop_io::{ToolMessageRecorder, ToolRoundRecord};
use tokio::time::sleep;
use uuid::Uuid;

use crate::db_fixtures::{create_test_db, seed_user};

const MODEL: &str = "group-chat-rows-model";

/// Real turns in a coach's room: the model reads each athlete through
/// `get_group_member_plan_status`, which only the group tools register.
#[cfg(feature = "tools-groups")]
mod turns {
    use super::*;
    use dravr_tronc::dates::Elapsed;

    use dravr_tronc::dates::{CivilDate, Timestamp};
    use embacle::types::ToolCallRequest;
    use pierre_chat_pipeline::{
        ChatPipelineContext, InputSource, PipelineHooks, QuotaState, SurfaceId, SurfaceProfile,
        SurfaceRequest, TurnInput,
    };
    use pierre_core::models::groups::{
        CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    };
    use pierre_core::models::{ConnectionType, ConversationTurnId};
    use pierre_llm::MessageRole;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_providers::ai_scope::{self, Provenance};
    use pierre_test_support::delegation::{create_group_agent, grant_coach_access};
    use pierre_tool_runtime::tool_access::CallerRole;
    use serde_json::{json, Value};

    use crate::common::{
        create_test_server_resources_with_chat_provider, create_test_user_with_plan,
        generate_test_token,
    };
    use crate::helpers::axum_test::AxumTestRequest;
    use axum::http::StatusCode;
    use pierre_core::models::COMMAND_FINISH_REASON;
    use pierre_mcp_server::routes::chat::ChatRoutes;

    const REPLY: &str = "Noted.";
    const ALICE_NAME: &str = "Alice Archer";
    const BRUNO_NAME: &str = "Bruno Baker";
    const ALICE_RACE: &str = "Alice Spring Marathon";
    const BRUNO_RACE: &str = "Bruno Autumn Ultra";
    /// The coach's asks name nobody, so no peer-mention grounding reads anyone:
    /// the model alone decides whom the turn reads.
    const ASK_ONE: &str = "status check one";
    const ASK_TWO: &str = "status check two";
    const LATER: &str = "what should I look at next?";
    const STATUS_TOOL: &str = "get_group_member_plan_status";

    /// Records every call's non-system messages. On the newest of [`ASK_ONE`] or
    /// [`ASK_TWO`], it reads that athlete's plan status once, then answers.
    struct CoachModel {
        conversations: Mutex<Vec<String>>,
        models: Vec<String>,
    }

    impl CoachModel {
        fn new() -> Self {
            Self {
                conversations: Mutex::new(Vec::new()),
                models: vec![MODEL.to_owned()],
            }
        }

        fn calls(&self) -> usize {
            self.conversations.lock().unwrap().len()
        }

        /// Every non-system message of the calls after `from`, joined.
        fn conversation_since(&self, from: usize) -> String {
            self.conversations
                .lock()
                .unwrap()
                .iter()
                .skip(from)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    /// The member the newest ask names, unless the status it asked for already
    /// came back after it.
    fn member_to_read(messages: &[ChatMessage]) -> Option<&'static str> {
        let (at, member) = messages.iter().enumerate().rev().find_map(|(i, m)| {
            if m.content.contains(ASK_ONE) {
                Some((i, "Alice"))
            } else if m.content.contains(ASK_TWO) {
                Some((i, "Bruno"))
            } else {
                None
            }
        })?;
        let answered = messages[at + 1..].iter().any(|m| {
            m.content
                .contains(&format!("[Tool Result for {STATUS_TOOL}]"))
        });
        (!answered).then_some(member)
    }

    #[async_trait]
    impl LlmProvider for CoachModel {
        fn name(&self) -> &'static str {
            "group_chat_rows_mock"
        }
        fn display_name(&self) -> &'static str {
            "Group-chat rows mock (carnet#1174)"
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
            let conversation = request
                .messages
                .iter()
                .filter(|m| !matches!(m.role, MessageRole::System))
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            self.conversations.lock().unwrap().push(conversation);
            let tool_calls = member_to_read(&request.messages).map(|member| {
                vec![ToolCallRequest {
                    id: format!("call-{member}"),
                    function_name: STATUS_TOOL.to_owned(),
                    arguments: json!({ "member": member }),
                }]
            });
            let content = if tool_calls.is_some() {
                String::new()
            } else {
                REPLY.to_owned()
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

    #[derive(Debug, Clone)]
    struct Person {
        id: Uuid,
        tenant: TenantId,
        auth: String,
    }

    async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
        let email = format!("{label}-{}@group-chat-rows.test", Uuid::new_v4());
        let (id, user, tenant) =
            create_test_user_with_plan(&res.agent.database, &email, "professional")
                .await
                .unwrap();
        let repos = &res.common.repos;
        repos.users.update_display_name(id, name).await.unwrap();
        repos.email_verification.mark_verified(id).await.unwrap();
        let token = generate_test_token(res, &user).await;
        Person {
            id,
            tenant,
            auth: format!("Bearer {token}"),
        }
    }

    fn monday() -> CivilDate {
        let today = Timestamp::now().utc_date();
        today - Elapsed::days(i64::from(today.weekday().num_days_from_monday()))
    }

    fn season(race: &str) -> Value {
        let start = monday();
        let days: Vec<Value> = (0..7)
            .map(|offset| {
                json!({
                    "date": (start + Elapsed::days(offset)).to_string(),
                    "sport": "run",
                    "workout": "easy aerobic run",
                    "duration_min": 40,
                    "intensity": "Z2",
                })
            })
            .collect();
        json!({
            "outline": {
                "goal_race": { "name": race, "date": "2027-05-16", "discipline": "run", "priority": "A" },
                "strategy": format!("build toward {race}"),
            },
            "weeks": [{
                "week_start": start.to_string(),
                "focus": "aerobic base",
                "days": days,
            }],
        })
    }

    /// A group its owner owns and the coach coaches, Alice and Bruno consenting
    /// members with a published season each, and the coach's own conversation in
    /// the group's room.
    struct Fixture {
        res: Arc<ServerContext>,
        ctx: ChatPipelineContext,
        model: Arc<CoachModel>,
        coach: Person,
        alice: Person,
        bruno: Person,
        group: CoachingGroup,
        room: String,
    }

    impl Fixture {
        async fn new() -> Self {
            use dravr_tronc::mcp::tool::{McpTool, ToolContext};
            use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
            use pierre_tool_runtime::runtime::ToolRuntime;

            let model = Arc::new(CoachModel::new());
            let res = create_test_server_resources_with_chat_provider(model.clone())
                .await
                .unwrap();
            let owner = person(&res, "owner", "Olive Owner").await;
            let coach = person(&res, "coach", "Casey Coach").await;
            let alice = person(&res, "alice", ALICE_NAME).await;
            let bruno = person(&res, "bruno", BRUNO_NAME).await;
            let repos = &res.common.repos;
            let agent_id = create_group_agent(repos, owner.id, owner.tenant)
                .await
                .unwrap();
            let now = Timestamp::now();
            let group = repos
                .groups
                .create_group(
                    owner.tenant,
                    &CoachingGroup {
                        id: Uuid::new_v4(),
                        tenant_id: owner.tenant.to_string(),
                        name: "Long Run Club".to_owned(),
                        description: None,
                        agent_id,
                        owner_id: owner.id,
                        coach_user_id: None,
                        peer_data_sharing: false,
                        respond_mode: GroupRespondMode::default(),
                        digest_mode: GroupDigestMode::Off,
                        max_members: 20,
                        is_active: true,
                        channel_type: None,
                        channel_chat_id: None,
                        chat_connection: None,
                        created_at: now,
                        updated_at: now,
                    },
                )
                .await
                .unwrap();
            grant_coach_access(repos, coach.id).await.unwrap();
            assert!(
                repos
                    .groups
                    .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
                    .await
                    .unwrap(),
                "fixture: the coach is attached to the group"
            );
            for (athlete, race) in [(&alice, ALICE_RACE), (&bruno, BRUNO_RACE)] {
                repos
                    .groups
                    .add_member(&GroupMember {
                        id: Uuid::new_v4(),
                        group_id: group.id,
                        user_id: athlete.id,
                        tenant_id: athlete.tenant.to_string(),
                        role: GroupRole::Member,
                        peer_sharing_consent: false,
                        coach_sharing_consent: true,
                        coach_sharing_consent_at: None,
                        coach_consents: CoachConsents::default(),
                        consent_given_at: now,
                        joined_at: now,
                        left_at: None,
                        display_name: None,
                    })
                    .await
                    .unwrap();
                let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&res);
                let ctx = ToolContext::new()
                    .with_user(athlete.id.to_string())
                    .with_tenant(athlete.tenant.to_string())
                    .with_auth_method("jwt_bearer");
                let saved = SaveTrainingPlanTool
                    .execute(&runtime, &ctx, season(race))
                    .await
                    .structured_content
                    .expect("the save answers with structured content");
                assert!(
                    saved.get("error").is_none(),
                    "fixture: {race} is saved: {saved}"
                );
            }
            // A connected source, so no onboarding step stands before the turn.
            repos
                .provider_connections
                .register_connection(
                    coach.id,
                    coach.tenant,
                    "synthetic",
                    &ConnectionType::Synthetic,
                    None,
                )
                .await
                .unwrap();
            let room = repos
                .chat
                .create_conversation(
                    &coach.id.to_string(),
                    coach.tenant,
                    "Long Run Club",
                    MODEL,
                    None,
                    Some(&group.id.to_string()),
                )
                .await
                .unwrap()
                .id;
            let ctx = res.chat_pipeline_context();
            Self {
                res,
                ctx,
                model,
                coach,
                alice,
                bruno,
                group,
                room,
            }
        }

        fn repos(&self) -> &RepositoryRegistry {
            &self.res.common.repos
        }

        /// One coach turn in the room: every non-system message the model was
        /// sent on it. The turn runs under its own provenance and over the web
        /// app, as the turn service runs a web turn: the reply is stamped from
        /// it, and the coach tools run only on Dravr's own surfaces.
        async fn turn(&self, content: &str) -> String {
            let from = self.model.calls();
            let profile = SurfaceProfile::resolve(&SurfaceRequest {
                surface: SurfaceId::Web,
                locale: "en".to_owned(),
                transport: None,
                prose_contract: None,
            });
            let input = TurnInput {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                conversation_id: self.room.clone(),
                user_id: self.coach.id.to_string(),
                conversation_tenant_id: self.coach.tenant,
                tool_tenant_id: self.coach.tenant,
                is_direct_message: false,
                content: content.to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                quote: None,
                quota: QuotaState::Ok,
                mentioned_agent: None,
                steps_after: None,
                grant: OAuthScope::self_grant(),
                // The role the web route resolves for the coach's turn.
                role: CallerRole::resolve(self.repos(), self.coach.id).await,
            };
            ai_scope::serve_over(
                Transport::WebApp,
                ai_scope::tracking(
                    Provenance::new(),
                    pierre_chat_pipeline::run(&self.ctx, input, &profile, &PipelineHooks::none()),
                ),
            )
            .await
            .expect("the turn completes");
            self.model.conversation_since(from)
        }

        /// The room's rows, oldest first.
        async fn rows(&self) -> Vec<MessageRecord> {
            self.repos()
                .chat
                .get_messages(&self.room, &self.coach.id.to_string(), self.coach.tenant)
                .await
                .unwrap()
        }

        /// Wait for the tool rounds the turns wrote: their rows are written off
        /// the turn's path.
        async fn tool_results(&self, count: usize) -> Vec<MessageRecord> {
            for _ in 0..100 {
                let results: Vec<MessageRecord> = self
                    .rows()
                    .await
                    .into_iter()
                    .filter(|row| row.role == "tool_result")
                    .collect();
                if results.len() >= count {
                    return results;
                }
                sleep(StdDuration::from_millis(20)).await;
            }
            panic!("the room never held {count} tool results");
        }

        /// The coach reads Alice, then Bruno; the second turn replays Alice's
        /// status while she shares.
        async fn read_both(&self) {
            self.turn(ASK_ONE).await;
            self.tool_results(1).await;
            let second = self.turn(ASK_TWO).await;
            assert!(
                second.contains(ALICE_RACE),
                "premise: a sharing athlete's earlier status is replayed: {second}"
            );
            self.tool_results(2).await;
        }

        /// The one member each of `conversation`'s rows carries a read of,
        /// by row id: `None` for a row carrying none or several
        /// (carnet#1511 call 19: the link rows are the one carrier).
        async fn sole_reads(&self, conversation: &str) -> HashMap<String, Option<Uuid>> {
            let mut members: HashMap<String, BTreeSet<Uuid>> = HashMap::new();
            for read in self
                .repos()
                .chat
                .list_member_reads(conversation, &self.coach.id.to_string(), self.coach.tenant)
                .await
                .unwrap()
            {
                if let Some(carried) = CarriedRead::from_key(&read.key) {
                    members
                        .entry(read.message_id)
                        .or_default()
                        .insert(carried.member);
                }
            }
            members
                .into_iter()
                .map(|(row, members)| {
                    let sole = (members.len() == 1)
                        .then(|| members.into_iter().next())
                        .flatten();
                    (row, sole)
                })
                .collect()
        }

        /// The id of the coach's row that says `content`.
        async fn question_id(&self, content: &str) -> String {
            self.rows()
                .await
                .into_iter()
                .find(|row| row.role == "user" && row.content == content)
                .unwrap_or_else(|| panic!("the coach's question {content:?}"))
                .id
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    // Stamps
    // ════════════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn each_round_and_reply_names_the_athlete_it_read() {
        let fx = Fixture::new().await;
        fx.read_both().await;

        let rows = fx.rows().await;
        let sole = fx.sole_reads(&fx.room).await;
        let about = |role: &str| -> Vec<(Option<Uuid>, bool)> {
            rows.iter()
                .filter(|row| row.role == role)
                .map(|row| {
                    (
                        sole.get(&row.id).copied().flatten(),
                        row.content.contains(ALICE_RACE),
                    )
                })
                .collect()
        };
        assert_eq!(
            about("tool_result"),
            vec![(Some(fx.alice.id), true), (Some(fx.bruno.id), false)],
            "each status names the athlete it read"
        );
        assert_eq!(
            about("assistant")
                .into_iter()
                .map(|(member, _)| member)
                .collect::<Vec<_>>(),
            vec![None, None],
            "each reply was drawn from a prompt carrying both athletes' cards, so \
             neither stamp names one of them alone (carnet#1511 design call 4)"
        );
        let reads = fx
            .repos()
            .chat
            .list_member_reads(&fx.room, &fx.coach.id.to_string(), fx.coach.tenant)
            .await
            .unwrap();
        for reply in rows.iter().filter(|row| row.role == "assistant") {
            assert_eq!(
                reads
                    .iter()
                    .filter(|read| read.message_id == reply.id)
                    .filter_map(|read| CarriedRead::from_key(&read.key).map(|read| read.member))
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([fx.alice.id, fx.bruno.id]),
                "a reply drawn from both cards, and the second one's reads, is about \
                 both athletes (carnet#1422, carnet#1511)"
            );
        }
        assert!(
            rows.iter()
                .filter(|row| row.role == "user")
                .all(|row| row.about_user_id.is_none()),
            "the coach's own questions are about nobody"
        );
        assert_eq!(
            fx.repos()
                .chat
                .list_member_reads(&fx.room, &fx.coach.id.to_string(), fx.coach.tenant)
                .await
                .unwrap()
                .into_iter()
                .filter_map(|read| CarriedRead::from_key(&read.key).map(|read| read.member))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([fx.alice.id, fx.bruno.id]),
            "the room's rows are about the two athletes the coach read"
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // Withdrawal
    // ════════════════════════════════════════════════════════════════════════

    /// Bruno, read after Alice, withdraws: his rows go, and Alice's, which
    /// came before any read of him, stay. Alice withdrawing instead would
    /// take Bruno's rows too: the turn that read him replayed her status, so
    /// what it wrote may restate her (carnet#1422).
    #[tokio::test]
    async fn a_revoked_athlete_is_not_replayed_while_a_sharing_one_is() {
        let fx = Fixture::new().await;
        fx.read_both().await;

        assert!(
            fx.repos()
                .groups
                .update_coach_sharing_consent(&fx.group.id.to_string(), fx.bruno.id, false)
                .await
                .unwrap(),
            "premise: Bruno's membership takes the revocation"
        );
        let later = fx.turn(LATER).await;

        assert!(
            !later.contains(BRUNO_RACE),
            "Bruno stopped sharing: his earlier status must not reach the model: {later}"
        );
        assert!(
            later.contains(ALICE_RACE),
            "Alice still shares: her status, read before any of Bruno's, stays: {later}"
        );
        assert!(
            later.contains(ASK_ONE),
            "the coach's own question stays in the thread: {later}"
        );

        assert!(
            fx.repos()
                .groups
                .update_coach_sharing_consent(&fx.group.id.to_string(), fx.bruno.id, true)
                .await
                .unwrap(),
            "Bruno's coach-sharing consent is granted"
        );
        assert!(
            fx.repos()
                .groups
                .update_coach_sharing_consent(&fx.group.id.to_string(), fx.alice.id, false)
                .await
                .unwrap(),
            "Alice's coach-sharing consent is withdrawn"
        );
        let restated = fx.turn(LATER).await;
        assert!(
            !restated.contains(ALICE_RACE) && !restated.contains(BRUNO_RACE),
            "Alice withdrew: nothing from her first read on reaches the model, the turn \
             that replayed her included: {restated}"
        );
    }

    #[tokio::test]
    async fn an_athlete_who_left_is_not_replayed_while_one_who_stayed_is() {
        let fx = Fixture::new().await;
        fx.read_both().await;

        assert!(
            fx.repos()
                .groups
                .remove_member(&fx.group.id.to_string(), fx.bruno.id)
                .await
                .unwrap(),
            "premise: Bruno leaves the group"
        );
        let later = fx.turn(LATER).await;

        assert!(
            !later.contains(BRUNO_RACE),
            "Bruno left the group: his earlier status must not reach the model: {later}"
        );
        assert!(
            later.contains(ALICE_RACE),
            "Alice stayed: her status, read before any of Bruno's, stays: {later}"
        );
    }

    /// A coach still attached whose coach access was withdrawn is replayed no
    /// athlete's rows (carnet#1511): the rows stay, the read withholds them,
    /// and they are replayed again once access is back.
    #[tokio::test]
    async fn a_coach_whose_access_is_off_is_replayed_no_athletes_rows() {
        let fx = Fixture::new().await;
        fx.read_both().await;

        let users = &fx.repos().users;
        users.set_manages_roster(fx.coach.id, false).await.unwrap();
        let later = fx.turn(LATER).await;

        for race in [ALICE_RACE, BRUNO_RACE] {
            assert!(
                !later.contains(race),
                "bug: {race} reached the model for a coach whose coach access is off: {later}"
            );
        }
        assert!(
            later.contains(ASK_ONE),
            "the coach's own question stays in the thread: {later}"
        );

        users.set_manages_roster(fx.coach.id, true).await.unwrap();
        let back = fx.turn(LATER).await;
        for race in [ALICE_RACE, BRUNO_RACE] {
            assert!(
                back.contains(race),
                "access is back: {race} is replayed again: {back}"
            );
        }
    }

    /// No turn has compacted the room yet, so each summary is seeded over one
    /// of the coach's questions: a row that stays in the window whoever is
    /// withdrawn, so only the summary's own stamp can keep it out.
    #[tokio::test]
    async fn a_summary_about_a_revoked_athlete_is_not_replayed() {
        const ALICE_SUMMARY: &str = "Earlier: Alice's long runs held 5:10/km.";
        const BRUNO_SUMMARY: &str = "Earlier: Bruno's hill block went well.";
        let fx = Fixture::new().await;
        fx.read_both().await;
        for (question, summary, member) in [
            (ASK_ONE, ALICE_SUMMARY, fx.alice.id),
            (ASK_TWO, BRUNO_SUMMARY, fx.bruno.id),
        ] {
            let id = fx.question_id(question).await;
            fx.repos()
                .memory
                .insert_compaction_block(&InsertCompactionBlockParams {
                    tenant_id: fx.coach.tenant,
                    conversation_id: &fx.room,
                    summary,
                    summary_tokens: 10,
                    original_tokens: 40,
                    first_message_id: &id,
                    last_message_id: &id,
                    transport_policy: TransportPolicy::AnyTransport,
                    about_user_id: Some(member),
                    ai_consent_sources: &[],
                })
                .await
                .unwrap();
        }
        let shared = fx.turn(LATER).await;
        assert!(
            shared.contains(ALICE_SUMMARY) && shared.contains(BRUNO_SUMMARY),
            "premise: both summaries replay while both athletes share: {shared}"
        );

        assert!(
            fx.repos()
                .groups
                .update_coach_sharing_consent(&fx.group.id.to_string(), fx.bruno.id, false)
                .await
                .unwrap(),
            "premise: Bruno's membership takes the revocation"
        );
        let later = fx.turn(LATER).await;

        assert!(
            !later.contains(BRUNO_SUMMARY),
            "a summary of Bruno's rows goes where his rows go: {later}"
        );
        assert!(
            later.contains(ALICE_SUMMARY),
            "a summary of Alice's rows, read before any of Bruno's, stays while she shares: \
             {later}"
        );
    }

    /// `/plan <athlete>` answers the coach alone with the athlete's plan, in
    /// the coach's own conversation: the reply names her too.
    #[tokio::test]
    async fn a_plan_command_reply_names_the_athlete_it_read() {
        let fx = Fixture::new().await;
        let dm = fx
            .repos()
            .chat
            .create_conversation(
                &fx.coach.id.to_string(),
                fx.coach.tenant,
                "Coach",
                MODEL,
                None,
                None,
            )
            .await
            .unwrap()
            .id;

        let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{dm}/messages"))
            .header("authorization", &fx.coach.auth)
            .json(&json!({ "content": "/plan Alice" }))
            .send(ChatRoutes::routes(Arc::clone(&fx.res)))
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::OK,
            "the coach's /plan command is answered"
        );

        let rows = fx
            .repos()
            .chat
            .get_messages(&dm, &fx.coach.id.to_string(), fx.coach.tenant)
            .await
            .unwrap();
        let reply = rows
            .iter()
            .find(|row| {
                row.role == "assistant"
                    && row.finish_reason.as_deref() == Some(COMMAND_FINISH_REASON)
            })
            .expect("the command's reply");
        assert!(
            reply.content.contains(ALICE_RACE),
            "premise: the reply is Alice's plan: {}",
            reply.content
        );
        assert_eq!(
            fx.sole_reads(&dm).await.get(&reply.id).copied().flatten(),
            Some(fx.alice.id),
            "the reply carries the read of Alice, so it goes with her account"
        );
        assert!(
            rows.iter().all(|row| row.about_user_id.is_none()),
            "bug: a new row names no member in about_user_id"
        );
        assert!(
            rows.iter()
                .filter(|row| row.role == "user")
                .all(|row| row.about_user_id.is_none()),
            "the coach's command line is about nobody"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════
// The history load
// ════════════════════════════════════════════════════════════════════════

fn message<'a>(
    tenant: TenantId,
    conversation: &'a str,
    user: &'a str,
    role: &'a str,
    content: &'a str,
    about: Option<Uuid>,
) -> AddMessageParams<'a> {
    AddMessageParams {
        tenant_id: tenant,
        conversation_id: conversation,
        user_id: user,
        role,
        content,
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
        content_blocks: None,
        transport_policy: TransportPolicy::AnyTransport,
        channel_type: None,
        about_user_id: about,
        turn_attempted_write: None,
        ai_consent_sources: &[],
    }
}

/// Rows about an athlete written after the rest fill the newest page; a turn
/// that may not read her still gets `limit` rows, because the rule is in the
/// statement, before its `LIMIT`.
#[tokio::test]
async fn a_limit_counts_only_rows_about_readable_athletes() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (coach, tenant) = seed_user(&db).await;
    let (alice, _) = seed_user(&db).await;
    let coach = coach.to_string();
    let room = repos
        .chat
        .create_conversation(&coach, tenant, "Room", MODEL, None, None)
        .await
        .unwrap();
    for (prefix, about) in [("open", None), ("alice", Some(alice))] {
        for n in 0..3 {
            let content = format!("{prefix} {n}");
            repos
                .chat
                .add_message(&message(
                    tenant,
                    &room.id,
                    &coach,
                    "tool_result",
                    &content,
                    about,
                ))
                .await
                .unwrap();
            sleep(StdDuration::from_millis(5)).await;
        }
    }
    let contents = |rows: Vec<MessageRecord>| -> Vec<String> {
        rows.into_iter().map(|row| row.content).collect()
    };

    let unreadable = repos
        .chat
        .get_recent_prompt_messages(
            &room.id,
            &coach,
            tenant,
            2,
            TransportPolicy::FirstPartyOnly,
            MemberReplay::about_nobody(),
        )
        .await
        .unwrap();
    assert_eq!(
        contents(unreadable),
        vec!["open 1", "open 2"],
        "two rows the turn may replay, though three newer ones are about Alice"
    );

    let readable = repos
        .chat
        .get_recent_prompt_messages(
            &room.id,
            &coach,
            tenant,
            2,
            TransportPolicy::FirstPartyOnly,
            MemberReplay {
                readable_about: &[alice],
                ..MemberReplay::about_nobody()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        contents(readable),
        vec!["alice 1", "alice 2"],
        "a turn that may read Alice reads her newest rows"
    );
    assert!(
        readable_rows_name_alice(&repos, &room.id, &coach, tenant, alice).await,
        "a row read back names the athlete it was written about"
    );

    let transcript = repos
        .chat
        .get_recent_messages(&room.id, &coach, tenant, 2, TransportPolicy::FirstPartyOnly)
        .await
        .unwrap();
    assert_eq!(
        contents(transcript),
        vec!["alice 1", "alice 2"],
        "the transcript itself is the coach's record and keeps every row"
    );
    assert_eq!(
        repos
            .chat
            .list_member_reads(&room.id, &coach, tenant)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|read| CarriedRead::legacy_about_member(&read.key))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([alice]),
        "the room's rows, written with a legacy stamp, are about Alice alone"
    );
}

async fn readable_rows_name_alice(
    repos: &RepositoryRegistry,
    room: &str,
    coach: &str,
    tenant: TenantId,
    alice: Uuid,
) -> bool {
    repos
        .chat
        .get_messages(room, coach, tenant)
        .await
        .unwrap()
        .iter()
        .filter(|row| row.content.starts_with("alice"))
        .all(|row| row.about_user_id == Some(alice))
}

// ════════════════════════════════════════════════════════════════════════
// Tool rounds and compaction
// ════════════════════════════════════════════════════════════════════════

/// A platform-composed turn holds its rounds and writes them on
/// `persist_held`, so what the recorder kept can be read back at once.
#[tokio::test]
async fn every_round_is_kept_with_every_read_it_made_in_the_link_rows() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (coach, tenant) = seed_user(&db).await;
    let (alice, _) = seed_user(&db).await;
    let (bruno, _) = seed_user(&db).await;
    let (coach_id, group) = (coach, Uuid::new_v4());
    let coach = coach.to_string();
    let room = repos
        .chat
        .create_conversation(&coach, tenant, "Room", MODEL, None, None)
        .await
        .unwrap();
    let recorder = ChatRepoToolMessageRecorder::new(
        Arc::clone(&repos.chat),
        room.id.clone(),
        coach.clone(),
        tenant,
        TurnOrigin::Platform,
    );
    recorder.record(ToolRoundRecord {
        assistant_text: "Comparing both.".to_owned(),
        tool_result_text: "[Tool Result for get_roster_overview]: both athletes".to_owned(),
        members_read: BTreeSet::from([
            CarriedRead::coach(alice, group, coach_id),
            CarriedRead::coach(bruno, group, coach_id),
        ]),
    });
    recorder.record(ToolRoundRecord {
        assistant_text: "Reading Alice.".to_owned(),
        tool_result_text: "[Tool Result for get_group_member_plan_status]: Alice".to_owned(),
        members_read: BTreeSet::from([CarriedRead::coach(alice, group, coach_id)]),
    });
    recorder.persist_held().await;

    let rows = repos
        .chat
        .get_messages(&room.id, &coach, tenant)
        .await
        .unwrap();
    let reads = repos
        .chat
        .list_member_reads(&room.id, &coach, tenant)
        .await
        .unwrap();
    let carried = |row: &MessageRecord| -> BTreeSet<String> {
        reads
            .iter()
            .filter(|read| read.message_id == row.id)
            .map(|read| read.key.clone())
            .collect()
    };
    let both = BTreeSet::from([
        CarriedRead::coach(alice, group, coach_id).key(),
        CarriedRead::coach(bruno, group, coach_id).key(),
    ]);
    let alice_only = BTreeSet::from([CarriedRead::coach(alice, group, coach_id).key()]);
    let kept: Vec<(&str, BTreeSet<String>)> = rows
        .iter()
        .map(|row| (row.content.as_str(), carried(row)))
        .collect();
    assert_eq!(
        kept,
        vec![
            ("Comparing both.", both.clone()),
            ("[Tool Result for get_roster_overview]: both athletes", both),
            ("Reading Alice.", alice_only.clone()),
            (
                "[Tool Result for get_group_member_plan_status]: Alice",
                alice_only
            ),
        ],
        "a round about two athletes is kept, carrying both reads as they were made, in the \
         one carrier (carnet#1511 call 19)"
    );
    assert!(
        rows.iter().all(|row| row.about_user_id.is_none()),
        "bug: a new row names no member in about_user_id"
    );
}

/// What the stub summarizer returns, and the text it was asked to summarize.
#[derive(Default)]
struct StubSummarizer {
    asked: Mutex<Vec<String>>,
}

#[async_trait]
impl LlmProvider for StubSummarizer {
    fn name(&self) -> &'static str {
        "stub-summarizer"
    }
    fn display_name(&self) -> &'static str {
        "stub-summarizer"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "stub-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }
    async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.asked.lock().unwrap().push(
            request
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        Ok(ChatResponse {
            replay_blocks: None,
            content: "SUMMARY of the earlier turns.".to_owned(),
            usage: Some(TokenUsage::new(1, 1, 2)),
            model: "stub-model".to_owned(),
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        let chunk = StreamChunk {
            delta: "SUMMARY of the earlier turns.".to_owned(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Pin::from(Box::new(stream::iter(vec![Ok(chunk)]))))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// carnet#1511 design A18 (round-12 still-works SW1): a row carrying a
/// member read is covered like any other. In a coach's thread about one
/// athlete every reply carries the athlete's read, and refusing such rows
/// left no run to summarize, so the thread slid its oldest turns away with
/// no summary once over the cap (registre#198). The block is read by its
/// covered rows' reads, which stay recorded on those rows: the replay
/// splices it only over rows it replays, and the export withholds it with
/// any row it covers.
#[tokio::test]
async fn a_thread_whose_every_reply_carries_a_read_still_compacts() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (coach, tenant) = seed_user(&db).await;
    let room = repos
        .chat
        .create_conversation(&coach.to_string(), tenant, "Alice", MODEL, None, None)
        .await
        .unwrap();
    let rows = [
        ("u1", "how is she?"),
        ("a1", "Alice's status: 5:10/km, on track."),
        ("u2", "and her long run?"),
        ("a2", "Alice ran 24 km on Sunday."),
        ("u3", "anything else?"),
        ("a3", "Alice slept well."),
        ("u4", "thanks"),
        ("a4", "Alice is welcome."),
        ("u5", "bye"),
    ];
    let mut llm_messages = vec![ChatMessage::system("system prompt")];
    let mut source_ids = vec![None];
    for (id, content) in rows {
        llm_messages.push(if id.starts_with('a') {
            ChatMessage::assistant(content)
        } else {
            ChatMessage::user(content)
        });
        source_ids.push(Some(id.to_owned()));
    }
    let summarizer = Arc::new(StubSummarizer::default());
    let provider = ChatProvider::Custom(summarizer.clone());
    let config = CompactionConfig {
        max_messages: 4,
        summarize_oldest_n: 4,
        ..CompactionConfig::default()
    };
    let outcome = ConversationCompactor::new(config)
        .compact_if_needed(CompactionContext {
            repo: repos.memory.as_ref(),
            provider: &provider,
            summary_prompt: "Summarize the conversation.",
            recorder: None,
            tenant_id: tenant,
            conversation_id: &room.id,
            source_ids: &source_ids,
            source_consents: &vec![Vec::new(); source_ids.len()],
            llm_messages: &mut llm_messages,
            summary_for_prompt: str::to_owned,
        })
        .await
        .unwrap();
    let CompactionOutcome::Summarized { block, .. } = outcome else {
        panic!("bug: a thread whose every reply carries a read is not summarized: {outcome:?}");
    };
    assert_eq!(
        block.first_message_id, "u1",
        "the block opens on the oldest turn"
    );
    assert_ne!(
        block.last_message_id, "u1",
        "bug: the block stopped at the first reply carrying a read"
    );
    assert_eq!(block.about_user_id, None, "a block names no member itself");
    let asked = summarizer.asked.lock().unwrap().join("\n");
    assert!(
        asked.contains("5:10/km"),
        "the covered replies are summarized: {asked}"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Account deletion
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn deleting_an_athlete_deletes_the_coachs_rows_about_them() {
    let db = create_test_db().await;
    let repos: Arc<RepositoryRegistry> = Arc::clone(db.repositories());
    let (coach, tenant) = seed_user(&db).await;
    let (alice, _) = seed_user(&db).await;
    let (bruno, _) = seed_user(&db).await;
    let coach = coach.to_string();
    let room = repos
        .chat
        .create_conversation(&coach, tenant, "Room", MODEL, None, None)
        .await
        .unwrap();
    let mut alice_first = String::new();
    for (role, content, about) in [
        ("user", "how are they doing?", None),
        ("tool_result", "Alice's status", Some(alice)),
        ("assistant", "Alice is on track.", Some(alice)),
        ("tool_result", "Bruno's status", Some(bruno)),
    ] {
        let row = repos
            .chat
            .add_message(&message(tenant, &room.id, &coach, role, content, about))
            .await
            .unwrap();
        if about == Some(alice) && alice_first.is_empty() {
            alice_first = row.id;
        }
    }
    repos
        .memory
        .insert_compaction_block(&InsertCompactionBlockParams {
            tenant_id: tenant,
            conversation_id: &room.id,
            summary: "Alice held her paces.",
            summary_tokens: 5,
            original_tokens: 20,
            first_message_id: &alice_first,
            last_message_id: &alice_first,
            transport_policy: TransportPolicy::AnyTransport,
            about_user_id: Some(alice),
            ai_consent_sources: &[],
        })
        .await
        .unwrap();

    let deletion = delete_account(&repos, alice, None).await.unwrap();

    assert_eq!(
        deletion.removed_from("chat_messages"),
        2,
        "the coach's two rows about Alice go with her account"
    );
    assert_eq!(
        deletion.removed_from("compaction_blocks"),
        1,
        "the summary of her rows is counted with the deletion"
    );
    let left: Vec<String> = repos
        .chat
        .get_messages(&room.id, &coach, tenant)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.content)
        .collect();
    assert_eq!(
        left,
        vec!["how are they doing?", "Bruno's status"],
        "the coach's own question and Bruno's row stay in the coach's record"
    );
    assert!(
        repos
            .memory
            .list_compaction_blocks(&room.id, tenant)
            .await
            .unwrap()
            .is_empty(),
        "the summary of her rows goes too"
    );
}
