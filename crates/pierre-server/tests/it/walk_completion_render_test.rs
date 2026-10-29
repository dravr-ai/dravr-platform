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
//! The pillars walk ends through the guided resolver: it reports
//! `onboarding.completed` from the same window, and its wrap-up states how
//! many of the pillars its audience may hear are covered. Each ending also
//! picks its next steps from the transition table (carnet#830), filtered by what the athlete's state allows — the
//! season after the pillars, or the walk again while pillars are open, the
//! fortnight once a season is set, the calendar and today's session once the
//! fortnight landed — and a platform
//! reply delivers them the way a command's controls are: on the live
//! envelope, and on the stored row a reload reads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use pierre_core::transport::TransportPolicy;
use std::sync::Arc;

use chrono::{Datelike, Duration, Utc};
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user_with_plan};
use helpers::notify_capture::{capture_notify, named, only};
use pierre_chat_pipeline::stages::completion;
use pierre_chat_pipeline::stages::deterministic_reply::{
    deliver, DeterministicReplyInputs, PlatformReply, DETERMINISTIC_FINISH_REASON,
};
use pierre_chat_pipeline::stages::onboarding::{self, GuidedResolution, GuidedTarget};
use pierre_chat_pipeline::{
    ActionKind, ChatPipelineContext, InputSource, QuotaState, ReplyBlock, SurfaceId,
    SurfaceProfile, SurfaceRequest, TurnAction, TurnEnvelope, TurnInput, TurnOrigin,
};
use pierre_contremaitre::messaging_strings::{
    KEY_CALIBRATE_COMPLETE_HEADER, KEY_CALIBRATE_COMPLETE_MISSING, KEY_CALIBRATE_TOPIC_INJURY,
    KEY_CALIBRATE_TOPIC_RECOVERY, KEY_FORTNIGHT_RECHECK_LANDED, KEY_FORTNIGHT_RECHECK_MISSING,
    KEY_PILLARS_COMPLETED, KEY_SEASON_COMPLETE_HEADER, KEY_SEASON_COMPLETE_MISSING_GOAL,
    KEY_SEASON_FOLLOWUP_LAY_OUT,
};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{
    AddMessageParams, CalibrationTopic, ConnectionType, ConversationRecord, ConversationTurnId,
    CoverageTarget, Dossier, DossierFact, GuidedFlow, MessageRecord, OnboardingState,
    PersistedAction, PersistedReplyBlock, Pillar, SeasonTopic, TenantId, TopicVisibility,
    WalkAudience, MAX_PROBE_ATTEMPTS,
};
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{
    PlanOutlineInput, PlanWeekInput, SavePlanBundleParams, UpsertUserFactParams,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use std::collections::BTreeMap;

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
    assert_eq!(a.named_missing(&wrap_up), []);
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
async fn the_pillars_walk_states_its_count_and_answers_its_last_turn() {
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

    let GuidedResolution::WalkComplete { reply, answered } = resolution else {
        panic!("the walk had nothing left to ask, so the platform's wrap-up answers the turn");
    };
    assert_eq!(
        reply.text,
        a.render(
            KEY_PILLARS_COMPLETED,
            &["1", &Pillar::ALL.len().to_string()]
        ),
        "the count is the platform's, read from the facts that landed, and counts \
         pillars: the North Star landed too but is not one"
    );
    assert_eq!(
        values(&reply.next_steps.into_actions()),
        ["nx:0:about_me_more"],
        "pillars still untold: no season to plan yet and no plan to calibrate, so the \
         walk is offered again"
    );
    assert_eq!(
        answered,
        Pillar::ALL
            .last()
            .map(|pillar| GuidedTarget::Coverage(CoverageTarget::Pillar(*pillar))),
        "the turn answers the walk's last question — the last pillar its ledger \
         asked — which still has to be extracted"
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

/// The postbacks `actions` send, slot by slot.
fn values(actions: &[TurnAction]) -> Vec<String> {
    actions.iter().map(|action| action.value.clone()).collect()
}

/// Every pillar told, and the North Star: the dossier is complete.
async fn every_pillar_told(a: &Athlete) {
    let mut answers = vec![(FactKind::NorthStar, None)];
    answers.extend(
        Pillar::ALL
            .into_iter()
            .map(|pillar| (FactKind::Preference, Some(pillar))),
    );
    a.land_answers(&answers).await;
}

impl Athlete {
    /// A season laid out from this Monday, with `weeks` written weeks — two
    /// cover the fortnight the rail drafts.
    async fn season(&self, weeks: usize) {
        let today = Utc::now().date_naive();
        let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
        let race = GoalRace {
            name: "Gran Fondo".to_owned(),
            date: (monday + Duration::weeks(12)).to_string(),
            discipline: "road".to_owned(),
            priority: RacePriority::A,
        };
        let phases = [PlanPhase {
            kind: PhaseKind::Build,
            start: monday.to_string(),
            weeks: 12,
            intent: "build the engine".to_owned(),
            target_hours: Some(8.0),
            purpose: String::new(),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: None,
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
        }];
        let starts: Vec<String> = (0..weeks)
            .map(|week| (monday + Duration::weeks(i64::try_from(week).unwrap())).to_string())
            .collect();
        let days: Vec<[PlannedDay; 1]> = starts
            .iter()
            .map(|start| {
                [PlannedDay {
                    date: start.clone(),
                    sport: "ride".to_owned(),
                    workout: "endurance".to_owned(),
                    duration_min: Some(90),
                    intensity: "Z2".to_owned(),
                    steps: Vec::new(),
                    fueling: None,
                    fuel_product_ids: Vec::new(),
                    template_slug: None,
                    template_params: None,
                    template_source: None,
                }]
            })
            .collect();
        let written: Vec<PlanWeekInput<'_>> = starts
            .iter()
            .zip(&days)
            .map(|(start, days)| PlanWeekInput {
                week_start: start,
                focus: "volume",
                days,
                adjustment_reason: "",
                phase_index: Some(0),
            })
            .collect();
        self.ctx
            .repos
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &self.tenant_id.to_string(),
                user_id: &self.user_id.to_string(),
                author: PlanAuthor::agent("endurance-coach"),
                goal_fact_id: None,
                replace_season: false,
                outline: Some(PlanOutlineInput {
                    goal_race: &race,
                    races: Some(&[]),
                    strategy: "build, then sharpen",
                    flavour: None,
                    season_start: None,
                    season_end: None,
                    phases: &phases,
                    source_conversation_id: None,
                }),
                weeks: &written,
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();
    }

    /// A connection to a provider that takes a pushed plan.
    async fn calendar(&self) {
        self.ctx
            .repos
            .provider_connections
            .register_connection(
                self.user_id,
                self.tenant_id,
                "intervals_icu",
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
    }

    async fn season_reply(&self) -> PlatformReply {
        let state = OnboardingState::start(an_hour_ago(), GuidedFlow::Season);
        completion::render_season(
            &self.ctx,
            &state,
            self.tenant_id,
            &self.user_id.to_string(),
            LOCALE,
        )
        .await
    }

    async fn fortnight_reply(&self, audience: WalkAudience) -> PlatformReply {
        completion::render_fortnight(
            &self.ctx,
            self.tenant_id,
            &self.user_id.to_string(),
            audience,
            LOCALE,
        )
        .await
    }
}

#[tokio::test]
async fn a_pillars_walk_with_every_pillar_told_leads_to_the_season() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    every_pillar_told(&a).await;
    let conversation = a.conversation_in(&spent_pillars_walk(an_hour_ago())).await;

    let GuidedResolution::WalkComplete { reply, .. } = a.resolve_pillars_end(&conversation).await
    else {
        panic!("the walk had nothing left to ask");
    };
    let steps = reply.next_steps.into_actions();
    assert_eq!(
        values(&steps),
        ["nx:0:season"],
        "no plan yet, so no calibration: the season is the one step"
    );
    assert_eq!(steps[0].kind, ActionKind::Postback);
    assert_eq!(steps[0].label, a.render("use_cases.season.label", &[]));
}

#[tokio::test]
async fn a_room_walk_offers_no_step_even_when_its_subject_could_take_one() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    every_pillar_told(&a).await;
    let conversation = a
        .conversation_in(&spent_walk_for(an_hour_ago(), WalkAudience::Room))
        .await;

    let GuidedResolution::WalkComplete { reply, .. } = a.resolve_pillars_end(&conversation).await
    else {
        panic!("the walk had nothing left to ask");
    };
    assert!(
        reply.next_steps.is_empty(),
        "steps are picked from one athlete's state, and the room would read them"
    );
    let room_pillars = Pillar::ALL
        .into_iter()
        .filter(|pillar| pillar.visibility() == TopicVisibility::RoomSafe)
        .count()
        .to_string();
    assert_eq!(
        reply.text,
        a.render(KEY_PILLARS_COMPLETED, &[&room_pillars, &room_pillars]),
        "a room is told only about the pillars it may hear, never the DM-only ones"
    );
}

#[tokio::test]
async fn a_rerun_over_a_complete_profile_reports_it_complete() {
    // `/pillars` on a profile that was already complete asks nothing, so the
    // walk's own count is nothing of nothing; the wrap-up states the profile.
    let resources = resources().await;
    let a = athlete(&resources).await;
    every_pillar_told(&a).await;
    let conversation = a
        .conversation_in(&OnboardingState::start(an_hour_ago(), GuidedFlow::Pillars))
        .await;

    let (events, _guard) = capture_notify();
    let GuidedResolution::WalkComplete { reply, .. } = a.resolve_pillars_end(&conversation).await
    else {
        panic!("every topic was covered before the walk began");
    };
    let total = Pillar::ALL.len().to_string();
    assert_eq!(
        reply.text,
        a.render(KEY_PILLARS_COMPLETED, &[&total, &total])
    );
    assert_eq!(
        only(&events, "onboarding.completed").field("topics_asked"),
        "0"
    );
}

#[tokio::test]
async fn the_calibration_wrap_up_closes_on_its_question_and_offers_no_step() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.season(2).await;
    let state = OnboardingState::start(an_hour_ago(), GuidedFlow::Calibration);
    let reply = completion::render(
        &a.ctx,
        &state,
        a.tenant_id,
        &a.user_id.to_string(),
        &core_dossier(),
        LOCALE,
    )
    .await;
    assert!(reply.next_steps.is_empty());
}

#[tokio::test]
async fn the_season_wrap_up_offers_the_fortnight_only_over_a_season_already_set() {
    let resources = resources().await;

    let first = athlete(&resources).await;
    assert!(
        first.season_reply().await.next_steps.is_empty(),
        "a first season is not laid out until the athlete says yes"
    );

    let rerun = athlete(&resources).await;
    rerun.season(1).await;
    assert_eq!(
        values(&rerun.season_reply().await.next_steps.into_actions()),
        ["nx:0:fortnight"]
    );
}

#[tokio::test]
async fn a_landed_fortnight_leads_to_the_calendar_and_today() {
    let resources = resources().await;

    let landed = athlete(&resources).await;
    landed.season(2).await;
    landed.calendar().await;
    let reply = landed.fortnight_reply(WalkAudience::Private).await;
    assert_eq!(reply.text, landed.render(KEY_FORTNIGHT_RECHECK_LANDED, &[]));
    assert_eq!(
        values(&reply.next_steps.into_actions()),
        ["nx:0:calendar", "nx:1:today"]
    );

    let no_calendar = athlete(&resources).await;
    no_calendar.season(2).await;
    assert_eq!(
        values(
            &no_calendar
                .fortnight_reply(WalkAudience::Private)
                .await
                .next_steps
                .into_actions()
        ),
        ["nx:0:today"],
        "no connection takes a pushed plan"
    );

    assert!(
        landed
            .fortnight_reply(WalkAudience::Room)
            .await
            .next_steps
            .is_empty(),
        "a room rail offers nothing"
    );

    let missing = athlete(&resources).await;
    let reply = missing.fortnight_reply(WalkAudience::Private).await;
    assert_eq!(
        reply.text,
        missing.render(KEY_FORTNIGHT_RECHECK_MISSING, &[])
    );
    assert!(
        reply.next_steps.is_empty(),
        "every step stands on weeks that did not land"
    );
}

impl Athlete {
    /// Deliver `reply` as the platform's answer to the athlete's line in a
    /// fresh thread of theirs: alone with the agent, or read by a room.
    async fn deliver_reply(
        &self,
        is_direct_message: bool,
        reply: PlatformReply,
    ) -> (ConversationRecord, TurnEnvelope) {
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
                content: "That covers my season.",
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
            })
            .await
            .unwrap();
        let input = TurnInput {
            conversation_id: conversation.id.clone(),
            user_id,
            conversation_tenant_id: self.tenant_id,
            tool_tenant_id: self.tenant_id,
            is_direct_message,
            content: user_message.content.clone(),
            origin: TurnOrigin::Athlete,
            input_source: InputSource::Typed,
            turn_id: ConversationTurnId::new(),
            ambient_context: None,
            quote: None,
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
        (conversation, envelope)
    }

    /// The platform's persisted reply in `conversation`.
    async fn platform_row(&self, conversation: &ConversationRecord) -> MessageRecord {
        self.ctx
            .repos
            .chat
            .get_messages(&conversation.id, &self.user_id.to_string(), self.tenant_id)
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.finish_reason.as_deref() == Some(DETERMINISTIC_FINISH_REASON))
            .expect("the platform reply is persisted")
    }
}

