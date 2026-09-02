// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: GET /api/me/parq serves the PAR-Q+ questions in the caller's stored locale, ids stable across languages
// ABOUTME: A French athlete reads the same instrument the messaging intake asks, in French; an English one in English

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_services::intake::INTAKE_TOPICS;
use pierre_services::parq::PARQ_QUESTION_IDS;
use serde_json::Value;

/// The `(id, text)` pairs the endpoint answers with, in order.
async fn fetch_questions(resources: &Arc<ServerContext>, token: &str) -> Vec<(String, String)> {
    let router = OnboardingRoutes::routes(Arc::clone(resources));
    let resp = AxumTestRequest::get("/api/me/parq")
        .header("Authorization", token)
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: Value = resp.json();
    body["questions"]
        .as_array()
        .expect("questions array")
        .iter()
        .map(|q| {
            (
                q["id"].as_str().expect("id").to_owned(),
                q["text"].as_str().expect("text").to_owned(),
            )
        })
        .collect()
}

/// The registry key the intake asks question `id` with.
fn key_for(id: &str) -> &'static str {
    INTAKE_TOPICS
        .iter()
        .find(|topic| topic.parq_id() == Some(id))
        .map(|topic| topic.string_key())
        .expect("every PAR-Q id has an intake topic")
}

#[tokio::test]
async fn questions_are_served_in_the_callers_locale_with_stable_ids() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let registry = &resources.mcp.messaging_strings_registry;
    let users = &resources.common.repos.users;

    users.update_locale(user_id, "fr").await.expect("set fr");
    let french = fetch_questions(&resources, &token).await;
    let ids: Vec<&str> = french.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        PARQ_QUESTION_IDS.to_vec(),
        "the REST screen asks the seven questions in the instrument's order"
    );
    let heart_fr = registry.get(key_for("heart_condition"), "fr");
    assert_eq!(
        french[0].1, heart_fr,
        "a French athlete reads the French registry line"
    );
    assert!(
        heart_fr.contains("cardiaque") || heart_fr.contains("cœur"),
        "the French line must be French, got {heart_fr}"
    );

    users.update_locale(user_id, "en").await.expect("set en");
    let english = fetch_questions(&resources, &token).await;
    assert_eq!(
        english[0].0, "heart_condition",
        "ids do not move with the language"
    );
    assert_eq!(english[0].1, registry.get(key_for("heart_condition"), "en"));
    assert_ne!(
        english[0].1, french[0].1,
        "the two locales must not serve the same text"
    );
    assert!(
        english[0].1.contains("heart condition"),
        "the English line must be English, got {}",
        english[0].1
    );
}
