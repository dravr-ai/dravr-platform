// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: GET /api/memory/facts names the coach behind a fact by title, never by its raw id alone
// ABOUTME: One lookup per distinct coach; a fact with no coach, or a coach that no longer resolves, carries no title

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::Value;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::routes::memory::MemoryRoutes;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};

#[tokio::test]
async fn facts_carry_the_coach_title_when_the_coach_resolves() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let repos = resources.coach.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("the test user has a tenant")
        .id;

    let coach = repos
        .coaches
        .create(
            user_id,
            tenant_id,
            &CreateCoachRequest {
                title: "Marathon Coach".to_owned(),
                description: Some("Long runs.".to_owned()),
                system_prompt: "You are a test coach.".to_owned(),
                category: CoachCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .expect("coach");
    let coach_id = coach.id.to_string();
    let user_s = user_id.to_string();

    for (coach, object) in [
        (Some(coach_id.as_str()), "a sub-4 marathon"),
        (None, "morning sessions"),
    ] {
        repos
            .memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id,
                user_id: &user_s,
                coach_id: coach,
                scope: MemoryScope::User,
                kind: FactKind::Goal,
                pillar: None,
                predicate_code: PredicateCode::WorkingToward,
                object,
                confidence: 0.9,
                source: FactSource::Conversation,
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
    assert_eq!(facts.len(), 2, "both facts are listed");

    let coached = facts
        .iter()
        .find(|f| f["object"] == "a sub-4 marathon")
        .expect("the coach-scoped fact");
    assert_eq!(coached["coach_id"], coach_id.as_str());
    assert_eq!(
        coached["coach_title"], "Marathon Coach",
        "the row names the coach by title"
    );

    let uncoached = facts
        .iter()
        .find(|f| f["object"] == "morning sessions")
        .expect("the user-wide fact");
    assert!(uncoached["coach_id"].is_null());
    assert!(
        uncoached["coach_title"].is_null(),
        "a fact with no coach carries no title"
    );
}
