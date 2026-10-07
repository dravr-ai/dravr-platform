// ABOUTME: A withheld reply still learns from the athlete's own message, and from nothing the agent said
// ABOUTME: Drives real turns through the chat pipeline and reads the background requests each one makes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! When the identity-leak detector withholds a reply, the withheld text must
//! never reach the fact store — a leaked narration minted as a fact re-enters
//! every future prompt bundle (reinforcement loop, 2026-07-10). The original
//! gate achieved that by dropping *all* background learning for the turn, which
//! also dropped the athlete's own message.
//!
//! That is what stalls a guided profile walk. The athlete answers, the agent's
//! reply is withheld, no fact is extracted, coverage never flips, and the next
//! turn asks the same question — indefinitely, since every recorded withhold to
//! date is on the agent this flow runs against.
//!
//! The invariant: user-side extraction runs either way, with a marker standing
//! in for the reply; only assistant-side learning — playbook advice capture,
//! whose whole purpose is learning from what the agent said — is skipped.
//!
//! Both passes are detached tasks that call the model, so a real turn runs
//! against a model that records every request, and the assertions are on what
//! those tasks sent, never on the text of `spawn_turn_background_learning`
//! (carnet#661), where an early `return;` can be written in many shapes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use pierre_core::transport::Transport;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;
use uuid::Uuid;

