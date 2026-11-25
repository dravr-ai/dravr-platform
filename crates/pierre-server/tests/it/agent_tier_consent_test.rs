// ABOUTME: Dravr's own agent reads its memory (T3) and the athlete's PAR-Q facts (T2) only under the matching tier consent
// ABOUTME: Real in-app and messaging turns and the memory extractor, each with and without consent (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! JF 2026-11-25, "Gate on tier consent" (carnet#1544), replacing
//! 2026-11-20's "Keep reading": the agent's memory and notes (T3) and the
//! PAR-Q medical facts (T2) reach the LLM only under the athlete's matching
//! tier consent, `sensitive_t3` and `sensitive_t2`. Without it the agent
//! works without that context, and its prompt says so whatever is stored, so
//! it neither pretends to remember nor learns whether a flag is on file.
//!
//! Each path is asserted on what the model received: the system prompt of a
//! real turn on the web and on a messaging channel, and the memory
//! extractor's existing-facts list. The agent proposal's re-rank is
//! `agent_recommendations_routes_test`'s.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use pierre_chat_pipeline::{
    ChatPipelineContext, CommandPersistence, InputSource, PipelineHooks, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_core::constants::oauth::providers::{
    sensitive_display_name, SENSITIVE_T2, SENSITIVE_T3,
};
use pierre_core::constants::oauth_providers::NoticeKind;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_services::parq::persist_parq_flags;
use pierre_services::provider_notice::{grant_consent, withdraw_consent};
use uuid::Uuid;

use crate::common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};
use crate::helpers::recording_llm::RecordingProvider;

/// What the agent remembers of the athlete: a goal (T3).
const GOAL: &str = "a Boston qualifier next April";
/// Why they train: a North Star (T3).
const NORTH_STAR: &str = "keeping up with my daughters";
/// The OKF bundle's marker for a PAR-Q flag on file (T2).
const MEDICAL_FLAGS: &str = "## Medical flags";
/// The line said in place of the agent's memory.
const MEMORY_WITHHELD: &str = "keeps their notes and their agent's memory from AI";
/// The line said in place of the medical screening answers.
const MEDICAL_WITHHELD: &str = "you cannot see whether any medical flag is on file";

const REPLY: &str = "On garde la semaine facile.";
const ASK: &str = "Que fais-je demain ?";

struct Fixture {
    resources: Arc<ServerContext>,
    ctx: ChatPipelineContext,
    tenant: TenantId,
    user_id: Uuid,
    provider: Arc<RecordingProvider>,
}

async fn setup(email: &str) -> Fixture {
    let provider = Arc::new(RecordingProvider::answering(REPLY));
    let resources = create_test_server_resources_with_chat_provider(provider.clone())
        .await
        .unwrap();
    let (user_id, _user, tenant) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    let ctx = resources.chat_pipeline_context();
    Fixture {
        resources,
        ctx,
        tenant,
        user_id,
        provider,
    }
}

impl Fixture {
    /// The athlete's goal and North Star, and a PAR-Q "yes".
    async fn remember(&self) {
        let user = self.user_id.to_string();
        let memory = &self.resources.common.repos.memory;
        for (kind, code, object) in [
            (FactKind::Goal, PredicateCode::TrainingFor, GOAL),
            (FactKind::NorthStar, PredicateCode::TrainBecause, NORTH_STAR),
        ] {
            memory
                .upsert_user_fact(&UpsertUserFactParams {
                    tenant_id: self.tenant,
                    user_id: &user,
                    agent_id: None,
                    scope: MemoryScope::User,
                    kind,
                    pillar: None,
                    predicate_code: code,
                    object,
                    confidence: 0.95,
                    source: FactSource::Conversation,
                    valid_until: None,
                    source_msg_id: None,
                    transport_policy: TransportPolicy::AnyTransport,
                    ai_consent_sources: &[],
                })
                .await
                .unwrap();
        }
        let raised = persist_parq_flags(
            memory.as_ref(),
            self.tenant,
            &user,
            &["chest_pain".to_owned()],
        )
        .await
        .unwrap();
        assert_eq!(raised, 1, "premise: the PAR-Q flag is on file");
    }

    /// The athlete gives (`true`) or withdraws the consent class `class`.
    async fn consent(&self, class: &str, given: bool) {
        let repos = &self.resources.common.repos;
        let brand = sensitive_display_name(class).expect("a consent class");
        if given {
            grant_consent(
                repos,
                self.tenant.as_uuid(),
                self.user_id,
                class,
                NoticeKind::AiConsent,
                brand,
            )
            .await
            .unwrap();
        } else {
            withdraw_consent(repos, self.user_id, class, NoticeKind::AiConsent, brand)
                .await
                .unwrap();
        }
    }

