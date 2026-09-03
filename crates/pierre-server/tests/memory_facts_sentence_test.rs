// ABOUTME: GET /api/memory/facts serves each fact as one sentence in the athlete's stored locale
// ABOUTME: A French athlete's goal reads in French on the wire, with no English glue and no raw code
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::Value;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::routes::memory::MemoryRoutes;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};

#[tokio::test]
async fn a_french_athlete_reads_her_facts_in_french() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let repos = resources.coach.database.repositories();
    repos
        .users
        .update_locale(user_id, "fr")
        .await
        .expect("locale stored");
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;
    let user_s = user_id.to_string();

    for (kind, code, object) in [
        (
            FactKind::Goal,
            PredicateCode::TrainingFor,
            "un ultra de 26 km au Mont Albert",
        ),
        (FactKind::Medical, PredicateCode::ParqYes, "heart_condition"),
        (
            FactKind::Other,
            PredicateCode::States,
            "je veux rester en forme pour mes enfants",
        ),
    ] {
        repos
            .memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id,
                user_id: &user_s,
                coach_id: None,
                scope: MemoryScope::User,
                kind,
                pillar: None,
                predicate_code: code,
                object,
                confidence: 1.0,
                source: FactSource::Onboarding,
                valid_until: None,
                source_msg_id: None,
            })
            .await
            .expect("fact");
    }

    let router = MemoryRoutes::routes(Arc::clone(&resources));
    let resp = AxumTestRequest::get("/api/memory/facts")
        .header("Authorization", &token)
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: Value = resp.json();
    let facts = body["facts"].as_array().expect("facts");
    assert_eq!(facts.len(), 3);

    let goal = facts
        .iter()
        .find(|f| f["predicate_code"] == "training_for")
        .expect("the goal row");
    assert_eq!(
        goal["sentence"],
        "Tu t'entraînes pour un ultra de 26 km au Mont Albert"
    );
    assert_eq!(goal["object"], "un ultra de 26 km au Mont Albert");
    assert!(
        goal.get("subject").is_none() && goal.get("predicate").is_none(),
        "the triple left the wire"
    );

    let parq = facts
        .iter()
        .find(|f| f["predicate_code"] == "parq_yes")
        .expect("the PAR-Q row");
    let sentence = parq["sentence"].as_str().unwrap();
    assert!(sentence.starts_with("Tu as répondu oui : "), "{sentence}");
    assert!(!sentence.contains("heart_condition"), "{sentence}");

    let words = facts
        .iter()
        .find(|f| f["predicate_code"] == "states")
        .expect("the states row");
    assert_eq!(
        words["sentence"],
        "je veux rester en forme pour mes enfants"
    );
}
