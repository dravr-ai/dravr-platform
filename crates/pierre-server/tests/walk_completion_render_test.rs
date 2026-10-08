// ABOUTME: The guided walks' completions report what actually landed, read through the real renderer and resolver
// ABOUTME: Pins the time window, shared-kind crediting, named gaps, the pillars walk's event and a wrap-up's next steps
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `completion::render` and `completion::render_season` close a guided walk
//! with a platform-written message: a header stating how many asked topics
//! produced a fact, then either the safety answers still missing (calibration)
//! or whether the race calendar landed (season). These tests seed the facts an
//! interview would have written, render the wrap-up against a real
//! `ChatPipelineContext`, and compare it line by line with the strings the
//! same registry renders for the expected counts.
//!
//! The pillars walk ends through the guided resolver rather than a renderer:
//! it reports `onboarding.completed` from the same window and leaves the turn
//! to the agent. And a platform reply's next steps (carnet#830) are delivered
//! the way a command's controls are — on the live envelope, and on the stored
//! row a reload reads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use pierre_core::transport::TransportPolicy;
use std::sync::Arc;

use chrono::{Duration, Utc};
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user_with_plan};
use helpers::notify_capture::{capture_notify, named, only};
use pierre_chat_pipeline::next_steps::NextSteps;
use pierre_chat_pipeline::stages::completion;
use pierre_chat_pipeline::stages::deterministic_reply::{
    deliver, DeterministicReplyInputs, PlatformReply, DETERMINISTIC_FINISH_REASON,
};
use pierre_chat_pipeline::stages::onboarding::{self, GuidedResolution};
use pierre_chat_pipeline::{
    ActionKind, ChatPipelineContext, InputSource, QuotaState, ReplyBlock, SurfaceId,
    SurfaceProfile, SurfaceRequest, TurnAction, TurnInput, TurnOrigin,
};
use pierre_contremaitre::messaging_strings::{
    KEY_CALIBRATE_COMPLETE_HEADER, KEY_CALIBRATE_COMPLETE_MISSING, KEY_CALIBRATE_TOPIC_INJURY,
    KEY_CALIBRATE_TOPIC_RECOVERY, KEY_SEASON_COMPLETE_HEADER, KEY_SEASON_COMPLETE_MISSING_GOAL,
    KEY_SEASON_FOLLOWUP_LAY_OUT,
};
use pierre_core::models::{
    AddMessageParams, CalibrationTopic, ConversationRecord, ConversationTurnId, CoverageTarget,
    Dossier, DossierFact, GuidedFlow, OnboardingState, PersistedAction, PersistedReplyBlock,
    Pillar, SeasonTopic, TenantId, TopicVisibility, WalkAudience, MAX_PROBE_ATTEMPTS,
};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};

const LOCALE: &str = "en";

/// One athlete with a fresh fact store, and the pipeline context to render for them.
struct Athlete {
    ctx: ChatPipelineContext,
    tenant_id: TenantId,
    user_id: Uuid,
}

async fn resources() -> Arc<ServerContext> {
    create_test_server_resources().await.unwrap()
}

async fn athlete(resources: &Arc<ServerContext>) -> Athlete {
    let email = format!("walk-{}@example.com", Uuid::new_v4());
    let (user_id, _user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, &email, "professional")
            .await
            .unwrap();
    Athlete {
        ctx: resources.chat_pipeline_context(),
        tenant_id,
        user_id,
    }
}

impl Athlete {
    /// Write the facts an interview answer produces: source onboarding, one per kind given.
    async fn land(&self, kinds: &[FactKind]) {
        let answers: Vec<(FactKind, Option<Pillar>)> =
            kinds.iter().map(|kind| (*kind, None)).collect();
        self.land_answers(&answers).await;
    }

