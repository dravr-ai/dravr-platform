// ABOUTME: The calibration and season walks' wrap-ups report what actually landed, read through the real renderer
// ABOUTME: Seeds onboarding facts in a real store and pins the time window, shared-kind crediting and named gaps
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

#![allow(clippy::unwrap_used)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_chat_pipeline::stages::completion;
use pierre_chat_pipeline::ChatPipelineContext;
use pierre_contremaitre::messaging_strings::{
    KEY_CALIBRATE_COMPLETE_HEADER, KEY_CALIBRATE_COMPLETE_MISSING, KEY_CALIBRATE_TOPIC_INJURY,
    KEY_CALIBRATE_TOPIC_RECOVERY, KEY_SEASON_COMPLETE_HEADER, KEY_SEASON_COMPLETE_MISSING_GOAL,
    KEY_SEASON_FOLLOWUP_LAY_OUT,
};
use pierre_core::models::{
    CalibrationTopic, ConversationRecord, Dossier, DossierFact, GuidedFlow, OnboardingState,
    Pillar, SeasonTopic, TenantId,
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
        let user_id = self.user_id.to_string();
        for (n, kind) in kinds.iter().enumerate() {
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
                    pillar: None,
                    predicate_code: PredicateCode::States,
                    object: &object,
                    confidence: 0.9,
                    source: FactSource::Onboarding,
                    valid_until: None,
                    source_msg_id: None,
                })
                .await
                .unwrap();
        }
    }

    async fn calibration_wrap_up(&self, started_at: String, dossier: &Dossier) -> String {
        let state = OnboardingState::start(started_at, GuidedFlow::Calibration);
        completion::render(
            &self.ctx,
            &conversation(self.user_id),
            &state,
            self.tenant_id,
            &self.user_id.to_string(),
            dossier,
            LOCALE,
        )
        .await
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

fn conversation(user_id: Uuid) -> ConversationRecord {
    let now = Utc::now().to_rfc3339();
    ConversationRecord {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        tenant_id: Uuid::nil().to_string(),
        title: "calibration".to_owned(),
        model: "test".to_owned(),
        agent_id: None,
        session_id: None,
        total_tokens: 0,
        created_at: now.clone(),
        updated_at: now,
        group_id: None,
        channel_type: "web".to_owned(),
        onboarding_state: None,
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
