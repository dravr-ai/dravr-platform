// ABOUTME: External tests for the rhetoric-vs-propositional classifier (rhetoric_detector.rs)
// ABOUTME: Verifies motivational/greeting/question/empty text vs factual claims
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
use pierre_evals::rhetoric_detector::{classify, RhetoricVerdict};

#[test]
fn motivational_is_rhetorical() {
    assert_eq!(classify("You're crushing it!"), RhetoricVerdict::Rhetorical);
    assert_eq!(
        classify("Great job on that workout."),
        RhetoricVerdict::Rhetorical
    );
    assert_eq!(classify("Keep it up."), RhetoricVerdict::Rhetorical);
}

#[test]
fn greetings_are_rhetorical() {
    assert_eq!(classify("Hi there!"), RhetoricVerdict::Rhetorical);
    assert_eq!(
        classify("Good morning, champ."),
        RhetoricVerdict::Rhetorical
    );
}

#[test]
fn questions_are_rhetorical() {
    assert_eq!(
        classify("How do you feel today?"),
        RhetoricVerdict::Rhetorical
    );
    assert_eq!(
        classify("Are you ready for tomorrow?"),
        RhetoricVerdict::Rhetorical
    );
}

#[test]
fn factual_claim_is_propositional() {
    assert_eq!(
        classify("Your VO2max is 58 ml/kg/min."),
        RhetoricVerdict::Propositional
    );
    assert_eq!(
        classify("Creatine at 5g per day increases power output."),
        RhetoricVerdict::Propositional
    );
}

#[test]
fn empty_is_rhetorical() {
    assert_eq!(classify(""), RhetoricVerdict::Rhetorical);
    assert_eq!(classify("   "), RhetoricVerdict::Rhetorical);
}

#[test]
fn punctuation_only_is_rhetorical() {
    assert_eq!(classify("..."), RhetoricVerdict::Rhetorical);
}
