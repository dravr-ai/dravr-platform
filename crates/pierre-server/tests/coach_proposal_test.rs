// ABOUTME: Unit tests for the coach-proposal re-rank prompt builder and response parser
// ABOUTME: Covers JSON extraction tolerance, id validation, dedup, capping, and prompt contents
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for `pierre_services::coaches` re-ranking helpers (pure functions: no
//! LLM, no DB) plus the messaging coach-proposal idempotency flag round-trip
//! against a real `SQLite` repo.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashSet;

use pierre_core::models::TenantId;
use pierre_database::repositories::CreateChannelLinkParams;
use pierre_services::coaches::{
    build_rerank_user_prompt, parse_rerank_response, ProposalCandidate,
};

mod common;

fn candidate(id: &str, title: &str, blurb: &str) -> ProposalCandidate {
    ProposalCandidate {
        id: id.to_owned(),
        title: title.to_owned(),
        category: "training".to_owned(),
        tags: vec!["endurance".to_owned()],
        blurb: blurb.to_owned(),
        match_score: 0.8,
    }
}

fn ids(values: &[&str]) -> HashSet<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn parse_clean_json_array_preserves_order_and_reasons() {
    let valid = ids(&["a", "b", "c"]);
    let content = r#"[
        {"id": "b", "reason": "Fits your cycling base."},
        {"id": "a", "reason": "Good for your running volume."}
    ]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 2);
    assert_eq!(selections[0].id, "b");
    assert_eq!(selections[0].reason, "Fits your cycling base.");
    assert_eq!(selections[1].id, "a");
}

#[test]
fn parse_tolerates_markdown_fence_and_prose() {
    let valid = ids(&["x"]);
    let content = "Here are my picks:\n```json\n[{\"id\": \"x\", \"reason\": \"Best match.\"}]\n```\nHope that helps!";

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].id, "x");
    assert_eq!(selections[0].reason, "Best match.");
}

#[test]
fn parse_drops_ids_outside_candidate_set() {
    let valid = ids(&["a"]);
    let content = r#"[{"id": "a", "reason": "real"}, {"id": "ghost", "reason": "hallucinated"}]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].id, "a");
}

#[test]
fn parse_dedupes_keeping_first_occurrence() {
    let valid = ids(&["a"]);
    let content = r#"[{"id": "a", "reason": "first"}, {"id": "a", "reason": "second"}]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].reason, "first");
}

#[test]
fn parse_caps_at_max() {
    let valid = ids(&["a", "b", "c", "d"]);
    let content = r#"[
        {"id": "a", "reason": "1"},
        {"id": "b", "reason": "2"},
        {"id": "c", "reason": "3"},
        {"id": "d", "reason": "4"}
    ]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 3);
    assert_eq!(selections[2].id, "c");
}

#[test]
fn parse_returns_empty_on_garbage() {
    let valid = ids(&["a"]);

    assert!(parse_rerank_response("not json at all", &valid, 3).is_empty());
    assert!(parse_rerank_response("{\"id\": \"a\"}", &valid, 3).is_empty());
    assert!(parse_rerank_response("", &valid, 3).is_empty());
}

#[test]
fn parse_trims_reason_whitespace() {
    let valid = ids(&["a"]);
    let content = r#"[{"id": "a", "reason": "  padded reason  "}]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections[0].reason, "padded reason");
}

#[test]
fn build_prompt_includes_profile_candidates_and_cap() {
    let candidates = vec![
        candidate(
            "coach-1",
            "Marathon Coach",
            "Use for marathon build phases.",
        ),
        candidate(
            "coach-2",
            "Recovery Coach",
            "Use when overtraining is a risk.",
        ),
    ];

    let prompt = build_rerank_user_prompt("Trains primarily run; 40 activities.", &candidates, 3);

    assert!(prompt.contains("Trains primarily run"));
    assert!(prompt.contains("coach-1"));
    assert!(prompt.contains("Marathon Coach"));
    assert!(prompt.contains("Use for marathon build phases."));
    assert!(prompt.contains("coach-2"));
    assert!(prompt.contains("at most 3"));
}

/// The messaging auto-send idempotency flag must round-trip on a real link:
/// unset → marked → set, idempotent re-marks, and scoped to the exact
/// `(tenant, channel, channel_user_id)` identity.
#[tokio::test]
async fn coach_proposal_flag_round_trips() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());

    let channel = "telegram";
    let channel_user_id = "tg-12345";

    repos
        .messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: "link-1",
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: channel,
            channel_user_id,
            display_name: None,
        })
        .await
        .unwrap();

    // Fresh link: proposal not yet sent.
    assert!(
        !repos
            .messaging
            .coach_proposal_sent(tenant_id, channel, channel_user_id)
            .await
            .unwrap(),
        "a fresh link must report the proposal as not yet sent"
    );

    repos
        .messaging
        .mark_coach_proposal_sent(tenant_id, channel, channel_user_id)
        .await
        .unwrap();

    // Now flagged.
    assert!(
        repos
            .messaging
            .coach_proposal_sent(tenant_id, channel, channel_user_id)
            .await
            .unwrap(),
        "after marking, the proposal must report as sent"
    );

    // Re-marking is idempotent (no error, stays set).
    repos
        .messaging
        .mark_coach_proposal_sent(tenant_id, channel, channel_user_id)
        .await
        .unwrap();
    assert!(repos
        .messaging
        .coach_proposal_sent(tenant_id, channel, channel_user_id)
        .await
        .unwrap());

    // A different channel identity on the same tenant is independent.
    assert!(
        !repos
            .messaging
            .coach_proposal_sent(tenant_id, channel, "tg-other")
            .await
            .unwrap(),
        "the flag must be scoped to the exact channel_user_id"
    );
}
