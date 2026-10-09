// ABOUTME: The verdict conversation seeder leaves one reply whose chip opens a supported verdict backed by a DOI
// ABOUTME: The conversation, its two messages, the verdict on the reply, a rerun that seeds nothing twice, and no model

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_core::errors::ErrorCode;
use pierre_core::transport::TransportPolicy;
use pierre_memory::{ClaimStatus, VerdictLayer};
use pierre_seeders::verdict_conversation::{
    run, SeedArgs, CLAIM, CONVERSATION_TITLE, EVIDENCE_REF, QUESTION,
};

const ATHLETE: &str = "athlete@seed.test";

fn args() -> SeedArgs {
    SeedArgs {
        email: ATHLETE.to_owned(),
        model: Some("seed-model".to_owned()),
    }
}

#[tokio::test]
async fn the_seeder_leaves_one_reply_with_a_supported_verdict() {
    let res = create_test_server_resources().await.unwrap();
    let database = &res.agent.database;
    let (athlete, _, tenant) = create_test_user_with_plan(database, ATHLETE, "professional")
        .await
        .unwrap();
    let repos = &res.common.repos;

    run(args(), repos).await.unwrap();

    let page = repos
        .chat
        .list_conversations(
            &athlete.to_string(),
            tenant,
            50,
            0,
            TransportPolicy::AnyTransport,
        )
        .await
        .unwrap();
    let seeded: Vec<_> = page
        .items
        .iter()
        .filter(|c| c.title == CONVERSATION_TITLE)
        .collect();
    assert_eq!(seeded.len(), 1, "one conversation of the seeded title");
    let conversation = seeded[0];
    assert_eq!(conversation.model, "seed-model");

    let messages = repos
        .chat
        .get_messages(&conversation.id, &athlete.to_string(), tenant)
        .await
        .unwrap();
    let question = messages
        .iter()
        .find(|m| m.role == "user")
        .expect("the athlete's question");
    assert_eq!(question.content, QUESTION);
    let reply = messages
        .iter()
        .find(|m| m.role == "assistant")
        .expect("the agent's reply");
    assert!(
        reply.content.contains(CLAIM),
        "the reply states the claim the verdict judged, so the sheet can highlight it: {}",
        reply.content
    );

    // The verdict the reply's chip opens: on that reply, supported, from the
    // evidence layer, backed by the DOI the sheet links to.
    let verdicts = repos
        .claim_verdicts
        .list_verdicts_for_conversation(&conversation.id, tenant)
        .await
        .unwrap();
    assert_eq!(verdicts.len(), 1);
    let verdict = &verdicts[0];
    assert_eq!(verdict.message_id.as_deref(), Some(reply.id.as_str()));
    assert_eq!(verdict.user_id, athlete.to_string());
    assert_eq!(verdict.claim_text, CLAIM);
    assert_eq!(verdict.status, ClaimStatus::Supported);
    assert_eq!(verdict.layer_fired, VerdictLayer::Evidence);
    assert_eq!(verdict.evidence_refs.as_deref(), Some(EVIDENCE_REF));
    assert!(verdict.explanation.is_some(), "the sheet's findings line");
    assert_eq!(
        verdict.transport_policy,
        TransportPolicy::AnyTransport,
        "a verdict every surface may serve"
    );

    // A rerun finds the conversation and seeds nothing twice.
    run(args(), repos).await.unwrap();
    let rerun = repos
        .chat
        .list_conversations(
            &athlete.to_string(),
            tenant,
            50,
            0,
            TransportPolicy::AnyTransport,
        )
        .await
        .unwrap();
    assert_eq!(
        rerun
            .items
            .iter()
            .filter(|c| c.title == CONVERSATION_TITLE)
            .count(),
        1
    );
    assert_eq!(
        repos
            .claim_verdicts
            .list_verdicts_for_conversation(&conversation.id, tenant)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// With no model resolved the seeder refuses before writing anything, and
/// names both ways to give it one.
#[tokio::test]
async fn the_seeder_refuses_without_a_model_and_writes_nothing() {
    let res = create_test_server_resources().await.unwrap();
    let database = &res.agent.database;
    let (athlete, _, tenant) = create_test_user_with_plan(database, ATHLETE, "professional")
        .await
        .unwrap();
    let repos = &res.common.repos;

    let error = run(
        SeedArgs {
            model: None,
            ..args()
        },
        repos,
    )
    .await
    .expect_err("no model to create the conversation with");
    assert_eq!(error.code, ErrorCode::ConfigError, "{error}");
    assert!(error.message.contains("pass --model"), "{error}");
    let page = repos
        .chat
        .list_conversations(
            &athlete.to_string(),
            tenant,
            50,
            0,
            TransportPolicy::AnyTransport,
        )
        .await
        .unwrap();
    assert!(page.items.is_empty());
}

/// An athlete nobody seeded is a configuration error naming the email.
#[tokio::test]
async fn the_seeder_names_a_missing_athlete() {
    let res = create_test_server_resources().await.unwrap();
    let error = run(args(), &res.common.repos)
        .await
        .expect_err("no such athlete");
    assert_eq!(error.code, ErrorCode::ConfigError, "{error}");
    assert!(error.message.contains(ATHLETE), "{error}");
}
