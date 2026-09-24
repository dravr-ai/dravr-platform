// ABOUTME: Unit tests for the PAR-Q question set — seven unique ids and the lookup
// ABOUTME: Pins that every question id has a localized intake topic, in the same order

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_services::intake::{IntakeTopic, INTAKE_TOPICS};
use pierre_services::parq::{is_parq_question, PARQ_QUESTION_IDS};

#[test]
fn seven_questions_with_unique_ids() {
    assert_eq!(PARQ_QUESTION_IDS.len(), 7);
    let mut ids: Vec<&str> = PARQ_QUESTION_IDS.to_vec();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 7, "question ids must be unique");
}

#[test]
fn question_lookup() {
    assert!(is_parq_question("heart_condition"));
    assert!(!is_parq_question("not_a_question"));
}

#[test]
fn every_id_has_a_localized_intake_topic_in_the_same_order() {
    let from_intake: Vec<&str> = INTAKE_TOPICS
        .iter()
        .filter_map(|topic| topic.parq_id())
        .collect();
    assert_eq!(
        from_intake,
        PARQ_QUESTION_IDS.to_vec(),
        "the intake walk and the REST screen must ask the same questions in the same order"
    );
    assert_eq!(
        IntakeTopic::HeartCondition.parq_id(),
        Some("heart_condition")
    );
}
