// ABOUTME: The first turn after a guided interview must be told the interview's rules are lifted
// ABOUTME: Covers the retired-marker lifecycle (OnboardingState) and the release directive's wording
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! 2026-07-28, conversation `043fdd88`: a calibration interview ran for eight
//! turns, each one carrying the guided directive — the most forcefully worded
//! block in the prompt, closing with "do not build, propose, or save a training
//! plan on this turn". `guided interview completed` landed at 21:24:26. At
//! 21:25:14 the coach called `get_training_history` and `export_dossier`, then
//! told the athlete it could not save his plan "cette fois-ci". It never called
//! `save_training_plan`, which was in that turn's catalogue: it reported a tool
//! failure that had not happened.
//!
//! Dropping a prohibition does not retract it from the transcript it shaped, so
//! the turn after a flow ends carries a directive that retracts it explicitly.
//! These tests pin the marker lifecycle that makes that turn identifiable and
//! the wording that makes the retraction land.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, Utc};
use pierre_chat_pipeline::stages::onboarding::release_directive;
use pierre_core::models::{GuidedFlow, OnboardingState, COMPLETION_RELEASE_WINDOW_MINUTES};

fn active_column(flow: GuidedFlow) -> String {
    OnboardingState::start(Utc::now().to_rfc3339(), flow)
        .to_column()
        .expect("serialize active state")
}

fn completed_column(flow: GuidedFlow, minutes_ago: i64) -> String {
    let at = Utc::now() - Duration::minutes(minutes_ago);
    OnboardingState::start(Utc::now().to_rfc3339(), flow)
        .completed(at.to_rfc3339())
        .to_column()
        .expect("serialize completed state")
}

#[test]
fn a_running_interview_is_not_a_completed_one() {
    // While the flow owns the turn the guided directive is what applies; the
    // release directive must not fire alongside it and contradict it.
    for flow in [GuidedFlow::Pillars, GuidedFlow::Calibration] {
        let column = active_column(flow);
        assert!(
            OnboardingState::from_column(Some(&column)).is_some(),
            "{flow:?}: an active flow must still resolve as active"
        );
        assert!(
            !OnboardingState::just_completed(Some(&column), Utc::now()),
            "{flow:?}: an active flow has not just completed"
        );
    }
}

#[test]
fn a_freshly_retired_interview_is_completed_but_no_longer_active() {
    // The incident's gap was 48 seconds. One minute stands in for it.
    let column = completed_column(GuidedFlow::Calibration, 1);
    assert!(
        OnboardingState::from_column(Some(&column)).is_none(),
        "a retired marker must not put the turn back into guided mode"
    );
    assert!(
        OnboardingState::just_completed(Some(&column), Utc::now()),
        "the turn right after the wrap-up must see the interview as just completed"
    );
}

#[test]
fn a_stale_marker_stops_claiming_the_interview_just_ended() {
    // Clearing the marker is best-effort, like every other write on this path.
    // The window is what stops a failed clear from telling a coach days later
    // that the interview "just" finished.
    let column = completed_column(
        GuidedFlow::Calibration,
        COMPLETION_RELEASE_WINDOW_MINUTES + 1,
    );
    assert!(
        !OnboardingState::just_completed(Some(&column), Utc::now()),
        "a completion older than the release window must not fire the directive"
    );
}

#[test]
fn an_absent_or_unreadable_marker_is_not_a_completion() {
    assert!(!OnboardingState::just_completed(None, Utc::now()));
    assert!(!OnboardingState::just_completed(Some("{"), Utc::now()));
    // Rows written before `completed_at` existed parse fine and carry no
    // stamp; they must not be read as a completion that just happened.
    let legacy = r#"{"active":false,"started_at":"","probed":[],"flow":"calibration"}"#;
    assert!(
        !OnboardingState::just_completed(Some(legacy), Utc::now()),
        "an un-stamped legacy row must not fire the directive"
    );
    // ...and a row from before the field existed must still parse at all.
    let legacy_active = r#"{"active":true,"started_at":"2026-07-28T21:07:00Z"}"#;
    assert!(
        OnboardingState::from_column(Some(legacy_active)).is_some(),
        "adding completed_at must not break rows written without it"
    );
}

#[test]
fn the_release_directive_retracts_the_interviews_no_saving_rule() {
    let text = release_directive();

    // The guided directive is stated as an override of everything else, so the
    // release has to be stated as an override of *it*. "You may now save" would
    // simply be one more instruction competing with eight turns of "do not
    // save".
    assert!(
        text.contains("overrides the interview rules"),
        "the release must claim precedence over the block it retracts: {text}"
    );
    assert!(
        text.contains("no longer applies"),
        "the release must retract the earlier rule, not merely add permission: {text}"
    );

    // The specific prohibition being retracted, in the words the guided
    // directive used — if that wording is reworded, this pairing should be
    // re-read rather than silently drift apart.
    assert!(
        text.contains("build, propose or save a training plan"),
        "the release must name the prohibition it lifts: {text}"
    );
    assert!(
        text.contains("save_training_plan"),
        "the withheld tool must be named as callable again: {text}"
    );

    // The fabricated failure is its own defect: the coach announced a save
    // failure without ever calling the tool.
    assert!(
        text.contains("unless you called the tool on this turn"),
        "the release must forbid reporting a failure that was never attempted: {text}"
    );
}

#[test]
fn the_release_directive_does_not_instruct_the_coach_to_save() {
    // Retracting a prohibition is not the same as ordering the write. A plan
    // save is the athlete's decision, and a directive that pushed for one would
    // reintroduce the 2026-07-24 derail from the other direction — an unasked-for
    // plan, this time on the turn right after an interview.
    let text = release_directive().to_lowercase();
    for pushy in ["you must save", "save the plan now", "immediately save"] {
        assert!(
            !text.contains(pushy),
            "the release lifts a restriction; it must not prescribe the write: {pushy}"
        );
    }
}
