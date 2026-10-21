// ABOUTME: The pillars walk's wrap-up offers the agent a newly covered pillar names, and a tap adds it or declines it
// ABOUTME: Pins the two links, the once-per-pillar rule, the join through the Store path, Not now, and who is never offered

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1097: "During Flow 3, a newly covered topic may offer the matching
//! agent as a quiet 'Add X to my team · Not now'".
//!
//! These end a real pillars walk through the guided resolver, deliver its
//! wrap-up the way the turn does, and tap the offer's links through the app's
//! own chat route, with a model that records whether it was ever called.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use pierre_chat_pipeline::stages::deterministic_reply::{
    deliver, DeterministicReplyInputs, PlatformReply,
};
use pierre_chat_pipeline::stages::onboarding::{self, GuidedResolution};
use pierre_chat_pipeline::{
    ChatPipelineContext, InputSource, QuotaState, ReplyBlock, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnAction, TurnInput, TurnOrigin,
};
use pierre_contremaitre::messaging_strings::{
    KEY_AGENTS_TEAM_OFFER_ADD, KEY_AGENTS_TEAM_OFFER_ADDED, KEY_AGENTS_TEAM_OFFER_DISMISSED,
    KEY_AGENTS_TEAM_OFFER_NOT_NOW,
};
use pierre_core::models::{
    AddMessageParams, ConversationRecord, ConversationTurnId, CoverageTarget, GuidedFlow,
    OnboardingState, Pillar, TenantId, WalkAudience, MAX_PROBE_ATTEMPTS,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, TurnResponse};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use serde_json::json;
use uuid::Uuid;

use common::{
    create_test_server_resources_with_chat_provider, create_test_user_with_plan,
    generate_test_token,
};
use helpers::agent_fixtures::{install_catalogue_agent, publish_catalogue_agent, select_own_agent};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only};
use helpers::recording_llm::RecordingProvider;
use pierre_services::locale::resolve_user_locale;

const LOCALE: &str = "en";

/// The Fuelling agent's name, which derives its `fuelling-agent` handle.
const FUELLING_TITLE: &str = "Fuelling Agent";

struct Fixture {
    resources: Arc<ServerContext>,
    ctx: ChatPipelineContext,
    provider: Arc<RecordingProvider>,
    user_id: Uuid,
    tenant_id: TenantId,
    auth: String,
    fuelling: Uuid,
    /// The athlete's stored locale, which the chat route answers a tap in.
    tap_locale: String,
}