    /// Write one onboarding fact per answer, filed under the pillar given —
    /// how extraction stamps a pillars-walk answer.
    async fn land_answers(&self, answers: &[(FactKind, Option<Pillar>)]) {
        let user_id = self.user_id.to_string();
        for (n, (kind, pillar)) in answers.iter().enumerate() {
            let object = format!("answer {n} ({})", kind.as_str());
            self.ctx
                .repos
                .memory
                .upsert_user_fact(&UpsertUserFactParams {
                    tenant_id: self.tenant_id,
                    user_id: &user_id,
                    agent_id: None,
                    scope: MemoryScope::User,
                    kind: *kind,
                    pillar: *pillar,
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

    async fn calibration_wrap_up(&self, started_at: String, dossier: &Dossier) -> String {
        let state = OnboardingState::start(started_at, GuidedFlow::Calibration);
        completion::render(
            &self.ctx,
            &state,
            self.tenant_id,
            &self.user_id.to_string(),
            dossier,
            LOCALE,
        )
        .await
        .text
    }

    async fn season_wrap_up(&self, started_at: String) -> String {
        let state = OnboardingState::start(started_at, GuidedFlow::Season);
        completion::render_season(
            &self.ctx,
            &state,
            self.tenant_id,
            &self.user_id.to_string(),
            LOCALE,
        )
        .await
        .text
    }

    /// A conversation of the athlete's own, holding `state` as its guided flow.
    async fn conversation_in(&self, state: &OnboardingState) -> ConversationRecord {
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

    /// Resolve the turn that finds `conversation`'s pillars walk with nothing
    /// left to ask.
    async fn resolve_pillars_end(&self, conversation: &ConversationRecord) -> GuidedResolution {
        onboarding::resolve(
            &self.ctx,
            conversation,
            &self.user_id.to_string(),
            self.tenant_id,
            self.tenant_id,
            LOCALE,
        )
        .await
    }

    fn render(&self, key: &str, args: &[&str]) -> String {
        self.ctx
            .messaging_strings_registry
            .render(key, LOCALE, args)
    }

    /// The calibration header for `captured` of `asked`.
    fn calibration_header(&self, captured: usize, asked: usize) -> String {
        self.render(
            KEY_CALIBRATE_COMPLETE_HEADER,
            &[&captured.to_string(), &asked.to_string()],
        )
    }

    /// The safety topics the wrap-up names as missing, in the order it names them.
    fn named_missing(&self, wrap_up: &str) -> Vec<CalibrationTopic> {
        let line = |key: &str| {
            let label = self.render(key, &[]);
            self.render(KEY_CALIBRATE_COMPLETE_MISSING, &[&label])
        };
        let mut named: Vec<(usize, CalibrationTopic)> = [
            (KEY_CALIBRATE_TOPIC_INJURY, CalibrationTopic::Injury),
            (
                KEY_CALIBRATE_TOPIC_RECOVERY,
                CalibrationTopic::RecoverySpeed,
            ),
        ]
        .into_iter()
        .filter_map(|(key, topic)| wrap_up.find(&line(key)).map(|at| (at, topic)))
        .collect();
        named.sort_by_key(|(at, _)| *at);
        named.into_iter().map(|(_, topic)| topic).collect()
    }
}

/// No conditional topic applies: the six core topics are asked.
fn core_dossier() -> Dossier {
    Dossier::empty(Uuid::nil(), Uuid::nil())
}

/// A dated goal on file: fueling and event demand join the six core topics.
fn dated_goal_dossier() -> Dossier {
    let mut dossier = core_dossier();
    dossier.pillars.insert(
        Pillar::TrainingAndMovement,
        vec![DossierFact {
            kind: FactKind::Goal.as_str().to_owned(),
            predicate_code: "working_toward".to_owned(),
            object: "a spring marathon".to_owned(),
            confidence: 0.9,
            source: "onboarding".to_owned(),
            updated_at: Utc::now(),
            valid_until: None,
            stale: false,
        }],
    );
    dossier
}

/// An interview that began an hour ago: every fact landed since counts.
fn an_hour_ago() -> String {
    (Utc::now() - Duration::hours(1)).to_rfc3339()
}

/// An interview that begins after the seeded facts: none of them count.
fn after_the_facts() -> String {
    (Utc::now() + Duration::minutes(1)).to_rfc3339()
}

const CORE_ASKED: usize = CalibrationTopic::CORE.len();

#[tokio::test]
async fn a_silent_interview_reports_zero_and_names_both_safety_gaps() {
    // The failure this whole module exists for: every question asked,
    // nothing extracted. It must not read as success.
    let resources = resources().await;
    let a = athlete(&resources).await;
    let wrap_up = a.calibration_wrap_up(an_hour_ago(), &core_dossier()).await;
    assert!(wrap_up.starts_with(&a.calibration_header(0, CORE_ASKED)));
    assert_eq!(
        a.named_missing(&wrap_up),
        vec![CalibrationTopic::Injury, CalibrationTopic::RecoverySpeed]
    );
}

#[tokio::test]
async fn facts_from_before_the_interview_do_not_count() {
    // A pillars walk months ago also wrote `source=onboarding` facts. If
    // those counted, an interview that landed nothing would report a full
    // house and the athlete would never be asked again.
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.land(&[FactKind::Injury, FactKind::Physiology]).await;
    let wrap_up = a
        .calibration_wrap_up(after_the_facts(), &core_dossier())
        .await;
    assert!(
        wrap_up.starts_with(&a.calibration_header(0, CORE_ASKED)),
        "facts older than the interview predate it: {wrap_up}"
    );
    assert_eq!(a.named_missing(&wrap_up).len(), 2);
}

#[tokio::test]
async fn a_landed_injury_answer_clears_only_that_safety_gap() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.land(&[FactKind::Injury]).await;
    let wrap_up = a.calibration_wrap_up(an_hour_ago(), &core_dossier()).await;
    assert!(wrap_up.starts_with(&a.calibration_header(1, CORE_ASKED)));
    assert_eq!(
        a.named_missing(&wrap_up),
        vec![CalibrationTopic::RecoverySpeed],
        "recovery speed is still unanswered and must still be named"
    );
}

#[tokio::test]
async fn a_complete_interview_names_no_gaps() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.land(&[
        FactKind::Preference, // progression intent
        FactKind::Preference, // baseline confirm
        FactKind::Schedule,   // availability
        FactKind::Injury,     // injury
        FactKind::Preference, // rpe headroom
        FactKind::Physiology, // recovery speed
    ])
    .await;
    let wrap_up = a.calibration_wrap_up(an_hour_ago(), &core_dossier()).await;
    assert!(
        wrap_up.starts_with(&a.calibration_header(6, CORE_ASKED)),
        "all six core topics produced a fact: {wrap_up}"
    );
    assert!(a.named_missing(&wrap_up).is_empty());
}

#[tokio::test]
async fn shared_kinds_are_credited_once_per_fact_not_once_per_topic() {
    // Three topics write `preference`. One preference fact must credit one
    // topic, not three — over-reporting is the direction that makes the
    // wrap-up a lie.
    let resources = resources().await;

    let one = athlete(&resources).await;
    one.land(&[FactKind::Preference]).await;
    let wrap_up = one
        .calibration_wrap_up(an_hour_ago(), &core_dossier())
        .await;
    assert!(wrap_up.starts_with(&one.calibration_header(1, CORE_ASKED)));

    let two = athlete(&resources).await;
    two.land(&[FactKind::Preference, FactKind::Preference])
        .await;
    let wrap_up = two
        .calibration_wrap_up(an_hour_ago(), &core_dossier())
        .await;
    assert!(wrap_up.starts_with(&two.calibration_header(2, CORE_ASKED)));
}

#[tokio::test]
async fn a_conditional_topic_that_was_asked_is_counted_in_the_denominator() {
    // A dated goal adds fueling and event demand to the six core topics, so
    // eight are asked; the goal answer credits event demand alone.
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.land(&[FactKind::Goal]).await;
    let wrap_up = a
        .calibration_wrap_up(an_hour_ago(), &dated_goal_dossier())
        .await;
    assert!(
        wrap_up.starts_with(&a.calibration_header(1, CORE_ASKED + 2)),
        "only the event-demand answer landed, out of eight asked: {wrap_up}"
    );
    assert_eq!(
        a.named_missing(&wrap_up).len(),
        2,
        "both safety topics are still missing"
    );
}

/// The season header for `captured` of the core season topics, and whether
/// the wrap-up says the calendar is missing.
fn season_verdict(a: &Athlete, wrap_up: &str, captured: usize) -> bool {
    let asked = SeasonTopic::CORE.len().to_string();
    let header = a.render(KEY_SEASON_COMPLETE_HEADER, &[&captured.to_string(), &asked]);
    assert!(
        wrap_up.starts_with(&header),
        "expected {captured} of {asked} captured: {wrap_up}"
    );
    let missing = wrap_up.ends_with(&a.render(KEY_SEASON_COMPLETE_MISSING_GOAL, &[]));
    let lay_out = wrap_up.ends_with(&a.render(KEY_SEASON_FOLLOWUP_LAY_OUT, &[]));
    assert!(
        missing != lay_out,
        "exactly one follow-up closes the wrap-up"
    );
    missing
}

#[tokio::test]
async fn a_season_walk_with_no_goal_fact_is_short_a_calendar() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.land(&[
        FactKind::Physiology,
        FactKind::Preference,
        FactKind::Equipment,
        FactKind::Preference,
    ])
    .await;
    let wrap_up = a.season_wrap_up(an_hour_ago()).await;
    // Bests, background, tools and coaching fit landed.
    let goal_missing = season_verdict(&a, &wrap_up, 4);
    assert!(
        goal_missing,
        "no goal fact means no calendar to lay a season on"
    );
}

#[tokio::test]
async fn a_goal_fact_credits_the_calendar_and_the_horizon_separately() {
    let resources = resources().await;

    let both = athlete(&resources).await;
    both.land(&[FactKind::Goal, FactKind::Goal]).await;
    let wrap_up = both.season_wrap_up(an_hour_ago()).await;
    // Two goal facts credit both goal topics.
    assert!(!season_verdict(&both, &wrap_up, 2));

    let one = athlete(&resources).await;
    one.land(&[FactKind::Goal]).await;
    let wrap_up = one.season_wrap_up(an_hour_ago()).await;
    // One goal fact credits the calendar, asked first.
    assert!(!season_verdict(&one, &wrap_up, 1));
}

#[tokio::test]
async fn season_facts_before_the_window_do_not_count() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.land(&[FactKind::Goal, FactKind::Equipment]).await;
    let wrap_up = a.season_wrap_up(after_the_facts()).await;
    assert!(season_verdict(&a, &wrap_up, 0));
}

/// Every pillars-walk topic: the North Star and the six pillars.
const PILLARS_TOPICS: usize = 1 + Pillar::ALL.len();

/// A pillars walk whose every topic has burned its probe budget, so the turn
/// that reads it finds nothing left to ask.
fn spent_pillars_walk(started_at: String) -> OnboardingState {
    spent_walk_for(started_at, WalkAudience::Private)
}

/// [`spent_pillars_walk`] for `audience`: a room walk only ever asks the
/// topics a room may hear, so only those are in its ledger.
fn spent_walk_for(started_at: String, audience: WalkAudience) -> OnboardingState {
    let mut topics = vec![CoverageTarget::NorthStar];
    topics.extend(Pillar::ALL.into_iter().map(CoverageTarget::Pillar));
    topics.retain(|topic| {
        audience == WalkAudience::Private || topic.visibility() == TopicVisibility::RoomSafe
    });
    let mut state = OnboardingState::start(started_at, GuidedFlow::Pillars).with_audience(audience);
    for _ in 0..MAX_PROBE_ATTEMPTS {
        for topic in &topics {
            state = state.with_delivered_probe(topic.slug());
        }
    }
    state
}

#[tokio::test]
async fn the_pillars_walk_reports_its_completion_and_leaves_the_last_turn_to_the_agent() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    // The North Star answer and the fuelling answer landed; the five other
    // pillars were asked and produced nothing.
    a.land_answers(&[
        (FactKind::NorthStar, None),
        (FactKind::Preference, Some(Pillar::Fuelling)),
    ])
    .await;
    let conversation = a.conversation_in(&spent_pillars_walk(an_hour_ago())).await;