    async fn conversation(&self) -> String {
        self.resources
            .common
            .repos
            .chat
            .create_conversation(
                &self.user_id.to_string(),
                self.tenant,
                "Semaine",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id
    }

    /// One turn served over `transport`; the system prompt of its coaching
    /// call (the one carrying the tool-discipline block).
    async fn prompt(&self, conversation_id: &str, transport: Transport) -> String {
        let messaging = transport == Transport::Messaging;
        let before = self.provider.calls_so_far();
        pierre_chat_pipeline::execute(
            &self.ctx,
            TurnRequest {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                conversation_id: conversation_id.to_owned(),
                user_id: self.user_id,
                conversation_tenant_id: self.tenant,
                tool_tenant_id: self.tenant,
                content: ASK.to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                quote: None,
                channel_type: if messaging { "telegram" } else { "web" },
                transport,
                is_direct_message: true,
                ambient_group_fallback: false,
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                caller: None,
                hooks: PipelineHooks::none(),
                grant: OAuthScope::self_grant(),
            },
            &SurfaceProfile::resolve(&SurfaceRequest {
                surface: if messaging {
                    SurfaceId::Telegram
                } else {
                    SurfaceId::Web
                },
                locale: "fr".to_owned(),
                transport: None,
                prose_contract: None,
            }),
        )
        .await
        .expect("the turn is served");
        self.provider
            .system_prompt_since(before, &self.ctx.tool_discipline_prompt)
    }
}

/// The two surfaces Dravr's own agent answers on: its apps and its channels.
const SURFACES: [Transport; 2] = [Transport::WebApp, Transport::Messaging];

#[tokio::test]
async fn without_either_consent_no_memory_or_parq_flag_reaches_the_prompt_and_it_says_so() {
    for transport in SURFACES {
        let f = setup(&format!("tier-none-{transport:?}@example.com")).await;
        f.remember().await;
        let conversation = f.conversation().await;
        let prompt = f.prompt(&conversation, transport).await;
        for withheld in [GOAL, NORTH_STAR, MEDICAL_FLAGS] {
            assert!(
                !prompt.contains(withheld),
                "bug: {withheld:?} reached the model without its tier consent ({transport:?})"
            );
        }
        assert!(
            prompt.contains(MEMORY_WITHHELD) && prompt.contains(MEDICAL_WITHHELD),
            "the agent is told what it cannot see, not left to guess ({transport:?})"
        );
    }
}

#[tokio::test]
async fn sensitive_t3_opens_the_memory_and_sensitive_t2_the_parq_flag() {
    for transport in SURFACES {
        let f = setup(&format!("tier-each-{transport:?}@example.com")).await;
        f.remember().await;
        let conversation = f.conversation().await;

        f.consent(SENSITIVE_T3, true).await;
        let memory_only = f.prompt(&conversation, transport).await;
        assert!(
            memory_only.contains(GOAL) && memory_only.contains(NORTH_STAR),
            "sensitive_t3 given: the agent reads its memory ({transport:?})"
        );
        assert!(
            !memory_only.contains(MEDICAL_FLAGS) && memory_only.contains(MEDICAL_WITHHELD),
            "bug: T3 opens nothing of T2 ({transport:?})"
        );
        assert!(
            !memory_only.contains(MEMORY_WITHHELD),
            "bug: sensitive_t3 given, the memory line is not said ({transport:?})"
        );

        f.consent(SENSITIVE_T2, true).await;
        let both = f.prompt(&conversation, transport).await;
        assert!(
            both.contains(GOAL) && both.contains(MEDICAL_FLAGS),
            "both given: memory and the PAR-Q flag ({transport:?})"
        );
        assert!(
            !both.contains(MEMORY_WITHHELD) && !both.contains(MEDICAL_WITHHELD),
            "nothing withheld, nothing said ({transport:?})"
        );

        f.consent(SENSITIVE_T3, false).await;
        let withdrawn = f.prompt(&conversation, transport).await;
        assert!(
            !withdrawn.contains(GOAL) && withdrawn.contains(MEDICAL_FLAGS),
            "bug: a withdrawal applies on the next turn, to its own tier only ({transport:?})"
        );
    }
}

#[tokio::test]
async fn the_framing_is_the_same_whether_or_not_anything_is_stored() {
    let f = setup("tier-empty@example.com").await;
    let conversation = f.conversation().await;
    let empty = f.prompt(&conversation, Transport::WebApp).await;
    assert!(
        empty.contains(MEMORY_WITHHELD) && empty.contains(MEDICAL_WITHHELD),
        "said on consent alone, so it tells nothing of what is on file"
    );
}

/// How long a detached extraction pass is waited for.
const PATIENCE: Duration = Duration::from_secs(10);

/// The extractor's call after `from`, told apart by its "User turn:" payload.
async fn extraction_request(f: &Fixture, from: usize) -> String {
    f.provider
        .request_matching(from, PATIENCE, |request| {
            request.conversation.contains("User turn:")
        })
        .await
        .expect("the turn queues a memory extraction")
        .conversation
}

#[tokio::test]
async fn the_extractor_answers_against_the_existing_facts_only_under_their_consent() {
    let f = setup("tier-extract@example.com").await;
    f.remember().await;
    let conversation = f.conversation().await;

    let before = f.provider.calls_so_far();
    f.prompt(&conversation, Transport::WebApp).await;
    let withheld = extraction_request(&f, before).await;
    assert!(
        !withheld.contains(GOAL) && !withheld.contains("chest_pain"),
        "bug: the extractor is a model; no stored fact without its consent: {withheld}"
    );

    f.consent(SENSITIVE_T3, true).await;
    let before = f.provider.calls_so_far();
    f.prompt(&conversation, Transport::WebApp).await;
    let given = extraction_request(&f, before).await;
    assert!(
        given.contains(GOAL),
        "sensitive_t3 given: the extractor reads the memory it merges into: {given}"
    );
    assert!(
        !given.contains("chest_pain"),
        "bug: the PAR-Q flag needs sensitive_t2 even there: {given}"
    );
}