/// An athlete leading with an agent of their own, and the Fuelling agent
/// published in the Store by someone else.
async fn setup() -> Fixture {
    let provider = Arc::new(RecordingProvider::answering("Model reply."));
    let resources = create_test_server_resources_with_chat_provider(provider.clone())
        .await
        .unwrap();
    let email = format!("team-offer-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, &email, "professional")
            .await
            .unwrap();
    let author_email = format!("team-offer-author-{}@example.com", Uuid::new_v4());
    let (author_id, _, author_tenant) =
        create_test_user_with_plan(&resources.agent.database, &author_email, "professional")
            .await
            .unwrap();
    let repos = resources.agent.database.repositories();
    let fuelling =
        publish_catalogue_agent(repos, author_id, author_tenant, FUELLING_TITLE, "Fuel.").await;
    let published = repos
        .store_listings
        .get_published_agent(&fuelling.to_string())
        .await
        .unwrap()
        .expect("published");
    assert_eq!(
        published.agent.handle.as_deref(),
        Some("fuelling-agent"),
        "fixture precondition: the rule table keys on this handle"
    );
    select_own_agent(repos, user_id, tenant_id).await;
    let auth = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let tap_locale = resolve_user_locale(resources.common.repos.users.as_ref(), user_id).await;
    Fixture {
        ctx: resources.chat_pipeline_context(),
        resources,
        provider,
        user_id,
        tenant_id,
        auth,
        fuelling,
        tap_locale,
    }
}

impl Fixture {
    /// The facts a pillars-walk answer lands, one per pillar given.
    async fn land(&self, pillars: &[Pillar]) {
        let user_id = self.user_id.to_string();
        for pillar in pillars {
            let object = format!("an answer about {pillar}");
            self.ctx
                .repos
                .memory
                .upsert_user_fact(&UpsertUserFactParams {
                    tenant_id: self.tenant_id,
                    user_id: &user_id,
                    agent_id: None,
                    scope: MemoryScope::User,
                    kind: FactKind::Preference,
                    pillar: Some(*pillar),
                    predicate_code: PredicateCode::States,
                    object: &object,
                    confidence: 0.9,
                    source: FactSource::Onboarding,
                    valid_until: None,
                    source_msg_id: None,
                    transport_policy: TransportPolicy::AnyTransport,
                })
                .await
                .unwrap();
        }
    }

    /// A thread of the athlete's own, holding a pillars walk whose every
    /// topic burned its probe budget an hour ago, so the next turn ends it.
    async fn spent_walk(&self) -> ConversationRecord {
        let mut state = OnboardingState::start(
            (Utc::now() - Duration::hours(1)).to_rfc3339(),
            GuidedFlow::Pillars,
        )
        .with_audience(WalkAudience::Private);
        for _ in 0..MAX_PROBE_ATTEMPTS {
            state = state.with_delivered_probe(CoverageTarget::NorthStar.slug());
            for pillar in Pillar::ALL {
                state = state.with_delivered_probe(CoverageTarget::Pillar(pillar).slug());
            }
        }
        let user_id = self.user_id.to_string();
        let chat = &self.ctx.repos.chat;
        let conversation = chat
            .create_conversation(&user_id, self.tenant_id, "Walk", "mock-model", None, None)
            .await
            .unwrap();
        chat.set_conversation_onboarding_state(
            &conversation.id,
            Some(&state.to_column().unwrap()),
            self.tenant_id,
        )
        .await
        .unwrap();
        chat.get_conversation(&conversation.id, &user_id, self.tenant_id)
            .await
            .unwrap()
            .unwrap()
    }

    /// End a fresh pillars walk and return its wrap-up.
    async fn wrap_up(&self) -> PlatformReply {
        let conversation = self.spent_walk().await;
        let resolution = onboarding::resolve(
            &self.ctx,
            &conversation,
            &self.user_id.to_string(),
            self.tenant_id,
            self.tenant_id,
            LOCALE,
        )
        .await;
        let GuidedResolution::WalkComplete { reply, .. } = resolution else {
            panic!("the walk had nothing left to ask");
        };
        reply
    }

    /// Deliver `reply` on the web as the platform's answer in its own thread.
    async fn deliver_on_web(&self, reply: PlatformReply) -> Vec<TurnAction> {
        let user_id = self.user_id.to_string();
        let conversation = self
            .ctx
            .repos
            .chat
            .create_conversation(&user_id, self.tenant_id, "Walk", "mock-model", None, None)
            .await
            .unwrap();
        let user_message = self
            .ctx
            .repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: self.tenant_id,
                conversation_id: &conversation.id,
                user_id: &user_id,
                role: "user",
                content: "I eat well, mostly.",
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
            })
            .await
            .unwrap();
        let input = TurnInput {
            conversation_id: conversation.id.clone(),
            user_id,
            conversation_tenant_id: self.tenant_id,
            tool_tenant_id: self.tenant_id,
            is_direct_message: true,
            content: user_message.content.clone(),
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quota: QuotaState::Ok,
            mentioned_agent: None,
            steps_after: None,
        };
        let profile = SurfaceProfile::resolve(&SurfaceRequest {
            surface: SurfaceId::Web,
            locale: LOCALE.to_owned(),
            transport: None,
            prose_contract: None,
        });
        let envelope = deliver(
            DeterministicReplyInputs {
                ctx: &self.ctx,
                input: &input,
                profile: &profile,
                active_model: "mock-model".to_owned(),
                user_message,
                conv: &conversation,
            },
            reply,
        )
        .await
        .unwrap();
        envelope
            .assistant
            .blocks
            .into_iter()
            .filter_map(|block| match block {
                ReplyBlock::Actions { actions, .. } => Some(actions),
                _ => None,
            })
            .flatten()
            .collect()
    }

    /// Tap `postback` in a thread of the athlete's own, through the chat route.
    async fn tap(&self, postback: &str) -> TurnResponse {
        let conversation = self
            .ctx
            .repos
            .chat
            .create_conversation(
                &self.user_id.to_string(),
                self.tenant_id,
                "Taps",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap();
        let resp = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{}/messages",
            conversation.id
        ))
        .header("authorization", &self.auth)
        .json(&json!({ "content": postback }))
        .send(ChatRoutes::routes(Arc::clone(&self.resources)))
        .await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{postback}");
        resp.json()
    }

    async fn on_team(&self) -> bool {
        self.ctx
            .repos
            .store_listings
            .holds_install(&self.fuelling.to_string(), self.user_id, self.tenant_id)
            .await
            .unwrap()
    }

    fn render(&self, key: &str, args: &[&str]) -> String {
        self.ctx
            .messaging_strings_registry
            .render(key, LOCALE, args)
    }

    /// `key` as the chat route answers a tap: in the athlete's stored locale.
    fn tap_answer(&self, key: &str, args: &[&str]) -> String {
        self.ctx
            .messaging_strings_registry
            .render(key, &self.tap_locale, args)
    }
}

fn values(actions: &[TurnAction]) -> Vec<String> {
    actions.iter().map(|action| action.value.clone()).collect()
}