#[tokio::test]
async fn a_platform_reply_offers_its_next_steps_live_and_on_the_stored_row() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.season(1).await;
    let reply = a.season_reply().await;
    let text = reply.text.clone();
    let steps = reply.next_steps.clone().into_actions();
    assert_eq!(values(&steps), ["nx:0:fortnight"]);

    let (events, _guard) = capture_notify();
    let (conversation, envelope) = a.deliver_reply(true, reply).await;

    assert!(
        envelope.assistant.blocks.contains(&ReplyBlock::Actions {
            title: None,
            actions: steps.clone(),
        }),
        "the live reply carries the steps as buttons: {:?}",
        envelope.assistant.blocks
    );
    assert!(
        !envelope
            .assistant
            .blocks
            .iter()
            .any(|block| matches!(block, ReplyBlock::Scene { .. })),
        "the stored steps are not read back as a chart"
    );
    let shown = only(&events, "suggestion.shown");
    assert_eq!(shown.field("use_case"), "fortnight");
    assert_eq!(shown.field("source"), "next_step");
    assert_eq!(shown.field("surface"), "next_step");
    assert_eq!(shown.field("position"), "0");
    assert_eq!(shown.field("channel"), SurfaceId::Web.as_str());

    let stored_reply = a.platform_row(&conversation).await;
    assert_eq!(stored_reply.content, text);
    let stored: Vec<PersistedReplyBlock> = serde_json::from_str(
        stored_reply
            .content_blocks
            .as_deref()
            .expect("the steps are stored"),
    )
    .unwrap();
    assert_eq!(
        stored,
        [PersistedReplyBlock::Actions {
            title: None,
            actions: vec![PersistedAction {
                label: steps[0].label.clone(),
                action_type: "postback".to_owned(),
                value: "nx:0:fortnight".to_owned(),
            }],
        }],
        "a reload shows the buttons the live turn did"
    );
}

/// A wrap-up's steps are picked from its subject's state, so a reply a room
/// reads offers none — whatever the renderer picked — live, on the row, or
/// in the analytics a showing reports.
#[tokio::test]
async fn a_platform_reply_a_room_reads_offers_no_step() {
    let resources = resources().await;
    let a = athlete(&resources).await;
    a.season(1).await;
    let reply = a.season_reply().await;
    let text = reply.text.clone();
    assert_eq!(
        values(&reply.next_steps.clone().into_actions()),
        ["nx:0:fortnight"],
        "the renderer picked a step for the athlete"
    );

    let (events, _guard) = capture_notify();
    let (conversation, envelope) = a.deliver_reply(false, reply).await;

    assert!(
        !envelope
            .assistant
            .blocks
            .iter()
            .any(|block| matches!(block, ReplyBlock::Actions { .. })),
        "no step reaches the room: {:?}",
        envelope.assistant.blocks
    );
    assert!(named(&events, "suggestion.shown").is_empty());
    let stored_reply = a.platform_row(&conversation).await;
    assert_eq!(
        stored_reply.content, text,
        "the wrap-up's words still go out"
    );
    assert_eq!(stored_reply.content_blocks, None);
}