    let (events, _guard) = capture_notify();
    let resolution = a.resolve_pillars_end(&conversation).await;

    assert!(
        matches!(resolution, GuidedResolution::Inactive),
        "the platform has no pillars wrap-up, so the agent answers the walk's last turn"
    );
    let completed = only(&events, "onboarding.completed");
    assert_eq!(completed.field("flow"), "pillars");
    assert_eq!(completed.field("topics_asked"), PILLARS_TOPICS.to_string());
    assert_eq!(completed.field("topics_answered"), "2");
    assert_eq!(completed.field("facts_landed"), "2");

    // Retired rather than cleared, so the next turn is told the interview's
    // no-writing rule is lifted.
    let stored = a
        .ctx
        .repos
        .chat
        .get_conversation(&conversation.id, &a.user_id.to_string(), a.tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        OnboardingState::retired_flow(stored.onboarding_state.as_deref()),
        Some(GuidedFlow::Pillars)
    );
}

#[tokio::test]
async fn pillars_facts_from_before_the_walk_do_not_count() {
    // An earlier walk's answers cover these topics; this walk landed nothing.
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.land_answers(&[
        (FactKind::NorthStar, None),
        (FactKind::Preference, Some(Pillar::Fuelling)),
    ])
    .await;
    let conversation = a
        .conversation_in(&spent_pillars_walk(after_the_facts()))
        .await;

    let (events, _guard) = capture_notify();
    a.resolve_pillars_end(&conversation).await;

    let completed = only(&events, "onboarding.completed");
    assert_eq!(completed.field("topics_asked"), PILLARS_TOPICS.to_string());
    assert_eq!(completed.field("topics_answered"), "0");
    assert_eq!(completed.field("facts_landed"), "0");
}