#[tokio::test]
async fn covering_fuelling_offers_its_agent_once_as_two_quiet_links() {
    let fx = setup().await;
    fx.land(&[Pillar::Fuelling]).await;

    let reply = fx.wrap_up().await;
    let actions = reply.next_steps.clone().into_actions();
    assert_eq!(
        values(&actions),
        ["nx:0:about_me_more", "ta:add:fuelling", "ta:no:fuelling"],
        "the flow's own step keeps its slot; the offer takes the other two"
    );
    assert_eq!(
        actions[1].label,
        fx.render(KEY_AGENTS_TEAM_OFFER_ADD, &[FUELLING_TITLE])
    );
    assert_eq!(
        actions[2].label,
        fx.render(KEY_AGENTS_TEAM_OFFER_NOT_NOW, &[])
    );

    let (events, _guard) = capture_notify();
    let shown = fx.deliver_on_web(reply).await;
    assert_eq!(
        values(&shown),
        values(&actions),
        "the live reply draws them"
    );
    let offered: Vec<String> = named(&events, "suggestion.shown")
        .iter()
        .map(|event| event.field("use_case").to_owned())
        .collect();
    assert_eq!(
        offered,
        [
            "about_me_more",
            "team_offer_fuelling",
            "team_offer_fuelling"
        ]
    );

    assert_eq!(
        values(&fx.wrap_up().await.next_steps.into_actions()),
        ["nx:0:about_me_more"],
        "a pillar's agent is offered once"
    );
    assert!(!fx.on_team().await, "a showing joins nothing");
}

#[tokio::test]
async fn tapping_add_joins_the_team_through_the_store_and_confirms_in_one_line() {
    let fx = setup().await;
    let (events, _guard) = capture_notify();
    let before = fx.provider.calls_so_far();

    let turn = fx.tap("ta:add:fuelling").await;

    assert_eq!(
        turn.assistant.message.content,
        fx.tap_answer(KEY_AGENTS_TEAM_OFFER_ADDED, &[FUELLING_TITLE])
    );
    assert_eq!(fx.provider.calls_so_far(), before, "no model ran");
    assert!(fx.on_team().await, "the agent joined the athlete's team");
    assert_eq!(named(&events, "agent.installed").len(), 1);
    let tapped = only(&events, "suggestion.tapped");
    assert_eq!(tapped.field("use_case"), "team_offer_fuelling");
    assert_eq!(tapped.field("position"), "0");
    assert_eq!(tapped.field("surface"), "next_step");

    let again = fx.tap("ta:add:fuelling").await;
    assert_eq!(
        again.assistant.message.content,
        fx.tap_answer(KEY_AGENTS_TEAM_OFFER_ADDED, &[FUELLING_TITLE]),
        "a second tap converges"
    );
    assert_eq!(
        named(&events, "agent.installed").len(),
        1,
        "the agent joins once"
    );

    fx.land(&[Pillar::Fuelling]).await;
    assert_eq!(
        values(&fx.wrap_up().await.next_steps.into_actions()),
        ["nx:0:about_me_more"],
        "an agent on the team is not offered"
    );
}

#[tokio::test]
async fn not_now_suppresses_the_offer_for_that_pillar() {
    let fx = setup().await;
    let before = fx.provider.calls_so_far();

    let turn = fx.tap("ta:no:fuelling").await;

    assert_eq!(
        turn.assistant.message.content,
        fx.tap_answer(KEY_AGENTS_TEAM_OFFER_DISMISSED, &[])
    );
    assert_eq!(fx.provider.calls_so_far(), before, "no model ran");
    assert!(!fx.on_team().await, "Not now joins nothing");

    fx.land(&[Pillar::Fuelling]).await;
    assert_eq!(
        values(&fx.wrap_up().await.next_steps.into_actions()),
        ["nx:0:about_me_more"],
        "a declined pillar is not offered again"
    );
}

#[tokio::test]
async fn an_agent_already_on_the_team_is_never_offered() {
    let fx = setup().await;
    install_catalogue_agent(
        fx.resources.agent.database.repositories(),
        fx.fuelling,
        fx.user_id,
        fx.tenant_id,
    )
    .await;
    fx.land(&[Pillar::Fuelling, Pillar::MentalResilience]).await;

    assert_eq!(
        values(&fx.wrap_up().await.next_steps.into_actions()),
        ["nx:0:about_me_more"],
        "the member is not offered, and mental resilience has no agent to offer"
    );
}

#[tokio::test]
async fn the_agent_leading_the_walks_thread_is_never_offered() {
    let fx = setup().await;
    fx.land(&[Pillar::Fuelling]).await;
    let walk = fx.spent_walk().await;
    let user_id = fx.user_id.to_string();
    let chat = &fx.ctx.repos.chat;
    let led = chat
        .create_conversation(
            &user_id,
            fx.tenant_id,
            "Walk",
            "mock-model",
            Some(&fx.fuelling.to_string()),
            None,
        )
        .await
        .unwrap();
    chat.set_conversation_onboarding_state(&led.id, walk.onboarding_state.as_deref(), fx.tenant_id)
        .await
        .unwrap();
    let led = chat
        .get_conversation(&led.id, &user_id, fx.tenant_id)
        .await
        .unwrap()
        .unwrap();

    let GuidedResolution::WalkComplete { reply, .. } =
        onboarding::resolve(&fx.ctx, &led, &user_id, fx.tenant_id, fx.tenant_id, LOCALE).await
    else {
        panic!("the walk had nothing left to ask");
    };
    assert_eq!(
        values(&reply.next_steps.into_actions()),
        ["nx:0:about_me_more"],
        "the lead is never offered to its own athlete"
    );
}
