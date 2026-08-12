// ABOUTME: Compare-and-set on chat_conversations.onboarding_state against a real database
// ABOUTME: A turn's end-of-turn write-back must not overwrite a flow the athlete started mid-turn
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The guided-flow column has two writers that do not share a lock.
//!
//! An LLM turn reads `onboarding_state` when it starts and writes it back tens
//! of seconds later — after post-processing — through
//! `stages::onboarding::record_delivered_probe` or `clear_completed_marker`.
//! Slash commands are handled synchronously in the messaging webhook path,
//! outside the `CONVERSATION_DISPATCH_LOCKS` the background dispatch takes. So
//! an athlete who answers a pillars probe and then types `/calibrate` while the
//! coach is still thinking has a fresh interview written under a running turn,
//! and a blind write-back replaces it with the snapshot that turn loaded: the
//! calibration reverts silently and its load snapshot is lost.
//!
//! These tests pin the repository primitive that makes those writes conditional.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

use pierre_core::models::{
    CalibrationTopic, CoverageTarget, GuidedFlow, LoadSnapshot, OnboardingState, Pillar, TenantId,
};
use pierre_database::backends::factory::Database;

/// A pillars walk with one probe already delivered — the state a mid-flow turn
/// loads at its start.
fn pillars_state() -> String {
    OnboardingState::start("2026-07-30T09:00:00Z".to_owned(), GuidedFlow::Pillars)
        .with_delivered_probe(CoverageTarget::Pillar(Pillar::Fuelling).slug())
        .to_column()
        .unwrap()
}

/// The same walk with a second probe appended — what the running turn intends
/// to write back when its question reaches the athlete.
fn pillars_state_plus_probe() -> String {
    OnboardingState::start("2026-07-30T09:00:00Z".to_owned(), GuidedFlow::Pillars)
        .with_delivered_probe(CoverageTarget::Pillar(Pillar::Fuelling).slug())
        .with_delivered_probe(CoverageTarget::Pillar(Pillar::SleepAndRecovery).slug())
        .to_column()
        .unwrap()
}

/// A calibration interview started by `/calibrate` mid-turn, carrying the load
/// snapshot that command computed. Losing this row loses that snapshot.
fn calibration_state() -> String {
    OnboardingState::start("2026-07-30T09:00:31Z".to_owned(), GuidedFlow::Calibration)
        .with_snapshot(Some(LoadSnapshot {
            weekly_hours: 7.5,
            sessions_per_week: 4.0,
            longest_session_min: 195,
            weeks: 6,
        }))
        .to_column()
        .unwrap()
}

/// A finished interview retired to its timestamped marker.
fn completed_marker() -> String {
    OnboardingState::start("2026-07-30T09:00:00Z".to_owned(), GuidedFlow::Calibration)
        .with_delivered_probe(CalibrationTopic::RecoverySpeed.slug())
        .completed("2026-07-30T09:10:00Z".to_owned())
        .to_column()
        .unwrap()
}

async fn conversation(db: &Database, user_id: &str, tenant_id: TenantId) -> String {
    db.repositories()
        .chat
        .create_conversation(user_id, tenant_id, "guided", "test-model", None, None)
        .await
        .unwrap()
        .id
}

