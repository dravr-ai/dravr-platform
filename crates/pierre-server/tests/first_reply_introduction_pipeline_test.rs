// ABOUTME: An agent is told to introduce itself until a reply in the thread has named it, then never again
// ABOUTME: Drives the real chat pipeline — rebind, platform rows, guided walk, room — and reads each assembled prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#501.
//!
//! Live on dev, 2026-09-21 23:46Z: the first message of a web chat with the
//! Half Marathon Agent — « Montre-moi mes sorties de mars 2026 avec le
//! dénivelé. » — got a reply that opened straight into analysis, with no name
//! and no role. No introduction rule had ever existed in either prompt
//! history, so nothing asked for one.
//!
//! The rule is decided at Stage 7g.3 of prompt assembly and recorded at the
//! end of the turn, and both only run inside a real turn. So these tests run
//! real turns through `pierre_chat_pipeline::execute` against a model that
//! records what it was sent and answers what the test tells it to, and assert
//! on the system prompt of each coaching call. The pure decisions behind it
//! are pinned in `pierre-chat-pipeline/tests/first_reply_introduction_test.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;
use uuid::Uuid;

use common::{create_test_server_resources_with_llm, create_test_user_with_plan};
use pierre_chat_pipeline::stages::prompt_assembly::IDENTITY_ANCHOR;
use pierre_chat_pipeline::{
    CommandPersistence, PipelineHooks, SurfaceId, SurfaceProfile, SurfaceRequest, TurnOrigin,
    TurnRequest,
};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole, StreamChunk,
    TokenUsage,
};
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{
    AddMessageParams, ConversationTurnId, GuidedFlow, OnboardingState, TenantId,
    COMMAND_FINISH_REASON,
};
use pierre_database::seed_models::{SeedAgent, SeedAgentTranslation};
use pierre_mcp_server::mcp::resources::ServerContext;

/// The 2026-09-21 reply: straight into analysis, naming nobody.
const UNINTRODUCED_REPLY: &str = "Mars a été un mois de fond plutôt que de spécifique semi.";

/// The directive's own opening, present only when the turn carries it.
const INTRODUCTION_MARKER: &str = "You have not introduced yourself in this conversation yet.";

/// Heads the ordinary turn directive, which every ordinary coaching call
/// carries. It is how the coaching call is told apart from the background
/// passes a turn spawns — memory extraction, advice capture — which may land
/// in between.
const COACHING_CALL_MARKER: &str = "# This turn";

/// Heads a guided walk's directive, which replaces the ordinary one.
const GUIDED_CALL_MARKER: &str = "# Onboarding mode";

/// Turns the model answers itself. A data question asked with no provider
/// connected is answered by the platform's reconnect re-challenge instead,
/// which is its own test below.
const OPENER: &str = "Salut ! Tu peux m'aider à préparer mon premier trail ?";
const FOLLOW_UP: &str = "Merci ! Et pour rester motivé ?";

const TRAIL_COACH: &str = "Coach Trail Laurentides";
const CYCLING_COACH: &str = "Coach Vélo Outaouais";

/// Records the system prompt of every completion, in call order, and answers
/// with whatever reply the test last set.
struct CapturingProvider {
    system_prompts: Mutex<Vec<String>>,
    reply: Mutex<String>,
}

impl CapturingProvider {
    fn calls_so_far(&self) -> usize {
        self.system_prompts.lock().unwrap().len()
    }

    fn answer_with(&self, reply: &str) {
        reply.clone_into(&mut self.reply.lock().unwrap());
    }

    /// The system prompt of the first call after `from` carrying `marker`.
    fn prompt_since(&self, from: usize, marker: &str) -> String {
        self.system_prompts
            .lock()
            .unwrap()
            .iter()
            .skip(from)
            .find(|prompt| prompt.contains(marker))
            .cloned()
            .unwrap_or_else(|| panic!("no call carrying {marker:?} was made after call #{from}"))
    }
}

