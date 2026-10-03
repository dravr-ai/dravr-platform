// ABOUTME: What prompt assembly puts on the wire — which blocks a turn carries and the order they close in
// ABOUTME: Drives real turns through the chat pipeline and reads the system prompt the model received
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The order of an assembled prompt is behaviour, and it has been decided by
//! incident three times:
//!
//! - **2026-07-24** — the guided-walk directive sat mid-prompt, under the
//!   channel constraints, the tool-discipline block and an output contract. A
//!   builder agent whose persona demands a plan on its first reply won that
//!   recency contest and wrote a 16-week plan on the athlete's first answer.
//! - **2026-07-25** — binding an agent replaced the only prompt that said who
//!   the assistant is. A 48-run A/B then measured the prompt tail as the one
//!   place the identity anchor suppresses model disclosure.
//! - **2026-08-05** — the directive slot was empty on ordinary turns, the turns
//!   that broke persona; 1,536 completions put that at 6.51% against 1.69%.
//!
//! Each one is asserted on the prompt a real turn sent, never on the text of
//! `prompt_assembly.rs` (carnet#661): comparing the offsets of its `// Stage`
//! comments passes when a comment moves and the code does not, and fails when
//! rustfmt wraps a line. `pierre_chat_pipeline::execute` runs against a model
//! that records its requests, wired the way production wires a provider.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use pierre_core::transport::Transport;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};
use helpers::recording_llm::RecordingProvider;
use pierre_chat_pipeline::stages::introduction::introduction_directive;
use pierre_chat_pipeline::stages::onboarding::{
    directive, release_directive, GuidedTarget, OnboardingTurn,
};
use pierre_chat_pipeline::stages::prompt_assembly::{
    agent_voice_anchor, IDENTITY_ANCHOR, TURN_DIRECTIVE,
};
use pierre_chat_pipeline::stages::prompt_builder::{render_tool_index, TOOL_BOUNDARY};
use pierre_chat_pipeline::{
    ChatPipelineContext, CommandPersistence, PipelineHooks, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_contremaitre::messaging_strings::KEY_TURN_LANGUAGE;
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::{
    ConversationTurnId, CoverageTarget, GuidedFlow, OnboardingState, TenantId,
};
use pierre_core::prompt_fingerprint::{extract_canary_marker, inject_canary_marker};
use pierre_mcp_server::mcp::resources::ServerContext;

/// A question the model answers itself: no data fetch, so the platform never
/// answers in its place.
const OPENER: &str = "Salut ! Tu peux m'aider à préparer mon premier trail ?";
const FOLLOW_UP: &str = "Merci ! Et pour rester motivé ?";

/// The persona of the agents these tests bind. It says what the agent does
/// and never who the assistant is, which is the shape that made the anchor
/// necessary.
const AGENT_PERSONA: &str = "Tu aides des coureurs de trail à préparer leurs courses.";

const REPLY: &str = "Mars a été un mois de fond plutôt que de spécifique semi.";

/// Heads [`TURN_DIRECTIVE`]. The trailing newline tells it apart from the
/// fortnight brief, whose heading opens with the same words.
const ORDINARY_MARKER: &str = "# This turn\n";
/// Heads every interview directive, whichever walk owns the turn.
const INTERVIEW_MARKER: &str = "(overrides every other instruction in this prompt)";
/// Heads the directive that revokes a finished interview's rules.
const RELEASE_MARKER: &str = "# Interview complete";
/// Heads the brief the fortnight rail carries while it owns the turn.
const FORTNIGHT_MARKER: &str = "# This turn: write the next fortnight";

struct Fixture {
    resources: Arc<ServerContext>,
    ctx: ChatPipelineContext,
    tenant_id: TenantId,
    user_id: Uuid,
    provider: Arc<RecordingProvider>,
}

async fn setup(email: &str) -> Fixture {
    let provider = Arc::new(RecordingProvider::answering(REPLY));
    let resources = create_test_server_resources_with_chat_provider(provider.clone())
        .await
        .unwrap();
    let (user_id, _user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    let ctx = resources.chat_pipeline_context();
    Fixture {
        resources,
        ctx,
        tenant_id,
        user_id,
        provider,
    }
}

/// An agent the athlete authored, in the category under test.
struct Agent {
    id: String,
    slug: String,
    title: String,
}

impl Fixture {
    async fn agent(&self, title: &str, category: AgentCategory) -> Agent {
        let agents = &self.resources.common.repos.agents;
        let id = agents
            .create(
                self.user_id,
                self.tenant_id,
                &CreateAgentRequest {
                    title: title.to_owned(),
                    description: None,
                    system_prompt: AGENT_PERSONA.to_owned(),
                    category,
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
            .to_string();
        let runtime = agents
            .get_agent_runtime_context(&id, self.tenant_id)
            .await
            .unwrap()
            .expect("the agent just created resolves");
        Agent {
            id,
            slug: runtime.slug,
            title: title.to_owned(),
        }
    }

    async fn conversation(&self, agent: Option<&Agent>) -> String {
        self.resources
            .common
            .repos
            .chat
            .create_conversation(
                &self.user_id.to_string(),
                self.tenant_id,
                "Trail",
                "mock-model",
                agent.map(|a| a.id.as_str()),
                None,
            )
            .await
            .unwrap()
            .id
    }

    /// Put the conversation in the state a guided flow leaves it in.
    async fn set_flow(&self, conversation_id: &str, column: &str) {
        let written = self
            .resources
            .common
            .repos
            .chat
            .set_conversation_onboarding_state(conversation_id, Some(column), self.tenant_id)
            .await
            .unwrap();
        assert!(written, "the guided-flow marker is written");
    }

    /// Run one web turn and return the system prompt of its coaching call.
    ///
    /// A turn also spawns background passes — memory extraction, advice
    /// capture — that reach the same provider. The coaching call is the one
    /// carrying the tool-discipline block, which every assembled prompt does
    /// whichever directive owns the turn.
    async fn turn(&self, conversation_id: &str, content: &str, locale: &str) -> String {
        let before = self.provider.calls_so_far();
        pierre_chat_pipeline::execute(
            &self.ctx,
            TurnRequest {
                origin: TurnOrigin::Athlete,
                conversation_id: conversation_id.to_owned(),
                user_id: self.user_id,
                conversation_tenant_id: self.tenant_id,
                tool_tenant_id: self.tenant_id,
                content: content.to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                channel_type: "web",
                transport: Transport::WebApp,
                is_direct_message: true,
                ambient_group_fallback: false,
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                hooks: PipelineHooks::none(),
            },
            &SurfaceProfile::resolve(&SurfaceRequest {
                surface: SurfaceId::Web,
                locale: locale.to_owned(),
                transport: None,
                prose_contract: None,
            }),
        )
        .await
        .expect("the turn is served");
        self.provider
            .system_prompt_since(before, &self.ctx.tool_discipline_prompt)
    }

    fn language(&self, locale: &str) -> String {
        self.ctx
            .messaging_strings_registry
            .get(KEY_TURN_LANGUAGE, locale)
    }

    fn guardrails(&self) -> String {
        let block = self.ctx.prompt_registry.progression_guardrails_prompt();
        assert!(
            !block.trim().is_empty(),
            "premise: the registry ships a progression-guardrails block"
        );
        block
    }
}

/// Byte offset of a block that must be on the wire exactly once.
fn sole_offset(prompt: &str, block: &str, name: &str) -> usize {
    let count = prompt.matches(block).count();
    assert_eq!(
        count, 1,
        "expected {name} on the wire exactly once, found {count}"
    );
    prompt.find(block).unwrap()
}

/// Assert that from `from` on, the hardened prompt is exactly `expected`
/// followed by the canary — no block missing, reordered, repeated or added.
///
/// The canary is read back from the prompt itself and re-applied to
/// `expected` with the function that wrote it, so the comparison holds
/// whatever the token is.
fn assert_closes_with(prompt: &str, from: &str, expected: &str) {
    let start = sole_offset(prompt, from, "the block the tail is measured from");
    let canary = extract_canary_marker(prompt).expect("the prompt on the wire is hardened");
    let wanted = inject_canary_marker(expected, canary);
    let actual = &prompt[start..];
    assert!(
        actual.starts_with(&wanted),
        "the prompt does not close the way it must.\n--- on the wire ---\n{}\n--- expected ---\n{wanted}",
        actual
            .chars()
            .take(wanted.chars().count() + 600)
            .collect::<String>()
    );
}

/// An ordinary turn with no agent bound closes with, in order and with nothing
/// else: tool discipline, the visual contract, the turn's task, the turn's
/// language, the identity anchor, the canary.
///
/// Three guards in one string. The slot is never empty — the task is there
/// (2026-08-05). The task sits below every behavioural block and only the
/// language and the anchor follow it (2026-07-24). The anchor is the last
/// platform block before the prompt is hardened (2026-07-25).
#[tokio::test]
async fn an_ordinary_turn_closes_with_its_task_its_language_and_the_anchor() {
    let fx = setup("wire-ordinary@test.com").await;
    let conv = fx.conversation(None).await;

    let prompt = fx.turn(&conv, OPENER, "fr").await;

    assert_closes_with(
        &prompt,
        &fx.ctx.tool_discipline_prompt,
        &format!(
            "{}\n\n{}{TURN_DIRECTIVE}\n\n{}\n\n{IDENTITY_ANCHOR}",
            fx.ctx.tool_discipline_prompt,
            fx.ctx.visual_blocks_prompt,
            fx.language("fr"),
        ),
    );
    assert!(
        !prompt.contains(INTERVIEW_MARKER) && !prompt.contains(RELEASE_MARKER),
        "the slot's arms are alternatives: an ordinary turn carries one directive"
    );
}

/// The same close on an agent-bound turn — the path whose missing anchor WAS
/// the 2026-07-25 bug, since a bound agent's persona replaces the one prompt
/// that says who the assistant is.
///
/// It adds the two agent-only blocks where they belong: the introduction rides
/// the slot's directive, one line after the task and never in place of it, and
/// the voice anchor sits just ahead of the identity anchor, which stays last.
#[tokio::test]
async fn an_agent_bound_turn_closes_with_the_same_anchor() {
    let fx = setup("wire-agent@test.com").await;
    let agent = fx
        .agent("Coach Trail Laurentides", AgentCategory::Training)
        .await;
    let conv = fx.conversation(Some(&agent)).await;

    let prompt = fx.turn(&conv, OPENER, "en").await;

    assert!(
        prompt.contains(AGENT_PERSONA),
        "premise: this is the agent-bound path — the agent's own persona is on the wire"
    );
    assert_closes_with(
        &prompt,
        &fx.ctx.tool_discipline_prompt,
        &format!(
            "{}{TURN_DIRECTIVE}{}\n\n{}\n\n{}\n\n{IDENTITY_ANCHOR}",
            fx.ctx.tool_discipline_prompt,
            introduction_directive(&agent.title),
            fx.language("en"),
            agent_voice_anchor(&agent.slug),
        ),
    );
    assert!(
        !prompt.contains(&fx.ctx.visual_blocks_prompt),
        "an agent granted no visuals is never taught the syntax its fences would be \
         refused for"
    );
}

/// The turn's language follows the surface's resolved locale, on every turn.
///
/// carnet#159: a francophone athlete got an English answer on a surface that
/// carried no language rule at all, because the rule was inferred from the
/// athlete's words and outweighed by tens of KB of English scaffolding.
#[tokio::test]
async fn the_language_named_is_the_surfaces_locale() {
    let fx = setup("wire-language@test.com").await;
    let (french, english) = (fx.language("fr"), fx.language("en"));
    assert_ne!(
        french, english,
        "premise: the two locales name different languages"
    );

    let conv = fx.conversation(None).await;
    let first = fx.turn(&conv, OPENER, "fr").await;
    assert!(first.contains(&french) && !first.contains(&english));

    let second = fx.turn(&conv, FOLLOW_UP, "en").await;
    assert!(
        second.contains(&english) && !second.contains(&french),
        "the language comes from this turn's locale, not from the thread's first turn"
    );
}

/// A guided interview owns the slot: its directive replaces the ordinary task,
/// sits below the tool-discipline block, and is followed only by the
/// introduction, the language and the anchors.
///
/// It also takes the two blocks that would argue with it off the prompt — the
/// visual contract, an output contract that contradicts "ask one question",
/// and the progression guardrails, which are pure prompt cost on a turn that
/// prescribes no load.
#[tokio::test]
async fn an_interview_turn_carries_its_directive_last_and_no_competing_block() {
    let fx = setup("wire-interview@test.com").await;
    let agent = fx
        .agent("Coach Trail Laurentides", AgentCategory::Training)
        .await;
    let conv = fx.conversation(Some(&agent)).await;
    fx.set_flow(
        &conv,
        &OnboardingState::start_now_column(GuidedFlow::Pillars),
    )
    .await;

    let prompt = fx.turn(&conv, "Salut !", "fr").await;

    let probe = directive(&OnboardingTurn {
        target: Some(GuidedTarget::Coverage(CoverageTarget::NorthStar)),
        state: OnboardingState::start(Utc::now().to_rfc3339(), GuidedFlow::Pillars),
    });
    assert_closes_with(
        &prompt,
        &fx.ctx.tool_discipline_prompt,
        &format!(
            "{}{probe}{}\n\n{}\n\n{}\n\n{IDENTITY_ANCHOR}",
            fx.ctx.tool_discipline_prompt,
            introduction_directive(&agent.title),
            fx.language("fr"),
            agent_voice_anchor(&agent.slug),
        ),
    );
    assert!(
        !prompt.contains(ORDINARY_MARKER),
        "a guided turn carries the walk's directive instead of the ordinary task, never both"
    );
    assert!(
        !prompt.contains(&fx.guardrails()),
        "an interview asks questions and prescribes no load, so the guardrails stay off it"
    );
}

/// With no agent bound the visual contract is granted on a web turn — the
/// ordinary-turn test shows it there — so its absence here is the walk's doing.
#[tokio::test]
async fn an_interview_turn_withholds_the_visual_contract() {
    let fx = setup("wire-interview-visuals@test.com").await;
    let conv = fx.conversation(None).await;
    fx.set_flow(
        &conv,
        &OnboardingState::start_now_column(GuidedFlow::Pillars),
    )
    .await;

    let prompt = fx.turn(&conv, "Salut !", "fr").await;

    assert!(
        prompt.contains(INTERVIEW_MARKER),
        "premise: the walk owns this turn"
    );
    assert!(
        !prompt.contains(&fx.ctx.visual_blocks_prompt),
        "an output contract contradicts the profile-building directive it would precede"
    );
}

/// The withholding is about the flow being an interview, not about a flow
/// being active: the fortnight rail exists to write two weeks of plan, and a
/// plan is what the visual contract renders.
///
/// No agent is bound, so the grant is the platform baseline — the same one the
/// ordinary-turn test reads the contract from. The interview test above and
/// this one differ in the flow alone, which is what makes the pair a guard on
/// the gate rather than on the grant.
#[tokio::test]
async fn a_flow_that_writes_a_plan_keeps_the_visual_contract() {
    let fx = setup("wire-fortnight-visuals@test.com").await;
    let conv = fx.conversation(None).await;
    fx.set_flow(
        &conv,
        &OnboardingState::start_now_column(GuidedFlow::Fortnight),
    )
    .await;

    let prompt = fx.turn(&conv, OPENER, "fr").await;

    assert!(
        prompt.contains(FORTNIGHT_MARKER) && !prompt.contains(INTERVIEW_MARKER),
        "premise: the fortnight rail owns this turn, and it is not an interview"
    );
    let tool_discipline = sole_offset(&prompt, &fx.ctx.tool_discipline_prompt, "tool discipline");
    let visual_contract = sole_offset(&prompt, &fx.ctx.visual_blocks_prompt, "the visual contract");
    let brief = sole_offset(&prompt, FORTNIGHT_MARKER, "the fortnight brief");
    assert!(
        tool_discipline < visual_contract && visual_contract < brief,
        "a flow that writes a plan keeps the contract it draws the plan with, above \
         the brief that owns the slot"
    );
}

/// The turn after an interview ends carries the directive that revokes the
/// interview's rules, in the interview directive's own slot, once.
///
/// 2026-07-28: an agent told an athlete it could not save his plan 48 seconds
/// after a completed calibration, having never called the tool. A retraction
/// placed anywhere above the slot loses the recency contest the prohibition
/// won for eight turns.
#[tokio::test]
async fn the_turn_after_an_interview_carries_the_release_in_the_same_slot_once() {
    let fx = setup("wire-release@test.com").await;
    let conv = fx.conversation(None).await;
    let now = Utc::now().to_rfc3339();
    let retired = OnboardingState::start(now.clone(), GuidedFlow::Calibration)
        .completed(now)
        .to_column()
        .unwrap();
    fx.set_flow(&conv, &retired).await;

    let released = fx.turn(&conv, OPENER, "fr").await;
    assert_closes_with(
        &released,
        &fx.ctx.tool_discipline_prompt,
        &format!(
            "{}\n\n{}{}\n\n{}\n\n{IDENTITY_ANCHOR}",
            fx.ctx.tool_discipline_prompt,
            fx.ctx.visual_blocks_prompt,
            release_directive(Some(&retired)),
            fx.language("fr"),
        ),
    );
    assert!(
        !released.contains(ORDINARY_MARKER) && !released.contains(INTERVIEW_MARKER),
        "the release is an alternative to the other two directives, never stacked on one"
    );

    let after = fx.turn(&conv, FOLLOW_UP, "fr").await;
    assert!(
        after.contains(ORDINARY_MARKER) && !after.contains(RELEASE_MARKER),
        "the release fires on the turn after the wrap-up, not on every turn from then on"
    );
}

/// An agent not yet introduced in the thread is introduced on whichever
/// directive owns the slot — the release here; the ordinary task and the
/// interview probe in the tests above — one line after it, once.
///
/// carnet#501. The introduction is appended to the result of the slot's choice
/// rather than from inside one arm of it, so no arm can drop it or carry it
/// twice.
#[tokio::test]
async fn the_release_carries_the_introduction_like_every_other_directive() {
    let fx = setup("wire-release-intro@test.com").await;
    let agent = fx
        .agent("Coach Trail Laurentides", AgentCategory::Nutrition)
        .await;
    let conv = fx.conversation(Some(&agent)).await;
    let now = Utc::now().to_rfc3339();
    let retired = OnboardingState::start(now.clone(), GuidedFlow::Pillars)
        .completed(now)
        .to_column()
        .unwrap();
    fx.set_flow(&conv, &retired).await;

    let prompt = fx.turn(&conv, OPENER, "fr").await;

    assert_closes_with(
        &prompt,
        &fx.ctx.tool_discipline_prompt,
        &format!(
            "{}{}{}\n\n{}\n\n{}\n\n{IDENTITY_ANCHOR}",
            fx.ctx.tool_discipline_prompt,
            release_directive(Some(&retired)),
            introduction_directive(&agent.title),
            fx.language("fr"),
            agent_voice_anchor(&agent.slug),
        ),
    );
}

/// Only an interview leaves a prohibition behind, so only an interview is
/// released. A fortnight rail that failed to clear its marker must not open
/// the next turn with "the guided interview is over" to an athlete who was
/// never interviewed.
#[tokio::test]
async fn a_retired_flow_that_ran_no_interview_gets_no_release() {
    let fx = setup("wire-no-release@test.com").await;
    let conv = fx.conversation(None).await;
    let now = Utc::now().to_rfc3339();
    let retired = OnboardingState::start(now.clone(), GuidedFlow::Fortnight)
        .completed(now)
        .to_column()
        .unwrap();
    fx.set_flow(&conv, &retired).await;

    let prompt = fx.turn(&conv, OPENER, "fr").await;

    assert!(
        prompt.contains(ORDINARY_MARKER) && !prompt.contains(RELEASE_MARKER),
        "a retired fortnight rail leaves an ordinary turn behind, with nothing to revoke"
    );
}

/// The progression guardrails reach a load-prescribing agent's ordinary turn,
/// above the tool-discipline block and everything after it.
///
/// They give up recency on purpose: ~450 tokens of prose landing after an
/// output contract is the contest that derailed the 2026-07-24 walk, and the
/// enforcement that matters is the save-time ramp check.
#[tokio::test]
async fn the_guardrails_sit_above_the_output_format_blocks() {
    let fx = setup("wire-guardrails-order@test.com").await;
    let agent = fx
        .agent("Coach Trail Laurentides", AgentCategory::Training)
        .await;
    let conv = fx.conversation(Some(&agent)).await;

    let prompt = fx.turn(&conv, OPENER, "fr").await;

    let guardrails = sole_offset(&prompt, &fx.guardrails(), "the progression guardrails");
    let tool_discipline = sole_offset(&prompt, &fx.ctx.tool_discipline_prompt, "tool discipline");
    let task = sole_offset(&prompt, TURN_DIRECTIVE, "the turn directive");
    assert!(
        guardrails < tool_discipline && tool_discipline < task,
        "guardrails must precede the tool-discipline block and the turn's directive"
    );
}

/// The suppression is about the flow's nature, not about a flow being active.
/// The fortnight rail exists to WRITE two weeks of plan, so it is the turn
/// that needs the guardrails most.
#[tokio::test]
async fn a_flow_that_writes_a_plan_keeps_the_guardrails() {
    let fx = setup("wire-guardrails-fortnight@test.com").await;
    let agent = fx
        .agent("Coach Trail Laurentides", AgentCategory::Training)
        .await;
    let conv = fx.conversation(Some(&agent)).await;
    fx.set_flow(
        &conv,
        &OnboardingState::start_now_column(GuidedFlow::Fortnight),
    )
    .await;

    let prompt = fx.turn(&conv, OPENER, "fr").await;

    assert!(
        prompt.contains(FORTNIGHT_MARKER),
        "premise: the fortnight rail owns this turn"
    );
    assert!(
        prompt.contains(&fx.guardrails()),
        "a flow that exists to write a plan is not an interview and keeps the guardrails"
    );
    assert!(
        !prompt.contains(INTERVIEW_MARKER),
        "nor is it handed the interview's block, which forbids the one thing it is for"
    );
}

/// Training, Recovery and Analysis agents prescribe load; the rest do not, and
/// neither does a turn with no agent bound. Charging ~450 tokens on every turn
/// of an agent that never discusses load is the wrong default.
#[tokio::test]
async fn only_load_prescribing_agents_receive_the_guardrails() {
    let fx = setup("wire-guardrails-audience@test.com").await;
    let guardrails = fx.guardrails();

    for (category, prescribes_load) in [
        (AgentCategory::Training, true),
        (AgentCategory::Recovery, true),
        (AgentCategory::Analysis, true),
        (AgentCategory::Nutrition, false),
        (AgentCategory::Recipes, false),
        (AgentCategory::Mobility, false),
        (AgentCategory::Custom, false),
    ] {
        let agent = fx
            .agent(&format!("Coach {}", category.as_str()), category)
            .await;
        let conv = fx.conversation(Some(&agent)).await;
        let prompt = fx.turn(&conv, OPENER, "fr").await;
        assert_eq!(
            prompt.contains(&guardrails),
            prescribes_load,
            "a {} agent: guardrails on the wire must be {prescribes_load}",
            category.as_str()
        );
    }

    let unbound = fx.conversation(None).await;
    let prompt = fx.turn(&unbound, OPENER, "fr").await;
    assert!(
        !prompt.contains(&guardrails),
        "the guardrails bound how an agent prescribes; a turn with none bound gets nothing"
    );
}

/// The block is whatever the prompt registry holds when the turn runs, so a
/// contremaitre edit reaches the next turn with no redeploy — and an entry
/// emptied there appends nothing, not a blank section.
#[tokio::test]
async fn the_guardrails_are_read_from_the_registry_on_every_turn() {
    const EDITED: &str = "## Progression\nNever raise weekly load by more than a tenth.";
    /// Heads the block rendered just above where the guardrails are appended.
    const TOOL_INDEX_HEADING: &str = "## Your tools";

    let fx = setup("wire-guardrails-registry@test.com").await;
    let agent = fx
        .agent("Coach Trail Laurentides", AgentCategory::Training)
        .await;
    let registry = &fx.ctx.prompt_registry;
    // Everything from the tool index down to the tool-discipline block: the
    // stretch the guardrails are appended into, clear of the clock stamp in
    // the contract that leads the prompt.
    let stretch = |prompt: &str| {
        let from = sole_offset(prompt, TOOL_INDEX_HEADING, "the tool index");
        let to = sole_offset(prompt, &fx.ctx.tool_discipline_prompt, "tool discipline");
        prompt[from..to].to_owned()
    };

    registry.update_system_prompt(
        "progression_guardrails",
        EDITED.to_owned(),
        "edited".to_owned(),
    );
    let conv = fx.conversation(Some(&agent)).await;
    let edited = stretch(&fx.turn(&conv, OPENER, "fr").await);
    assert!(
        edited.contains(&format!("\n\n{EDITED}\n\n")),
        "the turn after a registry edit must carry the edited block"
    );

    registry.update_system_prompt(
        "progression_guardrails",
        " \n".to_owned(),
        "emptied".to_owned(),
    );
    let conv = fx.conversation(Some(&agent)).await;
    let emptied = stretch(&fx.turn(&conv, OPENER, "fr").await);
    assert_eq!(
        emptied,
        edited.replace(&format!("\n\n{EDITED}"), ""),
        "an empty registry entry must append nothing — the prompt is the edited one with \
         the block and its separator gone"
    );
}

/// The prompt states the tool boundary and indexes exactly the tools the chat
/// surface serves.
///
/// A prose list used to be generated here from the user-visible schema set
/// while the declarations came from the chat-callable one, so it advertised
/// agent CRUD and config writes the agent could not call.
#[tokio::test]
async fn the_tool_index_names_exactly_what_the_chat_surface_serves() {
    let fx = setup("wire-tool-index@test.com").await;
    let registry = &fx.ctx.tool_registry;
    let callable: Vec<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();
    let visible_only: Vec<String> = registry
        .user_visible_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .filter(|name| !callable.contains(name))
        .collect();
    assert!(
        !callable.is_empty() && !visible_only.is_empty(),
        "premise: the registry holds tools the agent can call and tools it can only see"
    );

    let conv = fx.conversation(None).await;
    let prompt = fx.turn(&conv, OPENER, "fr").await;

    assert!(
        prompt.contains(&format!("{TOOL_BOUNDARY}{}", render_tool_index(&callable))),
        "the boundary statement is followed by the index of chat-callable tools, rendered \
         from that one set — so none of {visible_only:?} is advertised"
    );
    sole_offset(&prompt, "## Your tools", "the tool index");
}