async fn stored(
    db: &Database,
    conv_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> Option<String> {
    db.repositories()
        .chat
        .get_conversation(conv_id, user_id, tenant_id)
        .await
        .unwrap()
        .unwrap()
        .onboarding_state
}

#[tokio::test]
async fn a_write_back_matching_the_turn_start_state_lands() {
    let db = create_test_db().await;
    let (user, tenant) = seed_user(&db).await;
    let user_id = user.to_string();
    let conv_id = conversation(&db, &user_id, tenant).await;
    let repos = db.repositories();

    let at_turn_start = pillars_state();
    repos
        .chat
        .set_conversation_onboarding_state(&conv_id, Some(&at_turn_start), tenant)
        .await
        .unwrap();

    let wrote = repos
        .chat
        .compare_and_set_conversation_onboarding_state(
            &conv_id,
            Some(&at_turn_start),
            Some(&pillars_state_plus_probe()),
            tenant,
        )
        .await
        .unwrap();

    assert!(wrote, "an uncontended turn must still record its probe");
    let column = stored(&db, &conv_id, &user_id, tenant).await.unwrap();
    let state = OnboardingState::from_column(Some(&column)).expect("the walk is still active");
    assert_eq!(
        state.probed.len(),
        2,
        "the second probe must be in the ledger: {column}"
    );
    assert_eq!(state.flow, GuidedFlow::Pillars);
}

#[tokio::test]
async fn a_stale_write_back_leaves_the_flow_the_athlete_just_started_alone() {
    // The incident shape: the turn loads a pillars walk, `/calibrate` lands
    // mid-turn, the turn ends and tries to append its probe to the snapshot it
    // read. That write must not resurrect the walk.
    let db = create_test_db().await;
    let (user, tenant) = seed_user(&db).await;
    let user_id = user.to_string();
    let conv_id = conversation(&db, &user_id, tenant).await;
    let repos = db.repositories();

    let at_turn_start = pillars_state();
    repos
        .chat
        .set_conversation_onboarding_state(&conv_id, Some(&at_turn_start), tenant)
        .await
        .unwrap();

    // `/calibrate` runs while the LLM turn is still in flight.
    let started_mid_turn = calibration_state();
    repos
        .chat
        .set_conversation_onboarding_state(&conv_id, Some(&started_mid_turn), tenant)
        .await
        .unwrap();

    let wrote = repos
        .chat
        .compare_and_set_conversation_onboarding_state(
            &conv_id,
            Some(&at_turn_start),
            Some(&pillars_state_plus_probe()),
            tenant,
        )
        .await
        .unwrap();

    assert!(
        !wrote,
        "a newer flow owns the column; the append must no-op"
    );
    let column = stored(&db, &conv_id, &user_id, tenant).await.unwrap();
    let state = OnboardingState::from_column(Some(&column)).expect("the interview is active");
    assert_eq!(
        state.flow,
        GuidedFlow::Calibration,
        "the calibration the athlete started was overwritten: {column}"
    );
    assert_eq!(
        state.snapshot.map(|s| s.longest_session_min),
        Some(195),
        "the load snapshot `/calibrate` computed must survive: {column}"
    );
    assert!(
        state.probed.is_empty(),
        "the stale walk's ledger must not bleed into the new interview"
    );
}

#[tokio::test]
async fn clearing_a_completed_marker_is_skipped_when_a_new_flow_owns_the_row() {
    // `clear_completed_marker` runs at the end of the turn that consumed the
    // release directive. Between the read and that clear, an athlete can start
    // a fresh interview — deleting it would end the interview on its first turn.
    let db = create_test_db().await;
    let (user, tenant) = seed_user(&db).await;
    let user_id = user.to_string();
    let conv_id = conversation(&db, &user_id, tenant).await;
    let repos = db.repositories();

    let marker = completed_marker();
    repos
        .chat
        .set_conversation_onboarding_state(&conv_id, Some(&marker), tenant)
        .await
        .unwrap();
    repos
        .chat
        .set_conversation_onboarding_state(&conv_id, Some(&calibration_state()), tenant)
        .await
        .unwrap();

    let cleared = repos
        .chat
        .compare_and_set_conversation_onboarding_state(&conv_id, Some(&marker), None, tenant)
        .await
        .unwrap();

    assert!(!cleared, "the clear must no-op against a newer flow");
    let column = stored(&db, &conv_id, &user_id, tenant).await;
    let state = OnboardingState::from_column(column.as_deref())
        .expect("the interview started mid-turn is still active");
    assert_eq!(state.flow, GuidedFlow::Calibration);
}

#[tokio::test]
async fn a_marker_still_in_place_is_cleared() {
    let db = create_test_db().await;
    let (user, tenant) = seed_user(&db).await;
    let user_id = user.to_string();
    let conv_id = conversation(&db, &user_id, tenant).await;
    let repos = db.repositories();

    let marker = completed_marker();
    repos
        .chat
        .set_conversation_onboarding_state(&conv_id, Some(&marker), tenant)
        .await
        .unwrap();

    let cleared = repos
        .chat
        .compare_and_set_conversation_onboarding_state(&conv_id, Some(&marker), None, tenant)
        .await
        .unwrap();

    assert!(cleared, "an uncontended clear must land");
    assert_eq!(
        stored(&db, &conv_id, &user_id, tenant).await,
        None,
        "the marker must be gone so the release directive fires exactly once"
    );
}

#[tokio::test]
async fn an_absent_state_matches_only_an_absent_expected() {
    // NULL-safe equality both ways: "the conversation carried no state" is a
    // value like any other, and it must not match stored JSON.
    let db = create_test_db().await;
    let (user, tenant) = seed_user(&db).await;
    let user_id = user.to_string();
    let conv_id = conversation(&db, &user_id, tenant).await;
    let repos = db.repositories();

    assert_eq!(
        stored(&db, &conv_id, &user_id, tenant).await,
        None,
        "a fresh conversation carries no guided state"
    );

    let wrong = repos
        .chat
        .compare_and_set_conversation_onboarding_state(
            &conv_id,
            Some(&pillars_state()),
            Some(&calibration_state()),
            tenant,
        )
        .await
        .unwrap();
    assert!(
        !wrong,
        "an empty column must not satisfy an expectation of stored JSON"
    );
    assert_eq!(stored(&db, &conv_id, &user_id, tenant).await, None);

    let right = repos
        .chat
        .compare_and_set_conversation_onboarding_state(
            &conv_id,
            None,
            Some(&calibration_state()),
            tenant,
        )
        .await
        .unwrap();
    assert!(right, "an absent expectation must match an absent column");
    let column = stored(&db, &conv_id, &user_id, tenant).await.unwrap();
    assert_eq!(
        OnboardingState::from_column(Some(&column)).map(|s| s.flow),
        Some(GuidedFlow::Calibration)
    );
}

#[tokio::test]
async fn the_write_is_tenant_scoped() {
    let db = create_test_db().await;
    let (user, tenant) = seed_user(&db).await;
    let user_id = user.to_string();
    let conv_id = conversation(&db, &user_id, tenant).await;
    let repos = db.repositories();

    let at_turn_start = pillars_state();
    repos
        .chat
        .set_conversation_onboarding_state(&conv_id, Some(&at_turn_start), tenant)
        .await
        .unwrap();

    let other_tenant = TenantId::generate();
    let wrote = repos
        .chat
        .compare_and_set_conversation_onboarding_state(
            &conv_id,
            Some(&at_turn_start),
            Some(&calibration_state()),
            other_tenant,
        )
        .await
        .unwrap();

    assert!(!wrote, "another tenant must not reach this conversation");
    let column = stored(&db, &conv_id, &user_id, tenant).await.unwrap();
    assert_eq!(
        OnboardingState::from_column(Some(&column)).map(|s| s.flow),
        Some(GuidedFlow::Pillars),
        "the owning tenant's state must be untouched"
    );
}