#[async_trait]
impl LlmProvider for CapturingProvider {
    fn name(&self) -> &'static str {
        "capturing_mock"
    }
    fn display_name(&self) -> &'static str {
        "Capturing Mock LLM (agent introduction)"
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
        let system = request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::System)
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.system_prompts.lock().unwrap().push(system);
        Ok(ChatResponse {
            content: self.reply.lock().unwrap().clone(),
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

struct Fixture {
    resources: Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
    provider: Arc<CapturingProvider>,
}

async fn setup(email: &str) -> Fixture {
    let provider = Arc::new(CapturingProvider {
        system_prompts: Mutex::new(Vec::new()),
        reply: Mutex::new(UNINTRODUCED_REPLY.to_owned()),
    });
    let resources = create_test_server_resources_with_llm(provider.clone())
        .await
        .unwrap();
    let (user_id, _user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    Fixture {
        resources,
        tenant_id,
        user_id,
        provider,
    }
}

/// A reply opening the way the directive asks, for an agent titled `title`.
fn introduced_reply(title: &str) -> String {
    format!("Salut, je suis {title} de Dravr, là pour préparer tes courses avec toi. {UNINTRODUCED_REPLY}")
}

/// An agent the athlete authored: its prompt is their text, its name the
/// `agents.title` column.
async fn custom_agent(fx: &Fixture, title: &str) -> String {
    fx.resources
        .common
        .repos
        .agents
        .create(
            fx.user_id,
            fx.tenant_id,
            &CreateAgentRequest {
                title: title.to_owned(),
                description: None,
                system_prompt: "Tu aides des coureurs de trail à préparer leurs courses."
                    .to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

/// A catalogue agent as the seeder writes it — canonical English title, a
/// French `agent_translations` overlay — with nothing in the live prompt
/// registry, which is how every instance starts before its first contremaitre
/// sync.
async fn catalogue_agent(fx: &Fixture) -> String {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let seeder = &fx.resources.common.repos.seeder;
    seeder
        .seed_insert_agent(&SeedAgent {
            id: id.clone(),
            user_id: fx.user_id,
            tenant_id: fx.tenant_id,
            title: "Half Marathon Agent".to_owned(),
            description: "13.1 mile race preparation".to_owned(),
            system_prompt: "You are a half marathon specialist.".to_owned(),
            category: "training".to_owned(),
            tags_json: "[]".to_owned(),
            sample_prompts_json: "[]".to_owned(),
            token_count: 10,
            visibility: "tenant".to_owned(),
            slug: "half-marathon-agent".to_owned(),
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            prerequisites_json: "{}".to_owned(),
            source_file: None,
            content_hash: None,
            startup_query: None,
            data_requirements: None,
            visuals: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    seeder
        .seed_upsert_agent_translation(&SeedAgentTranslation {
            agent_id: id.clone(),
            locale: "fr".to_owned(),
            title: Some("Agent Semi-Marathon".to_owned()),
            description: Some("Préparation au semi-marathon".to_owned()),
            purpose: None,
            instructions: None,
            source_sha: None,
            tags: None,
        })
        .await
        .unwrap();
    id
}

async fn conversation(
    fx: &Fixture,
    user_id: Uuid,
    agent_id: Option<&str>,
    group: Option<Uuid>,
) -> String {
    let group_id = group.map(|g| g.to_string());
    fx.resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            fx.tenant_id,
            "Semi",
            "mock-model",
            agent_id,
            group_id.as_deref(),
        )
        .await
        .unwrap()
        .id
}

fn web_profile(locale: &str) -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: locale.to_owned(),
        transport: None,
        prose_contract: None,
    })
}

/// Who sends a turn, and where the room's tools resolve for them.
#[derive(Clone, Copy)]
struct Sender {
    user_id: Uuid,
    tool_tenant_id: TenantId,
    is_direct_message: bool,
}

impl Fixture {
    const fn athlete(&self) -> Sender {
        Sender {
            user_id: self.user_id,
            tool_tenant_id: self.tenant_id,
            is_direct_message: true,
        }
    }
}

/// Run one turn and return the system prompt of its call carrying `marker`.
async fn turn_prompt(
    fx: &Fixture,
    sender: Sender,
    conversation_id: &str,
    content: &str,
    marker: &str,
) -> String {
    let before = fx.provider.calls_so_far();
    pierre_chat_pipeline::execute(
        &fx.resources.chat_pipeline_context(),
        TurnRequest {
            origin: TurnOrigin::Athlete,
            conversation_id: conversation_id.to_owned(),
            user_id: sender.user_id,
            conversation_tenant_id: fx.tenant_id,
            tool_tenant_id: sender.tool_tenant_id,
            content: content.to_owned(),
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            channel_type: "web",
            is_direct_message: sender.is_direct_message,
            ambient_group_fallback: false,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            hooks: PipelineHooks::none(),
        },
        &web_profile("fr"),
    )
    .await
    .expect("the turn is served");
    fx.provider.prompt_since(before, marker)
}

/// The defect and its bound in one conversation: the first reply is told to
/// introduce the agent, as Dravr's, ahead of the identity anchor; once a reply
/// has done so, the next is not.
#[tokio::test]
async fn a_bound_agent_introduces_itself_once() {
    let fx = setup("intro-once@test.com").await;
    let agent = custom_agent(&fx, TRAIL_COACH).await;
    let conv = conversation(&fx, fx.user_id, Some(&agent), None).await;
    fx.provider.answer_with(&introduced_reply(TRAIL_COACH));

    let first = turn_prompt(&fx, fx.athlete(), &conv, OPENER, COACHING_CALL_MARKER).await;
    assert!(
        first.contains("introducing yourself as Dravr's Coach Trail Laurentides and saying"),
        "the first reply must be told to open by naming the agent — got a prompt of {} \
         chars without it",
        first.len()
    );
    let task = first.find(COACHING_CALL_MARKER).unwrap();
    let introduction = first.find(INTRODUCTION_MARKER).unwrap();
    let anchor = first.find(IDENTITY_ANCHOR).unwrap();
    assert!(
        task < introduction && introduction < anchor,
        "the introduction follows the turn's task and precedes the identity anchor, \
         which stays last"
    );

    let second = turn_prompt(&fx, fx.athlete(), &conv, FOLLOW_UP, COACHING_CALL_MARKER).await;
    assert!(
        !second.contains(INTRODUCTION_MARKER),
        "the reply after the one that named the agent must not introduce it again"
    );
}

/// The incident's own question, asked with no provider connected: the model
/// was told to introduce itself and did, but the reconnect re-challenge the
/// platform wrote is what the thread received. That text names nobody, so the
/// agent is still unintroduced and the next reply is asked again.
#[tokio::test]
async fn a_reply_the_platform_replaced_introduced_nobody() {
    let fx = setup("intro-replaced@test.com").await;
    let agent = custom_agent(&fx, TRAIL_COACH).await;
    let conv = conversation(&fx, fx.user_id, Some(&agent), None).await;
    fx.provider.answer_with(&introduced_reply(TRAIL_COACH));

    let first = turn_prompt(
        &fx,
        fx.athlete(),
        &conv,
        "Montre-moi mes sorties de mars 2026 avec le dénivelé.",
        COACHING_CALL_MARKER,
    )
    .await;
    assert!(first.contains(INTRODUCTION_MARKER));
    let delivered = fx
        .resources
        .common
        .repos
        .chat
        .get_messages(&conv, &fx.user_id.to_string(), fx.tenant_id)
        .await
        .unwrap()
        .into_iter()
        .rfind(|row| row.role == "assistant")
        .expect("the turn persisted a reply");
    assert!(
        !delivered.content.contains(TRAIL_COACH),
        "premise: with no provider connected the platform answers this question itself —          got {:?}",
        delivered.content
    );

    let second = turn_prompt(&fx, fx.athlete(), &conv, FOLLOW_UP, COACHING_CALL_MARKER).await;
    assert!(
        second.contains(INTRODUCTION_MARKER),
        "the athlete read the platform's text, not the introduction, so the ask stands"
    );
}

/// The check that records the introduction reads the reply the thread got. A
/// reply that opened straight into analysis introduced nobody — the 2026-09-21
/// reply itself — so the next reply is asked again, and only that one.
#[tokio::test]
async fn a_reply_that_never_named_the_agent_leaves_it_to_the_next_one() {
    let fx = setup("intro-ignored@test.com").await;
    let agent = custom_agent(&fx, TRAIL_COACH).await;
    let conv = conversation(&fx, fx.user_id, Some(&agent), None).await;

    fx.provider.answer_with(UNINTRODUCED_REPLY);
    let first = turn_prompt(&fx, fx.athlete(), &conv, OPENER, COACHING_CALL_MARKER).await;
    assert!(first.contains(INTRODUCTION_MARKER));

    fx.provider.answer_with(&introduced_reply(TRAIL_COACH));
    let second = turn_prompt(&fx, fx.athlete(), &conv, FOLLOW_UP, COACHING_CALL_MARKER).await;
    assert!(
        second.contains(INTRODUCTION_MARKER),
        "the athlete has still not been told who is answering, so the ask stands"
    );

    let third = turn_prompt(
        &fx,
        fx.athlete(),
        &conv,
        "Et pour les étirements ?",
        COACHING_CALL_MARKER,
    )
    .await;
    assert!(
        !third.contains(INTRODUCTION_MARKER),
        "once a reply named the agent, no later reply is asked to"
    );
}

/// A messaging DM is one long-lived conversation re-pointed at whichever agent
/// the athlete selected last, and `/agent add` rebinds a web thread in place.
/// The agent that joins a thread already answered in must still introduce
/// itself; the one it replaced must not have used up the thread's only
/// introduction.
#[tokio::test]
async fn an_agent_rebound_onto_an_answered_thread_introduces_itself() {
    let fx = setup("intro-rebind@test.com").await;
    let trail = custom_agent(&fx, TRAIL_COACH).await;
    let cycling = custom_agent(&fx, CYCLING_COACH).await;
    let conv = conversation(&fx, fx.user_id, Some(&trail), None).await;

    fx.provider.answer_with(&introduced_reply(TRAIL_COACH));
    turn_prompt(&fx, fx.athlete(), &conv, OPENER, COACHING_CALL_MARKER).await;
    let settled = turn_prompt(&fx, fx.athlete(), &conv, FOLLOW_UP, COACHING_CALL_MARKER).await;
    assert!(
        !settled.contains(INTRODUCTION_MARKER),
        "the first agent was introduced"
    );

    let rebound = fx
        .resources
        .common
        .repos
        .chat
        .set_conversation_agent_id(&conv, Some(&cycling), fx.tenant_id)
        .await
        .unwrap();
    assert!(rebound, "the conversation row is re-pointed in place");

    fx.provider.answer_with(&introduced_reply(CYCLING_COACH));
    let joined = turn_prompt(
        &fx,
        fx.athlete(),
        &conv,
        "Et pour le vélo ?",
        COACHING_CALL_MARKER,
    )
    .await;
    assert!(
        joined.contains("introducing yourself as Dravr's Coach Vélo Outaouais and saying"),
        "the agent that took over the thread has not been introduced in it"
    );

    let after = turn_prompt(
        &fx,
        fx.athlete(),
        &conv,
        "Et pour les étirements ?",
        COACHING_CALL_MARKER,
    )
    .await;
    assert!(!after.contains(INTRODUCTION_MARKER));
}

/// Platform-authored rows are `assistant` rows that introduced nobody: the
/// interrupted-turn notice `close_interrupted_turn` writes, and a slash
/// command's answer — here one that even names the agent it added.
#[tokio::test]
async fn platform_rows_do_not_stand_in_for_the_introduction() {
    let fx = setup("intro-platform-rows@test.com").await;
    let agent = custom_agent(&fx, TRAIL_COACH).await;
    let conv = conversation(&fx, fx.user_id, Some(&agent), None).await;
    let user_id = fx.user_id.to_string();
    let chat = &fx.resources.common.repos.chat;
    for (role, content, finish_reason) in [
        (
            "user",
            "/agent add coach-trail-laurentides",
            Some(COMMAND_FINISH_REASON),
        ),
        (
            "assistant",
            "Coach Trail Laurentides a été ajouté à cette conversation.",
            Some(COMMAND_FINISH_REASON),
        ),
        ("user", OPENER, None),
        (
            "assistant",
            "Ma réponse a été interrompue. Renvoie ton dernier message.",
            Some("interrupted"),
        ),
    ] {
        chat.add_message(&AddMessageParams {
            tenant_id: fx.tenant_id,
            conversation_id: &conv,
            user_id: &user_id,
            role,
            content,
            token_count: None,
            finish_reason,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();
    }

    let first = turn_prompt(&fx, fx.athlete(), &conv, OPENER, COACHING_CALL_MARKER).await;
    assert!(
        first.contains(INTRODUCTION_MARKER),
        "a command answer and an interruption notice are the platform talking; the agent \
         has still not introduced itself"
    );
}

/// A guided walk owns the Stage 7g.3 slot on its turns. When its probe is the
/// agent's first reply in the thread, the probe is where the introduction goes.
#[tokio::test]
async fn a_guided_probe_that_opens_the_thread_carries_the_introduction() {
    let fx = setup("intro-guided@test.com").await;
    let agent = custom_agent(&fx, TRAIL_COACH).await;
    let conv = conversation(&fx, fx.user_id, Some(&agent), None).await;
    let walk = OnboardingState::start_now_column(GuidedFlow::Pillars);
    let written = fx
        .resources
        .common
        .repos
        .chat
        .set_conversation_onboarding_state(&conv, Some(&walk), fx.tenant_id)
        .await
        .unwrap();
    assert!(written);

    let probe = turn_prompt(&fx, fx.athlete(), &conv, "Salut !", GUIDED_CALL_MARKER).await;
    let guided = probe.find(GUIDED_CALL_MARKER).unwrap();
    let introduction = probe
        .find(INTRODUCTION_MARKER)
        .unwrap_or_else(|| panic!("the probe opening the thread must carry the introduction"));
    assert!(
        guided < introduction,
        "the introduction follows the walk's directive, so the rest of the reply is still \
         the probe"
    );
}

/// A catalogue agent introduces itself by the title the store showed the
/// athlete — the `agent_translations` overlay — even while the prompt registry
/// holds none of its persona files, as on a freshly started instance.
#[tokio::test]
async fn a_catalogue_agent_is_introduced_by_the_title_the_store_shows() {
    let fx = setup("intro-catalogue@test.com").await;
    let agent = catalogue_agent(&fx).await;
    let conv = conversation(&fx, fx.user_id, Some(&agent), None).await;

    let first = turn_prompt(
        &fx,
        fx.athlete(),
        &conv,
        "Montre-moi mes sorties de mars 2026 avec le dénivelé.",
        COACHING_CALL_MARKER,
    )
    .await;
    assert!(
        first.contains("introducing yourself as Dravr's Agent Semi-Marathon and saying"),
        "a French athlete meets « Agent Semi-Marathon », the name the store shows them"
    );
    assert!(
        !first.contains("as Dravr's Half Marathon Agent"),
        "the canonical English title must not be the one a French athlete is told"
    );
}

/// Dravr itself, with no agent bound, keeps the behaviour it always had.
#[tokio::test]
async fn a_coachless_turn_is_not_introduced() {
    let fx = setup("intro-coachless@test.com").await;
    let conv = conversation(&fx, fx.user_id, None, None).await;

    let first = turn_prompt(
        &fx,
        fx.athlete(),
        &conv,
        "Comment s'est passée ma semaine ?",
        COACHING_CALL_MARKER,
    )
    .await;
    assert!(
        !first.contains(INTRODUCTION_MARKER),
        "a turn with no agent bound carries no introduction"
    );
}

/// A shared room keeps one conversation row per member. The room hears the
/// agent introduce itself on the first reply it gets, whichever member asked,
/// and a second member's first message does not make it introduce itself
/// again to a room that already met it.
#[tokio::test]
async fn a_room_hears_its_agent_introduce_itself_once() {
    let fx = setup("intro-room@test.com").await;
    let agent = custom_agent(&fx, TRAIL_COACH).await;
    let (bob, _bob_user, bob_tenant) = create_test_user_with_plan(
        &fx.resources.agent.database,
        "intro-room-bob@test.com",
        "professional",
    )
    .await
    .unwrap();

    let group_id = Uuid::new_v4();
    let now = Utc::now();
    let groups = &fx.resources.common.repos.groups;
    groups
        .create_group(
            fx.tenant_id,
            &CoachingGroup {
                id: group_id,
                tenant_id: fx.tenant_id.to_string(),
                name: "Trail du samedi".to_owned(),
                description: None,
                agent_id: agent.clone(),
                owner_id: fx.user_id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::All,
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
    for (user_id, role) in [(fx.user_id, GroupRole::Owner), (bob, GroupRole::Member)] {
        groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id,
                user_id,
                tenant_id: fx.tenant_id.to_string(),
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
    let alice_row = conversation(&fx, fx.user_id, Some(&agent), Some(group_id)).await;
    let bob_row = conversation(&fx, bob, Some(&agent), Some(group_id)).await;
    let alice = Sender {
        is_direct_message: false,
        ..fx.athlete()
    };
    let bob = Sender {
        user_id: bob,
        tool_tenant_id: bob_tenant,
        is_direct_message: false,
    };

    fx.provider.answer_with(&introduced_reply(TRAIL_COACH));
    let room_first = turn_prompt(
        &fx,
        alice,
        &alice_row,
        "On fait quoi samedi ?",
        COACHING_CALL_MARKER,
    )
    .await;
    assert!(
        room_first.contains(INTRODUCTION_MARKER),
        "the room's first reply is the agent's first word to the room"
    );

    let newcomer = turn_prompt(
        &fx,
        bob,
        &bob_row,
        "Je peux venir aussi ?",
        COACHING_CALL_MARKER,
    )
    .await;
    assert!(
        !newcomer.contains(INTRODUCTION_MARKER),
        "a second member's own row is new, but the room already met the agent"
    );
}