use common::{create_test_server_resources_with_chat_provider, create_test_user_with_plan};
use helpers::recording_llm::{RecordedRequest, RecordingProvider};
use pierre_chat_pipeline::{
    ChatPipelineContext, CommandPersistence, InputSource, PipelineHooks, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_contremaitre::messaging_strings::KEY_REPLY_WITHHELD;
use pierre_core::models::{ConversationTurnId, TenantId, WITHHELD_REPLY_FINISH_REASON};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::advice_capture::looks_like_recommendation;
use pierre_services::memory_extraction::WITHHELD_REPLY_TRANSCRIPT_MARKER;

/// The 2026-07-22 Telegram break, verbatim: the model answering as itself.
const LEAKED_REPLY: &str = "I'm GitHub Copilot CLI, a terminal-based coding assistant.";

/// A reply worth learning from: it passes the advice gate.
const ADVICE_REPLY: &str =
    "Cette semaine, je te recommande deux sorties faciles de 45 minutes avant d'ajouter du dénivelé.";

const ANSWER: &str = "Je cours surtout pour être en forme avec mes enfants.";

const LOCALE: &str = "fr";

/// How long a detached learning pass is given to reach the model.
const PATIENCE: Duration = Duration::from_secs(10);

/// How long to keep watching for a pass that must NOT run, once the pass
/// spawned immediately before it has already been seen.
const SETTLE: Duration = Duration::from_millis(500);

struct Fixture {
    resources: Arc<ServerContext>,
    ctx: ChatPipelineContext,
    tenant_id: TenantId,
    user_id: Uuid,
    provider: Arc<RecordingProvider>,
}

async fn setup(email: &str, standing_reply: &str) -> Fixture {
    let provider = Arc::new(RecordingProvider::answering(standing_reply));
    // Wired as production wires it: both learning passes need the shared chat
    // provider and skip cleanly without one.
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

impl Fixture {
    async fn conversation(&self) -> String {
        self.resources
            .common
            .repos
            .chat
            .create_conversation(
                &self.user_id.to_string(),
                self.tenant_id,
                "Profil",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id
    }

    /// Run one turn and return the assistant row it persisted, as
    /// `(content, finish_reason)`.
    async fn turn(&self, conversation_id: &str, content: &str) -> (String, Option<String>) {
        pierre_chat_pipeline::execute(
            &self.ctx,
            TurnRequest {
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
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
                locale: LOCALE.to_owned(),
                transport: None,
                prose_contract: None,
            }),
        )
        .await
        .expect("the turn is served");
        let row = self
            .resources
            .common
            .repos
            .chat
            .get_messages(conversation_id, &self.user_id.to_string(), self.tenant_id)
            .await
            .unwrap()
            .into_iter()
            .rfind(|row| row.role == "assistant")
            .expect("the turn persisted a reply");
        (row.content, row.finish_reason)
    }

    /// The apology the athlete reads in place of a withheld reply.
    fn withheld_apology(&self) -> String {
        self.ctx
            .messaging_strings_registry
            .get(KEY_REPLY_WITHHELD, LOCALE)
    }

    /// Whether `request` is the Tier 2 fact extraction over a turn.
    fn is_fact_extraction(&self, request: &RecordedRequest) -> bool {
        request
            .system
            .starts_with(&self.ctx.memory_extraction_prompt)
    }

    /// Whether `request` is the playbook advice capture over a turn.
    fn is_advice_capture(&self, request: &RecordedRequest) -> bool {
        request.system == self.ctx.prompt_registry.advice_extraction_prompt().trim()
    }
}

/// A withheld turn still extracts from what the athlete said, with the marker
/// standing in for the reply — and neither the leaked text nor the apology
/// that replaced it is handed to the extractor as if the agent had said it.
#[tokio::test]
async fn a_withheld_turn_extracts_from_the_athletes_message_alone() {
    let fx = setup("withheld-extracts@test.com", LEAKED_REPLY).await;
    let conv = fx.conversation().await;

    let (delivered, finish_reason) = fx.turn(&conv, ANSWER).await;
    assert_eq!(
        (delivered.as_str(), finish_reason.as_deref()),
        (
            fx.withheld_apology().as_str(),
            Some(WITHHELD_REPLY_FINISH_REASON)
        ),
        "premise: the reply leaked on the ask and on the re-ask, so the turn was withheld"
    );

    let extraction = fx
        .provider
        .request_matching(0, PATIENCE, |r| fx.is_fact_extraction(r))
        .await
        .expect(
            "a withheld turn must still run fact extraction — dropping it is what stalls a \
             guided walk on the topic the athlete just answered",
        );
    assert!(
        extraction.conversation.contains(ANSWER),
        "the extractor reads the athlete's own message"
    );
    assert!(
        extraction
            .conversation
            .contains(WITHHELD_REPLY_TRANSCRIPT_MARKER),
        "the marker stands in for the reply, telling the extractor to use the user turn only"
    );

    // Every request the turn made, the detached ones included: nothing the
    // model was sent after the leak carries the leak or its replacement.
    sleep(SETTLE).await;
    let apology = fx.withheld_apology();
    let learning: Vec<RecordedRequest> = fx
        .provider
        .requests_since(0)
        .into_iter()
        .filter(|r| fx.is_fact_extraction(r) || fx.is_advice_capture(r))
        .collect();
    assert_eq!(
        learning.len(),
        1,
        "exactly one learning pass runs on a withheld turn — the marker-carrying extraction"
    );
    for request in &learning {
        assert!(
            !request.conversation.contains(LEAKED_REPLY),
            "the withheld text must never reach a learning pass"
        );
        assert!(
            !request.conversation.contains(&apology),
            "the apology is the platform's text, not something the agent said to learn from"
        );
    }
}

/// Advice capture learns from what the agent said, so it runs on a delivered
/// reply and stays skipped on a withheld one.
///
/// The delivered turn is the control: it shows the capture reaches the model
/// through this fixture, so its absence on the withheld turn is the branch's
/// doing rather than a pass this harness never wires.
#[tokio::test]
async fn advice_capture_runs_on_a_delivered_reply_and_not_on_a_withheld_one() {
    let fx = setup("withheld-advice@test.com", ADVICE_REPLY).await;
    // The apology is a hot-reloadable string, and the capture's own gate is a
    // keyword screen. Today's wording happens to fail that screen, which would
    // hide a capture spawned on a withheld turn; an edit that words it as the
    // suggestion it is ("try sending…") would not. Word it that way here so the
    // skip is what keeps the capture off the model, not the screen.
    fx.ctx.messaging_strings_registry.update(
        KEY_REPLY_WITHHELD,
        LOCALE,
        "Ma réponse n'est pas passée. Essaie de renvoyer ton dernier message.".to_owned(),
        "test-edit".to_owned(),
    );
    assert!(
        looks_like_recommendation(&fx.withheld_apology()),
        "premise: this apology passes the advice gate, so a capture spawned on a withheld \
         turn would reach the model and be seen here"
    );

    let conv = fx.conversation().await;
    let (delivered, _) = fx
        .turn(&conv, "Comment je reprends après deux semaines d'arrêt ?")
        .await;
    assert_eq!(
        delivered, ADVICE_REPLY,
        "premise: the first turn is delivered"
    );
    let captured = fx
        .provider
        .request_matching(0, PATIENCE, |r| fx.is_advice_capture(r))
        .await
        .expect("a delivered recommendation is captured as pending advice");
    assert!(captured.conversation.contains(ADVICE_REPLY));

    fx.provider.answer_with(LEAKED_REPLY);
    let withheld_from = fx.provider.calls_so_far();
    let (_, finish_reason) = fx.turn(&conv, ANSWER).await;
    assert_eq!(
        finish_reason.as_deref(),
        Some(WITHHELD_REPLY_FINISH_REASON),
        "premise: the second turn is withheld"
    );
    // Extraction is spawned first on both branches, so once it has reached the
    // model a capture spawned alongside it has had its chance to.
    fx.provider
        .request_matching(withheld_from, PATIENCE, |r| fx.is_fact_extraction(r))
        .await
        .expect("the withheld turn still extracts");
    sleep(SETTLE).await;
    assert!(
        !fx.provider
            .requests_since(withheld_from)
            .iter()
            .any(|r| fx.is_advice_capture(r)),
        "nothing the agent said reached the athlete, so there is no advice to capture"
    );
}