#[tokio::test]
async fn a_room_walk_counts_only_the_topics_a_room_may_hear() {
    // The mental-resilience answer landed in the window, but a room walk never
    // asks a DM-only pillar: it is a fact landed, not a topic answered.
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.land_answers(&[
        (FactKind::NorthStar, None),
        (FactKind::Preference, Some(Pillar::MentalResilience)),
    ])
    .await;
    let conversation = a
        .conversation_in(&spent_walk_for(an_hour_ago(), WalkAudience::Room))
        .await;

    let (events, _guard) = capture_notify();
    a.resolve_pillars_end(&conversation).await;

    let room_topics = 1 + Pillar::ALL
        .into_iter()
        .filter(|pillar| pillar.visibility() == TopicVisibility::RoomSafe)
        .count();
    assert!(room_topics < PILLARS_TOPICS, "a room hears fewer topics");
    let completed = only(&events, "onboarding.completed");
    assert_eq!(completed.field("topics_asked"), room_topics.to_string());
    assert_eq!(completed.field("topics_answered"), "1");
    assert_eq!(completed.field("facts_landed"), "2");
}

#[tokio::test]
async fn a_walk_still_under_way_reports_no_completion() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    let conversation = a
        .conversation_in(&OnboardingState::start(an_hour_ago(), GuidedFlow::Pillars))
        .await;

    let (events, _guard) = capture_notify();
    let resolution = a.resolve_pillars_end(&conversation).await;

    assert!(matches!(resolution, GuidedResolution::Probe(_)));
    assert!(named(&events, "onboarding.completed").is_empty());
}

fn step(label: &str, value: &str) -> TurnAction {
    TurnAction {
        label: label.to_owned(),
        kind: ActionKind::Postback,
        value: value.to_owned(),
    }
}

#[tokio::test]
async fn a_platform_reply_offers_its_next_steps_live_and_on_the_stored_row() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    let user_id = a.user_id.to_string();
    let conversation = a
        .ctx
        .repos
        .chat
        .create_conversation(&user_id, a.tenant_id, "Walk", "mock-model", None, None)
        .await
        .unwrap();
    let user_message = a
        .ctx
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: a.tenant_id,
            conversation_id: &conversation.id,
            user_id: &user_id,
            role: "user",
            content: "That covers my season.",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
    let input = TurnInput {
        conversation_id: conversation.id.clone(),
        user_id: user_id.clone(),
        conversation_tenant_id: a.tenant_id,
        tool_tenant_id: a.tenant_id,
        is_direct_message: true,
        content: user_message.content.clone(),
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
    };
    let profile = SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: LOCALE.to_owned(),
        transport: None,
        prose_contract: None,
    });
    let steps = vec![
        step("Plan my fortnight", "/fortnight"),
        step("Today's session", "/plan today"),
    ];

    let envelope = deliver(
        DeterministicReplyInputs {
            ctx: &a.ctx,
            input: &input,
            profile: &profile,
            active_model: "mock-model".to_owned(),
            user_message,
            conv: &conversation,
        },
        PlatformReply {
            text: "Your season is laid out.".to_owned(),
            next_steps: NextSteps::new(steps.clone()),
        },
    )
    .await
    .unwrap();

    assert!(
        envelope.assistant.blocks.contains(&ReplyBlock::Actions {
            title: None,
            actions: steps,
        }),
        "the live reply carries the steps as buttons: {:?}",
        envelope.assistant.blocks
    );

    let rows = a
        .ctx
        .repos
        .chat
        .get_messages(&conversation.id, &user_id, a.tenant_id)
        .await
        .unwrap();
    let reply = rows
        .iter()
        .find(|row| row.finish_reason.as_deref() == Some(DETERMINISTIC_FINISH_REASON))
        .expect("the platform reply is persisted");
    assert_eq!(reply.content, "Your season is laid out.");
    let stored: Vec<PersistedReplyBlock> = serde_json::from_str(
        reply
            .content_blocks
            .as_deref()
            .expect("the steps are stored"),
    )
    .unwrap();
    let persisted = |label: &str, value: &str| PersistedAction {
        label: label.to_owned(),
        action_type: "postback".to_owned(),
        value: value.to_owned(),
    };
    assert_eq!(
        stored,
        [PersistedReplyBlock::Actions {
            title: None,
            actions: vec![
                persisted("Plan my fortnight", "/fortnight"),
                persisted("Today's session", "/plan today"),
            ],
        }],
        "a reload shows the buttons the live turn did"
    );
}
